use super::*;

impl ExpansionEngine {
    pub fn async_commands_enabled(&self) -> bool {
        self.async_commands.is_some()
    }

    /// Snapshot command execution counters for diagnostics and status output.
    pub fn command_metrics(&self) -> CommandMetrics {
        let expansion_command_queue_depth =
            self.expansion_metrics.queue_depth.load(Ordering::Relaxed);
        let expansion_command_in_flight = self.expansion_metrics.in_flight.load(Ordering::Relaxed);
        let hotkey_queue_depth = self.hotkey_metrics.queue_depth.load(Ordering::Relaxed);
        let hotkey_in_flight = self.hotkey_metrics.in_flight.load(Ordering::Relaxed);
        CommandMetrics {
            command_queue_depth: expansion_command_queue_depth + hotkey_queue_depth,
            command_in_flight: expansion_command_in_flight + hotkey_in_flight,
            expansion_command_queue_depth,
            expansion_command_in_flight,
            hotkey_queue_depth,
            hotkey_in_flight,
            command_queue_rejected_total: self
                .expansion_metrics
                .queue_rejected_total
                .load(Ordering::Relaxed)
                + self
                    .hotkey_metrics
                    .queue_rejected_total
                    .load(Ordering::Relaxed),
            command_timeout_total: self.expansion_metrics.timeout_total.load(Ordering::Relaxed)
                + self.hotkey_metrics.timeout_total.load(Ordering::Relaxed),
            command_failure_total: self.expansion_metrics.failure_total.load(Ordering::Relaxed)
                + self.hotkey_metrics.failure_total.load(Ordering::Relaxed),
        }
    }

    /// Whether libei portal restoration tokens may be read and persisted.
    pub fn libei_token_persistence(&self) -> bool {
        self.config.settings.libei_token_persistence
    }

    /// Whether the optional Fcitx5 surrounding-text bridge may be used by
    /// the libei output route.
    pub fn fcitx5_direct_commit(&self) -> bool {
        self.config.settings.fcitx5_direct_commit
    }

    /// Applies the administrator's command-execution decision before command
    /// jobs are queued. This is separate from the config-owned organization
    /// policy because the daemon also loads `/etc/wayexpand/policy.toml`.
    pub fn set_commands_disabled(&mut self, disabled: bool) {
        self.config.organization.disable_commands = disabled;
    }

    pub fn commands_disabled(&self) -> bool {
        self.config.organization.disable_commands
    }

    /// Disable only direct executable commands while retaining managed broker
    /// actions. Backends outside the daemon sandbox, such as IBus, use this
    /// to preserve the broker isolation boundary.
    pub fn set_direct_commands_disabled(&mut self, disabled: bool) {
        self.direct_commands_disabled = disabled;
    }

    pub fn direct_commands_disabled(&self) -> bool {
        self.direct_commands_disabled
    }

    pub(super) fn command_execution_disabled(&self, command: &CommandConfig) -> bool {
        self.config.organization.disable_commands
            || (self.direct_commands_disabled && command.action.is_none())
    }

    /// Check the active, enforcement-mode backend requirements against the
    /// connected injector and input source. Keeping this decision in the
    /// engine's policy snapshot ensures reloads and fleet policy cannot drift
    /// from startup validation.
    pub fn capability_violation(
        &self,
        injector: InjectorCapabilities,
        sensitive_focus: bool,
    ) -> Option<String> {
        self.config
            .organization
            .effective_enforcement_policy()
            .capability_violation(injector, sensitive_focus)
    }

    /// Check the active enforcement policy against the negotiated capture
    /// and injection guarantees.
    pub fn capability_violation_for_source(
        &self,
        injector: InjectorCapabilities,
        source: crate::InputSourceCapabilities,
    ) -> Option<String> {
        self.config
            .organization
            .effective_enforcement_policy()
            .capability_violation_for_source(injector, source)
    }

    pub fn set_title_matching_disabled(&mut self, disabled: bool) {
        self.config.organization.disable_title_matching = disabled;
    }

    pub fn title_matching_disabled(&self) -> bool {
        self.config.organization.disable_title_matching
    }
}
