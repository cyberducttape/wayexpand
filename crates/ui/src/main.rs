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
    os::unix::net::UnixStream,
    path::PathBuf,
    time::Duration,
    time::Instant,
};
use wayexpand_core::{default_config_path, Config, ExpansionEngine, InputEvent};

const CONTROL_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_CONTROL_RESPONSE_BYTES: usize = 4096;

struct App {
    path: PathBuf,
    config: Config,
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
}

enum Prompt {
    NewTrigger,
    NewReplacement { trigger: String },
    EditReplacement { index: usize, trigger: String },
    EditDescription { index: usize, trigger: String },
    EditTags { index: usize, trigger: String },
}

impl App {
    fn load(path: PathBuf) -> Result<Self> {
        let config = Config::load(&path)
            .map_err(|error| anyhow::anyhow!("configuration invalid: {}", error.safe_summary()))?;
        Ok(Self {
            path,
            config,
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
        })
    }

    fn visible_indices(&self) -> Vec<usize> {
        let query = self.query.to_lowercase();
        self.config
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
            .collect()
    }

    fn selected_index(&self) -> Option<usize> {
        self.visible_indices().get(self.selected).copied()
    }

    fn toggle_selected(&mut self) {
        let Some(index) = self.selected_index() else {
            self.message = "No matching snippet".into();
            return;
        };
        let previous = self.config.clone();
        self.config.expansion[index].enabled = !self.config.expansion[index].enabled;
        match self.config.save_atomic(&self.path) {
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
        match self.config.save_atomic(&self.path) {
            Ok(()) => {
                self.message = format!(
                    "{label} matching enabled for {}",
                    self.config.expansion[index].trigger
                );
                self.undo = Some(previous);
            }
            Err(error) => {
                self.config.expansion[index].match_mode = previous.expansion[index].match_mode;
                self.message = format!("Save failed: {}", error.safe_summary());
            }
        }
    }

    fn reload(&mut self) {
        match Config::load(&self.path) {
            Ok(config) => {
                self.config = config;
                self.undo = None;
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
        self.input = self.config.expansion[index].tags.join(", ");
        self.prompt = Some(Prompt::EditTags {
            index,
            trigger: self.config.expansion[index].trigger.clone(),
        });
        self.message = "Enter comma-separated tags and press Enter".into();
    }

    fn request_delete(&mut self) {
        let Some(index) = self.selected_index() else {
            self.message = "No snippet selected".into();
            return;
        };
        self.confirm_delete = Some(index);
        self.message = "Press d again to confirm deletion, or Esc to cancel".into();
    }

    fn delete_confirmed(&mut self, index: usize) {
        let previous = self.config.clone();
        let trigger = self.config.expansion[index].trigger.clone();
        self.config.expansion.remove(index);
        self.selected = self
            .selected
            .min(self.visible_indices().len().saturating_sub(1));
        if let Err(error) = self.config.save_atomic(&self.path) {
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
        if let Err(error) = self.config.save_atomic(&self.path) {
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
                });
                if let Err(error) = self.config.save_atomic(&self.path) {
                    self.config = previous;
                    self.message = format!("Create failed: {}", error.safe_summary());
                } else {
                    self.selected = self.visible_indices().len().saturating_sub(1);
                    self.message = "Snippet created".into();
                    self.undo = Some(previous);
                }
            }
            Some(Prompt::EditReplacement { index, trigger }) => {
                let previous = self.config.clone();
                self.config.expansion[index].replacement = input;
                if let Err(error) = self.config.save_atomic(&self.path) {
                    self.config = previous;
                    self.message = format!("Edit failed: {}", error.safe_summary());
                } else {
                    self.message = format!("Updated {trigger}");
                    self.undo = Some(previous);
                }
            }
            Some(Prompt::EditDescription { index, trigger }) => {
                let previous = self.config.clone();
                self.config.expansion[index].description = input;
                if let Err(error) = self.config.save_atomic(&self.path) {
                    self.config = previous;
                    self.message = format!("Description edit failed: {}", error.safe_summary());
                } else {
                    self.message = format!("Updated description for {trigger}");
                    self.undo = Some(previous);
                }
            }
            Some(Prompt::EditTags { index, trigger }) => {
                let previous = self.config.clone();
                self.config.expansion[index].tags = input
                    .split(',')
                    .map(str::trim)
                    .filter(|tag| !tag.is_empty())
                    .map(str::to_owned)
                    .collect();
                if let Err(error) = self.config.save_atomic(&self.path) {
                    self.config = previous;
                    self.message = format!("Tag edit failed: {}", error.safe_summary());
                } else {
                    self.message = format!("Updated tags for {trigger}");
                    self.undo = Some(previous);
                }
            }
            None => {}
        }
    }

    fn preview(&self) -> String {
        let Some(index) = self.selected_index() else {
            return "No snippet selected".into();
        };
        let Ok(mut engine) = ExpansionEngine::new(self.config.clone()) else {
            return "Configuration is invalid".into();
        };
        let trigger = self.config.expansion[index].trigger.clone();
        let mut results = engine.process(InputEvent::Text(trigger));
        results.extend(engine.process(InputEvent::Boundary));
        results
            .last()
            .map(|result| result.insert.clone())
            .unwrap_or_else(|| "No expansion matched".into())
    }

    fn refresh_daemon_state(&mut self) {
        if self.last_status_poll.elapsed() < Duration::from_secs(1) {
            return;
        }
        self.last_status_poll = Instant::now();
        if let Ok(status) = control_command("status") {
            self.paused = status.lines().any(|line| line == "paused=true");
        }
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
    if let Ok(status) = control_command("status") {
        app.paused = status.lines().any(|line| line == "paused=true");
    }
    terminal::enable_raw_mode().context("enabling terminal input mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, terminal::EnterAlternateScreen, cursor::Hide)?;
    let result = run(&mut stdout, &mut app);
    let restore_result = execute!(stdout, cursor::Show, terminal::LeaveAlternateScreen)
        .and_then(|_| terminal::disable_raw_mode());
    result.and(restore_result.map_err(Into::into))
}

fn run(stdout: &mut io::Stdout, app: &mut App) -> Result<()> {
    loop {
        app.refresh_daemon_state();
        draw(stdout, app)?;
        if !event::poll(Duration::from_millis(250))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if handle_key(app, key)? {
            return Ok(());
        }
        if app.external_edit {
            app.external_edit = false;
            edit_with_external_editor(stdout, app)?;
        }
    }
}

fn handle_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    if let Some(index) = app.confirm_delete {
        match key.code {
            KeyCode::Char('d') | KeyCode::Enter => app.delete_confirmed(index),
            KeyCode::Esc => {
                app.confirm_delete = None;
                app.message = "Deletion cancelled".into();
            }
            _ => {}
        }
        return Ok(false);
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
        return Ok(false);
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
        return Ok(false);
    }
    match key {
        KeyEvent {
            code: KeyCode::Char('q') | KeyCode::Esc,
            ..
        } => return Ok(true),
        KeyEvent {
            code: KeyCode::Char('c'),
            modifiers: KeyModifiers::CONTROL,
            ..
        } => return Ok(true),
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
            code: KeyCode::Char('m'),
            ..
        } => app.toggle_match_mode(),
        KeyEvent {
            code: KeyCode::Char('E'),
            ..
        } => app.external_edit = true,
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
    Ok(false)
}

fn edit_with_external_editor(stdout: &mut io::Stdout, app: &mut App) -> Result<()> {
    let Some(index) = app.selected_index() else {
        app.message = "No snippet selected".into();
        return Ok(());
    };
    let temp = std::env::temp_dir().join(format!("wayexpand-edit-{}.tmp", std::process::id()));
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
        let editor = std::env::var("VISUAL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| std::env::var("EDITOR").ok())
            .unwrap_or_else(|| "vi".into());
        let status = std::process::Command::new(&editor)
            .arg(&temp)
            .status()
            .with_context(|| format!("starting editor {editor:?}"))?;
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
        candidate.save_atomic(&app.path).map_err(|error| {
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

fn draw(stdout: &mut io::Stdout, app: &App) -> Result<()> {
    let visible = app.visible_indices();
    let selected_config_index = app.selected_index();
    let preview = app.preview();
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
        Print("/ search   j/k move   n new   e edit   D description   t tags   m mode   d+d delete   u undo   space toggle   p pause   r reload   q quit"),
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
    for (row, index) in visible.iter().enumerate() {
        let expansion = &app.config.expansion[*index];
        let marker = if row == app.selected { "❯" } else { " " };
        let state = if expansion.enabled { "●" } else { "○" };
        if Some(*index) == selected_config_index {
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
    let path = env::var_os("WAYEXPAND_SOCKET")
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("XDG_RUNTIME_DIR").map(|dir| PathBuf::from(dir).join("wayexpand.sock"))
        })
        .context("XDG_RUNTIME_DIR or WAYEXPAND_SOCKET is required")?;
    let mut stream =
        UnixStream::connect(&path).with_context(|| format!("connecting to {}", path.display()))?;
    stream.set_read_timeout(Some(CONTROL_TIMEOUT))?;
    stream.set_write_timeout(Some(CONTROL_TIMEOUT))?;
    writeln!(stream, "{command}")?;
    let mut response = String::new();
    stream
        .take((MAX_CONTROL_RESPONSE_BYTES + 1) as u64)
        .read_to_string(&mut response)?;
    if response.len() > MAX_CONTROL_RESPONSE_BYTES {
        anyhow::bail!("daemon control response exceeded {MAX_CONTROL_RESPONSE_BYTES} bytes");
    }
    Ok(response)
}
