use anyhow::{Context, Result};
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute, queue,
    style::{Attribute, Print, ResetColor, SetAttribute, SetForegroundColor},
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

mod render;

/// How the message line is colored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tone {
    Info,
    Success,
    Warning,
    Error,
}

/// What the last status poll learned about the daemon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DaemonState {
    Checking,
    Running,
    Paused,
    Stopped,
}

struct App {
    path: PathBuf,
    config: Config,
    config_revision: ConfigRevision,
    selected: usize,
    /// First list row on screen; kept so the selection stays visible.
    scroll: usize,
    /// Snippet rows the list showed in the last frame, for paging.
    list_rows: usize,
    query: String,
    searching: bool,
    message: String,
    tone: Tone,
    show_help: bool,
    paused: bool,
    daemon: DaemonState,
    /// How the daemon's output route replaces triggers, from its status.
    insertion: Option<InsertionAssurance>,
    prompt: Option<Prompt>,
    input: String,
    /// Caret position in `input`, in characters.
    cursor: usize,
    confirm_delete: Option<usize>,
    undo: Option<Config>,
    external_edit: bool,
    last_status_poll: Instant,
    status_requests: Option<SyncSender<()>>,
    status_results: Option<Receiver<StatusSummary>>,
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

impl Prompt {
    fn label(&self) -> &'static str {
        match self {
            Prompt::NewTrigger => "New trigger",
            Prompt::NewReplacement { .. } | Prompt::EditReplacement { .. } => "Replacement",
            Prompt::EditDescription { .. } => "Description",
            Prompt::EditTags { .. } => "Tags",
            Prompt::PreviewApp => "Preview app id",
        }
    }
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
            scroll: 0,
            list_rows: 10,
            query: String::new(),
            searching: false,
            message: "Ready".into(),
            tone: Tone::Info,
            show_help: false,
            paused: false,
            daemon: DaemonState::Checking,
            insertion: None,
            prompt: None,
            input: String::new(),
            cursor: 0,
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

    fn say(&mut self, tone: Tone, message: String) {
        self.tone = tone;
        self.message = message;
    }

    /// Opens a prompt with `input` as its text and the caret at the end.
    fn open_prompt(&mut self, prompt: Prompt, input: String) {
        self.cursor = input.chars().count();
        self.input = input;
        self.prompt = Some(prompt);
    }

    fn move_selection(&mut self, target: usize) {
        let count = self.visible_indices().len();
        self.selected = target.min(count.saturating_sub(1));
    }

    fn insert_at_cursor(&mut self, character: char) {
        let at = byte_offset(&self.input, self.cursor);
        self.input.insert(at, character);
        self.cursor += 1;
    }

    fn delete_before_cursor(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.cursor -= 1;
        let at = byte_offset(&self.input, self.cursor);
        self.input.remove(at);
    }

    fn delete_at_cursor(&mut self) {
        if self.cursor < self.input.chars().count() {
            let at = byte_offset(&self.input, self.cursor);
            self.input.remove(at);
        }
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
            self.say(Tone::Warning, "No matching snippet".into());
            return;
        };
        let previous = self.config.clone();
        self.config.expansion[index].enabled = !self.config.expansion[index].enabled;
        match self.save_current() {
            Ok(()) => {
                self.say(
                    Tone::Success,
                    format!(
                        "{} {}",
                        if self.config.expansion[index].enabled {
                            "Enabled"
                        } else {
                            "Disabled"
                        },
                        self.config.expansion[index].trigger
                    ),
                );
                self.undo = Some(previous);
                self.visible_cache = None;
                self.preview_cache = None;
            }
            Err(error) => {
                self.config.expansion[index].enabled = !self.config.expansion[index].enabled;
                self.say(
                    Tone::Error,
                    format!("Save failed: {}", error.safe_summary()),
                );
            }
        }
    }

    fn toggle_match_mode(&mut self) {
        let Some(index) = self.selected_index() else {
            self.say(Tone::Warning, "No matching snippet".into());
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
                self.say(
                    Tone::Success,
                    format!(
                        "{label} matching enabled for {}",
                        self.config.expansion[index].trigger
                    ),
                );
                self.undo = Some(previous);
                self.visible_cache = None;
                self.preview_cache = None;
            }
            Err(error) => {
                self.config.expansion[index].match_mode = previous.expansion[index].match_mode;
                self.say(
                    Tone::Error,
                    format!("Save failed: {}", error.safe_summary()),
                );
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
                self.say(Tone::Success, "Configuration reloaded".into());
            }
            Err(error) => self.say(
                Tone::Error,
                format!("Reload failed: {}", error.safe_summary()),
            ),
        }
    }

    fn begin_new(&mut self) {
        self.open_prompt(Prompt::NewTrigger, String::new());
        self.say(
            Tone::Info,
            "New snippet: type its trigger, such as ;sig".into(),
        );
    }

    fn begin_edit(&mut self) {
        let Some(index) = self.selected_index() else {
            self.say(Tone::Warning, "No snippet selected".into());
            return;
        };
        let prompt = Prompt::EditReplacement {
            index,
            trigger: self.config.expansion[index].trigger.clone(),
        };
        self.open_prompt(prompt, self.config.expansion[index].replacement.clone());
        self.say(
            Tone::Info,
            "Edit the replacement (↵ marks a line break; for multi-line text, Esc then E)".into(),
        );
    }

    fn begin_edit_description(&mut self) {
        let Some(index) = self.selected_index() else {
            self.say(Tone::Warning, "No snippet selected".into());
            return;
        };
        let prompt = Prompt::EditDescription {
            index,
            trigger: self.config.expansion[index].trigger.clone(),
        };
        self.open_prompt(prompt, self.config.expansion[index].description.clone());
        self.say(Tone::Info, "Edit the description".into());
    }

    fn begin_edit_tags(&mut self) {
        let Some(index) = self.selected_index() else {
            self.say(Tone::Warning, "No snippet selected".into());
            return;
        };
        let prompt = Prompt::EditTags {
            index,
            trigger: self.config.expansion[index].trigger.clone(),
        };
        self.open_prompt(prompt, encode_tags(&self.config.expansion[index].tags));
        self.say(
            Tone::Info,
            "Comma-separated tags, such as ops, email (a JSON array for tags with commas)".into(),
        );
    }

    fn request_delete(&mut self) {
        let Some(index) = self.selected_index() else {
            self.say(Tone::Warning, "No snippet selected".into());
            return;
        };
        self.confirm_delete = Some(index);
        let trigger = self.config.expansion[index].trigger.clone();
        self.say(
            Tone::Warning,
            format!("Delete {trigger}? Press d again to delete it, or Esc to keep it."),
        );
    }

    fn begin_preview_app(&mut self) {
        self.open_prompt(Prompt::PreviewApp, self.preview_app.clone());
        self.say(
            Tone::Info,
            "Preview as if typed in this app id, such as firefox (empty clears it)".into(),
        );
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
            self.say(
                Tone::Error,
                format!("Delete failed: {}", error.safe_summary()),
            );
        } else {
            self.say(Tone::Success, format!("Deleted {trigger}"));
            self.undo = Some(previous);
        }
        self.confirm_delete = None;
    }

    fn undo_last(&mut self) {
        let Some(previous) = self.undo.take() else {
            self.say(Tone::Warning, "Nothing to undo".into());
            return;
        };
        let current = std::mem::replace(&mut self.config, previous);
        self.visible_cache = None;
        self.preview_cache = None;
        if let Err(error) = self.save_current() {
            self.config = current;
            self.say(
                Tone::Error,
                format!("Undo failed: {}", error.safe_summary()),
            );
        } else {
            self.selected = self
                .selected
                .min(self.visible_indices().len().saturating_sub(1));
            self.say(Tone::Success, "Undid the last saved change".into());
        }
    }

    fn submit_prompt(&mut self) {
        let prompt = self.prompt.take();
        let input = std::mem::take(&mut self.input);
        self.cursor = 0;
        match prompt {
            Some(Prompt::NewTrigger) if !input.trim().is_empty() => {
                self.say(Tone::Info, format!("Now type the text {input} expands to"));
                self.prompt = Some(Prompt::NewReplacement { trigger: input });
            }
            Some(Prompt::NewTrigger) => self.say(Tone::Error, "Trigger cannot be empty".into()),
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
                    output_file: None,
                });
                if let Err(error) = self.save_current() {
                    self.config = previous;
                    self.say(
                        Tone::Error,
                        format!("Create failed: {}", error.safe_summary()),
                    );
                } else {
                    self.visible_cache = None;
                    self.preview_cache = None;
                    self.selected = self.visible_indices().len().saturating_sub(1);
                    self.say(Tone::Success, "Snippet created".into());
                    self.undo = Some(previous);
                }
            }
            Some(Prompt::EditReplacement { index, trigger }) => {
                let previous = self.config.clone();
                self.config.expansion[index].replacement = input;
                if let Err(error) = self.save_current() {
                    self.config = previous;
                    self.say(
                        Tone::Error,
                        format!("Edit failed: {}", error.safe_summary()),
                    );
                } else {
                    self.say(Tone::Success, format!("Updated {trigger}"));
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
                    self.say(
                        Tone::Error,
                        format!("Description edit failed: {}", error.safe_summary()),
                    );
                } else {
                    self.say(Tone::Success, format!("Updated description for {trigger}"));
                    self.undo = Some(previous);
                    self.visible_cache = None;
                    self.preview_cache = None;
                }
            }
            Some(Prompt::EditTags { index, trigger }) => {
                let tags = match decode_tags(&input) {
                    Ok(tags) => tags,
                    Err(error) => {
                        self.say(Tone::Error, format!("Tags JSON array is invalid: {error}"));
                        self.open_prompt(Prompt::EditTags { index, trigger }, input);
                        return;
                    }
                };
                let previous = self.config.clone();
                self.config.expansion[index].tags = tags;
                if let Err(error) = self.save_current() {
                    self.config = previous;
                    self.say(
                        Tone::Error,
                        format!("Tag edit failed: {}", error.safe_summary()),
                    );
                } else {
                    self.say(Tone::Success, format!("Updated tags for {trigger}"));
                    self.undo = Some(previous);
                    self.visible_cache = None;
                    self.preview_cache = None;
                }
            }
            Some(Prompt::PreviewApp) => {
                self.preview_app = input.trim().to_owned();
                self.preview_cache = None;
                self.say(
                    Tone::Success,
                    if self.preview_app.is_empty() {
                        "Preview app cleared".into()
                    } else {
                        format!("Previewing as {}", self.preview_app)
                    },
                );
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
                Ok(summary) => {
                    if let Some(paused) = summary.paused {
                        self.paused = paused;
                    }
                    self.daemon = match (summary.reachable, self.paused) {
                        (false, _) => DaemonState::Stopped,
                        (true, true) => DaemonState::Paused,
                        (true, false) => DaemonState::Running,
                    };
                    self.insertion = summary.insertion;
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
                    let summary = control_command(wayexpand_core::DaemonOperation::Status)
                        .ok()
                        .map_or_else(StatusSummary::default, |status| StatusSummary {
                            reachable: true,
                            paused: paused_from_status(&status),
                            insertion: insertion_from_status(&status),
                        });
                    if result_sender.send(summary).is_err() {
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
        println!(
            "Usage: wayexpand-ui [CONFIG]\n\n\
             Interactive snippet browser and settings editor.\n\n\
             Keys: ↑/↓ or j/k move, / search, n new, e edit, E edit in $EDITOR,\n\
             D description, t tags, m matching mode, Space enable/disable,\n\
             d d delete, u undo, p pause/resume, r reload, ? all keys, q quit.\n\n\
             Set NO_COLOR to turn colors off."
        );
        return Ok(());
    }
    if matches!(argument.as_deref(), Some("--version" | "-V")) {
        println!("wayexpand-ui {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    // An unrecognized option is an error rather than a path: treating
    // `--version` as a config file would create it in the working directory.
    let path = match argument {
        Some(option) if option.starts_with('-') => {
            anyhow::bail!("unknown option {option}; see wayexpand-ui --help")
        }
        Some(path) => PathBuf::from(path),
        None => default_config_path(),
    };
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
        let (daemon, insertion) = (app.daemon, app.insertion);
        app.refresh_daemon_state();
        if needs_draw || app.daemon != daemon || app.insertion != insertion {
            draw(stdout, app)?;
            needs_draw = false;
        }
        if !event::poll(Duration::from_millis(250))? {
            continue;
        }
        let key = match event::read()? {
            Event::Key(key) => key,
            Event::Resize(..) => {
                // Rows past a shrunken frame would otherwise keep old text.
                execute!(stdout, terminal::Clear(ClearType::All))?;
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
            execute!(stdout, terminal::Clear(ClearType::All))?;
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
    let plain = !key
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER);
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return Ok(UiEffect::Quit);
    }
    if app.show_help {
        app.show_help = false;
        return Ok(UiEffect::Continue);
    }
    if let Some(index) = app.confirm_delete {
        match key.code {
            KeyCode::Char('d') | KeyCode::Enter => app.delete_confirmed(index),
            KeyCode::Esc | KeyCode::Char('n') => {
                app.confirm_delete = None;
                app.say(Tone::Info, "Deletion cancelled".into());
            }
            _ => {}
        }
        return Ok(UiEffect::Continue);
    }
    if app.prompt.is_some() {
        match key.code {
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                app.input.clear();
                app.cursor = 0;
            }
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => app.cursor = 0,
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                app.cursor = app.input.chars().count();
            }
            KeyCode::Esc => {
                app.prompt = None;
                app.input.clear();
                app.cursor = 0;
                app.say(Tone::Info, "Edit cancelled".into());
            }
            KeyCode::Enter => app.submit_prompt(),
            KeyCode::Backspace => app.delete_before_cursor(),
            KeyCode::Delete => app.delete_at_cursor(),
            KeyCode::Left => app.cursor = app.cursor.saturating_sub(1),
            KeyCode::Right => app.cursor = (app.cursor + 1).min(app.input.chars().count()),
            KeyCode::Home => app.cursor = 0,
            KeyCode::End => app.cursor = app.input.chars().count(),
            KeyCode::Char(character) if plain => app.insert_at_cursor(character),
            _ => {}
        }
        return Ok(UiEffect::Continue);
    }
    if app.searching {
        match key.code {
            KeyCode::Enter => app.searching = false,
            KeyCode::Esc => {
                app.searching = false;
                app.query.clear();
                app.selected = 0;
            }
            KeyCode::Backspace => {
                app.query.pop();
                app.selected = 0;
            }
            KeyCode::Up => app.selected = app.selected.saturating_sub(1),
            KeyCode::Down => app.move_selection(app.selected + 1),
            KeyCode::Char(character) if plain => {
                app.query.push(character);
                app.selected = 0;
            }
            _ => {}
        }
        return Ok(UiEffect::Continue);
    }
    let page = app.list_rows.max(1);
    match key.code {
        KeyCode::Char('q') => return Ok(UiEffect::Quit),
        // Esc first clears an active search, then quits.
        KeyCode::Esc if !app.query.is_empty() => {
            app.query.clear();
            app.selected = 0;
            app.say(Tone::Info, "Search cleared".into());
        }
        KeyCode::Esc => return Ok(UiEffect::Quit),
        KeyCode::Char('?') | KeyCode::F(1) => app.show_help = true,
        KeyCode::Char('/') => app.searching = true,
        KeyCode::Char(' ') => app.toggle_selected(),
        KeyCode::Char('r') => app.reload(),
        KeyCode::Char('n') => app.begin_new(),
        KeyCode::Char('e') => app.begin_edit(),
        KeyCode::Char('D') => app.begin_edit_description(),
        KeyCode::Char('t') => app.begin_edit_tags(),
        KeyCode::Char('a') => app.begin_preview_app(),
        KeyCode::Char('m') => app.toggle_match_mode(),
        KeyCode::Char('E') => return Ok(UiEffect::OpenExternalEditor),
        KeyCode::Char('d') => app.request_delete(),
        KeyCode::Char('u') => app.undo_last(),
        KeyCode::Char('p') => {
            let operation = if app.paused {
                wayexpand_core::DaemonOperation::Resume
            } else {
                wayexpand_core::DaemonOperation::Pause
            };
            match control_command(operation) {
                Ok(_) => {
                    app.paused = !app.paused;
                    app.daemon = if app.paused {
                        DaemonState::Paused
                    } else {
                        DaemonState::Running
                    };
                    app.say(
                        Tone::Success,
                        if app.paused {
                            "Expansion paused".into()
                        } else {
                            "Expansion resumed".into()
                        },
                    );
                }
                Err(error) => app.say(Tone::Error, format!("Control unavailable: {error}")),
            }
        }
        KeyCode::Up | KeyCode::Char('k') => app.selected = app.selected.saturating_sub(1),
        KeyCode::Down | KeyCode::Char('j') => app.move_selection(app.selected + 1),
        KeyCode::PageUp => app.selected = app.selected.saturating_sub(page),
        KeyCode::PageDown => app.move_selection(app.selected + page),
        KeyCode::Home | KeyCode::Char('g') => app.selected = 0,
        KeyCode::End | KeyCode::Char('G') => app.move_selection(usize::MAX),
        _ => {}
    }
    Ok(UiEffect::Continue)
}

/// Byte index of the `chars`-th character of `text` (its length past the end).
fn byte_offset(text: &str, chars: usize) -> usize {
    text.char_indices()
        .nth(chars)
        .map_or(text.len(), |(offset, _)| offset)
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
        app.say(Tone::Warning, "No snippet selected".into());
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
        app.say(
            Tone::Success,
            "Replacement edited in external editor".into(),
        );
        Ok(())
    })();
    let _ = std::fs::remove_file(&temp);
    let restore_result = execute!(stdout, terminal::EnterAlternateScreen, cursor::Hide)
        .and_then(|_| terminal::enable_raw_mode());
    restore_result?;
    if let Err(error) = edit_result {
        app.say(Tone::Error, format!("External edit failed: {error}"));
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

/// Paints one frame in place. Every row is written at full width, so rows
/// from the previous frame are overwritten without clearing the screen.
fn draw(stdout: &mut io::Stdout, app: &mut App) -> Result<()> {
    // A pseudo-terminal nobody sized (for example under `script`) reports 0×0.
    let (width, height) = terminal::size()
        .ok()
        .filter(|&(width, height)| width > 0 && height > 0)
        .unwrap_or((80, 24));
    let lines = frame_for(app, width, height);
    let color = color_enabled();
    for (row, line) in lines.iter().enumerate() {
        queue!(
            stdout,
            cursor::MoveTo(0, u16::try_from(row).unwrap_or(u16::MAX))
        )?;
        for piece in line {
            queue!(stdout, SetAttribute(Attribute::Reset), ResetColor)?;
            if let (true, Some(foreground)) = (color, piece.style.fg) {
                queue!(stdout, SetForegroundColor(foreground))?;
            }
            for (enabled, attribute) in [
                (piece.style.bold, Attribute::Bold),
                (piece.style.dim, Attribute::Dim),
                (piece.style.italic, Attribute::Italic),
                (piece.style.reverse, Attribute::Reverse),
            ] {
                if enabled {
                    queue!(stdout, SetAttribute(attribute))?;
                }
            }
            queue!(stdout, Print(&piece.text))?;
        }
    }
    queue!(stdout, SetAttribute(Attribute::Reset), ResetColor)?;
    stdout.flush()?;
    Ok(())
}

/// Lays out the current state for a `width` by `height` terminal, scrolling
/// the list so the selection is on screen.
fn frame_for(app: &mut App, width: u16, height: u16) -> Vec<render::Line> {
    let visible = app.visible_indices().to_vec();
    let layout = render::Layout::new(width, height, visible.len());
    app.selected = app.selected.min(visible.len().saturating_sub(1));
    app.list_rows = layout.list_rows.max(1);
    app.scroll = render::keep_visible(app.scroll, app.selected, layout.list_rows, visible.len());
    let preview = app.preview();
    let path = display_path(&app.path, env::var_os("HOME").map(PathBuf::from).as_deref());
    let prompt = app.prompt.as_ref().map(|prompt| render::PromptView {
        label: prompt.label(),
        input: &app.input,
        cursor: app.cursor,
    });
    let view = render::View {
        path: &path,
        config: &app.config,
        visible: &visible,
        selected: app.selected,
        scroll: app.scroll,
        query: &app.query,
        searching: app.searching,
        daemon: app.daemon,
        insertion: app.insertion,
        message: &app.message,
        tone: app.tone,
        prompt,
        confirm_delete: app.confirm_delete.is_some(),
        preview: &preview,
        preview_app: &app.preview_app,
        show_help: app.show_help,
    };
    render::frame(&view, width, height)
}

/// Honors <https://no-color.org>: any non-empty `NO_COLOR` turns colors off;
/// bold, dim and reverse video still mark state.
fn color_enabled() -> bool {
    env::var_os("NO_COLOR").is_none_or(|value| value.is_empty())
}

/// `path` with the home directory shown as `~`.
fn display_path(path: &std::path::Path, home: Option<&std::path::Path>) -> String {
    if let Some(rest) = home
        .filter(|home| !home.as_os_str().is_empty())
        .and_then(|home| path.strip_prefix(home).ok())
    {
        return format!("~/{}", rest.display());
    }
    path.display().to_string()
}

fn control_command(operation: wayexpand_core::DaemonOperation) -> Result<String> {
    wayexpand_core::DaemonClient::from_environment()?
        .execute(operation)
        .map_err(Into::into)
}

fn paused_from_status(response: &str) -> Option<bool> {
    DaemonStatus::parse(response).bool_field("paused")
}

/// What one daemon status poll reports to the TUI.
#[derive(Default)]
struct StatusSummary {
    reachable: bool,
    paused: Option<bool>,
    insertion: Option<InsertionAssurance>,
}

/// Whether trigger replacement is verified against the field's text, relies
/// on unverified keystrokes, or is refused. `None` when no route replaces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InsertionAssurance {
    Verified,
    FocusTrackedBestEffort,
    Unverified,
    Compatibility,
    Blocked,
}

fn insertion_from_status(response: &str) -> Option<InsertionAssurance> {
    let status = DaemonStatus::parse(response);
    match status
        .field("replacement_assurance")
        .or_else(|| status.field("insertion_assurance"))?
    {
        "verified" => Some(InsertionAssurance::Verified),
        "focus-tracked-best-effort" => Some(InsertionAssurance::FocusTrackedBestEffort),
        "unverified" => Some(InsertionAssurance::Unverified),
        "compatibility" => Some(InsertionAssurance::Compatibility),
        "blocked" => Some(InsertionAssurance::Blocked),
        _ => None,
    }
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
mod tests;
