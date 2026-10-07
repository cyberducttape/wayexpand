//! Organization-policy enforcement for the IBus route.

use tracing::{error, warn};
use wayexpand_core::{
    ExpansionEngine, InjectorCapabilities, InputSourceCapabilities, OrganizationPolicy,
};

/// Enforce an organization-policy violation: safe mode logs an error and
/// blocks (returns true); audit mode logs a warning and lets the operation
/// proceed. `what` names the checked operation in the log line.
pub(super) fn blocks(policy: &OrganizationPolicy, violation: Option<String>, what: &str) -> bool {
    let Some(violation) = violation else {
        return false;
    };
    if policy.safe_mode {
        error!(
            audit_prefix = %policy.audit_prefix,
            violation = %violation,
            "IBus {what} blocked by organization policy"
        );
        true
    } else {
        warn!(
            audit_prefix = %policy.audit_prefix,
            violation = %violation,
            "IBus {what} violates organization policy; audit mode permits it"
        );
        false
    }
}

pub(super) fn apply_to_engine(engine: &mut ExpansionEngine, policy: &OrganizationPolicy) {
    let enforcement = policy.effective_enforcement_policy();
    // IBus runs outside the hardened wayexpand.service boundary. Direct
    // executable commands remain disabled here, but managed Action Broker
    // requests cross a separate authenticated Unix-socket boundary and are
    // allowed to fail closed if that broker is unavailable.
    engine.set_direct_commands_disabled(true);
    engine.set_title_matching_disabled(enforcement.disable_title_matching);
}

pub(super) fn capability_block(
    policy: &OrganizationPolicy,
    injector: InjectorCapabilities,
    source: InputSourceCapabilities,
) -> Option<String> {
    let violation = policy.capability_violation_for_source(injector, source)?;
    blocks(policy, Some(violation.clone()), "route capability check").then_some(violation)
}
