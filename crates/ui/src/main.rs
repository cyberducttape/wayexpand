use anyhow::{Context, Result};
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{self, ClearType},
};
use std::{
    env,
    io::{self, Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
    sync::mpsc::{self, Receiver, SyncSender},
    time::Duration,
    time::Instant,
};
use wayexpand_core::{
    default_config_path, Config, ConfigRevision, DaemonStatus, ExpansionEngine, InputEvent,
};

struct App {
    path: PathBuf,
    config: Config,
    config_revision: ConfigRevision,
    selected: usize,
    query: String,
    searching: bool,
    message: String,
    paused: bool,
    prompt: Option<Prompt>,
    input: String,
    confirm_delete: Option<usize>,
    undo: Option<Config>,
    external_edit: bool,
    last_status_poll: Instant,
    status_requests: Option<SyncSender<()>>,
    status_results: Option<Receiver<Option<bool>>>,
    status_in_flight: bool,
    /// Cached visible_indices result and the query that produced it
    visible_cache: Option<(String, Vec<usize>)>,
    /// Cached preview result: (selected_index, trigger, result)
    preview_cache: Option<(usize, String, String)>,
    preview_app: String,
}

enum Prompt {
    NewTrigger,
    NewReplacement { trigger: String },
    EditReplacement { index: usize, trigger: String },
    EditDescription { index: usize, trigger: String },
    EditTags { index: usize, trigger: String },
    PreviewApp,
}

impl App {
    fn load(path: PathBuf) -> Result<Self> {
        let loaded = Config::ensure_user_config(&path)
            .map_err(|error| anyhow::anyhow!("configuration invalid: {}", error.safe_summary()))?;
        Ok(Self {
            path,
            config: loaded.config,
            config_revision: loaded.revision,
            selected: 0,
            query: String::new(),
            searching: false,
            message: "Ready".into(),
            paused: false,
            prompt: None,
            input: String::new(),
            confirm_delete: None,
            undo: None,
            external_edit: false,
            last_status_poll: Instant::now(),
            status_requests: None,
            status_results: None,
            status_in_flight: false,
            visible_cache: None,
            preview_cache: None,
            preview_app: String::new(),
        })
    }

    fn save_candidate(&mut self, candidate: &Config) -> Result<(), wayexpand_core::ConfigError> {
        let revision =
            candidate.save_atomic_if_revision_matches(&self.path, &self.config_revision)?;
        self.config_revision = revision;
        Ok(())
    }

    fn save_current(&mut self) -> Result<(), wayexpand_core::ConfigError> {
        let candidate = self.config.clone();
        self.save_candidate(&candidate)
    }

    fn visible_indices(&mut self) -> &[usize] {
        if self
            .visible_cache
            .as_ref()
            .is_some_and(|(cached_query, _)| cached_query == &self.query)
        {
            return self
                .visible_cache
                .as_ref()
                .map(|(_, indices)| indices.as_slice())
                .unwrap_or(&[]);
        }
        let query = self.query.to_lowercase();
        let indices: Vec<usize> = self
            .config
            .expansion
            .iter()
            .enumerate()
            .filter(|(_, expansion)| {
                query.is_empty()
                    || format!(
                        "{} {} {}",
                        expansion.trigger,
                        expansion.description,
                        expansion.tags.join(" ")
                    )
                    .to_lowercase()
                    .contains(&query)
            })
            .map(|(index, _)| index)
            .collect();
        self.visible_cache = Some((self.query.clone(), indices));
        self.visible_cache
            .as_ref()
            .map(|(_, indices)| indices.as_slice())
            .unwrap_or(&[])
    }

    fn selected_index(&mut self) -> Option<usize> {
        let selected = self.selected;
        self.visible_indices().get(selected).copied()
    }

    fn toggle_selected(&mut self) {
        let Some(index) = self.selected_index() else {
            self.message = "No matching snippet".into();
            return;
        };
        let previous = self.config.clone();
        self.config.expansion[index].enabled = !self.config.expansion[index].enabled;
        match self.save_current() {
            Ok(()) => {
                self.message = format!(
                    "{} {}",
                    if self.config.expansion[index].enabled {
                        "Enabled"
                    } else {
                        "Disabled"
                    },
                    self.config.expansion[index].trigger
                );
                self.undo = Some(previous);
                self.visible_cache = None;
                self.preview_cache = None;
            }
            Err(error) => {
                self.config.expansion[index].enabled = !self.config.expansion[index].enabled;
                self.message = format!("Save failed: {}", error.safe_summary());
            }
        }
    }

    fn toggle_match_mode(&mut self) {
        let Some(index) = self.selected_index() else {
            self.message = "No matching snippet".into();
            return;
        };
        let previous = self.config.clone();
        let mode = &mut self.config.expansion[index].match_mode;
        *mode = match *mode {
            wayexpand_core::MatchMode::Immediate => wayexpand_core::MatchMode::WordBoundary,
            wayexpand_core::MatchMode::WordBoundary => wayexpand_core::MatchMode::Immediate,
        };
        let label = match self.config.expansion[index].match_mode {
            wayexpand_core::MatchMode::Immediate => "Immediate",
            wayexpand_core::MatchMode::WordBoundary => "Word-boundary",
        };
        match self.save_current() {
            Ok(()) => {
                self.message = format!(
                    "{label} matching enabled for {}",
                    self.config.expansion[index].trigger
                );
                self.undo = Some(previous);
                self.visible_cache = None;
                self.preview_cache = None;
            }
            Err(error) => {
                self.config.expansion[index].match_mode = previous.expansion[index].match_mode;
                self.message = format!("Save failed: {}", error.safe_summary());
            }
        }
    }

    fn reload(&mut self) {
        match Config::load_versioned(&self.path) {
            Ok(loaded) => {
                self.config = loaded.config;
                self.config_revision = loaded.revision;
                self.undo = None;
                // Config changed: both caches are now stale regardless of
                // whether the query string changed.
                self.visible_cache = None;
                self.preview_cache = None;
                self.selected = self
                    .selected
                    .min(self.visible_indices().len().saturating_sub(1));
                self.message = "Configuration reloaded".into();
            }
            Err(error) => self.message = format!("Reload failed: {}", error.safe_summary()),
        }
    }

    fn begin_new(&mut self) {
        self.prompt = Some(Prompt::NewTrigger);
        self.input.clear();
        self.message = "Type a trigger and press Enter".into();
    }

    fn begin_edit(&mut self) {
        let Some(index) = self.selected_index() else {
            self.message = "No snippet selected".into();
            return;
        };
        self.input = self.config.expansion[index].replacement.clone();
        self.prompt = Some(Prompt::EditReplacement {
            index,
            trigger: self.config.expansion[index].trigger.clone(),
        });
        self.message = "Edit replacement and press Enter".into();
    }

    fn begin_edit_description(&mut self) {
        let Some(index) = self.selected_index() else {
            self.message = "No snippet selected".into();
            return;
        };
        self.input = self.config.expansion[index].description.clone();
        self.prompt = Some(Prompt::EditDescription {
            index,
            trigger: self.config.expansion[index].trigger.clone(),
        });
        self.message = "Edit description and press Enter".into();
    }

    fn begin_edit_tags(&mut self) {
        let Some(index) = self.selected_index() else {
            self.message = "No snippet selected".into();
            return;
        };
        self.input = encode_tags(&self.config.expansion[index].tags);
        self.prompt = Some(Prompt::EditTags {
            index,
            trigger: self.config.expansion[index].trigger.clone(),
        });
        self.message = "Enter comma-separated tags and press Enter (use a JSON array such as [\"customer, west\"] for tags containing commas)".into();
    }

    fn request_delete(&mut self) {
        let Some(index) = self.selected_index() else {
            self.message = "No snippet selected".into();
            return;
        };
        self.confirm_delete = Some(index);
        self.message = "Press d again to confirm deletion, or Esc to cancel".into();
    }

    fn begin_preview_app(&mut self) {
        self.input = self.preview_app.clone();
        self.prompt = Some(Prompt::PreviewApp);
        self.message = "Enter an app id for preview and press Enter (empty clears it)".into();
    }

    fn delete_confirmed(&mut self, index: usize) {
        let previous = self.config.clone();
        let trigger = self.config.expansion[index].trigger.clone();
        self.config.expansion.remove(index);
        self.visible_cache = None;
        self.preview_cache = None;
        self.selected = self
            .selected
            .min(self.visible_indices().len().saturating_sub(1));
        if let Err(error) = self.save_current() {
            self.config = previous;
            self.message = format!("Delete failed: {}", error.safe_summary());
        } else {
            self.message = format!("Deleted {trigger}");
            self.undo = Some(previous);
        }
        self.confirm_delete = None;
    }

    fn undo_last(&mut self) {
        let Some(previous) = self.undo.take() else {
            self.message = "Nothing to undo".into();
            return;
        };
        let current = std::mem::replace(&mut self.config, previous);
        self.visible_cache = None;
        self.preview_cache = None;
        if let Err(error) = self.save_current() {
            self.config = current;
            self.message = format!("Undo failed: {}", error.safe_summary());
        } else {
            self.selected = self
                .selected
                .min(self.visible_indices().len().saturating_sub(1));
            self.message = "Undid the last saved change".into();
        }
    }

    fn submit_prompt(&mut self) {
        let prompt = self.prompt.take();
        let input = std::mem::take(&mut self.input);
        match prompt {
            Some(Prompt::NewTrigger) if !input.trim().is_empty() => {
                self.prompt = Some(Prompt::NewReplacement { trigger: input });
                self.message = "Type replacement text and press Enter".into();
            }
            Some(Prompt::NewTrigger) => self.message = "Trigger cannot be empty".into(),
            Some(Prompt::NewReplacement { trigger }) => {
                let previous = self.config.clone();
                self.config.expansion.push(wayexpand_core::ExpansionConfig {
                    id: wayexpand_core::ExpansionConfig::new_id(),
                    trigger,
                    replacement: input,
                    description: String::new(),
                    tags: Vec::new(),
                    category: String::new(),
                    app_filter: Vec::new(),
                    match_mode: wayexpand_core::MatchMode::Immediate,
                    command: None,
                    enabled: true,
                    propagate_case: false,
                    aliases: Vec::new(),
                });
                if let Err(error) = self.save_current() {
                    self.config = previous;
                    self.message = format!("Create failed: {}", error.safe_summary());
                } else {
                    self.visible_cache = None;
                    self.preview_cache = None;
                    self.selected = self.visible_indices().len().saturating_sub(1);
                    self.message = "Snippet created".into();
                    self.undo = Some(previous);
                }
            }
            Some(Prompt::EditReplacement { index, trigger }) => {
                let previous = self.config.clone();
                self.config.expansion[index].replacement = input;
                if let Err(error) = self.save_current() {
                    self.config = previous;
                    self.message = format!("Edit failed: {}", error.safe_summary());
                } else {
                    self.message = format!("Updated {trigger}");
                    self.undo = Some(previous);
                    self.visible_cache = None;
                    self.preview_cache = None;
                }
            }
            Some(Prompt::EditDescription { index, trigger }) => {
                let previous = self.config.clone();
                self.config.expansion[index].description = input;
                if let Err(error) = self.save_current() {
                    self.config = previous;
                    self.message = format!("Description edit failed: {}", error.safe_summary());
                } else {
                    self.message = format!("Updated description for {trigger}");
                    self.undo = Some(previous);
                    self.visible_cache = None;
                    self.preview_cache = None;
                }
            }
            Some(Prompt::EditTags { index, trigger }) => {
                let tags = match decode_tags(&input) {
                    Ok(tags) => tags,
                    Err(error) => {
                        self.message = format!("Tags JSON array is invalid: {error}");
                        self.input = input;
                        self.prompt = Some(Prompt::EditTags { index, trigger });
                        return;
                    }
                };
                let previous = self.config.clone();
                self.config.expansion[index].tags = tags;
                if let Err(error) = self.save_current() {
                    self.config = previous;
                    self.message = format!("Tag edit failed: {}", error.safe_summary());
                } else {
                    self.message = format!("Updated tags for {trigger}");
                    self.undo = Some(previous);
                    self.visible_cache = None;
                    self.preview_cache = None;
                }
            }
            Some(Prompt::PreviewApp) => {
                self.preview_app = input.trim().to_owned();
                self.preview_cache = None;
                self.message = if self.preview_app.is_empty() {
                    "Preview app cleared".into()
                } else {
                    format!("Previewing as {}", self.preview_app)
                };
            }
            None => {}
        }
    }

    fn preview(&mut self) -> String {
        let Some(index) = self.selected_index() else {
            return "No snippet selected".into();
        };
        let trigger = self.config.expansion[index].trigger.clone();
        if let Some((cached_index, cached_trigger, cached_result)) = &self.preview_cache {
            if *cached_index == index && cached_trigger == &trigger {
                return cached_result.clone();
            }
        }
        let Ok(mut engine) = ExpansionEngine::new(self.config.clone()) else {
            let result: String = "Configuration is invalid".into();
            self.preview_cache = Some((index, trigger, result.clone()));
            return result;
        };
        if !self.preview_app.trim().is_empty() {
            engine.set_current_window(Some(wayexpand_core::WindowContext {
                app_id: Some(self.preview_app.trim().to_owned()),
                title: None,
                instance_id: None,
            }));
        }
        let mut results = engine.process(InputEvent::Text(trigger.clone()));
        results.extend(engine.process(InputEvent::EndOfInput));
        let result = results
            .last()
            .map(|result| result.insert.clone())
            .unwrap_or_else(|| "No expansion matched".into());
        self.preview_cache = Some((index, trigger, result.clone()));
        result
    }

    fn refresh_daemon_state(&mut self) {
        if let Some(results) = self.status_results.as_ref() {
            match results.try_recv() {
                Ok(paused) => {
                    if let Some(paused) = paused {
                        self.paused = paused;
                    }
                    self.status_in_flight = false;
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.status_results = None;
                    self.status_requests = None;
                    self.status_in_flight = false;
                }
            }
        }
        if self.last_status_poll.elapsed() < Duration::from_secs(1) {
            return;
        }
        self.last_status_poll = Instant::now();
        if self.status_in_flight {
            return;
        }
        if let Some(requests) = self.status_requests.as_ref() {
            if requests.try_send(()).is_ok() {
                self.status_in_flight = true;
            }
        }
    }

    fn start_status_worker(&mut self) {
        let (request_sender, request_receiver) = mpsc::sync_channel(1);
        let (result_sender, result_receiver) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("wayexpand-tui-status".into())
            .spawn(move || {
                while request_receiver.recv().is_ok() {
                    let paused = control_command("status")
                        .ok()
                        .and_then(|status| paused_from_status(&status));
                    if result_sender.send(paused).is_err() {
                        break;
                    }
                }
            })
            .ok();
        self.status_requests = Some(request_sender);
        self.status_results = Some(result_receiver);
    }
}

/// Restores the terminal to its normal (cooked, main-screen, visible-cursor)
/// state when dropped. Without this, a panic anywhere in `run()` -- which
/// spends nearly all of this program's runtime in raw mode with the
/// alternate screen active -- unwinds straight past a plain
/// enable-then-restore-at-the-end sequence and leaves the user's terminal
/// stuck showing nothing and echoing nothing until they run `reset` or
/// `stty sane` blind. `Drop` still runs during a panicking unwind (Rust's
/// default panic strategy), so tying the restore to this guard's lifetime
/// covers that case as well as the normal and early-return-on-error ones.
struct TerminalGuard;

impl TerminalGuard {
    fn enter(stdout: &mut io::Stdout) -> Result<Self> {
        terminal::enable_raw_mode().context("enabling terminal input mode")?;
        execute!(stdout, terminal::EnterAlternateScreen, cursor::Hide)?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        // Best-effort: if these fail (e.g. stdout already gone), there is
        // nothing further to do, and this must not panic-while-panicking.
        let _ = execute!(io::stdout(), cursor::Show, terminal::LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
    }
}

fn main() -> Result<()> {
    let argument = env::args().nth(1);
    if matches!(argument.as_deref(), Some("--help" | "-h")) {
        println!("Usage: wayexpand-ui [CONFIG]\n\nInteractive snippet browser and settings editor.\n\nKeys: / search, j/k navigate, e replacement, D description, t tags, m mode, space toggle, p pause/resume, r reload, q quit.");
        return Ok(());
    }
    let path = argument
        .map(PathBuf::from)
        .unwrap_or_else(default_config_path);
    let mut app = App::load(path)?;
    app.start_status_worker();
    let mut stdout = io::stdout();
    let _terminal_guard = TerminalGuard::enter(&mut stdout)?;
    run(&mut stdout, &mut app)
}

fn run(stdout: &mut io::Stdout, app: &mut App) -> Result<()> {
    // `draw` clears and repaints the whole screen, so it runs only when
    // something visible may have changed. Repainting on every 250 ms idle
    // tick made the TUI flicker, noticeably so over SSH.
    let mut needs_draw = true;
    loop {
        let paused = app.paused;
        app.refresh_daemon_state();
        if needs_draw || app.paused != paused {
            draw(stdout, app)?;
            needs_draw = false;
        }
        if !event::poll(Duration::from_millis(250))? {
            continue;
        }
        let key = match event::read()? {
            Event::Key(key) => key,
            Event::Resize(..) => {
                needs_draw = true;
                continue;
            }
            _ => continue,
        };
        needs_draw = true;
        if handle_key(app, key)? {
            return Ok(());
        }
        if app.external_edit {
            app.external_edit = false;
            edit_with_external_editor(stdout, app)?;
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum UiEffect {
    Continue,
    Quit,
    OpenExternalEditor,
}

/// Apply one user event to the TUI state. Rendering and terminal I/O consume
/// the returned effect, leaving state transitions directly testable.
fn update(app: &mut App, key: KeyEvent) -> Result<UiEffect> {
    if let Some(index) = app.confirm_delete {
        match key.code {
            KeyCode::Char('d') | KeyCode::Enter => app.delete_confirmed(index),
            KeyCode::Esc => {
                app.confirm_delete = None;
                app.message = "Deletion cancelled".into();
            }
            _ => {}
        }
        return Ok(UiEffect::Continue);
    }
    if app.prompt.is_some() {
        match key.code {
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                app.input.clear();
            }
            KeyCode::Esc => {
                app.prompt = None;
                app.input.clear();
                app.message = "Edit cancelled".into();
            }
            KeyCode::Enter => app.submit_prompt(),
            KeyCode::Backspace => {
                app.input.pop();
            }
            KeyCode::Char(character)
                if !key.modifiers.intersects(
                    KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER,
                ) =>
            {
                app.input.push(character)
            }
            _ => {}
        }
        return Ok(UiEffect::Continue);
    }
    if app.searching {
        match key.code {
            KeyCode::Esc | KeyCode::Enter => app.searching = false,
            KeyCode::Backspace => {
                app.query.pop();
                app.selected = 0;
            }
            KeyCode::Char(character)
                if !key.modifiers.intersects(
                    KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER,
                ) =>
            {
                app.query.push(character);
                app.selected = 0;
            }
            _ => {}
        }
        return Ok(UiEffect::Continue);
    }
    match key {
        KeyEvent {
            code: KeyCode::Char('q') | KeyCode::Esc,
            ..
        } => return Ok(UiEffect::Quit),
        KeyEvent {
            code: KeyCode::Char('c'),
            modifiers: KeyModifiers::CONTROL,
            ..
        } => return Ok(UiEffect::Quit),
        KeyEvent {
            code: KeyCode::Char('/'),
            ..
        } => app.searching = true,
        KeyEvent {
            code: KeyCode::Char(' '),
            ..
        } => app.toggle_selected(),
        KeyEvent {
            code: KeyCode::Char('r'),
            ..
        } => app.reload(),
        KeyEvent {
            code: KeyCode::Char('n'),
            ..
        } => app.begin_new(),
        KeyEvent {
            code: KeyCode::Char('e'),
            ..
        } => app.begin_edit(),
        KeyEvent {
            code: KeyCode::Char('D'),
            ..
        } => app.begin_edit_description(),
        KeyEvent {
            code: KeyCode::Char('t'),
            ..
        } => app.begin_edit_tags(),
        KeyEvent {
            code: KeyCode::Char('a'),
            ..
        } => app.begin_preview_app(),
        KeyEvent {
            code: KeyCode::Char('m'),
            ..
        } => app.toggle_match_mode(),
        KeyEvent {
            code: KeyCode::Char('E'),
            ..
        } => return Ok(UiEffect::OpenExternalEditor),
        KeyEvent {
            code: KeyCode::Char('d'),
            ..
        } => app.request_delete(),
        KeyEvent {
            code: KeyCode::Char('u'),
            ..
        } => app.undo_last(),
        KeyEvent {
            code: KeyCode::Char('p'),
            ..
        } => {
            let command = if app.paused { "resume" } else { "pause" };
            match control_command(command) {
                Ok(_) => {
                    app.paused = !app.paused;
                    app.message = if app.paused {
                        "Expansion paused".into()
                    } else {
                        "Expansion resumed".into()
                    };
                }
                Err(error) => app.message = format!("Control unavailable: {error}"),
            }
        }
        KeyEvent {
            code: KeyCode::Up | KeyCode::Char('k'),
            ..
        } => app.selected = app.selected.saturating_sub(1),
        KeyEvent {
            code: KeyCode::Down | KeyCode::Char('j'),
            ..
        } => {
            let count = app.visible_indices().len();
            if count > 0 {
                app.selected = (app.selected + 1).min(count - 1);
            }
        }
        _ => {}
    }
    Ok(UiEffect::Continue)
}

fn handle_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    match update(app, key)? {
        UiEffect::Quit => Ok(true),
        UiEffect::OpenExternalEditor => {
            app.external_edit = true;
            Ok(false)
        }
        UiEffect::Continue => Ok(false),
    }
}

fn edit_with_external_editor(stdout: &mut io::Stdout, app: &mut App) -> Result<()> {
    let Some(index) = app.selected_index() else {
        app.message = "No snippet selected".into();
        return Ok(());
    };
    let temp = external_edit_directory(std::env::var_os("XDG_RUNTIME_DIR"))
        .join(format!("wayexpand-edit-{}.tmp", std::process::id()));
    execute!(stdout, cursor::Show, terminal::LeaveAlternateScreen)?;
    terminal::disable_raw_mode()?;
    let edit_result = (|| -> Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)
            .with_context(|| format!("creating editor file {}", temp.display()))?;
        file.write_all(app.config.expansion[index].replacement.as_bytes())?;
        file.sync_all()?;
        let editor_spec = std::env::var("VISUAL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| std::env::var("EDITOR").ok())
            .unwrap_or_else(|| "vi".into());

        let parts = shlex::split(&editor_spec).unwrap_or_else(|| vec![editor_spec.clone()]);

        let editor_name = if parts.is_empty() {
            "vi".to_string()
        } else {
            parts[0].clone()
        };

        let mut cmd = if parts.is_empty() {
            std::process::Command::new("vi")
        } else {
            let mut cmd = std::process::Command::new(&parts[0]);
            for arg in &parts[1..] {
                cmd.arg(arg);
            }
            cmd
        };
        cmd.arg(&temp);

        let status = cmd
            .status()
            .with_context(|| format!("starting editor {editor_name:?}"))?;
        if !status.success() {
            anyhow::bail!("editor exited unsuccessfully");
        }
        let mut contents = Vec::new();
        std::fs::File::open(&temp)?
            .take(1_048_577)
            .read_to_end(&mut contents)?;
        if contents.len() > 1_048_576 {
            anyhow::bail!("replacement exceeds 1 MiB");
        }
        let replacement = String::from_utf8(contents).context("editor file is not UTF-8")?;
        let previous = app.config.clone();
        let mut candidate = app.config.clone();
        candidate.expansion[index].replacement = replacement;
        candidate
            .validate()
            .map_err(|error| anyhow::anyhow!("replacement is invalid: {}", error.safe_summary()))?;
        app.save_candidate(&candidate).map_err(|error| {
            anyhow::anyhow!("could not save replacement: {}", error.safe_summary())
        })?;
        app.config = candidate;
        app.undo = Some(previous);
        app.message = "Replacement edited in external editor".into();
        Ok(())
    })();
    let _ = std::fs::remove_file(&temp);
    let restore_result = execute!(stdout, terminal::EnterAlternateScreen, cursor::Hide)
        .and_then(|_| terminal::enable_raw_mode());
    restore_result?;
    if let Err(error) = edit_result {
        app.message = format!("External edit failed: {error}");
    }
    Ok(())
}

/// Where the replacement is handed to `$VISUAL`/`$EDITOR`. The per-user
/// runtime directory (mode 0700, usually tmpfs) keeps the snippet body -- and
/// the swap/backup files editors create beside it -- out of the shared,
/// world-listable `/tmp`, where another user could also pre-create the
/// predictable name to block editing. `/tmp` remains the fallback.
fn external_edit_directory(runtime_dir: Option<std::ffi::OsString>) -> PathBuf {
    runtime_dir
        .map(PathBuf::from)
        .filter(|directory| directory.is_absolute() && directory.is_dir())
        .unwrap_or_else(std::env::temp_dir)
}

fn draw(stdout: &mut io::Stdout, app: &mut App) -> Result<()> {
    let selected_config_index = app.selected_index();
    let preview = app.preview();
    let visible_len = app.visible_indices().len();
    execute!(
        stdout,
        cursor::MoveTo(0, 0),
        terminal::Clear(ClearType::All),
        SetForegroundColor(Color::Cyan),
        Print("WayExpand Settings"),
        ResetColor,
        Print("  "),
        Print(if app.paused { "[PAUSED]" } else { "[ACTIVE]" }),
        Print("\n"),
        SetForegroundColor(Color::DarkGrey),
        Print("/ search   j/k move   n new   e edit   D description   t tags   a preview app   m mode   d+d delete   u undo   space toggle   p pause   r reload   q quit"),
        ResetColor,
        Print("\n\n"),
        Print("Filter: "),
        Print(&app.query),
        if app.searching {
            Print("▌")
        } else {
            Print("")
        },
        Print("\n\n")
    )?;
    for row in 0..visible_len {
        let index = app.visible_indices()[row];
        let expansion = &app.config.expansion[index];
        let marker = if row == app.selected { "❯" } else { " " };
        let state = if expansion.enabled { "●" } else { "○" };
        if Some(index) == selected_config_index {
            execute!(stdout, SetForegroundColor(Color::Yellow))?;
        }
        execute!(
            stdout,
            Print(format!(
                "{marker} {state} {:<18} {:<24} [{}] {} {}\n",
                expansion.trigger,
                expansion.description,
                match expansion.match_mode {
                    wayexpand_core::MatchMode::Immediate => "immediate",
                    wayexpand_core::MatchMode::WordBoundary => "word-boundary",
                },
                expansion.tags.join(", "),
                if expansion.command.is_some() {
                    "[command]"
                } else {
                    ""
                }
            )),
            ResetColor
        )?;
    }
    execute!(
        stdout,
        Print("\nPreview\n"),
        SetForegroundColor(Color::DarkGrey),
        Print("Preview app: "),
        Print(if app.preview_app.is_empty() {
            "(none)"
        } else {
            &app.preview_app
        }),
        Print("\n"),
        SetForegroundColor(Color::Green),
        Print(preview),
        ResetColor,
        Print("\n\n"),
        SetForegroundColor(Color::DarkGrey),
        Print("Selected mode: "),
        Print(
            selected_config_index
                .map(|index| match app.config.expansion[index].match_mode {
                    wayexpand_core::MatchMode::Immediate => "immediate",
                    wayexpand_core::MatchMode::WordBoundary => "word-boundary",
                })
                .unwrap_or("none"),
        ),
        Print("\n\n"),
        SetForegroundColor(Color::DarkGrey),
        Print(&app.message),
        ResetColor
    )?;
    if let Some(prompt) = &app.prompt {
        let label = match prompt {
            Prompt::NewTrigger => "New trigger",
            Prompt::NewReplacement { .. } => "Replacement",
            Prompt::EditReplacement { .. } => "Replacement",
            Prompt::EditDescription { .. } => "Description",
            Prompt::EditTags { .. } => "Tags",
            Prompt::PreviewApp => "Preview app id",
        };
        execute!(
            stdout,
            Print(format!("\n\n{label}: {}▌", app.input)),
            ResetColor
        )?;
    }
    stdout.flush()?;
    Ok(())
}

fn control_command(command: &str) -> Result<String> {
    wayexpand_core::DaemonClient::from_environment()?
        .request(command)
        .map_err(Into::into)
}

fn paused_from_status(response: &str) -> Option<bool> {
    DaemonStatus::parse(response).bool_field("paused")
}

/// Present tags for editing as plain `a, b` text whenever that form parses
/// back to exactly the same list, and as a JSON array only for tags it cannot
/// represent (commas, surrounding whitespace, empty tags, newlines, or a
/// leading `[`).
fn encode_tags(tags: &[String]) -> String {
    let plain = tags.join(", ");
    if decode_plain_tags(&plain) == tags {
        plain
    } else {
        serde_json::to_string(tags).expect("serializing strings to JSON cannot fail")
    }
}

/// Parse the tag prompt: a JSON string array when the input starts with `[`,
/// otherwise comma-separated tags with surrounding whitespace and empty
/// entries dropped. JSON-only input was hostile to type in a terminal and
/// broke the documented `ops, email` form; JSON stays available for the
/// rare tag that itself contains a comma.
fn decode_tags(input: &str) -> serde_json::Result<Vec<String>> {
    if input.trim_start().starts_with('[') {
        serde_json::from_str(input)
    } else {
        Ok(decode_plain_tags(input))
    }
}

fn decode_plain_tags(input: &str) -> Vec<String> {
    input
        .split(',')
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_app() -> (App, PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "wayexpand-tui-reducer-{}-{:?}.toml",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_file(&path);
        let config = Config::parse(
            "[[expansion]]\ntrigger = \":one\"\nreplacement = \"first\"\n\n[[expansion]]\ntrigger = \":two\"\nreplacement = \"second\"\n",
        )
        .unwrap();
        config.save_atomic(&path).unwrap();
        (App::load(path.clone()).unwrap(), path)
    }

    fn test_app_with_triggers(triggers: &[&str]) -> (App, PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "wayexpand-tui-reducer-many-{}-{:?}.toml",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_file(&path);
        let config = Config::parse(
            &triggers
                .iter()
                .enumerate()
                .map(|(index, trigger)| {
                    format!(
                        "[[expansion]]\ntrigger = \"{trigger}\"\nreplacement = \"value-{index}\"\n"
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
        config.save_atomic(&path).unwrap();
        (App::load(path.clone()).unwrap(), path)
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn reducer_delete_cancel_preserves_selection_and_config() {
        let (mut app, path) = test_app();
        update(&mut app, key(KeyCode::Char('d'))).unwrap();
        assert_eq!(app.confirm_delete, Some(0));
        update(&mut app, key(KeyCode::Esc)).unwrap();
        assert_eq!(app.confirm_delete, None);
        assert_eq!(app.config.expansion.len(), 2);
        assert_eq!(Config::load(path).unwrap().expansion.len(), 2);
    }

    #[test]
    fn reducer_delete_confirm_then_undo_restores_the_config() {
        let (mut app, path) = test_app();
        update(&mut app, key(KeyCode::Char('d'))).unwrap();
        update(&mut app, key(KeyCode::Char('d'))).unwrap();
        assert_eq!(app.config.expansion.len(), 1);
        update(&mut app, key(KeyCode::Char('u'))).unwrap();
        assert_eq!(app.config.expansion.len(), 2);
        assert_eq!(Config::load(path).unwrap().expansion.len(), 2);
    }

    #[test]
    fn reducer_delete_last_visible_item_clamps_selection_after_cache_invalidation() {
        let (mut app, _path) = test_app();
        app.selected = 1;
        assert_eq!(app.visible_indices(), &[0, 1]);

        app.delete_confirmed(1);

        assert_eq!(app.config.expansion.len(), 1);
        assert_eq!(app.selected, 0);
        assert_eq!(app.visible_indices(), &[0]);
    }

    #[test]
    fn reducer_delete_while_filtering_invalidates_visible_cache() {
        let (mut app, _path) = test_app();
        app.query = "two".to_string();
        assert_eq!(app.visible_indices(), &[1]);

        app.delete_confirmed(1);

        assert!(app.visible_indices().is_empty());
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn reducer_undo_while_filtering_rebuilds_visible_cache() {
        let (mut app, _path) = test_app();
        app.query = "two".to_string();
        assert_eq!(app.visible_indices(), &[1]);
        app.delete_confirmed(1);
        assert!(app.visible_indices().is_empty());

        app.undo_last();

        assert_eq!(app.visible_indices(), &[1]);
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn reducer_delete_before_selection_preserves_following_item() {
        let (mut app, _path) = test_app_with_triggers(&[":one", ":two", ":three"]);
        app.selected = 2;
        assert_eq!(app.selected_index(), Some(2));

        app.delete_confirmed(0);

        assert_eq!(app.selected, 1);
        assert_eq!(app.selected_index(), Some(1));
        assert_eq!(app.config.expansion[1].trigger, ":three");
    }

    #[test]
    fn reducer_search_then_edit_updates_the_matching_snippet() {
        let (mut app, path) = test_app();
        update(&mut app, key(KeyCode::Char('/'))).unwrap();
        update(&mut app, key(KeyCode::Char('t'))).unwrap();
        update(&mut app, key(KeyCode::Char('w'))).unwrap();
        update(&mut app, key(KeyCode::Enter)).unwrap();
        update(&mut app, key(KeyCode::Char('e'))).unwrap();
        update(
            &mut app,
            KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
        )
        .unwrap();
        update(&mut app, key(KeyCode::Char('u'))).unwrap();
        update(&mut app, key(KeyCode::Char('p'))).unwrap();
        update(&mut app, key(KeyCode::Char('d'))).unwrap();
        update(&mut app, key(KeyCode::Char('a'))).unwrap();
        update(&mut app, key(KeyCode::Char('t'))).unwrap();
        update(&mut app, key(KeyCode::Char('e'))).unwrap();
        update(&mut app, key(KeyCode::Enter)).unwrap();
        assert_eq!(app.config.expansion[1].replacement, "update");
        assert_eq!(
            Config::load(path).unwrap().expansion[1].replacement,
            "update"
        );
    }

    #[test]
    fn external_edits_prefer_the_private_runtime_directory() {
        let runtime = std::env::temp_dir();
        assert_eq!(
            external_edit_directory(Some(runtime.clone().into_os_string())),
            runtime
        );
        // Unset, relative, or missing runtime directories fall back to /tmp.
        assert_eq!(external_edit_directory(None), std::env::temp_dir());
        assert_eq!(
            external_edit_directory(Some("relative/dir".into())),
            std::env::temp_dir()
        );
        assert_eq!(
            external_edit_directory(Some("/nonexistent/wayexpand-runtime".into())),
            std::env::temp_dir()
        );
    }

    #[test]
    fn tui_status_uses_the_shared_daemon_status_contract() {
        assert_eq!(paused_from_status("running\npaused=true\n"), Some(true));
        assert_eq!(paused_from_status("running\npaused=false\n"), Some(false));
        assert_eq!(paused_from_status("running\npaused=maybe\n"), None);
        assert_eq!(paused_from_status("running\nwarning: paused=true\n"), None);
    }

    use std::fs;

    #[test]
    fn tui_tag_editor_round_trips_exact_values() {
        let tags = vec![
            "customer, west".into(),
            " email ".into(),
            String::new(),
            "line one\nline two".into(),
        ];
        assert_eq!(decode_tags(&encode_tags(&tags)).unwrap(), tags);
    }

    #[test]
    fn tui_tag_editor_accepts_plain_comma_separated_tags() {
        assert_eq!(decode_tags(" edited,  ui ,, ").unwrap(), ["edited", "ui"]);
        assert_eq!(decode_tags("").unwrap(), Vec::<String>::new());
        // Simple tags are offered back in the same plain form...
        let simple = vec!["ops".to_owned(), "email".to_owned()];
        assert_eq!(encode_tags(&simple), "ops, email");
        assert_eq!(decode_tags(&encode_tags(&simple)).unwrap(), simple);
        // ...and a tag the plain form cannot carry switches to JSON.
        let comma = vec!["customer, west".to_owned()];
        assert_eq!(encode_tags(&comma), r#"["customer, west"]"#);
        assert!(decode_tags("[not json").is_err());
    }

    #[test]
    fn tui_refuses_to_overwrite_a_newer_external_revision() {
        let path = std::env::temp_dir().join(format!(
            "wayexpand-tui-revision-{}.toml",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        let initial =
            Config::parse("[[expansion]]\ntrigger = \":x\"\nreplacement = \"initial\"\n").unwrap();
        initial.save_atomic(&path).unwrap();
        let mut app = App::load(path.clone()).unwrap();

        let mut external = initial;
        external.expansion[0].replacement = "external edit".into();
        external.save_atomic(&path).unwrap();

        app.toggle_selected();
        let on_disk = Config::load(&path).unwrap();
        assert_eq!(on_disk.expansion[0].replacement, "external edit");
        assert!(on_disk.expansion[0].enabled);
        assert!(app.config.expansion[0].enabled);
        assert!(app.message.contains("changed externally"));

        let filename = path.file_name().unwrap().to_string_lossy();
        let lock_path = path.with_file_name(format!(".{filename}.wayexpand.lock"));
        fs::remove_file(path).unwrap();
        fs::remove_file(lock_path).unwrap();
    }
}
