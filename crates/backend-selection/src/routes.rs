//! Route contracts, availability planning, and organization-policy filtering.

use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

use super::{Capabilities, Compositor};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteBackend {
    IBus,
    Evdev,
    Libei,
    WlrootsVirtualKeyboard,
    InputMethodV2,
}

impl RouteBackend {
    fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "ibus" => Self::IBus,
            "evdev" => Self::Evdev,
            "libei" => Self::Libei,
            "wlroots-virtual-keyboard" => Self::WlrootsVirtualKeyboard,
            "input-method-v2" => Self::InputMethodV2,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RecommendedRoute(&'static RouteContract);

impl PartialEq for RecommendedRoute {
    fn eq(&self, other: &Self) -> bool {
        self.0.id == other.0.id
    }
}

impl Eq for RecommendedRoute {}

#[derive(Debug, Deserialize)]
pub struct RouteContract {
    pub id: String,
    pub label: String,
    pub capture: String,
    pub injection: String,
    pub sensitive_fields: bool,
    pub sensitive_field_support: SensitiveFieldSupport,
    /// What the route checks before attempting to erase the trigger. This is
    /// independent of whether output is atomic or the application result can
    /// be observed.
    pub trigger_precondition: String,
    /// How deletion and insertion are submitted to the client/compositor.
    pub output_transaction: String,
    /// Whether this route can observe that the application applied the text.
    /// Current routes cannot; user-confirmed desktop tests are separate.
    pub application_result: String,
    pub atomic_replace: bool,
    pub app_identity: String,
    pub focus_tracking: bool,
    pub status: String,
    pub setup_backend: String,
    pub setup_detail: String,
}

/// Checked-in operator guidance for one desktop environment. Technical
/// readiness still comes from the live assessment; this profile only links
/// that evidence to reviewed setup guidance and known limitations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesktopIntegrationProfile {
    pub id: String,
    pub desktop: String,
    pub recommended_route_id: String,
    pub supported_injection_backends: Vec<String>,
    pub required_permissions: Vec<String>,
    pub setup_instructions: Vec<String>,
    pub known_limitations: Vec<String>,
    pub verified_applications: Vec<String>,
    pub last_certification_result: String,
}

#[derive(Debug, Deserialize)]
pub struct SensitiveFieldSupport {
    pub implemented: bool,
    pub protocol_signal: String,
    pub compositor_observation: String,
    pub certified: bool,
}

impl RouteContract {
    pub fn is_certified(&self) -> bool {
        self.status == "certified"
    }

    pub fn capture_backend(&self) -> Option<RouteBackend> {
        RouteBackend::parse(&self.capture)
    }

    pub fn injection_backend(&self) -> Option<RouteBackend> {
        RouteBackend::parse(&self.injection)
    }

    pub fn requires_raw_input(&self) -> bool {
        self.capture == "evdev"
    }
}

#[derive(Debug, Deserialize)]
struct RouteCatalog {
    recommended_description: String,
    routes: Vec<RouteContract>,
    #[serde(default)]
    desktop_profiles: Vec<DesktopIntegrationProfile>,
}

static ROUTE_CATALOG: OnceLock<RouteCatalog> = OnceLock::new();

fn route_catalog() -> &'static RouteCatalog {
    ROUTE_CATALOG.get_or_init(|| {
        serde_json::from_str(include_str!("../routes.json"))
            .expect("checked-in certification route contract must be valid")
    })
}

pub fn recommended_mode_description() -> &'static str {
    &route_catalog().recommended_description
}

pub fn desktop_integration_profiles() -> &'static [DesktopIntegrationProfile] {
    &route_catalog().desktop_profiles
}

pub fn desktop_integration_profile(id: &str) -> Option<&'static DesktopIntegrationProfile> {
    desktop_integration_profiles()
        .iter()
        .find(|profile| profile.id == id)
}

pub fn injection_status_label(backend: &str) -> &str {
    match backend {
        "wlroots" => "wlroots-virtual-keyboard",
        other => other,
    }
}

pub fn route_contract_for(source: &str, injection: &str) -> Option<&'static RouteContract> {
    let (capture, injection) = match source {
        "input-method" | "input-method-v2" => ("input-method-v2", "input-method-v2"),
        other => (other, injection_status_label(injection)),
    };
    route_catalog()
        .routes
        .iter()
        .find(|route| route.capture == capture && route.injection == injection)
}

/// Return the stable catalog ID for a capture/injection pair reported by a
/// running daemon. The stdin route is a test harness and is intentionally not
/// part of the selectable production route catalog.
pub fn route_id_for(source: &str, injection: &str) -> &'static str {
    if source == "stdin" {
        return "stdin-test";
    }
    route_contract_for(source, injection)
        .map(|route| route.id.as_str())
        .unwrap_or("unknown")
}

pub fn route_contract_by_id(id: &str) -> Option<&'static RouteContract> {
    route_catalog().routes.iter().find(|route| route.id == id)
}

impl RecommendedRoute {
    pub fn contract(self) -> &'static RouteContract {
        self.0
    }

    pub fn id(self) -> &'static str {
        &self.0.id
    }

    pub fn capture_label(self) -> &'static str {
        &self.contract().capture
    }

    pub fn injection_label(self) -> &'static str {
        &self.contract().injection
    }

    pub fn setup_backend(self) -> &'static str {
        &self.contract().setup_backend
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteStanding {
    Recommended,
    Available,
    RequiresConsent,
    BlockedByPolicy,
    Unavailable,
}

impl RouteStanding {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Recommended => "recommended",
            Self::Available => "available",
            Self::RequiresConsent => "requires-consent",
            Self::BlockedByPolicy => "blocked-by-policy",
            Self::Unavailable => "unavailable",
        }
    }
}

#[derive(Debug, Clone)]
pub struct PlannedRoute {
    pub contract: &'static RouteContract,
    pub standing: RouteStanding,
    pub reason: &'static str,
    /// Normalized suitability and readiness facts used by CLI and GUI.
    pub assessment: BackendAssessment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssessmentEvidence {
    Yes,
    No,
    Unknown,
    RequiresAction,
}

impl AssessmentEvidence {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Yes => "yes",
            Self::No => "no",
            Self::Unknown => "unknown",
            Self::RequiresAction => "requires_action",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendLimitation {
    GlobalKeyboardObservation,
    NoSensitiveFieldDetection,
    NonAtomicReplacement,
    LayoutMayBeUnknown,
    PortalConsentRequired,
    NoCertifiedTargetIntegrity,
}

impl BackendLimitation {
    pub fn description(self) -> &'static str {
        match self {
            Self::GlobalKeyboardObservation => "Requires access to keyboard event devices and observes input globally.",
            Self::NoSensitiveFieldDetection => "Cannot reliably identify password or other sensitive fields.",
            Self::NonAtomicReplacement => "Trigger deletion and replacement insertion are separate operations.",
            Self::LayoutMayBeUnknown => "The active keyboard layout may be unavailable; the route must refuse matching rather than guess.",
            Self::PortalConsentRequired => "A desktop portal may request authorization before an EIS connection is available.",
            Self::NoCertifiedTargetIntegrity => "This route has not passed real-session target-integrity certification.",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PortalAuthorization {
    NotRequested,
    Granted,
    Denied,
    Unknown,
}

impl PortalAuthorization {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotRequested => "not_requested",
            Self::Granted => "granted",
            Self::Denied => "denied",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PortalProgress {
    /// Desktop-name discovery is a candidate hint, not a provider probe.
    pub provider_candidate: bool,
    /// No non-interactive implementation probe is currently available.
    pub implementation_discovered: AssessmentEvidence,
    pub interface_available: AssessmentEvidence,
    pub authorization: PortalAuthorization,
    pub direct_eis_socket_configured: bool,
    pub eis_connected: AssessmentEvidence,
    pub injection_negotiated: AssessmentEvidence,
    pub injection_verified: AssessmentEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BackendAssessment {
    pub backend_id: &'static str,
    pub installed: AssessmentEvidence,
    pub supported: AssessmentEvidence,
    pub authorized: AssessmentEvidence,
    pub connected: AssessmentEvidence,
    pub policy_allowed: bool,
    pub certified: bool,
    pub requires_user_action: bool,
    pub limitations: Vec<BackendLimitation>,
    pub portal: Option<PortalProgress>,
}

#[derive(Debug, Clone)]
pub struct RoutePlan {
    pub routes: Vec<PlannedRoute>,
}

impl RoutePlan {
    pub fn recommended(&self) -> Option<RecommendedRoute> {
        self.routes
            .iter()
            .find(|route| route.standing == RouteStanding::Recommended)
            .map(|route| RecommendedRoute(route.contract))
    }
}

pub(super) fn route_rank(route: &RouteContract) -> (bool, bool, u8) {
    let kind = match route.capture.as_str() {
        "input-method-v2" => 0,
        "ibus" => 1,
        _ => 2,
    };
    (route.requires_raw_input(), !route.is_certified(), kind)
}

fn route_availability(
    route: &RouteContract,
    capabilities: &Capabilities,
    ibus_available: bool,
) -> Result<(), &'static str> {
    let libei_reachable = capabilities.has_direct_libei_socket;
    match route.id.as_str() {
        "ibus" if ibus_available => Ok(()),
        "ibus" => Err("the WayExpand IBus engine is not installed or IBus is unavailable"),
        "input-method-v2" if !capabilities.has_input_method_v2 => {
            Err("the compositor does not offer zwp_input_method_manager_v2")
        }
        "input-method-v2"
            if !libei_reachable
                && !matches!(
                    capabilities.compositor,
                    Compositor::KdePlasma | Compositor::Gnome
                ) =>
        {
            Err("libei key pass-through is unavailable for the exclusive input-method grab")
        }
        "input-method-v2" => Ok(()),
        _ if route.requires_raw_input() && !capabilities.has_dev_input => {
            Err("no readable /dev/input keyboard")
        }
        "kde-evdev-libei"
            if !libei_reachable
                && !matches!(
                    capabilities.compositor,
                    Compositor::KdePlasma | Compositor::Gnome
                ) =>
        {
            Err("no direct EIS socket; desktop candidate discovery is unavailable")
        }
        "sway-evdev-wlroots" if !capabilities.has_virtual_keyboard => {
            Err("the compositor does not offer the wlroots virtual-keyboard protocol")
        }
        "kde-evdev-libei" | "sway-evdev-wlroots" => Ok(()),
        _ => Err("this route is not known to the planner"),
    }
}

pub fn plan_routes(
    capabilities: &Capabilities,
    ibus_available: bool,
    allowed: impl Fn(&RouteContract) -> bool,
) -> RoutePlan {
    let mut routes: Vec<PlannedRoute> = route_catalog()
        .routes
        .iter()
        .map(|contract| {
            let policy_allowed = allowed(contract);
            let (standing, reason) =
                match route_availability(contract, capabilities, ibus_available) {
                    Err(reason) => (RouteStanding::Unavailable, reason),
                    Ok(()) if !policy_allowed => (
                        RouteStanding::BlockedByPolicy,
                        "organization policy does not allow this route",
                    ),
                    Ok(()) if contract.requires_raw_input() => (
                        RouteStanding::RequiresConsent,
                        if uses_libei_passthrough(contract)
                            && !capabilities.has_direct_libei_socket
                        {
                            "portal authorization has not been requested; desktop name is only a provider candidate"
                        } else {
                            "raw keyboard capture needs explicit consent; never selected automatically"
                        },
                    ),
                    Ok(())
                        if uses_libei_passthrough(contract)
                            && !capabilities.has_direct_libei_socket
                            && matches!(capabilities.compositor, Compositor::KdePlasma | Compositor::Gnome) =>
                    {
                        (
                            RouteStanding::RequiresConsent,
                            "portal authorization has not been requested; desktop name is only a provider candidate",
                        )
                    }
                    Ok(()) => (RouteStanding::Available, "available"),
                };
            let assessment = assess_route(
                contract,
                capabilities,
                ibus_available,
                policy_allowed,
                standing,
            );
            PlannedRoute {
                contract,
                standing,
                reason,
                assessment,
            }
        })
        .collect();
    let standing_rank = |standing: RouteStanding| match standing {
        RouteStanding::Recommended | RouteStanding::Available => 0,
        RouteStanding::RequiresConsent => 1,
        RouteStanding::BlockedByPolicy => 2,
        RouteStanding::Unavailable => 3,
    };
    routes.sort_by_key(|route| (standing_rank(route.standing), route_rank(route.contract)));
    if let Some(best) = routes
        .iter_mut()
        .find(|route| route.standing == RouteStanding::Available)
    {
        best.standing = RouteStanding::Recommended;
        best.reason = if best.contract.is_certified() {
            "highest-ranked certified route"
        } else {
            "highest-ranked available route; experimental until certified"
        };
    }
    RoutePlan { routes }
}

fn assess_route(
    contract: &'static RouteContract,
    capabilities: &Capabilities,
    ibus_available: bool,
    policy_allowed: bool,
    standing: RouteStanding,
) -> BackendAssessment {
    let portal_candidate = matches!(
        capabilities.compositor,
        Compositor::KdePlasma | Compositor::Gnome
    );
    let ibus = contract.id == "ibus";
    let input_method = contract.id == "input-method-v2";
    let evdev = contract.requires_raw_input();
    let installed = if ibus {
        if ibus_available {
            AssessmentEvidence::Yes
        } else {
            AssessmentEvidence::No
        }
    } else {
        // The adapters are compiled into the WayExpand installation. Runtime
        // availability is represented independently in `supported`.
        AssessmentEvidence::Yes
    };
    let supported = if ibus {
        if ibus_available {
            AssessmentEvidence::Yes
        } else {
            AssessmentEvidence::No
        }
    } else if input_method {
        if !capabilities.has_input_method_v2 {
            AssessmentEvidence::No
        } else if capabilities.has_direct_libei_socket {
            AssessmentEvidence::Yes
        } else if portal_candidate {
            AssessmentEvidence::Unknown
        } else {
            AssessmentEvidence::No
        }
    } else if evdev && !capabilities.has_dev_input {
        AssessmentEvidence::No
    } else if contract.injection == "wlroots-virtual-keyboard" {
        if capabilities.has_virtual_keyboard {
            AssessmentEvidence::Yes
        } else {
            AssessmentEvidence::No
        }
    } else if contract.injection == "libei" && capabilities.has_direct_libei_socket {
        AssessmentEvidence::Yes
    } else {
        AssessmentEvidence::Unknown
    };
    let uses_libei = uses_libei_passthrough(contract);
    let portal = uses_libei.then_some(PortalProgress {
        provider_candidate: portal_candidate,
        implementation_discovered: AssessmentEvidence::Unknown,
        interface_available: AssessmentEvidence::Unknown,
        authorization: if portal_candidate && !capabilities.has_direct_libei_socket {
            PortalAuthorization::NotRequested
        } else {
            PortalAuthorization::Unknown
        },
        direct_eis_socket_configured: capabilities.has_direct_libei_socket,
        eis_connected: AssessmentEvidence::Unknown,
        injection_negotiated: AssessmentEvidence::Unknown,
        injection_verified: AssessmentEvidence::Unknown,
    });
    let mut limitations = Vec::new();
    if evdev {
        limitations.push(BackendLimitation::GlobalKeyboardObservation);
        if !contract.sensitive_fields {
            limitations.push(BackendLimitation::NoSensitiveFieldDetection);
        }
        limitations.push(BackendLimitation::LayoutMayBeUnknown);
    }
    if !contract.atomic_replace {
        limitations.push(BackendLimitation::NonAtomicReplacement);
    }
    if !contract.is_certified() {
        limitations.push(BackendLimitation::NoCertifiedTargetIntegrity);
    }
    if portal_candidate && uses_libei && !capabilities.has_direct_libei_socket {
        limitations.push(BackendLimitation::PortalConsentRequired);
    }
    BackendAssessment {
        backend_id: contract.id.as_str(),
        installed,
        supported,
        authorized: match portal.as_ref().map(|progress| progress.authorization) {
            Some(PortalAuthorization::Granted) => AssessmentEvidence::Yes,
            Some(PortalAuthorization::Denied) => AssessmentEvidence::No,
            Some(PortalAuthorization::NotRequested) => AssessmentEvidence::RequiresAction,
            Some(PortalAuthorization::Unknown) | None => AssessmentEvidence::Unknown,
        },
        // Static probes cannot establish that the daemon has connected.
        connected: AssessmentEvidence::Unknown,
        policy_allowed,
        certified: contract.is_certified(),
        requires_user_action: evdev
            || standing == RouteStanding::RequiresConsent
            || portal
                .as_ref()
                .is_some_and(|p| p.authorization == PortalAuthorization::NotRequested),
        limitations,
        portal,
    }
}

fn uses_libei_passthrough(contract: &RouteContract) -> bool {
    contract.injection == "libei" || contract.id == "input-method-v2"
}

pub fn setup_backend_allowed(policy: &wayexpand_core::OrganizationPolicy, backend: &str) -> bool {
    use wayexpand_core::{InjectorCapabilities, InputSourceCapabilities, ReplacementGuarantee};
    let enforcement = policy.effective_enforcement_policy();
    match backend {
        "ibus" => {
            enforcement.backend_allowed(wayexpand_backend_ibus::IBUS_BACKEND_NAME)
                && enforcement
                    .capability_violation_for_source(
                        wayexpand_backend_ibus::injector_capabilities(),
                        wayexpand_backend_ibus::source_capabilities(),
                    )
                    .is_none()
        }
        "input-method" => {
            let injector = InjectorCapabilities {
                atomic_replace: true,
                replacement_guarantee: ReplacementGuarantee::Atomic,
                full_unicode: true,
                ..InjectorCapabilities::default()
            };
            enforcement.backend_allowed("input-method-v2")
                && enforcement
                    .capability_violation_for_source(
                        injector,
                        InputSourceCapabilities::INPUT_METHOD_V2,
                    )
                    .is_none()
        }
        "evdev" => {
            let injector = InjectorCapabilities {
                replacement_guarantee: ReplacementGuarantee::BestEffort,
                ..InjectorCapabilities::default()
            };
            enforcement.backend_allowed("libei")
                && enforcement
                    .capability_violation_for_source(injector, InputSourceCapabilities::EVDEV)
                    .is_none()
        }
        _ => false,
    }
}

pub fn route_allowed_by_policy(
    policy: &wayexpand_core::OrganizationPolicy,
    route: &RouteContract,
) -> bool {
    setup_backend_allowed(policy, &route.setup_backend)
}

pub fn recommended_route(
    capabilities: &Capabilities,
    ibus_available: bool,
    allowed: impl Fn(&RouteContract) -> bool,
) -> Option<RecommendedRoute> {
    plan_routes(capabilities, ibus_available, allowed).recommended()
}

#[cfg(test)]
pub(super) fn catalog_for_tests() -> &'static [RouteContract] {
    &route_catalog().routes
}
