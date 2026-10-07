//! Route contracts, availability planning, and organization-policy filtering.

use serde::Deserialize;
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
    pub atomic_replace: bool,
    pub app_identity: String,
    pub focus_tracking: bool,
    pub status: String,
    pub setup_backend: String,
    pub setup_detail: String,
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

#[derive(Debug, Clone, Copy)]
pub struct PlannedRoute {
    pub contract: &'static RouteContract,
    pub standing: RouteStanding,
    pub reason: &'static str,
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
    let libei_reachable = capabilities.has_direct_libei_socket
        || matches!(
            capabilities.compositor,
            Compositor::KdePlasma | Compositor::Gnome
        );
    match route.id.as_str() {
        "ibus" if ibus_available => Ok(()),
        "ibus" => Err("the WayExpand IBus engine is not installed or IBus is unavailable"),
        "input-method-v2" if !capabilities.has_input_method_v2 => {
            Err("the compositor does not offer zwp_input_method_manager_v2")
        }
        "input-method-v2" if !libei_reachable => {
            Err("libei key pass-through is unavailable for the exclusive input-method grab")
        }
        "input-method-v2" => Ok(()),
        _ if route.requires_raw_input() && !capabilities.has_dev_input => {
            Err("no readable /dev/input keyboard")
        }
        "kde-evdev-libei" if !libei_reachable => Err("no libei/EIS path was detected"),
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
            let (standing, reason) =
                match route_availability(contract, capabilities, ibus_available) {
                    Err(reason) => (RouteStanding::Unavailable, reason),
                    Ok(()) if !allowed(contract) => (
                        RouteStanding::BlockedByPolicy,
                        "organization policy does not allow this route",
                    ),
                    Ok(()) if contract.requires_raw_input() => (
                        RouteStanding::RequiresConsent,
                        "raw keyboard capture needs explicit consent; never selected automatically",
                    ),
                    Ok(()) => (RouteStanding::Available, "available"),
                };
            PlannedRoute {
                contract,
                standing,
                reason,
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

pub fn setup_backend_allowed(policy: &wayexpand_core::OrganizationPolicy, backend: &str) -> bool {
    match backend {
        "ibus" => {
            let enforcement = policy.effective_enforcement_policy();
            policy.backend_allowed(wayexpand_backend_ibus::IBUS_BACKEND_NAME)
                && enforcement
                    .capability_violation_for_source(
                        wayexpand_backend_ibus::injector_capabilities(),
                        wayexpand_backend_ibus::source_capabilities(),
                    )
                    .is_none()
        }
        "input-method" => policy.backend_allowed("input-method-v2"),
        "evdev" => policy.backend_allowed("libei"),
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
