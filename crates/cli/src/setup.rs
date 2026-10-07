//! Guided setup: compatibility-mode selection, policy gates, backend configuration, and user-service enablement.

use crate::*;

pub(crate) struct SetupRecommendation {
    pub(crate) backend: String,
    pub(crate) label: String,
    pub(crate) detail: String,
}

pub(crate) fn recommended_setup_backend(
    capabilities: &wayexpand_backend_selection::Capabilities,
    policy: &OrganizationPolicy,
) -> SetupRecommendation {
    match recommended_route(capabilities, ibus_engine_available(), |route| {
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
        "recommended" => Ok(recommended_setup_backend(capabilities, policy)
            .backend
            .to_owned()),
        "maximum" => {
            if !capabilities.has_dev_input {
                bail!("Maximum compatibility requires a readable /dev/input keyboard")
            }
            if !capabilities.has_direct_libei_socket && !libei_portal_candidate() {
                bail!(
                    "Maximum compatibility requires a detected libei/EIS path or a KDE/GNOME portal candidate"
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
            let route = plan
                .routes
                .iter()
                .find(|route| route.contract.id == "input-method-v2")
                .expect("route catalog must declare input-method-v2");
            if !matches!(
                route.standing,
                wayexpand_backend_selection::RouteStanding::Available
                    | wayexpand_backend_selection::RouteStanding::Recommended
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

pub(crate) fn configure_setup_backend(backend: &str) -> Result<()> {
    match backend {
        "ibus" => {
            if !ibus_engine_available() {
                bail!("WayExpand's IBus engine is not installed or IBus is unavailable")
            }
            println!("Configuring the WayExpand IBus engine for the current session...");
            let restart = Command::new("ibus").arg("restart").status()?;
            if !restart.success() {
                bail!("IBus restart failed; no WayExpand service was enabled")
            }
            let select = Command::new("ibus")
                .args(["engine", "wayexpand"])
                .status()?;
            if !select.success() {
                bail!("could not select the WayExpand IBus engine")
            }
            // Keep the hardened daemon available as the session's control and
            // Quick Insert broker. It runs in stdin mode, so it does not
            // compete with IBus for keyboard capture; it only supplies the
            // same authenticated control endpoint and approved injector.
            enable_user_service("wayexpand.service")?;
            println!("WayExpand IBus integration is active. Run `wayexpand doctor` to inspect the session.");
        }
        "input-method" => enable_user_service("wayexpand-input-method.service")?,
        "evdev" => {
            println!(
                "Warning: evdev is an explicit compatibility fallback, not the recommended setup."
            );
            println!("It observes every keyboard event, including password fields; it has no password-field signal.");
            println!("Portal consent may be requested by libei; raw-input permissions are not changed by setup.");
            enable_user_service("wayexpand-evdev.service")?;
        }
        _ => bail!("unknown setup backend {backend:?}; choose ibus, input-method, or evdev"),
    }
    Ok(())
}

pub(crate) fn enable_user_service(service: &str) -> Result<()> {
    let installed = Command::new("systemctl")
        .args(["--user", "cat", service])
        .status()?;
    if !installed.success() {
        bail!("user service {service} is not installed; run the WayExpand installer first")
    }
    let reload_arguments = ["--user", "daemon-reload"];
    let enable_arguments = ["--user", "enable", "--now", service];
    for arguments in [&reload_arguments[..], &enable_arguments[..]] {
        let status = Command::new("systemctl").args(arguments).status()?;
        if !status.success() {
            bail!("systemd could not apply {service}; no further setup actions were taken")
        }
    }
    println!("Enabled and started {service}.");
    println!("Run `wayexpand status` and `wayexpand doctor` to verify the live integration.");
    Ok(())
}
