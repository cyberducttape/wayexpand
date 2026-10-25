//! `wayexpand new`: fill in a snippet's fields and write it to a file.
//!
//! This is for file templates such as Dockerfiles, Ansible playbooks and
//! Compose files. Typing a long, whitespace-sensitive file into an editor is
//! slow and fragile (editors re-indent typed YAML), so `new` renders the
//! snippet itself and writes the result. It needs no keyboard route or
//! running daemon.

use std::{
    collections::HashMap,
    io::{BufRead, IsTerminal, Read},
    os::fd::AsRawFd,
    os::unix::{ffi::OsStrExt, fs::OpenOptionsExt},
    process::{Command, ExitStatus, Stdio},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use wayexpand_core::{
    form_fields, render_template_with_cursor, ExpansionConfig, FormField, FormFieldKind,
    TemplateContext,
};
use wayexpand_process_supervisor::{configure_process_group, ChildSupervisor};

use super::Args;
use crate::*;

const USAGE: &str = "usage: wayexpand new [<name> [--set NAME=VALUE]... [-o FILE|--stdout] [--gui] [--force]] [--show NAME] [--json] [--config FILE]";

/// Templates that ship with WayExpand. A snippet in the user's configuration
/// with the same name takes precedence.
const BUILTIN_TEMPLATES: &str = include_str!("../../templates/builtin.toml");

/// Where a template came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    User,
    BuiltIn,
}

fn builtin_templates() -> Config {
    Config::parse(BUILTIN_TEMPLATES).expect("the built-in templates are valid (checked by a test)")
}

/// The user's configuration, or an empty one when the default file does not
/// exist yet: the built-in templates need no configuration.
fn load_user_config(path: &Path, explicit: bool) -> Result<Config> {
    match Config::load(path) {
        Ok(config) => Ok(config),
        Err(wayexpand_core::ConfigError::Read { source, .. })
            if !explicit && source.kind() == io::ErrorKind::NotFound =>
        {
            Ok(Config::parse("").expect("an empty configuration is valid"))
        }
        Err(error) => Err(config_load_error(path, error)),
    }
}

/// Bytes in one value given with `--set` or typed at a prompt.
const MAX_VALUE_BYTES: usize = 64 * 1024;
const GENERATED_FILE_MODE: u32 = 0o600;
const FORM_HELPER_TIMEOUT: Duration = Duration::from_secs(600);
const FORM_HELPER_DRAIN_TIMEOUT: Duration = Duration::from_millis(100);
const FORM_HELPER_READS_PER_DRAIN: usize = 64;

pub(crate) fn new_command(args: Args) -> Result<()> {
    let mut rest: Vec<String> = args.collect();
    let requested_json = take_json_flag(&mut rest);
    let force = take_flag(&mut rest, "--force");
    let to_stdout = take_flag(&mut rest, "--stdout");
    let gui = take_flag(&mut rest, "--gui");
    let output =
        match take_option(&mut rest, "--output").map_err(|error| usage_error(error.to_string()))? {
            Some(output) => Some(output),
            None => take_option(&mut rest, "-o").map_err(|error| usage_error(error.to_string()))?,
        };
    let show = take_option(&mut rest, "--show").map_err(|error| usage_error(error.to_string()))?;
    let explicit_path = take_option(&mut rest, "--config")
        .map_err(|error| usage_error(error.to_string()))?
        .map(PathBuf::from);
    let path = explicit_path.clone().unwrap_or_else(default_config_path);
    let mut assignments = Vec::new();
    while let Some(assignment) =
        take_option(&mut rest, "--set").map_err(|error| usage_error(error.to_string()))?
    {
        let Some((name, value)) = assignment.split_once('=') else {
            usage_bail!("--set needs NAME=VALUE, got {assignment:?}");
        };
        assignments.push((name.trim().to_owned(), value.to_owned()));
    }
    if rest.len() > 1
        || rest
            .first()
            .is_some_and(|argument| argument.starts_with('-'))
    {
        usage_bail!("{USAGE}");
    }
    let config = load_user_config(&path, explicit_path.is_some())?;
    let builtins = builtin_templates();
    let layers = [(&config, Source::User), (&builtins, Source::BuiltIn)];
    if let Some(name) = show {
        if !rest.is_empty()
            || output.is_some()
            || to_stdout
            || gui
            || force
            || !assignments.is_empty()
        {
            usage_bail!("{USAGE}");
        }
        let (snippet, _) = find_template(&layers, &name)?;
        print!("{}", snippet_toml(snippet)?);
        return Ok(());
    }
    let Some(name) = rest.into_iter().next() else {
        if output.is_some() || to_stdout || gui || force || !assignments.is_empty() {
            usage_bail!("{USAGE}");
        }
        print_templates(&layers, requested_json);
        return Ok(());
    };
    if requested_json {
        usage_bail!("--json lists templates; it does not apply to `wayexpand new <name>`");
    }
    if to_stdout && output.is_some() {
        usage_bail!("use either --stdout or -o FILE, not both");
    }
    let (snippet, _) = find_template(&layers, &name)?;
    if snippet.command.is_some() {
        usage_bail!(
            "{} runs a command; only text snippets can be written with `wayexpand new`",
            snippet.trigger
        );
    }
    let fields = form_fields(&snippet.replacement)
        .map_err(|error| config_error(format!("{}: {error}", snippet.trigger)))?;
    let mut values = values_from_assignments(&fields, assignments)?;
    let missing: Vec<&FormField> = fields
        .iter()
        .filter(|field| !values.contains_key(&field.key))
        .collect();
    if !missing.is_empty() {
        if gui {
            values.extend(ask_with_form(&snippet.trigger, &missing)?);
        } else if io::stdin().is_terminal() {
            values.extend(ask_in_terminal(&missing)?);
        } else {
            // Not interactive: defaults apply as they would on Enter, and
            // only a field without one is an error.
            let mut unanswered = Vec::new();
            for field in missing {
                match default_value(field) {
                    Some(value) => {
                        values.insert(field.key.clone(), value);
                    }
                    None => unanswered.push(field.label.as_str()),
                }
            }
            if !unanswered.is_empty() {
                usage_bail!(
                    "no value for {}; pass --set NAME=VALUE, --gui, or run in a terminal",
                    unanswered.join(", ")
                );
            }
        }
    }
    let context = TemplateContext {
        fields: Arc::new(values),
        ..config.template_context(None)
    };
    let (rendered, _) = render_template_with_cursor(&snippet.replacement, &context)
        .map_err(|error| config_error(format!("{}: {error}", snippet.trigger)))?;

    let destination = if to_stdout {
        None
    } else {
        output
            .or_else(|| snippet.output_file.clone())
            .filter(|file| file != "-")
    };
    match destination {
        None => {
            io::stdout().write_all(rendered.as_bytes())?;
            io::stdout().flush()?;
        }
        Some(file) => {
            write_output(Path::new(&file), rendered.as_bytes(), force)?;
            eprintln!("Wrote {file}");
        }
    }
    Ok(())
}

/// A snippet as TOML to paste into a configuration, without its id (one is
/// assigned when the configuration is first loaded).
fn snippet_toml(snippet: &ExpansionConfig) -> Result<String> {
    let mut table = toml::Table::try_from(snippet).context("could not serialize the snippet")?;
    table.remove("id");
    // Most telling keys first and the long replacement last; toml::Table
    // itself is sorted.
    let order = ["trigger", "output_file", "description", "category", "tags"];
    let mut keys: Vec<String> = order
        .iter()
        .map(|key| (*key).to_owned())
        .filter(|key| table.contains_key(key))
        .collect();
    keys.extend(
        table
            .keys()
            .filter(|key| !order.contains(&key.as_str()) && *key != "replacement")
            .cloned(),
    );
    keys.push("replacement".into());
    let mut text = String::from("[[expansion]]\n");
    for key in keys {
        if let Some(value) = table.remove(&key) {
            let mut single = toml::Table::new();
            single.insert(key, value);
            text.push_str(&toml::to_string(&single).context("could not serialize the snippet")?);
        }
    }
    Ok(text)
}

fn take_flag(args: &mut Vec<String>, flag: &str) -> bool {
    match args.iter().position(|argument| argument == flag) {
        Some(position) => {
            args.remove(position);
            true
        }
        None => false,
    }
}

/// A short name for a snippet: its trigger without leading and trailing
/// punctuation, so `;;docker` is `docker`.
fn short_name(trigger: &str) -> String {
    trigger
        .trim_matches(|character: char| !character.is_alphanumeric())
        .to_lowercase()
}

/// Find the snippet `name` refers to, searching the user's configuration
/// before the built-in templates.
fn find_template<'a>(
    layers: &[(&'a Config, Source)],
    name: &str,
) -> Result<(&'a ExpansionConfig, Source)> {
    for (config, source) in layers {
        // An ambiguous name in the user's layer is an error rather than a
        // reason to fall through to a built-in template.
        if let Some(snippet) = find_in(config, name)? {
            return Ok((snippet, *source));
        }
    }
    Err(usage_error(format!(
        "no enabled snippet is called {name:?}; `wayexpand new` lists file templates"
    )))
}

/// The snippet `name` refers to in one configuration: an exact trigger or
/// alias first, then a trigger's short name, then the file name it writes.
fn find_in<'a>(config: &'a Config, name: &str) -> Result<Option<&'a ExpansionConfig>> {
    let enabled = || {
        config
            .expansion
            .iter()
            .filter(|expansion| expansion.enabled)
    };
    if let Some(exact) = enabled().find(|expansion| expansion.answers_to(name)) {
        return Ok(Some(exact));
    }
    let wanted = name.to_lowercase();
    let by_short_name = |expansion: &&ExpansionConfig| {
        std::iter::once(&expansion.trigger)
            .chain(&expansion.aliases)
            .any(|trigger| short_name(trigger) == wanted)
    };
    let by_file_name = |expansion: &&ExpansionConfig| {
        expansion.output_file.as_deref().is_some_and(|file| {
            let file = Path::new(file);
            [file.file_name(), file.file_stem()]
                .into_iter()
                .flatten()
                .any(|part| part.to_string_lossy().to_lowercase() == wanted)
        })
    };
    // A name match beats a file-name match: `dockerfile` is ;;dockerfile,
    // not every template that writes a Dockerfile.
    for candidates in [
        enabled().filter(by_short_name).collect::<Vec<_>>(),
        enabled().filter(by_file_name).collect::<Vec<_>>(),
    ] {
        match candidates.as_slice() {
            [] => {}
            [only] => return Ok(Some(only)),
            several => {
                let triggers: Vec<&str> = several
                    .iter()
                    .map(|snippet| snippet.trigger.as_str())
                    .collect();
                return Err(usage_error(format!(
                    "{name:?} matches {}; use the full trigger",
                    triggers.join(", ")
                )));
            }
        }
    }
    Ok(None)
}

fn print_templates(layers: &[(&Config, Source)], requested_json: bool) {
    // A built-in template is hidden when its name now finds a user snippet.
    let templates: Vec<(&ExpansionConfig, Source)> = layers
        .iter()
        .flat_map(|(config, source)| {
            config
                .expansion
                .iter()
                .filter(|expansion| {
                    expansion.enabled
                        && expansion.command.is_none()
                        && expansion.output_file.is_some()
                })
                .map(move |expansion| (expansion, *source))
        })
        .filter(|(expansion, source)| {
            *source == Source::User
                || find_template(layers, &short_name(&expansion.trigger))
                    .is_ok_and(|(found, _)| std::ptr::eq(found, *expansion))
        })
        .collect();
    if requested_json {
        let entries: Vec<serde_json::Value> = templates
            .iter()
            .map(|(snippet, source)| {
                let fields: Vec<String> = form_fields(&snippet.replacement)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|field| field.label)
                    .collect();
                serde_json::json!({
                    "trigger": snippet.trigger,
                    "name": short_name(&snippet.trigger),
                    "output_file": snippet.output_file,
                    "description": snippet.description,
                    "fields": fields,
                    "built_in": *source == Source::BuiltIn,
                })
            })
            .collect();
        println!("{}", serde_json::json!({ "templates": entries }));
        return;
    }
    if templates.is_empty() {
        println!("No file templates. Give a snippet an output_file to list it here, for example:");
        println!();
        println!("  [[expansion]]");
        println!("  trigger = \";;dockerfile\"");
        println!("  output_file = \"Dockerfile\"");
        println!("  replacement = \"\"\"FROM {{{{field:image=debian:stable-slim}}}}\n  \"\"\"");
        return;
    }
    let width = templates
        .iter()
        .map(|(snippet, _)| short_name(&snippet.trigger).chars().count())
        .max()
        .unwrap_or(0);
    let file_width = templates
        .iter()
        .map(|(snippet, _)| {
            snippet
                .output_file
                .as_deref()
                .unwrap_or_default()
                .chars()
                .count()
        })
        .max()
        .unwrap_or(0);
    for (snippet, source) in templates {
        let mut line = format!(
            "{:<width$}  {:<file_width$}",
            short_name(&snippet.trigger),
            snippet.output_file.as_deref().unwrap_or_default()
        );
        if !snippet.description.is_empty() {
            line.push_str(&format!("  {}", snippet.description));
        }
        if source == Source::User {
            line.push_str("  (yours)");
        }
        println!("{}", line.trim_end());
    }
    println!();
    println!("Run `wayexpand new <name>` to fill one in; --set NAME=VALUE skips a question.");
    println!("`wayexpand new --show <name>` prints one as a snippet you can copy and change.");
}

/// Match `--set` names to fields by label (`--set port=8080` fills
/// `{{field:port}}` and `{{choice:port=...}}`). Unknown names and choice
/// values outside the options are errors rather than ignored.
fn values_from_assignments(
    fields: &[FormField],
    assignments: Vec<(String, String)>,
) -> Result<HashMap<String, String>> {
    let mut values = HashMap::new();
    for (name, value) in assignments {
        let Some(field) = fields.iter().find(|field| field.label == name) else {
            let names: Vec<&str> = fields.iter().map(|field| field.label.as_str()).collect();
            usage_bail!(
                "this template has no field {name:?}; its fields are: {}",
                if names.is_empty() {
                    "(none)".to_owned()
                } else {
                    names.join(", ")
                }
            );
        };
        check_value(field, &value)?;
        values.insert(field.key.clone(), value);
    }
    Ok(values)
}

/// What Enter accepts: a text field's default, or a choice's first option.
fn default_value(field: &FormField) -> Option<String> {
    match &field.kind {
        FormFieldKind::Text { default } if default.is_empty() => None,
        FormFieldKind::Text { default } => Some(default.clone()),
        FormFieldKind::Choice { options } => options.first().cloned(),
    }
}

fn check_value(field: &FormField, value: &str) -> Result<()> {
    if value.len() > MAX_VALUE_BYTES || value.contains('\0') {
        usage_bail!("the value for {} is too long or contains NUL", field.label);
    }
    if let FormFieldKind::Choice { options } = &field.kind {
        if !options.iter().any(|option| option == value) {
            usage_bail!("{} must be one of: {}", field.label, options.join(", "));
        }
    }
    Ok(())
}

fn ask_in_terminal(fields: &[&FormField]) -> Result<HashMap<String, String>> {
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    let mut values = HashMap::new();
    for field in fields {
        let value = loop {
            match &field.kind {
                FormFieldKind::Text { default } if default.is_empty() => {
                    eprint!("{}: ", field.label)
                }
                FormFieldKind::Text { default } => eprint!("{} [{default}]: ", field.label),
                FormFieldKind::Choice { options } => {
                    eprintln!("{}:", field.label);
                    for (number, option) in options.iter().enumerate() {
                        eprintln!("  {}) {option}", number + 1);
                    }
                    eprint!("Choose 1-{} [1]: ", options.len());
                }
            }
            io::stderr().flush()?;
            let Some(line) = lines.next().transpose()? else {
                usage_bail!("input ended before {} was answered", field.label);
            };
            let answer = line.trim_end_matches(['\r', '\n']);
            let value = match &field.kind {
                FormFieldKind::Text { .. } if answer.is_empty() => {
                    default_value(field).unwrap_or_default()
                }
                FormFieldKind::Text { .. } => answer.to_owned(),
                FormFieldKind::Choice { options } => {
                    let answer = answer.trim();
                    if answer.is_empty() {
                        default_value(field).unwrap_or_default()
                    } else if let Some(option) = answer
                        .parse::<usize>()
                        .ok()
                        .and_then(|number| number.checked_sub(1))
                        .and_then(|index| options.get(index))
                    {
                        option.clone()
                    } else if options.iter().any(|option| option == answer) {
                        answer.to_owned()
                    } else {
                        eprintln!("Please pick a number from the list.");
                        continue;
                    }
                }
            };
            match check_value(field, &value) {
                Ok(()) => break value,
                Err(error) => eprintln!("{error}"),
            }
        };
        values.insert(field.key.clone(), value);
    }
    Ok(values)
}

/// Ask with the same window the daemon uses for form snippets. It prints
/// the values as a JSON object and exits 0, or exits non-zero on cancel.
fn ask_with_form(title: &str, fields: &[&FormField]) -> Result<HashMap<String, String>> {
    let spec = serde_json::json!({ "title": title, "fields": fields }).to_string();
    let program = env::var_os("WAYEXPAND_FORM_HELPER")
        .map(PathBuf::from)
        .or_else(|| {
            env::current_exe()
                .ok()
                .and_then(|exe| exe.parent().map(|dir| dir.join("wayexpand-gui")))
                .filter(|candidate| candidate.is_file())
        })
        .unwrap_or_else(|| PathBuf::from("wayexpand-gui"));
    let mut command = Command::new(&program);
    command
        .args(["--form", &spec])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    let (status, output) = run_form_helper_process(command, FORM_HELPER_TIMEOUT)
        .with_context(|| format!("could not run {}", program.display()))?;
    if !status.success() {
        bail!("cancelled; nothing was written");
    }
    let values: HashMap<String, String> =
        serde_json::from_slice(&output).context("the form window returned an unreadable answer")?;
    let mut checked = HashMap::new();
    for field in fields {
        let value = values
            .get(&field.key)
            .with_context(|| format!("the form window returned no value for {}", field.label))?;
        check_value(field, value)?;
        checked.insert(field.key.clone(), value.clone());
    }
    Ok(checked)
}

fn set_nonblocking_form_stdout(stdout: &std::process::ChildStdout) -> io::Result<()> {
    let fd = stdout.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn drain_form_stdout(
    stdout: &mut std::process::ChildStdout,
    output: &mut Vec<u8>,
) -> io::Result<bool> {
    let mut buffer = [0_u8; 8192];
    for _ in 0..FORM_HELPER_READS_PER_DRAIN {
        match stdout.read(&mut buffer) {
            Ok(0) => return Ok(true),
            Ok(count) => {
                let next_len = output.len().checked_add(count).ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "form output size overflow")
                })?;
                if next_len > wayexpand_core::limits::MAX_FORM_OUTPUT_BYTES {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "form output exceeded its size limit",
                    ));
                }
                output.extend_from_slice(&buffer[..count]);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(false),
            Err(error) => return Err(error),
        }
    }
    Ok(false)
}

/// Run the interactive form with bounded output and a hard deadline. Killing
/// the whole process group also closes stdout if a helper descendant inherits
/// its descriptor after the GUI process exits.
fn run_form_helper_process(
    mut command: Command,
    timeout: Duration,
) -> Result<(ExitStatus, Vec<u8>)> {
    configure_process_group(&mut command);
    let child = command.spawn().context("could not start the form helper")?;
    let mut supervisor = ChildSupervisor::new(child);
    let mut stdout = match supervisor.child_mut().and_then(|child| child.stdout.take()) {
        Some(stdout) => stdout,
        None => {
            supervisor.kill_group();
            let _ = supervisor.reap();
            bail!("form helper stdout was not captured");
        }
    };
    if let Err(error) = set_nonblocking_form_stdout(&stdout) {
        supervisor.kill_group();
        let _ = supervisor.reap();
        return Err(error).context("could not configure form output");
    }

    let deadline = Instant::now()
        .checked_add(timeout)
        .context("form helper timeout is too large")?;
    let mut output = Vec::new();
    loop {
        let mut stdout_eof = match drain_form_stdout(&mut stdout, &mut output) {
            Ok(eof) => eof,
            Err(error) => {
                supervisor.kill_group();
                let _ = supervisor.reap();
                return Err(error).context("could not read form output");
            }
        };
        let exited = match supervisor.has_exited() {
            Ok(exited) => exited,
            Err(error) => {
                supervisor.kill_group();
                let _ = supervisor.reap();
                return Err(error).context("could not monitor form helper");
            }
        };
        if exited {
            supervisor.kill_group();
            let status = supervisor.reap().context("could not reap form helper")?;
            let drain_deadline = Instant::now() + FORM_HELPER_DRAIN_TIMEOUT;
            while !stdout_eof && Instant::now() < drain_deadline {
                stdout_eof = match drain_form_stdout(&mut stdout, &mut output) {
                    Ok(eof) => eof,
                    Err(error) => return Err(error).context("could not finish form output"),
                };
                if !stdout_eof {
                    thread::sleep(Duration::from_millis(5));
                }
            }
            if !stdout_eof {
                bail!("form helper stdout did not close after process-group cleanup");
            }
            return Ok((status, output));
        }
        if Instant::now() >= deadline {
            supervisor.kill_group();
            let _ = supervisor.reap();
            bail!("form helper exceeded its deadline");
        }
        thread::sleep(Duration::from_millis(5));
    }
}

/// Write `contents` to `path`, creating missing parent directories. An
/// existing file is never replaced without `force`; with it, the new file is
/// written beside the old one and renamed over it, so a failure leaves the
/// old file intact.
fn write_output(path: &Path, contents: &[u8], force: bool) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("could not create {}", parent.display()))?;
    }
    if !force && path.exists() {
        usage_bail!(
            "{} already exists; add --force to replace it",
            path.display()
        );
    }
    let file_name = path
        .file_name()
        .with_context(|| format!("{} is not a file name", path.display()))?;
    let temporary = temporary_output_path(path, file_name)?;
    let result = (|| -> io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(GENERATED_FILE_MODE)
            .open(&temporary)?;
        if let Ok(existing) = fs::metadata(path) {
            // Keep an executable script executable when it is replaced.
            file.set_permissions(existing.permissions())?;
        }
        file.write_all(contents)?;
        file.sync_all()?;
        if force {
            fs::rename(&temporary, path)?;
        } else {
            rename_without_replacement(&temporary, path)?;
        }
        sync_parent_directory(path)
    })();
    if let Err(error) = result {
        let _ = fs::remove_file(&temporary);
        return Err(error).with_context(|| format!("could not write {}", path.display()));
    }
    Ok(())
}

fn temporary_output_path(path: &Path, file_name: &std::ffi::OsStr) -> Result<PathBuf> {
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random).context("could not obtain randomness for temporary output")?;
    let suffix = random
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(path.with_file_name(format!(
        ".{}.wayexpand-new-{suffix}",
        file_name.to_string_lossy()
    )))
}

#[cfg(target_os = "linux")]
fn rename_without_replacement(from: &Path, to: &Path) -> io::Result<()> {
    let from_path = from;
    let to_path = to;
    let from = std::ffi::CString::new(from_path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "temporary path contains NUL"))?;
    let to = std::ffi::CString::new(to_path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "output path contains NUL"))?;
    // RENAME_NOREPLACE makes the create path atomic even when another writer
    // races the initial existence check.
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            from.as_ptr(),
            libc::AT_FDCWD,
            to.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if !matches!(
        error.raw_os_error(),
        Some(libc::ENOSYS | libc::EINVAL | libc::EOPNOTSUPP)
    ) {
        return Err(error);
    }
    link_without_replacement(from_path, to_path)
}

#[cfg(not(target_os = "linux"))]
fn rename_without_replacement(from: &Path, to: &Path) -> io::Result<()> {
    link_without_replacement(from, to)
}

/// A hard link creates the destination with no-replacement semantics on Unix.
/// Both paths are in the same parent, so this is also a portable fallback when
/// Linux's renameat2 is unavailable.
fn link_without_replacement(from: &Path, to: &Path) -> io::Result<()> {
    fs::hard_link(from, to)?;
    fs::remove_file(from)
}

fn sync_parent_directory(path: &Path) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::File::open(parent)?.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn form_helper_timeout_kills_its_process_group() {
        let mut command = Command::new("sh");
        command
            .args(["-c", "sleep 30 & wait"])
            .stdout(Stdio::piped());
        let started = Instant::now();
        let error = run_form_helper_process(command, Duration::from_millis(75))
            .expect_err("the hanging form helper should time out");
        assert!(error.to_string().contains("deadline"));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn form_helper_reaps_descendant_holding_stdout_after_parent_exit() {
        let mut command = Command::new("sh");
        command
            .args(["-c", "sleep 30 & printf '{\"answer\":\"ok\"}'"])
            .stdout(Stdio::piped());
        let started = Instant::now();
        let (status, output) = run_form_helper_process(command, Duration::from_secs(2))
            .expect("the parent result should be collected after group cleanup");
        assert!(status.success());
        assert_eq!(output, br#"{"answer":"ok"}"#);
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn form_helper_output_is_bounded() {
        let mut command = Command::new("sh");
        command
            .args(["-c", "head -c 2097153 /dev/zero"])
            .stdout(Stdio::piped());
        let started = Instant::now();
        let error = run_form_helper_process(command, Duration::from_secs(2))
            .expect_err("oversized form output should be rejected");
        assert!(format!("{error:#}").contains("size limit"));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    fn config(text: &str) -> Config {
        Config::parse(text).unwrap()
    }

    #[test]
    fn generated_files_are_published_atomically_with_private_permissions() {
        let root = std::env::temp_dir().join(format!(
            "wayexpand-new-test-{}-{}",
            std::process::id(),
            getrandom::u64().unwrap()
        ));
        let path = root.join("nested/output.txt");
        write_output(&path, b"first", false).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"first");
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(write_output(&path, b"refused", false).is_err());
        write_output(&path, b"second", true).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"second");
        assert!(fs::read_dir(path.parent().unwrap())
            .unwrap()
            .all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains("wayexpand-new")));
        fs::remove_dir_all(root).unwrap();
    }

    const PLAYBOOK: &str = r#"
[[expansion]]
trigger = ";;playbook"
output_file = "site.yml"
replacement = """- hosts: {{field:hosts=all}}
  tasks:
    - ansible.builtin.package:
        name: "{{{{ item }}}}"
        state: {{choice:state=present|latest|absent}}
      loop: [{{field:package}}]
"""

[[expansion]]
trigger = ";;dockerfile"
output_file = "docker/Dockerfile"
replacement = "FROM {{field:image=debian:stable-slim}}\n"

[[expansion]]
trigger = ";;sig"
replacement = "Regards"
"#;

    fn found(layers: &[(&Config, Source)], name: &str) -> (String, Source) {
        let (snippet, source) = find_template(layers, name).unwrap();
        (snippet.trigger.clone(), source)
    }

    #[test]
    fn templates_are_found_by_trigger_short_name_or_file_name() {
        let config = config(PLAYBOOK);
        let layers = [(&config, Source::User)];
        assert_eq!(found(&layers, ";;playbook").0, ";;playbook");
        assert_eq!(found(&layers, "playbook").0, ";;playbook");
        assert_eq!(found(&layers, "site").0, ";;playbook");
        assert_eq!(found(&layers, "Dockerfile").0, ";;dockerfile");
        assert!(find_template(&layers, "nothing").is_err());
    }

    #[test]
    fn user_templates_take_precedence_over_built_in_ones() {
        let user = config(PLAYBOOK);
        let builtins = builtin_templates();
        let layers = [(&user, Source::User), (&builtins, Source::BuiltIn)];
        assert_eq!(
            found(&layers, "playbook"),
            (";;playbook".into(), Source::User)
        );
        assert_eq!(
            found(&layers, "compose"),
            (";;compose".into(), Source::BuiltIn)
        );
        // `dockerfile` is the user's ;;dockerfile, not the built-in one.
        assert_eq!(
            found(&layers, "dockerfile"),
            (";;dockerfile".into(), Source::User)
        );
    }

    #[test]
    fn every_built_in_template_renders_with_its_defaults() {
        let builtins = builtin_templates();
        assert!(builtins.expansion.len() >= 10);
        for snippet in &builtins.expansion {
            assert!(snippet.output_file.is_some(), "{}", snippet.trigger);
            let fields = form_fields(&snippet.replacement).unwrap();
            let values: HashMap<String, String> = fields
                .iter()
                .map(|field| {
                    (
                        field.key.clone(),
                        default_value(field).unwrap_or_else(|| "value".into()),
                    )
                })
                .collect();
            let context = TemplateContext {
                fields: Arc::new(values),
                ..builtins.template_context(None)
            };
            let (rendered, _) = render_template_with_cursor(&snippet.replacement, &context)
                .unwrap_or_else(|error| panic!("{}: {error}", snippet.trigger));
            assert!(
                rendered.ends_with('\n'),
                "{} ends without a newline",
                snippet.trigger
            );
            assert!(!rendered.contains("{{field:"), "{}", snippet.trigger);
            // The built-in templates are each found by their short name.
            let layers = [(&builtins, Source::BuiltIn)];
            assert_eq!(
                found(&layers, &short_name(&snippet.trigger)).0,
                snippet.trigger
            );
        }
        let playbook = builtins
            .expansion
            .iter()
            .find(|snippet| snippet.trigger == ";;playbook");
        let (rendered, _) = render_template_with_cursor(
            &playbook.unwrap().replacement,
            &TemplateContext {
                fields: Arc::new(
                    form_fields(&playbook.unwrap().replacement)
                        .unwrap()
                        .iter()
                        .map(|field| (field.key.clone(), default_value(field).unwrap()))
                        .collect(),
                ),
                ..TemplateContext::default()
            },
        )
        .unwrap();
        assert!(rendered.contains("name: \"{{ item }}\""));
        assert!(rendered.contains("loop: \"{{ packages }}\""));
    }

    #[test]
    fn a_shown_template_is_a_valid_snippet_without_an_id() {
        let builtins = builtin_templates();
        let layers = [(&builtins, Source::BuiltIn)];
        let (snippet, _) = find_template(&layers, "playbook").unwrap();
        let text = snippet_toml(snippet).unwrap();
        assert!(!text.contains("id ="), "{text}");
        let parsed = Config::parse(&text).unwrap();
        assert_eq!(parsed.expansion[0].replacement, snippet.replacement);
        assert_eq!(parsed.expansion[0].output_file, snippet.output_file);
    }

    #[test]
    fn set_values_fill_fields_by_label_and_check_choices() {
        let config = config(PLAYBOOK);
        let (snippet, _) = find_template(&[(&config, Source::User)], "playbook").unwrap();
        let fields = form_fields(&snippet.replacement).unwrap();
        let values = values_from_assignments(
            &fields,
            vec![
                ("state".into(), "latest".into()),
                ("package".into(), "nginx".into()),
            ],
        )
        .unwrap();
        assert_eq!(
            values.get("choice:state").map(String::as_str),
            Some("latest")
        );
        assert_eq!(
            values.get("field:package").map(String::as_str),
            Some("nginx")
        );
        assert!(values_from_assignments(&fields, vec![("state".into(), "gone".into())]).is_err());
        assert!(values_from_assignments(&fields, vec![("port".into(), "80".into())]).is_err());
    }

    #[test]
    fn output_is_never_overwritten_without_force() {
        let directory = std::env::temp_dir().join(format!("wayexpand-new-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        let target = directory.join("nested/Dockerfile");
        write_output(&target, b"FROM one\n", false).unwrap();
        assert!(write_output(&target, b"FROM two\n", false).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"FROM one\n");
        write_output(&target, b"FROM two\n", true).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"FROM two\n");
        let leftovers: Vec<_> = fs::read_dir(target.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(leftovers, [std::ffi::OsString::from("Dockerfile")]);
        fs::remove_dir_all(&directory).unwrap();
    }
}
