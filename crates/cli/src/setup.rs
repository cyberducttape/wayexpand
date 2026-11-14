//! Guided setup: compatibility-mode selection, policy gates, backend configuration, and user-service enablement.

use crate::*;
use std::process::{Command, ExitStatus};
use std::thread;
use std::time::{Duration, Instant};
use wayexpand_process_supervisor::{configure_process_group, ChildSupervisor};

const DEFAULT_SETUP_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

fn setup_command_timeout(program: &str, arguments: &[&str]) -> Duration {
    let joined = arguments.join(" ");
    if program == "systemctl"
        && (joined.contains(" is-active ")
            || joined.starts_with("--user is-active")
            || joined.contains(" is-enabled "))
    {
        Duration::from_secs(5)
    } else if (program == "systemctl" && joined.contains(" restart "))
        || (program == "ibus" && (joined == "restart" || joined == "exit"))
    {
        Duration::from_secs(60)
    } else if program == "systemctl"
        && (joined.contains("daemon-reload") || joined.contains("enable"))
    {
        Duration::from_secs(20)
    } else {
        DEFAULT_SETUP_COMMAND_TIMEOUT
    }
}

pub(crate) fn run_setup_command(program: &str, arguments: &[&str]) -> Result<ExitStatus> {
    run_setup_command_with_env(program, arguments, &[])
}

pub(crate) fn run_setup_command_with_env(
    program: &str,
    arguments: &[&str],
    environment: &[(&str, &str)],
) -> Result<ExitStatus> {
    let timeout = setup_command_timeout(program, arguments);
    let mut command = Command::new(program);
    command.args(arguments).envs(environment.iter().copied());
    configure_process_group(&mut command);
    let child = command
        .spawn()
        .map_err(|error| anyhow::anyhow!("could not start {program}: {error}"))?;
    let mut supervisor = ChildSupervisor::new(child);
    let deadline = Instant::now() + timeout;
    loop {
        match supervisor.has_exited() {
            Ok(true) => {
                supervisor.kill_group();
                return supervisor
                    .reap()
                    .map_err(|error| anyhow::anyhow!("could not reap {program}: {error}"));
            }
            Ok(false) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(25));
            }
            Ok(false) => {
                supervisor.kill_group();
                let cleanup = supervisor.reap();
                bail!(
                    "{program} {} timed out after {} seconds; process-group cleanup {}",
                    arguments.join(" "),
                    timeout.as_secs(),
                    if cleanup.is_ok() {
                        "completed"
                    } else {
                        "failed"
                    }
                );
            }
            Err(error) => {
                supervisor.kill_group();
                let cleanup = supervisor.reap();
                bail!(
                    "could not monitor {program} {} ({error}); process-group cleanup {}",
                    arguments.join(" "),
                    if cleanup.is_ok() {
                        "completed"
                    } else {
                        "failed"
                    }
                );
            }
        }
    }
}

pub(crate) struct SetupRecommendation {
    pub(crate) backend: String,
    pub(crate) label: String,
    pub(crate) detail: String,
}

pub(crate) fn recommended_setup_backend(
    capabilities: &wayexpand_backend_selection::Capabilities,
    policy: &OrganizationPolicy,
) -> SetupRecommendation {
    // Setup can install the component and refresh IBus, so the executable
    // client/engine pair is enough to offer IBus as the setup route. Live
    // registration remains a separate readiness check.
    let ibus_ready = crate::setup_ibus::setup_route_available()
        || ibus_engine_available()
        || ibus_engine_installed();
    recommended_setup_backend_with_ibus(capabilities, ibus_ready, policy)
}

pub(crate) fn recommended_setup_backend_with_ibus(
    capabilities: &wayexpand_backend_selection::Capabilities,
    ibus_available: bool,
    policy: &OrganizationPolicy,
) -> SetupRecommendation {
    match recommended_route(capabilities, ibus_available, |route| {
        route_allowed_by_policy(policy, route)
    }) {
        Some(route) => {
            let contract = route.contract();
            SetupRecommendation {
                backend: contract.setup_backend.clone(),
                label: contract.label.clone(),
                detail: contract.setup_detail.clone(),
            }
        }
        None => SetupRecommendation {
            backend: "unavailable".into(),
            label: "No safe automatic path".into(),
            detail:
                "setup will not enable an experimental or globally observing path automatically"
                    .into(),
        },
    }
}

pub(crate) fn setup_backend_for_mode(
    mode: &str,
    capabilities: &wayexpand_backend_selection::Capabilities,
    policy: &OrganizationPolicy,
) -> Result<String> {
    match mode {
        "recommended" => Ok(recommended_setup_backend(capabilities, policy).backend),
        "maximum" => {
            if !capabilities.has_dev_input {
                bail!("Maximum compatibility requires a readable /dev/input keyboard")
            }
            if !capabilities.has_direct_libei_socket && !libei_portal_candidate() {
                bail!(
                    "Maximum compatibility needs a direct EIS socket or a desktop provider candidate; a candidate is not proof that the portal is installed, available, or authorized"
                )
            }
            if !setup_backend_allowed(policy, "evdev") {
                bail!("Maximum compatibility is disallowed by organization policy")
            }
            Ok("evdev".into())
        }
        "experimental" => {
            if !capabilities.has_input_method_v2 {
                bail!(
                    "Experimental mode requires an available input-method-v2 compositor interface"
                )
            }
            if !setup_backend_allowed(policy, "input-method") {
                bail!("Experimental input-method-v2 mode is disallowed by organization policy")
            }
            let plan = wayexpand_backend_selection::plan_routes(capabilities, false, |route| {
                route.id == "input-method-v2" && route_allowed_by_policy(policy, route)
            });
            let Some(route) = plan
                .routes
                .iter()
                .find(|route| route.contract.id == "input-method-v2")
            else {
                bail!("input-method-v2 route is missing from the backend catalog")
            };
            if !matches!(
                route.standing,
                wayexpand_backend_selection::RouteStanding::Available
                    | wayexpand_backend_selection::RouteStanding::Recommended
                    | wayexpand_backend_selection::RouteStanding::RequiresConsent
            ) {
                bail!(
                    "Experimental mode is unavailable for this compositor: {}",
                    route.reason
                )
            }
            Ok("input-method".into())
        }
        other => bail!(
            "unknown compatibility mode {other:?}; choose recommended, maximum, or experimental"
        ),
    }
}

pub(crate) fn libei_portal_candidate() -> bool {
    if std::env::var_os("LIBEI_SOCKET").is_some() {
        return false;
    }
    // This is a discovery hint only. It allows an explicit setup flow to
    // proceed to the authorization step; it must never be treated as EIS
    // availability, connection, or injection readiness.
    std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .to_ascii_lowercase()
        .split(':')
        .any(|desktop| desktop.contains("kde") || desktop.contains("gnome"))
}

pub(crate) fn prompt_mode_choice(
    capabilities: &wayexpand_backend_selection::Capabilities,
    policy: &OrganizationPolicy,
) -> Result<String> {
    print!("Choose a mode [recommended/maximum/experimental/q]: ");
    io::stdout().flush()?;
    let mut choice = String::new();
    io::stdin().read_line(&mut choice)?;
    let choice = choice.trim().to_ascii_lowercase();
    match choice.as_str() {
        "recommended" | "maximum" | "experimental" => {
            setup_backend_for_mode(choice.as_str(), capabilities, policy)
        }
        _ => bail!("setup cancelled; choose recommended, maximum, or experimental"),
    }
}

pub(crate) fn configure_setup_backend(
    backend: &str,
    options: crate::setup_ibus::IbusSetupOptions,
) -> Result<()> {
    if options.terminals && backend != "ibus" {
        bail!("--terminals only applies beside the IBus route (`--backend ibus`)");
    }
    if options.evdev_apps && !matches!(backend, "ibus" | "evdev") {
        bail!("--evdev-apps requires an IBus or evdev route");
    }
    match backend {
        "ibus" => crate::setup_ibus::configure_ibus(options)?,
        "input-method" => {
            println!("Input Method v2 setup: protocol discovery does not establish portal availability, authorization, EIS connection, or verified injection.");
            if libei_portal_candidate() && std::env::var_os("LIBEI_SOCKET").is_none() {
                println!("A desktop portal may request authorization when the service connects.");
            }
            enable_user_service("wayexpand-input-method.service")?
        }
        "evdev" => {
            println!("Compatibility mode: Maximum");
            println!("Advantage: global keyboard capture can reach applications that do not use a supported input method.");
            println!("Limitations:");
            println!("  - Requires access to keyboard event devices and observes keyboard input globally.");
            println!(
                "  - Cannot reliably identify password fields or verify text before replacing it."
            );
            println!("  - libei may request desktop portal authorization. A KDE/GNOME desktop name only identifies a possible provider; it does not prove the portal is available or authorized.");
            println!("Recommended for operators who understand and accept these limitations.");
            if options.evdev_apps {
                println!("The requested browser scope limits matching by tracked application ID, not by individual browser field.");
            }
            println!("Automatic replacement requires an explicitly enabled compatibility mode (tracked-focus or untracked-focus under [settings]).");
            // The unit is inert without the consent marker, and systemd
            // reports a failed condition as a successful start. Setup does
            // not create the marker: that would let a scripted rollout opt a
            // fleet into raw capture.
            let marker = evdev_consent_marker()
                .context("cannot locate the evdev consent marker: HOME is not set")?;
            if !marker.exists() {
                bail!(
                    "wayexpand-evdev.service only starts after you accept that it captures password fields.\n\
                     Review docs/BACKENDS_SENSITIVE_FIELDS.md, then create the consent marker and rerun setup:\n  \
                     install -d -m 0700 {}\n  install -m 0600 /dev/null {}",
                    marker.parent().unwrap_or(&marker).display(),
                    marker.display()
                );
            }
            if options.evdev_apps {
                crate::setup_ibus::configure_evdev_apps_only()?;
            } else {
                enable_user_service("wayexpand-evdev.service")?;
            }
        }
        _ => bail!("unknown setup backend {backend:?}; choose ibus, input-method, or evdev"),
    }
    Ok(())
}

/// The marker `wayexpand-evdev.service` requires through
/// `ConditionPathExists=%h/.config/...`. systemd expands `%h` to the home
/// directory regardless of `XDG_CONFIG_HOME`, so this path does too.
/// Where older installers looked for the marker when
/// `XDG_CONFIG_HOME` was customized. The unit never read it; diagnostics use
/// it only to tell the user where to move their consent.
pub(crate) fn legacy_evdev_consent_marker() -> Option<PathBuf> {
    env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(|config| PathBuf::from(config).join("wayexpand/evdev-sensitive-fields-ack"))
}

/// The one evdev consent-marker location shared by setup, doctor, the GUI,
/// the installers and the unit's `ConditionPathExists`. It is fixed under
/// `$HOME/.config` because systemd evaluates unit conditions before any
/// `XDG_CONFIG_HOME` environment applies.
pub(crate) fn evdev_consent_marker() -> Option<PathBuf> {
    env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(|home| PathBuf::from(home).join(".config/wayexpand/evdev-sensitive-fields-ack"))
}

pub(crate) fn enable_user_service(service: &str) -> Result<()> {
    // `systemctl cat` prints the whole unit; only its exit status matters.
    let installed = {
        let mut command = Command::new("systemctl");
        command
            .args(["--user", "cat", service])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        command
            .status()
            .map_err(|error| anyhow::anyhow!("could not start systemctl: {error}"))?
    };
    if !installed.success() {
        bail!("user service {service} is not installed; run the WayExpand installer first")
    }
    let reload_arguments = ["--user", "daemon-reload"];
    let enable_arguments = ["--user", "enable", "--now", service];
    for arguments in [&reload_arguments[..], &enable_arguments[..]] {
        let status = run_setup_command("systemctl", arguments)?;
        if !status.success() {
            bail!("systemd could not apply {service}; no further setup actions were taken")
        }
    }
    println!("Enabled and started {service}.");
    println!("Run `wayexpand status` and `wayexpand doctor` to verify the live integration.");
    Ok(())
}

#[cfg(test)]
mod timeout_tests {
    use super::setup_command_timeout;
    use std::time::Duration;

    #[test]
    fn setup_deadlines_follow_operation_cost() {
        assert_eq!(
            setup_command_timeout("systemctl", &["--user", "is-active", "unit"]),
            Duration::from_secs(5)
        );
        assert_eq!(
            setup_command_timeout("systemctl", &["--user", "restart", "unit"]),
            Duration::from_secs(60)
        );
        assert_eq!(
            setup_command_timeout("systemctl", &["--user", "daemon-reload"]),
            Duration::from_secs(20)
        );
        assert_eq!(
            setup_command_timeout("unknown", &["operation"]),
            Duration::from_secs(30)
        );
    }
}
