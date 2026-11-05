//! The WayExpand user services, and which of them this installation really
//! has. Recovery advice names an installed unit, or recommends `wayexpand
//! setup`, instead of guessing a unit that may not exist.

/// A daemon unit that owns the control socket. Only one runs at a time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UserService {
    /// input-method-v2 capture and output.
    InputMethod,
    /// Raw evdev capture (all apps, or `terminal_apps` beside IBus).
    Evdev,
    /// No capture: control socket and quick insert beside IBus.
    Control,
}

impl UserService {
    pub(crate) const ALL: [Self; 3] = [Self::InputMethod, Self::Evdev, Self::Control];

    pub(crate) fn unit(self) -> &'static str {
        match self {
            Self::InputMethod => "wayexpand-input-method.service",
            Self::Evdev => "wayexpand-evdev.service",
            Self::Control => "wayexpand-control.service",
        }
    }

    /// The unit that runs a daemon reporting this capture source.
    pub(crate) fn for_source(source: &str) -> Option<Self> {
        match source {
            "evdev" => Some(Self::Evdev),
            "input-method" => Some(Self::InputMethod),
            // The control unit runs the stdin source with stdin closed.
            "stdin" => Some(Self::Control),
            _ => None,
        }
    }
}

/// What systemd reports for one unit.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct UnitState {
    pub(crate) installed: bool,
    pub(crate) enabled: bool,
    pub(crate) active: bool,
    pub(crate) failed: bool,
}

/// The state of every WayExpand daemon unit.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ServiceInventory {
    states: Vec<(UserService, UnitState)>,
}

impl ServiceInventory {
    #[cfg(test)]
    pub(crate) fn from_states(states: Vec<(UserService, UnitState)>) -> Self {
        Self { states }
    }

    /// Ask systemd once, with a deadline. Without systemd every unit reads
    /// as not installed, which yields setup/reinstall advice.
    pub(crate) fn probe() -> Self {
        let units: Vec<&str> = UserService::ALL
            .iter()
            .map(|service| service.unit())
            .collect();
        let mut arguments = vec![
            "--user",
            "show",
            "-p",
            "Id,LoadState,UnitFileState,ActiveState",
        ];
        arguments.extend(units.iter().copied());
        let output = crate::setup_ibus::command_output("systemctl", &arguments).unwrap_or_default();
        Self::parse_show(&output)
    }

    /// Parse `systemctl show -p Id,LoadState,UnitFileState,ActiveState`.
    pub(crate) fn parse_show(output: &str) -> Self {
        let mut states = Vec::new();
        for block in output.split("\n\n") {
            let field = |key: &str| {
                block
                    .lines()
                    .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
                    .unwrap_or("")
            };
            let Some(service) = UserService::ALL
                .into_iter()
                .find(|service| service.unit() == field("Id"))
            else {
                continue;
            };
            let installed = field("LoadState") == "loaded";
            states.push((
                service,
                UnitState {
                    installed,
                    // A linked user-install unit reads "enabled" once enabled.
                    enabled: installed
                        && matches!(field("UnitFileState"), "enabled" | "enabled-runtime"),
                    active: field("ActiveState") == "active",
                    failed: field("ActiveState") == "failed",
                },
            ));
        }
        Self { states }
    }

    pub(crate) fn state(&self, service: UserService) -> UnitState {
        self.states
            .iter()
            .find(|(candidate, _)| *candidate == service)
            .map(|(_, state)| state.clone())
            .unwrap_or_default()
    }

    pub(crate) fn installed(&self, service: UserService) -> bool {
        self.state(service).installed
    }

    /// Advice when no daemon answers on the control socket.
    pub(crate) fn start_advice(&self) -> String {
        if let Some(service) = UserService::ALL
            .into_iter()
            .find(|service| self.state(*service).enabled)
        {
            let unit = service.unit();
            let state = self.state(service);
            return if state.failed {
                format!("{unit} is enabled but failed; see `journalctl --user -u {unit} -n 50`, fix the cause, then `systemctl --user restart {unit}`.")
            } else if service == UserService::Evdev {
                format!("Start it with `systemctl --user start {unit}`. If it stays inactive, run `wayexpand doctor`: evdev only starts with its consent marker.")
            } else {
                format!("Start it with `systemctl --user start {unit}`, then run `wayexpand doctor` again.")
            };
        }
        if UserService::ALL
            .into_iter()
            .any(|service| self.installed(service))
        {
            "No WayExpand service is enabled; run `wayexpand setup` to choose and start a route."
                .into()
        } else {
            "No WayExpand user service is installed; reinstall WayExpand, then run `wayexpand setup`.".into()
        }
    }

    /// Advice to restart the daemon that reported `source`.
    pub(crate) fn restart_advice(&self, source: Option<&str>) -> String {
        match source.and_then(UserService::for_source) {
            Some(service) if self.installed(service) => {
                format!(
                    "restart it with `systemctl --user restart {}`",
                    service.unit()
                )
            }
            _ => "restart it with `wayexpand setup`, which restarts the configured route".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHOW: &str = "Id=wayexpand-input-method.service\nLoadState=loaded\nActiveState=inactive\nUnitFileState=linked\n\n\
                        Id=wayexpand-evdev.service\nLoadState=loaded\nActiveState=failed\nUnitFileState=enabled\n\n\
                        Id=wayexpand-control.service\nLoadState=not-found\nActiveState=inactive\nUnitFileState=\n";

    #[test]
    fn systemd_show_output_maps_to_unit_states() {
        let inventory = ServiceInventory::parse_show(SHOW);
        assert!(inventory.installed(UserService::InputMethod));
        assert!(!inventory.state(UserService::InputMethod).enabled);
        assert!(inventory.state(UserService::Evdev).enabled);
        assert!(inventory.state(UserService::Evdev).failed);
        assert!(!inventory.installed(UserService::Control));
    }

    #[test]
    fn advice_names_only_units_that_exist() {
        let failed = ServiceInventory::parse_show(SHOW);
        assert!(failed
            .start_advice()
            .contains("journalctl --user -u wayexpand-evdev.service"));
        // The stdin harness unit is never recommended.
        assert!(!failed.start_advice().contains("wayexpand.service"));
        assert!(failed
            .restart_advice(Some("stdin"))
            .contains("wayexpand setup"));
        assert!(failed
            .restart_advice(Some("evdev"))
            .contains("systemctl --user restart wayexpand-evdev.service"));

        let installed_only = ServiceInventory::from_states(vec![(
            UserService::Control,
            UnitState {
                installed: true,
                ..UnitState::default()
            },
        )]);
        assert!(installed_only.start_advice().contains("wayexpand setup"));

        let nothing = ServiceInventory::default();
        assert!(nothing.start_advice().contains("reinstall"));
        assert!(nothing
            .restart_advice(Some("evdev"))
            .contains("wayexpand setup"));
    }
}
