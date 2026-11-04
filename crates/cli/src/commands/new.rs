//! `wayexpand new`: fill in a snippet's fields and write it to a file.
//!
//! This is for file templates such as Dockerfiles, Ansible playbooks and
//! Compose files. Typing a long, whitespace-sensitive file into an editor is
//! slow and fragile (editors re-indent typed YAML), so `new` renders the
//! snippet itself and writes the result. It needs no keyboard route or
//! running daemon.

use std::{
    collections::HashMap,
    ffi::OsString,
    io::{BufRead, IsTerminal, Read},
    os::fd::AsRawFd,
    os::unix::{
        ffi::OsStrExt,
        fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    },
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

use crate::*;

const USAGE: &str = "usage: wayexpand new [<name> [--set NAME=VALUE]... [-o PATH|--stdout] [--gui] [--force] [--validate] [--review]] [--show NAME] [--json] [--config FILE]";

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
const ANSIBLE_ROLE_TRIGGER: &str = ";;ansible-role";
const VALIDATOR_TIMEOUT: Duration = Duration::from_secs(30);
const ANSIBLE_ROLE_COMPANIONS: [(&str, &str); 3] = [
    (
        "handlers/main.yml",
        "---\n- name: \"Restart {{field:service=nginx}}\"\n  ansible.builtin.service:\n    name: \"{{field:service=nginx}}\"\n    state: restarted\n",
    ),
    (
        "defaults/main.yml",
        "---\nnginx_listen_port: \"{{field:port=80}}\"\nserver_name: \"{{field:hostname=localhost}}\"\n",
    ),
    (
        "templates/default.conf.j2",
        "server {\n    listen {{{{ nginx_listen_port }}}};\n    server_name {{{{ server_name }}}};\n\n    location / {\n        try_files $uri $uri/ =404;\n    }\n}\n",
    ),
];
const FORM_HELPER_TIMEOUT: Duration = Duration::from_secs(600);
// A killed descendant may need scheduler time to close its inherited pipe.
// Keep that cleanup bounded, but leave enough room for loaded CI and desktops.
const FORM_HELPER_DRAIN_TIMEOUT: Duration = Duration::from_millis(500);
const FORM_HELPER_READS_PER_DRAIN: usize = 64;

pub(crate) fn new_command(args: impl Iterator<Item = String>) -> Result<()> {
    let mut rest: Vec<String> = args.collect();
    let requested_json = take_json_flag(&mut rest);
    let force = take_flag(&mut rest, "--force");
    let validate = take_flag(&mut rest, "--validate");
    let review = take_flag(&mut rest, "--review");
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
            || validate
            || review
            || !assignments.is_empty()
        {
            usage_bail!("{USAGE}");
        }
        let (snippet, _) = find_template(&layers, &name)?;
        print!("{}", snippet_toml(snippet)?);
        return Ok(());
    }
    let Some(name) = rest.into_iter().next() else {
        if output.is_some()
            || to_stdout
            || gui
            || force
            || validate
            || review
            || !assignments.is_empty()
        {
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
    let (snippet, source) = find_template(&layers, &name)?;
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
                match default_value(field)? {
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
        fields: Arc::new(values.into()),
        ..config.template_context(None)
    };
    let (rendered, _) = render_template_with_cursor(&snippet.replacement, &context)
        .map_err(|error| config_error(format!("{}: {error}", snippet.trigger)))?;

    let is_ansible_role = source == Source::BuiltIn && snippet.trigger == ANSIBLE_ROLE_TRIGGER;
    if is_ansible_role {
        if to_stdout {
            usage_bail!("ansible-role generates a directory; use -o DIR to choose its location");
        }
        let default_directory = snippet
            .output_file
            .as_deref()
            .context("the built-in ansible role has no output directory")?;
        let (default_directory, _) = render_template_with_cursor(default_directory, &context)
            .map_err(|error| {
                config_error(format!("{} output directory: {error}", snippet.trigger))
            })?;
        let directory = output.as_deref().unwrap_or(&default_directory);
        if directory == "-" {
            usage_bail!("ansible-role writes a directory and cannot use -o -");
        }
        validate_role_output_directory(Path::new(directory))?;

        let files = render_ansible_role_files(rendered, &context)?;
        if validate {
            let validator = validator_for(&snippet.trigger).ok_or_else(|| {
                usage_error(format!(
                    "no registered validator supports template {}",
                    short_name(&snippet.trigger)
                ))
            })?;
            let role_name = context
                .fields
                .get("field:role")
                .context("Ansible role name was not rendered")?;
            let workspace = ValidationWorkspace::new()?;
            let staged_role = workspace
                .path()
                .join("roles")
                .join(validate_role_name(role_name)?);
            write_scaffold_directory(&staged_role, &files, false)?;
            validator.validate(
                workspace.path(),
                &ValidationTarget::AnsibleRole {
                    path: staged_role,
                    name: role_name.to_owned(),
                },
            )?;
            eprintln!("Validated with {}", validator.name());
        }
        if review {
            review_scaffold(Path::new(directory), &files, force)?;
        }
        write_scaffold_directory(Path::new(directory), &files, force)?;
        eprintln!("Wrote Ansible role to {directory}");
        return Ok(());
    }

    let destination = if to_stdout {
        None
    } else {
        output
            .or_else(|| snippet.output_file.clone())
            .filter(|file| file != "-")
    };
    if validate {
        let validator = validator_for(&snippet.trigger).ok_or_else(|| {
            usage_error(format!(
                "no registered validator supports template {}",
                short_name(&snippet.trigger)
            ))
        })?;
        let relative_path = rendered_output_path(snippet, &context)?;
        let workspace = ValidationWorkspace::new()?;
        let staged_path = workspace.write_file(&relative_path, rendered.as_bytes())?;
        validator.validate(workspace.path(), &ValidationTarget::File(staged_path))?;
        eprintln!("Validated with {}", validator.name());
    }
    if review {
        let destination = destination.as_deref().ok_or_else(|| {
            usage_error("reviewed generation requires a file destination; use -o PATH")
        })?;
        review_file(Path::new(destination), &rendered, force)?;
    }
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

fn render_ansible_role_files(
    tasks: String,
    context: &TemplateContext,
) -> Result<Vec<(String, Vec<u8>)>> {
    let mut files = vec![("tasks/main.yml".into(), tasks.into_bytes())];
    for (relative_path, template) in ANSIBLE_ROLE_COMPANIONS {
        let (contents, _) = render_template_with_cursor(template, context)
            .map_err(|error| config_error(format!("{}: {error}", relative_path)))?;
        files.push((relative_path.into(), contents.into_bytes()));
    }
    Ok(files)
}

/// Validators are registered in WayExpand and invoked only after an explicit
/// `--validate`. A template cannot provide a program, shell fragment, or
/// validator arguments.
trait ArtifactValidator: Sync {
    fn name(&self) -> &'static str;
    fn supports(&self, template_trigger: &str) -> bool;
    fn validate(&self, workspace: &Path, target: &ValidationTarget) -> Result<()>;
}

enum ValidationTarget {
    File(PathBuf),
    AnsibleRole { path: PathBuf, name: String },
}

struct AnsibleValidator;

impl ArtifactValidator for AnsibleValidator {
    fn name(&self) -> &'static str {
        "ansible-playbook --syntax-check"
    }

    fn supports(&self, template_trigger: &str) -> bool {
        matches!(template_trigger, ";;playbook" | ANSIBLE_ROLE_TRIGGER)
    }

    fn validate(&self, workspace: &Path, target: &ValidationTarget) -> Result<()> {
        let mut command = Command::new("ansible-playbook");
        command
            .args(["--syntax-check", "--inventory", "localhost,"])
            .current_dir(workspace);
        // Modern Ansible rejects /dev/null as a config because it has no
        // recognized extension. Use an empty private config to avoid loading
        // user or system Ansible configuration.
        fs::write(workspace.join("wayexpand-ansible.cfg"), "[defaults]\n")?;
        let mut roles_path = None;
        match target {
            ValidationTarget::File(path) => {
                command.arg(path);
            }
            ValidationTarget::AnsibleRole { path, name } => {
                if !path.is_dir() {
                    bail!("staged Ansible role directory is missing");
                }
                let wrapper = workspace.join("wayexpand-validation.yml");
                path.strip_prefix(workspace)
                    .context("staged role escaped its validation workspace")?;
                // The role name has already been restricted to a single safe
                // path component; quote it as YAML data in the wrapper.
                fs::write(
                    &wrapper,
                    format!(
                        "---\n- hosts: localhost\n  connection: local\n  roles:\n    - name: {name}\n      tasks_from: main\n"
                    ),
                )?;
                let role_search_path = path
                    .parent()
                    .context("staged role has no roles directory")?;
                command.arg(&wrapper);
                roles_path = Some(role_search_path.to_path_buf());
            }
        }
        prepare_validator_environment(&mut command, workspace);
        if let Some(roles_path) = roles_path {
            command.env("ANSIBLE_ROLES_PATH", roles_path);
        }
        run_validator_process(command, self.name())
    }
}

struct FileCommandValidator {
    name: &'static str,
    executable: &'static str,
    triggers: &'static [&'static str],
    arguments: fn(&Path) -> Vec<OsString>,
}

impl ArtifactValidator for FileCommandValidator {
    fn name(&self) -> &'static str {
        self.name
    }

    fn supports(&self, template_trigger: &str) -> bool {
        self.triggers.contains(&template_trigger)
    }

    fn validate(&self, workspace: &Path, target: &ValidationTarget) -> Result<()> {
        let ValidationTarget::File(path) = target else {
            bail!("{} only validates a single generated file", self.name);
        };
        let mut command = Command::new(self.executable);
        command.args((self.arguments)(path)).current_dir(workspace);
        prepare_validator_environment(&mut command, workspace);
        run_validator_process(command, self.name)
    }
}

struct NginxValidator;

impl ArtifactValidator for NginxValidator {
    fn name(&self) -> &'static str {
        "nginx -t"
    }

    fn supports(&self, template_trigger: &str) -> bool {
        template_trigger == ";;nginx-site"
    }

    fn validate(&self, workspace: &Path, target: &ValidationTarget) -> Result<()> {
        let ValidationTarget::File(path) = target else {
            bail!("nginx -t validates a single generated file");
        };
        let include = path
            .to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "\\\"");
        let wrapper = workspace.join("wayexpand-nginx-test.conf");
        fs::write(
            &wrapper,
            format!("events {{ worker_connections 16; }}\nhttp {{ include \"{include}\"; }}\n"),
        )?;
        let mut command = Command::new("nginx");
        command
            .args(["-t", "-c"])
            .arg(wrapper)
            .current_dir(workspace);
        prepare_validator_environment(&mut command, workspace);
        run_validator_process(command, self.name())
    }
}

struct KubeconformValidator;

/// Validate the built-in HTTP 503 checklist without executing any of its
/// commands or invoking pack-defined programs.
struct Http503ChecklistValidator;

impl ArtifactValidator for Http503ChecklistValidator {
    fn name(&self) -> &'static str {
        "WayExpand read-only HTTP 503 checklist validator"
    }

    fn supports(&self, template_trigger: &str) -> bool {
        template_trigger == ";http503"
    }

    fn validate(&self, _workspace: &Path, target: &ValidationTarget) -> Result<()> {
        let ValidationTarget::File(path) = target else {
            bail!("HTTP 503 checklist validation requires a generated file");
        };
        let contents = fs::read_to_string(path)?;
        for required in ["Terminal session (operator-confirmed):", "Expected host:"] {
            if !contents.contains(required) {
                bail!("generated checklist is missing required review content: {required}");
            }
        }
        let mut in_commands = false;
        let mut hostname_check = false;
        let mut service_status = false;
        let mut journal_checks = 0usize;
        let mut socket_check = false;
        let mut host_probe = false;
        let mut nginx_check = false;
        for line in contents.lines() {
            let line = line.trim();
            if line == "```sh" {
                if in_commands {
                    bail!("generated checklist has nested shell code fences");
                }
                in_commands = true;
                continue;
            }
            if line == "```" {
                if !in_commands {
                    bail!("generated checklist has an unmatched shell code fence");
                }
                in_commands = false;
                continue;
            }
            if !in_commands || line.is_empty() || line.starts_with('#') {
                continue;
            }
            let readonly = [
                "hostname -f",
                "systemctl status --no-pager -- ",
                "journalctl -u ",
                "ss -lntp",
                "curl -sS -I --connect-timeout 3 --max-time 5 -H \"Host: ",
                "sudo nginx -t",
            ]
            .iter()
            .any(|prefix| line.starts_with(prefix));
            if !readonly {
                bail!("generated checklist contains an unapproved shell command");
            }
            hostname_check |= line.starts_with("hostname -f");
            service_status |= line.starts_with("systemctl status --no-pager -- ");
            journal_checks += usize::from(line.starts_with("journalctl -u "));
            socket_check |= line.starts_with("ss -lntp");
            host_probe |=
                line.starts_with("curl -sS -I --connect-timeout 3 --max-time 5 -H \"Host: ");
            nginx_check |= line.starts_with("sudo nginx -t");
        }
        if in_commands {
            bail!("generated checklist has an unclosed shell code fence");
        }
        if !(hostname_check
            && service_status
            && journal_checks >= 2
            && socket_check
            && host_probe
            && nginx_check)
        {
            bail!("generated checklist is missing one or more required read-only commands");
        }
        Ok(())
    }
}

impl ArtifactValidator for KubeconformValidator {
    fn name(&self) -> &'static str {
        "kubeconform -strict -summary"
    }

    fn supports(&self, template_trigger: &str) -> bool {
        template_trigger == ";;k8s-deployment"
    }

    fn validate(&self, workspace: &Path, target: &ValidationTarget) -> Result<()> {
        let ValidationTarget::File(path) = target else {
            bail!("kubeconform validates a single generated manifest file");
        };
        let mut command = Command::new("kubeconform");
        command
            .args(["-strict", "-summary", "-cache"])
            .arg(workspace.join(".cache/kubeconform"))
            .arg(path)
            .current_dir(workspace);
        prepare_validator_environment(&mut command, workspace);
        run_validator_process(command, self.name())
    }
}

fn file_argument(path: &Path) -> Vec<OsString> {
    vec![path.as_os_str().to_owned()]
}

fn compose_arguments(path: &Path) -> Vec<OsString> {
    ["compose", "-f"]
        .into_iter()
        .map(OsString::from)
        .chain([path.as_os_str().to_owned()])
        .chain(["config", "--quiet"].into_iter().map(OsString::from))
        .collect()
}

fn validator_registry() -> Vec<&'static dyn ArtifactValidator> {
    static ANSIBLE: AnsibleValidator = AnsibleValidator;
    static HADOLINT: FileCommandValidator = FileCommandValidator {
        name: "hadolint",
        executable: "hadolint",
        triggers: &[
            ";;dockerfile",
            ";;dockerfile-python",
            ";;dockerfile-node",
            ";;dockerfile-rust",
        ],
        arguments: file_argument,
    };
    static COMPOSE: FileCommandValidator = FileCommandValidator {
        name: "docker compose config",
        executable: "docker",
        triggers: &[";;compose"],
        arguments: compose_arguments,
    };
    static SYSTEMD: FileCommandValidator = FileCommandValidator {
        name: "systemd-analyze verify",
        executable: "systemd-analyze",
        triggers: &[";;systemd-service"],
        arguments: |path| vec!["verify".into(), path.as_os_str().to_owned()],
    };
    static ACTIONLINT: FileCommandValidator = FileCommandValidator {
        name: "actionlint",
        executable: "actionlint",
        triggers: &[";;github-actions"],
        arguments: file_argument,
    };
    static NGINX: NginxValidator = NginxValidator;
    static KUBECONFORM: KubeconformValidator = KubeconformValidator;
    static HTTP503: Http503ChecklistValidator = Http503ChecklistValidator;
    let validators: [&'static dyn ArtifactValidator; 8] = [
        &ANSIBLE,
        &HADOLINT,
        &COMPOSE,
        &SYSTEMD,
        &ACTIONLINT,
        &NGINX,
        &KUBECONFORM,
        &HTTP503,
    ];
    validators.into_iter().collect()
}

fn validator_for(template_trigger: &str) -> Option<&'static dyn ArtifactValidator> {
    validator_registry()
        .into_iter()
        .find(|validator| validator.supports(template_trigger))
}

fn validate_role_name(value: &str) -> Result<&str> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
    {
        usage_bail!("Ansible role names used with --validate must contain only ASCII letters, digits, '_' or '-'");
    }
    Ok(value)
}

fn rendered_output_path(snippet: &ExpansionConfig, context: &TemplateContext) -> Result<PathBuf> {
    use std::path::Component;

    let configured = snippet
        .output_file
        .as_deref()
        .context("--validate requires a file template with an output_file")?;
    let (rendered, _) = render_template_with_cursor(configured, context)
        .map_err(|error| config_error(format!("{} output path: {error}", snippet.trigger)))?;
    let path = PathBuf::from(rendered);
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        usage_bail!("--validate requires a safe relative output path");
    }
    Ok(path)
}

struct ValidationWorkspace(PathBuf);

impl ValidationWorkspace {
    fn new() -> Result<Self> {
        let root = std::env::temp_dir();
        for _ in 0..8 {
            let path = root.join(format!(
                "wayexpand-validate-{}-{:016x}",
                std::process::id(),
                getrandom::u64().context("could not obtain randomness for validator staging")?
            ));
            let mut builder = fs::DirBuilder::new();
            builder.mode(0o700);
            match builder.create(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("could not create {}", path.display()))
                }
            }
        }
        bail!("could not allocate a private validation workspace")
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn write_file(&self, relative: &Path, contents: &[u8]) -> Result<PathBuf> {
        use std::path::Component;
        if relative.as_os_str().is_empty()
            || !relative
                .components()
                .all(|component| matches!(component, Component::Normal(_)))
        {
            usage_bail!("validator staging path is unsafe");
        }
        let path = self.0.join(relative);
        let parent = path
            .parent()
            .context("validator output has no parent directory")?;
        fs::create_dir_all(parent)?;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(GENERATED_FILE_MODE)
            .open(&path)
            .with_context(|| format!("could not stage {} for validation", path.display()))?;
        file.write_all(contents)?;
        file.sync_all()?;
        Ok(path)
    }
}

impl Drop for ValidationWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn prepare_validator_environment(command: &mut Command, workspace: &Path) {
    let ansible_tmp = workspace.join(".ansible/tmp");
    let _ = fs::create_dir_all(&ansible_tmp);
    command
        .env_clear()
        .env(
            "PATH",
            std::env::var_os("PATH").unwrap_or_else(|| "/usr/bin:/bin".into()),
        )
        .env("HOME", workspace)
        // Ansible's Python runtime requires a UTF-8 locale even though the
        // validator output should otherwise be deterministic.
        .env("LANG", "C.UTF-8")
        .env("LC_ALL", "C.UTF-8")
        .env("ANSIBLE_CONFIG", workspace.join("wayexpand-ansible.cfg"))
        .env("ANSIBLE_LOCAL_TEMP", ansible_tmp)
        .env("ANSIBLE_NOCOLOR", "1")
        .env("ANSIBLE_FORKS", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
}

fn run_validator_process(mut command: Command, validator: &str) -> Result<()> {
    configure_process_group(&mut command);
    let child = command
        .spawn()
        .with_context(|| format!("could not start validator {validator}"))?;
    let mut supervisor = ChildSupervisor::new(child);
    let deadline = Instant::now() + VALIDATOR_TIMEOUT;
    loop {
        match supervisor.has_exited() {
            Ok(true) => break,
            Ok(false) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(false) => {
                supervisor.kill_group();
                let _ = supervisor.reap();
                bail!("validator {validator} exceeded its 30 second deadline");
            }
            Err(error) => {
                supervisor.kill_group();
                let _ = supervisor.reap();
                return Err(error).with_context(|| format!("could not monitor {validator}"));
            }
        }
    }
    let status = supervisor
        .reap()
        .with_context(|| format!("could not reap validator {validator}"))?;
    if !status.success() {
        bail!("validator {validator} failed with {status}");
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
fn default_value(field: &FormField) -> Result<Option<String>> {
    Ok(match &field.kind {
        FormFieldKind::Text { default } if default.is_empty() => None,
        FormFieldKind::Text { default } => Some(default.clone()),
        FormFieldKind::Choice { options, default } => {
            default.clone().or_else(|| options.first().cloned())
        }
        FormFieldKind::Date { default } if default == "today" => {
            Some(wayexpand_core::today_local_date()?)
        }
        FormFieldKind::Date { default } => Some(default.clone()),
    })
}

fn check_value(field: &FormField, value: &str) -> Result<()> {
    if value.len() > MAX_VALUE_BYTES || value.contains('\0') {
        usage_bail!("the value for {} is too long or contains NUL", field.label);
    }
    if let Err(error) = field.validate_value(value) {
        usage_bail!("invalid value for {}: {error}", field.label);
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
                FormFieldKind::Choice { options, .. } => {
                    eprintln!("{}:", field.label);
                    for (number, option) in options.iter().enumerate() {
                        eprintln!("  {}) {option}", number + 1);
                    }
                    eprint!("Choose 1-{} [1]: ", options.len());
                }
                FormFieldKind::Date { default } => eprint!(
                    "{} [{}] (YYYY-MM-DD): ",
                    field.label,
                    if default == "today" {
                        wayexpand_core::today_local_date()?
                    } else {
                        default.clone()
                    }
                ),
            }
            io::stderr().flush()?;
            let Some(line) = lines.next().transpose()? else {
                usage_bail!("input ended before {} was answered", field.label);
            };
            let answer = line.trim_end_matches(['\r', '\n']);
            let value = match &field.kind {
                FormFieldKind::Text { .. } if answer.is_empty() => {
                    default_value(field)?.unwrap_or_default()
                }
                FormFieldKind::Text { .. } => answer.to_owned(),
                FormFieldKind::Choice { options, .. } => {
                    let answer = answer.trim();
                    if answer.is_empty() {
                        default_value(field)?.unwrap_or_default()
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
                FormFieldKind::Date { .. } => {
                    if answer.is_empty() {
                        default_value(field)?.unwrap_or_default()
                    } else {
                        answer.to_owned()
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
    let mut stdout = match supervisor.take_stdout() {
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

fn validate_role_output_directory(path: &Path) -> Result<()> {
    use std::path::Component;
    if path.as_os_str().is_empty()
        || path.as_os_str().as_bytes().contains(&0)
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        usage_bail!("Ansible role output directory must not contain '..' or NUL");
    }
    Ok(())
}

fn write_scaffold_directory(
    destination: &Path,
    files: &[(String, Vec<u8>)],
    force: bool,
) -> Result<()> {
    use std::path::Component;

    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent).with_context(|| format!("could not create {}", parent.display()))?;
    let staging = temporary_output_path(
        destination,
        destination
            .file_name()
            .context("Ansible role output path has no final directory name")?,
    )?;
    let result = (|| -> Result<()> {
        fs::create_dir(&staging)
            .with_context(|| format!("could not create staging directory {}", staging.display()))?;
        fs::set_permissions(&staging, fs::Permissions::from_mode(0o700))?;

        for (relative, contents) in files {
            let relative_path = Path::new(relative);
            if relative_path.as_os_str().is_empty()
                || !relative_path
                    .components()
                    .all(|component| matches!(component, Component::Normal(_)))
            {
                bail!("unsafe path in Ansible role scaffold: {relative:?}");
            }
            let target = staging.join(relative_path);
            let file_parent = target
                .parent()
                .context("role file has no parent directory")?;
            fs::create_dir_all(file_parent)?;
            for ancestor in relative_path.parent().into_iter().flat_map(Path::ancestors) {
                if !ancestor.as_os_str().is_empty() && ancestor != Path::new(".") {
                    fs::set_permissions(staging.join(ancestor), fs::Permissions::from_mode(0o700))?;
                }
            }
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(GENERATED_FILE_MODE)
                .open(&target)
                .with_context(|| format!("could not create {}", target.display()))?;
            file.write_all(contents)?;
            file.sync_all()?;
        }

        // Sync directories from the leaves inward so every generated entry is
        // durable before the staging directory becomes visible at its final path.
        let mut directories = vec![staging.clone()];
        for (relative, _) in files {
            let mut current = staging.join(relative).parent().map(Path::to_path_buf);
            while let Some(directory) = current {
                if (directory == staging || directory.starts_with(&staging))
                    && !directories.contains(&directory)
                {
                    directories.push(directory.clone());
                }
                if directory == staging {
                    break;
                }
                current = directory.parent().map(Path::to_path_buf);
            }
        }
        directories.sort_by_key(|directory| std::cmp::Reverse(directory.components().count()));
        for directory in directories {
            fs::File::open(directory)?.sync_all()?;
        }

        match fs::symlink_metadata(destination) {
            Ok(_metadata) if !force => {
                bail!(
                    "{} already exists; add --force to replace it",
                    destination.display()
                );
            }
            Ok(metadata) if !metadata.file_type().is_dir() => {
                bail!("{} exists and is not a directory", destination.display());
            }
            Ok(_) => {
                exchange_directories(&staging, destination).with_context(|| {
                    format!("could not atomically replace {}", destination.display())
                })?;
                fs::remove_dir_all(&staging).with_context(|| {
                    format!("could not remove replaced role at {}", staging.display())
                })?;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                rename_directory_without_replacement(&staging, destination).with_context(|| {
                    format!("could not publish role at {}", destination.display())
                })?;
            }
            Err(error) => return Err(error.into()),
        }
        sync_parent_directory(destination)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

fn rename_directory_without_replacement(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let from_c = std::ffi::CString::new(from.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "source path contains NUL"))?;
    let to_c = std::ffi::CString::new(to.as_os_str().as_bytes()).map_err(|_| {
        io::Error::new(io::ErrorKind::InvalidInput, "destination path contains NUL")
    })?;
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            from_c.as_ptr(),
            libc::AT_FDCWD,
            to_c.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if matches!(
        error.raw_os_error(),
        Some(libc::ENOSYS | libc::EINVAL | libc::EOPNOTSUPP)
    ) {
        if fs::symlink_metadata(to).is_ok() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "destination already exists",
            ));
        }
        fs::rename(from, to)
    } else {
        Err(error)
    }
}

fn exchange_directories(staging: &Path, destination: &Path) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let staging_c = std::ffi::CString::new(staging.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "staging path contains NUL"))?;
    let destination_c =
        std::ffi::CString::new(destination.as_os_str().as_bytes()).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "destination path contains NUL")
        })?;
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            staging_c.as_ptr(),
            libc::AT_FDCWD,
            destination_c.as_ptr(),
            libc::RENAME_EXCHANGE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
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
        if let Ok(existing) = fs::symlink_metadata(path) {
            // Keep only the owner's execute bit on regular files. Generated
            // content stays private even when replacing a broadly readable file.
            if existing.file_type().is_file() {
                let mode = existing.permissions().mode() & 0o100;
                file.set_permissions(fs::Permissions::from_mode(GENERATED_FILE_MODE | mode))?;
            }
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

fn review_file(path: &Path, rendered: &str, force: bool) -> Result<()> {
    println!(
        "Mode: Generate File\nDestination: {}",
        terminal_safe(&path.display().to_string())
    );
    if path.exists() {
        if !force {
            usage_bail!(
                "{} already exists; add --force to replace it",
                path.display()
            );
        }
        let existing = fs::read(path)
            .with_context(|| format!("could not read existing output {}", path.display()))?;
        if existing.len() > wayexpand_core::limits::MAX_REPLACEMENT_BYTES {
            usage_bail!("existing file is too large to review safely");
        }
        let existing = String::from_utf8_lossy(&existing);
        print_review_diff(&existing, rendered);
    } else {
        print_review_diff("", rendered);
    }
    confirm_review("Type `write` to write this file: ")
}

fn review_scaffold(directory: &Path, files: &[(String, Vec<u8>)], force: bool) -> Result<()> {
    println!(
        "Mode: Generate File\nDestination directory: {}",
        directory.display()
    );
    for (relative, contents) in files {
        let path = directory.join(relative);
        println!("\nFile: {}", terminal_safe(&path.display().to_string()));
        if path.exists() {
            if !force {
                usage_bail!(
                    "{} already exists; add --force to replace it",
                    path.display()
                );
            }
            let old = fs::read(&path)
                .with_context(|| format!("could not read existing output {}", path.display()))?;
            if old.len() > wayexpand_core::limits::MAX_REPLACEMENT_BYTES {
                usage_bail!("{} is too large to review safely", path.display());
            }
            print_review_diff(
                &String::from_utf8_lossy(&old),
                &String::from_utf8_lossy(contents),
            );
        } else {
            print_review_diff("", &String::from_utf8_lossy(contents));
        }
    }
    confirm_review("Type `write` to write this scaffold: ")
}

fn print_review_diff(existing: &str, proposed: &str) {
    println!("--- existing\n+++ proposed");
    if existing == proposed {
        println!(" (unchanged)");
        return;
    }
    for line in existing.lines().take(500) {
        println!("-{}", terminal_safe(line));
    }
    for line in proposed.lines().take(500) {
        println!("+{}", terminal_safe(line));
    }
    if existing.lines().count() > 500 || proposed.lines().count() > 500 {
        println!("… diff truncated after 500 lines per side");
    }
}

fn confirm_review(prompt: &str) -> Result<()> {
    if !io::stdin().is_terminal() {
        usage_bail!("review approval requires an interactive terminal");
    }
    eprint!("{prompt}");
    io::stderr().flush()?;
    let mut answer = String::new();
    io::stdin().lock().read_line(&mut answer)?;
    if answer.trim() != "write" {
        bail!("cancelled; no files were written");
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
    fn validator_registry_is_fixed_to_supported_artifact_types() {
        for trigger in [
            ";;playbook",
            ANSIBLE_ROLE_TRIGGER,
            ";;dockerfile",
            ";;dockerfile-python",
            ";;dockerfile-node",
            ";;dockerfile-rust",
            ";;compose",
            ";;systemd-service",
            ";;github-actions",
            ";;nginx-site",
            ";;k8s-deployment",
        ] {
            assert!(
                validator_for(trigger).is_some(),
                "missing validator for {trigger}"
            );
        }
        assert!(validator_for(";;custom-template").is_none());
    }

    #[test]
    fn validator_staging_rejects_paths_that_escape_private_workspace() {
        let workspace = ValidationWorkspace::new().unwrap();
        assert!(workspace
            .write_file(Path::new("../escape.yml"), b"unsafe")
            .is_err());
        assert!(!workspace
            .path()
            .parent()
            .unwrap()
            .join("escape.yml")
            .exists());
    }

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

    #[test]
    fn forced_regeneration_keeps_only_owner_executable_bit() {
        let root = std::env::temp_dir().join(format!(
            "wayexpand-new-mode-test-{}-{}",
            std::process::id(),
            getrandom::u64().unwrap()
        ));
        fs::create_dir_all(&root).unwrap();
        for (old_mode, expected) in [
            (0o600, 0o600),
            (0o644, 0o600),
            (0o666, 0o600),
            (0o755, 0o700),
        ] {
            let path = root.join(format!("mode-{old_mode:o}"));
            fs::write(&path, b"old").unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(old_mode)).unwrap();
            write_output(&path, b"new", true).unwrap();
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                expected
            );
        }

        let target = root.join("target");
        let link = root.join("link");
        fs::write(&target, b"target").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o666)).unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        write_output(&link, b"replacement", true).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"target");
        assert_eq!(fs::read(&link).unwrap(), b"replacement");
        assert_eq!(
            fs::metadata(&link).unwrap().permissions().mode() & 0o777,
            0o600
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn scaffold_directories_publish_atomically_and_replace_as_a_unit() {
        let root = std::env::temp_dir().join(format!(
            "wayexpand-scaffold-test-{}-{}",
            std::process::id(),
            getrandom::u64().unwrap()
        ));
        let role = root.join("roles/example");
        let files = vec![
            ("tasks/main.yml".into(), b"---\n- name: task\n".to_vec()),
            (
                "handlers/main.yml".into(),
                b"---\n- name: handler\n".to_vec(),
            ),
        ];

        write_scaffold_directory(&role, &files, false).unwrap();
        assert_eq!(
            fs::read(role.join("tasks/main.yml")).unwrap(),
            b"---\n- name: task\n"
        );
        assert_eq!(
            fs::metadata(&role).unwrap().permissions().mode() & 0o777,
            0o700
        );

        let invalid = vec![
            ("tasks/main.yml".into(), b"new main".to_vec()),
            ("../escape".into(), b"must not publish".to_vec()),
        ];
        assert!(write_scaffold_directory(&role, &invalid, true).is_err());
        assert_eq!(
            fs::read(role.join("tasks/main.yml")).unwrap(),
            b"---\n- name: task\n",
            "a failed staging operation must preserve the existing complete role"
        );
        assert!(
            fs::read_dir(root.join("roles")).unwrap().all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains("wayexpand-new")),
            "failed staging directories must be cleaned up"
        );

        write_scaffold_directory(&role, &files, true).unwrap();
        assert!(role.join("handlers/main.yml").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ansible_role_builtin_renders_the_conventional_role_tree() {
        let ansible_available = Command::new("ansible-playbook")
            .arg("--version")
            .output()
            .is_ok_and(|out| out.status.success());
        let builtins = builtin_templates();
        let role = builtins
            .expansion
            .iter()
            .find(|snippet| snippet.trigger == ANSIBLE_ROLE_TRIGGER)
            .unwrap();
        let fields = form_fields(&role.replacement).unwrap();
        let mut values: HashMap<String, String> = fields
            .iter()
            .map(|field| {
                (
                    field.key.clone(),
                    default_value(field)
                        .unwrap()
                        .unwrap_or_else(|| "value".into()),
                )
            })
            .collect();
        let managed_file = std::env::temp_dir().join(format!(
            "wayexpand-ansible-managed-{}-{}.conf",
            std::process::id(),
            getrandom::u64().unwrap()
        ));
        if ansible_available {
            let owner = Command::new("id").arg("-un").output().unwrap();
            let group = Command::new("id").arg("-gn").output().unwrap();
            values.insert(
                "field:destination".into(),
                managed_file.to_string_lossy().into_owned(),
            );
            values.insert(
                "field:owner".into(),
                String::from_utf8(owner.stdout).unwrap().trim().into(),
            );
            values.insert(
                "field:group".into(),
                String::from_utf8(group.stdout).unwrap().trim().into(),
            );
        }
        let context = TemplateContext {
            fields: Arc::new(values.into()),
            ..builtins.template_context(None)
        };
        let (tasks, _) = render_template_with_cursor(&role.replacement, &context).unwrap();
        let files = render_ansible_role_files(tasks, &context).unwrap();
        let root = std::env::temp_dir().join(format!(
            "wayexpand-ansible-role-test-{}-{}",
            std::process::id(),
            getrandom::u64().unwrap()
        ));
        write_scaffold_directory(&root, &files, false).unwrap();

        for path in [
            "tasks/main.yml",
            "handlers/main.yml",
            "defaults/main.yml",
            "templates/default.conf.j2",
        ] {
            assert!(root.join(path).is_file(), "missing role file {path}");
        }
        let task_text = fs::read_to_string(root.join("tasks/main.yml")).unwrap();
        assert!(task_text.contains("notify: \"Restart nginx\""));
        let handler_text = fs::read_to_string(root.join("handlers/main.yml")).unwrap();
        assert!(handler_text.contains("state: restarted"));
        let template_text = fs::read_to_string(root.join("templates/default.conf.j2")).unwrap();
        assert!(template_text.contains("{{ nginx_listen_port }}"));

        // On developer machines with Ansible installed, syntax-check the
        // generated role and run it twice. Package and service actions are
        // replaced with debug tasks so this test never changes the host.
        if ansible_available {
            let project = root.with_file_name(format!(
                "wayexpand-ansible-project-{}",
                getrandom::u64().unwrap()
            ));
            fs::create_dir_all(project.join("roles")).unwrap();
            let role_name = "example";
            fs::rename(&root, project.join("roles").join(role_name)).unwrap();
            fs::write(
                project.join("site.yml"),
                format!(
                    "---\n- hosts: localhost\n  connection: local\n  become: false\n  roles:\n    - {role_name}\n"
                ),
            )
            .unwrap();
            AnsibleValidator
                .validate(
                    &project,
                    &ValidationTarget::AnsibleRole {
                        path: project.join("roles/example"),
                        name: role_name.into(),
                    },
                )
                .expect("registered Ansible role validator should find and check the staged role");
            let tasks_path = project.join("roles/example/tasks/main.yml");
            let tasks = fs::read_to_string(&tasks_path).unwrap().replace(
                "  ansible.builtin.package:\n    name: \"nginx\"\n    state: present",
                "  ansible.builtin.debug:\n    msg: \"package task test double\"",
            );
            fs::write(tasks_path, tasks).unwrap();
            let handler_path = project.join("roles/example/handlers/main.yml");
            let handler = fs::read_to_string(&handler_path).unwrap().replace(
                "  ansible.builtin.service:\n    name: \"nginx\"\n    state: restarted",
                "  ansible.builtin.debug:\n    msg: \"service task test double\"",
            );
            fs::write(handler_path, handler).unwrap();

            let first = Command::new("ansible-playbook")
                .args(["-i", "localhost,", "site.yml"])
                .current_dir(&project)
                .output()
                .unwrap();
            assert!(
                first.status.success(),
                "first role run failed: {}",
                String::from_utf8_lossy(&first.stderr)
            );
            assert!(
                String::from_utf8_lossy(&first.stdout).contains("changed=1"),
                "first run should apply the template once: {}",
                String::from_utf8_lossy(&first.stdout)
            );
            let second = Command::new("ansible-playbook")
                .args(["-i", "localhost,", "site.yml"])
                .current_dir(&project)
                .output()
                .unwrap();
            assert!(
                second.status.success(),
                "second role run failed: {}",
                String::from_utf8_lossy(&second.stderr)
            );
            assert!(
                String::from_utf8_lossy(&second.stdout).contains("changed=0"),
                "second role run should be idempotent: {}",
                String::from_utf8_lossy(&second.stdout)
            );
            fs::remove_dir_all(project).unwrap();
            let _ = fs::remove_file(managed_file);
        } else {
            fs::remove_dir_all(root).unwrap();
        }
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
                        default_value(field)
                            .unwrap()
                            .unwrap_or_else(|| "value".into()),
                    )
                })
                .collect();
            let context = TemplateContext {
                fields: Arc::new(values.into()),
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
                        .map(|field| (field.key.clone(), default_value(field).unwrap().unwrap()))
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
    fn infrastructure_generators_reject_invalid_field_values() {
        let builtins = builtin_templates();
        for (template, field, value) in [
            ("compose", "service", "api/backend"),
            ("compose", "host port", "70000"),
            ("playbook", "hosts", "web servers"),
            ("playbook", "package", "nginx\n  - name: injected"),
            ("systemd-service", "command", "api-server"),
            ("systemd-service", "user", "root;id"),
            ("nginx-site", "server name", "api.example.net;"),
            ("nginx-site", "upstream", "127.0.0.1:70000"),
            ("k8s-deployment", "name", "API"),
            ("k8s-deployment", "replicas", "many"),
            ("k8s-deployment", "port", "0"),
            ("k8s-deployment", "image", "nginx; touch /tmp/pwn"),
        ] {
            let (snippet, _) = find_template(&[(&builtins, Source::BuiltIn)], template).unwrap();
            let fields = form_fields(&snippet.replacement).unwrap();
            assert!(
                values_from_assignments(&fields, vec![(field.into(), value.into())]).is_err(),
                "{template} accepted invalid {field}={value:?}"
            );
        }
    }

    #[test]
    fn playbook_service_tasks_follow_the_selected_package_state() {
        let builtins = builtin_templates();
        let playbook = builtins
            .expansion
            .iter()
            .find(|snippet| snippet.trigger == ";;playbook")
            .unwrap();
        let fields = form_fields(&playbook.replacement).unwrap();

        for state in ["present", "latest", "absent"] {
            let mut values: HashMap<String, String> = fields
                .iter()
                .map(|field| {
                    (
                        field.key.clone(),
                        default_value(field)
                            .unwrap()
                            .unwrap_or_else(|| "value".into()),
                    )
                })
                .collect();
            values.insert("choice:state".into(), state.into());
            let (rendered, _) = render_template_with_cursor(
                &playbook.replacement,
                &TemplateContext {
                    fields: Arc::new(values.into()),
                    ..builtins.template_context(None)
                },
            )
            .unwrap();

            assert!(rendered.contains(&format!("package_state: {state}")));
            assert!(rendered.contains("state: \"{{ package_state }}\""));
            assert!(rendered.contains("when: package_state != 'absent'"));
            if state == "absent" {
                assert!(
                    rendered.contains("state: started\n        enabled: true\n      when: package_state != 'absent'"),
                    "service start must be skipped when the package is removed: {rendered}"
                );
            }
        }
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
