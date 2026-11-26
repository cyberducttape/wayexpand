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
            // The control unit runs without an input reader.
            "stdin" => Some(Self::Control),
            "control" => Some(Self::Control),
            _ => None,
        }
    }
}

/// Current resource consumption and configured hard limits for the optional
/// action broker unit. Values are best-effort: user managers may hide process
/// details or use cgroup versions without the corresponding counters.
pub(crate) fn broker_resource_headroom() -> serde_json::Value {
    let output = crate::setup_ibus::command_output(
        "systemctl",
        &[
            "--user",
            "show",
            "wayexpand-action-broker.service",
            "-p",
            "ActiveState",
            "-p",
            "MainPID",
            "-p",
            "ControlGroup",
            "-p",
            "MemoryCurrent",
            "-p",
            "MemoryMax",
            "-p",
            "TasksCurrent",
            "-p",
            "TasksMax",
            "-p",
            "LimitNOFILE",
            "-p",
            "Result",
        ],
    );
    let Some(output) = output else {
        return serde_json::json!({"available": false, "reason": "systemd user manager unavailable"});
    };
    let fields: std::collections::HashMap<_, _> = output
        .lines()
        .filter_map(|line| line.split_once('='))
        .collect();
    let number = |key: &str| fields.get(key).and_then(|value| value.parse::<u64>().ok());
    let pid = number("MainPID").unwrap_or(0);
    let cgroup = fields
        .get("ControlGroup")
        .filter(|path| path.starts_with('/') && !path.contains(".."));
    let cgroup_pids = cgroup
        .and_then(|path| std::fs::read_to_string(format!("/sys/fs/cgroup{path}/cgroup.procs")).ok())
        .map(|pids| {
            pids.lines()
                .filter_map(|pid| pid.parse::<u32>().ok())
                .collect::<Vec<_>>()
        })
        .filter(|pids| !pids.is_empty());
    let fd_pids =
        cgroup_pids.unwrap_or_else(|| (pid > 0).then_some(pid as u32).into_iter().collect());
    let open_files: Option<u64> = (!fd_pids.is_empty()).then(|| {
        fd_pids
            .iter()
            .filter_map(|pid| std::fs::read_dir(format!("/proc/{pid}/fd")).ok())
            .map(|entries| entries.count() as u64)
            .sum()
    });
    let memory_current = number("MemoryCurrent");
    let memory_max = number("MemoryMax");
    let tasks_current = number("TasksCurrent");
    let tasks_max = number("TasksMax");
    let nofile_limit = number("LimitNOFILE");
    // Inspect only for known resource signatures. Never include journal text
    // in the report because command output may contain private action data.
    let journal = crate::setup_ibus::command_output(
        "journalctl",
        &[
            "--user",
            "-u",
            "wayexpand-action-broker.service",
            "-n",
            "80",
            "--no-pager",
            "-o",
            "cat",
        ],
    )
    .unwrap_or_default()
    .to_ascii_lowercase();
    let mut failure_causes = Vec::new();
    if journal.contains("emfile") || journal.contains("too many open files") {
        failure_causes.push("EMFILE (open-file limit)");
    }
    if journal.contains("eagain") || journal.contains("resource temporarily unavailable") {
        failure_causes.push("EAGAIN (task or resource limit)");
    }
    if fields.get("Result").copied() == Some("oom-kill")
        || journal.contains("memory cgroup out of memory")
        || journal.contains("oom_kill")
        || journal.contains("memory.max")
    {
        failure_causes.push("cgroup memory limit / OOM");
    }
    let headroom = |current: Option<u64>, limit: Option<u64>| {
        current
            .zip(limit)
            .map(|(current, limit)| limit.saturating_sub(current))
    };
    let near_limit = |current: Option<u64>, limit: Option<u64>| {
        current
            .zip(limit)
            .is_some_and(|(current, limit)| limit > 0 && current >= limit)
    };
    serde_json::json!({
        "available": true,
        "unit": "wayexpand-action-broker.service",
        "active": fields.get("ActiveState").copied() == Some("active"),
        "result": fields.get("Result"),
        "memory_bytes": {"current": memory_current, "limit": memory_max, "remaining": headroom(memory_current, memory_max)},
        "tasks": {"current": tasks_current, "limit": tasks_max, "remaining": headroom(tasks_current, tasks_max)},
        "open_files": {"current": open_files, "limit": nofile_limit, "remaining": headroom(open_files, nofile_limit)},
        "resource_exhaustion_observed": near_limit(memory_current, memory_max)
            || near_limit(tasks_current, tasks_max)
            || near_limit(open_files, nofile_limit),
        "failure_cause_hints": failure_causes,
        "systemd_resource_failure": matches!(
            fields.get("Result").copied(),
            Some("resources" | "oom-kill")
        ),
    })
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
