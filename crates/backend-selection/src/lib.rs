//! Shared capability probing, backend resolution, and operator explanation.
//!
//! The daemon and CLI must ask this library the same question: given the
//! current session and any explicit overrides, what source/backend pair can
//! WayExpand actually start? Keeping probing, policy, and explanation here
//! prevents the user-facing answer from drifting away from daemon behavior.

use serde::Deserialize;
use std::{env, fmt, sync::OnceLock};
use tracing::{debug, warn};
use wayexpand_backend_evdev::readable_keyboard_available;
use wayexpand_backend_input_method::InputMethodSource;
use wayexpand_backend_kwin_window::KwinWindowTracker;
use wayexpand_backend_wlroots::WlrootsInjector;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjectorBackend {
    None,
    Libei,
    Wlroots,
}

/// Backends that can appear in the checked-in route contract. Keeping this
/// mapping here makes consumers exhaustive over the contract vocabulary
/// instead of independently matching its string representation.
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

/// A route that `setup --mode recommended` can configure without an expert
/// backend override. Raw evdev is intentionally absent: it remains an
/// explicit maximum-compatibility opt-in because it can observe password
/// fields. Keep this in the shared selection crate so GUI copy, CLI setup,
/// doctor, and certification describe the same topology.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecommendedRoute {
    IBus,
}

/// Operator-facing route metadata shared by setup, diagnostics, GUI copy,
/// and certification documentation. The data is maintained in the checked-in
/// certification matrix so route guarantees cannot silently diverge between
/// frontends.
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

/// Separates an implemented safety mechanism from live compositor observation
/// and reviewed certification evidence.
#[derive(Debug, Deserialize)]
pub struct SensitiveFieldSupport {
    pub implemented: bool,
    pub protocol_signal: String,
    pub compositor_observation: String,
    pub certified: bool,
}

impl RouteContract {
    pub fn capture_backend(&self) -> Option<RouteBackend> {
        RouteBackend::parse(&self.capture)
    }

    pub fn injection_backend(&self) -> Option<RouteBackend> {
        RouteBackend::parse(&self.injection)
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

/// Exact operator-facing description of Recommended mode. Keep published
/// documentation synchronized with this catalog value.
pub fn recommended_mode_description() -> &'static str {
    &route_catalog().recommended_description
}

/// Canonical status/catalog label for an output backend name. Selection and
/// the CLI use the short spelling `wlroots`, while the injector and the route
/// catalog name that output `wlroots-virtual-keyboard`. The daemon publishes
/// this label so status consumers never see two spellings of one backend.
pub fn injection_status_label(backend: &str) -> &str {
    match backend {
        "wlroots" => "wlroots-virtual-keyboard",
        other => other,
    }
}

/// Find the route contract represented by daemon status fields. The
/// input-method daemon uses `source=input-method` while the contract names
/// that capture path `input-method-v2`; keep that compatibility alias here so
/// GUI and CLI consumers do not duplicate it.
///
/// Input-method-v2 captures and injects through one protocol object, so its
/// route is identified by the source alone: status may carry the resolved
/// pair's `none`, the canonical `input-method-v2`, or the libei key
/// pass-through while that is reconnecting.
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
        let id = match self {
            Self::IBus => "ibus",
        };
        route_catalog()
            .routes
            .iter()
            .find(|route| route.id == id)
            .unwrap_or_else(|| panic!("missing route contract {id:?}"))
    }
}

impl RecommendedRoute {
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

/// Resolve the one-click/recommended route from one capability snapshot.
/// Policy decisions are explicit inputs so callers cannot silently fall back
/// to a different route after the UI has displayed its recommendation.
pub fn recommended_route(
    capabilities: &Capabilities,
    ibus_available: bool,
    input_method_allowed: bool,
) -> Option<RecommendedRoute> {
    if ibus_available && input_method_allowed {
        return Some(RecommendedRoute::IBus);
    }
    let _ = capabilities;
    None
}

impl InjectorBackend {
    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Libei => "libei",
            Self::Wlroots => "wlroots",
        }
    }
}

/// A source/output combination that the daemon can actually start.
///
/// Input-method-v2 is deliberately its own variant because it captures and
/// injects text through the same protocol object. It cannot be combined with
/// a separate output injector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedBackendPair {
    InputMethod,
    Evdev(InjectorBackend),
    Stdin(InjectorBackend),
}

impl ResolvedBackendPair {
    pub fn source(self) -> &'static str {
        match self {
            Self::InputMethod => "input-method",
            Self::Evdev(_) => "evdev",
            Self::Stdin(_) => "stdin",
        }
    }

    pub fn backend(self) -> &'static str {
        match self {
            Self::InputMethod => "none",
            Self::Evdev(backend) | Self::Stdin(backend) => backend.name(),
        }
    }

    fn capture_label(self) -> &'static str {
        match self {
            Self::InputMethod => "input-method-v2",
            Self::Evdev(_) => "evdev",
            Self::Stdin(_) => "stdin",
        }
    }

    fn injection_label(self) -> &'static str {
        match self {
            Self::InputMethod => "input-method-v2",
            Self::Evdev(backend) | Self::Stdin(backend) => backend.name(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendSelectionError {
    UnknownSource(String),
    UnknownBackend(String),
    Incompatible { source: String, backend: String },
    UnavailableSource { source: String, detail: String },
}

impl fmt::Display for BackendSelectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownSource(source) => write!(
                formatter,
                "unknown source {source:?}; expected stdin, input-method, or evdev"
            ),
            Self::UnknownBackend(backend) => write!(
                formatter,
                "unknown backend {backend:?}; expected none, wlroots, or libei"
            ),
            Self::Incompatible { source, backend } => write!(
                formatter,
                "source {source:?} cannot be combined with output backend {backend:?}"
            ),
            Self::UnavailableSource { source, detail } => {
                write!(
                    formatter,
                    "requested source {source:?} is unavailable: {detail}"
                )
            }
        }
    }
}

impl std::error::Error for BackendSelectionError {}

#[derive(Debug, Clone)]
pub struct BackendSelection {
    pub pair: ResolvedBackendPair,
    /// Why this selection was made (for logging and operator explanation).
    pub reason: String,
    /// The capabilities used by the resolver.
    pub capabilities: Capabilities,
}

/// Probed capabilities for the current session.
#[derive(Debug, Clone, Default)]
pub struct Capabilities {
    pub has_input_method_v2: bool,
    pub has_virtual_keyboard: bool,
    pub has_direct_libei_socket: bool,
    pub has_dev_input: bool,
    pub has_window_tracker: bool,
    pub compositor: Compositor,
}

/// Best-effort compositor classification used in the shared explanation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Compositor {
    KdePlasma,
    Sway,
    Hyprland,
    River,
    Gnome,
    X11,
    #[default]
    Unknown,
}

impl Compositor {
    pub fn name(self) -> &'static str {
        match self {
            Self::KdePlasma => "KDE Plasma",
            Self::Sway => "Sway/wlroots",
            Self::Hyprland => "Hyprland/wlroots",
            Self::River => "river/wlroots",
            Self::Gnome => "GNOME",
            Self::X11 => "X11",
            Self::Unknown => "unknown",
        }
    }

    fn detect() -> Self {
        if let Ok(desktop) = env::var("XDG_CURRENT_DESKTOP") {
            let desktop_lower = desktop.to_lowercase();
            if desktop_lower.contains("kde") {
                return Self::KdePlasma;
            }
            if desktop_lower.contains("gnome") {
                return Self::Gnome;
            }
            if desktop_lower.contains("hypr") {
                return Self::Hyprland;
            }
            if desktop_lower.contains("river") {
                return Self::River;
            }
            if desktop_lower.contains("sway") {
                return Self::Sway;
            }
        }

        // Check for Wayland before X11. Modern Wayland sessions often have
        // both WAYLAND_DISPLAY and DISPLAY set (XWayland), so checking DISPLAY
        // first would incorrectly label Wayland+XWayland as pure X11.
        if env::var_os("WAYLAND_DISPLAY").is_some() || env::var_os("WAYLAND_SOCKET").is_some() {
            return Self::Unknown;
        }

        // A Wayland session identifies the display protocol, not the
        // compositor or its implementation family. Do not turn an
        // unrecognized compositor into Sway/wlroots based on environment
        // variables alone; protocol probes and explicit desktop identifiers
        // are the only evidence used for a concrete classification.
        if env::var_os("DISPLAY").is_some() {
            return Self::X11;
        }
        Self::Unknown
    }
}

/// Probe for actual capabilities available in the session.
pub fn probe_capabilities() -> Capabilities {
    let is_wayland =
        env::var_os("WAYLAND_DISPLAY").is_some() || env::var_os("WAYLAND_SOCKET").is_some();
    let has_dev_input = readable_keyboard_available();
    let has_input_method_v2 = is_wayland && InputMethodSource::probe().is_ok();
    let has_virtual_keyboard = is_wayland && WlrootsInjector::probe().is_ok();
    // Portal probing would show a consent dialog. An existing direct EIS
    // socket is safe to recognize; portal availability remains an explicit
    // startup decision and is reported by doctor.
    let has_direct_libei_socket = env::var_os("LIBEI_SOCKET").is_some();
    // KWin window tracking is only available in KDE Plasma sessions over D-Bus.
    // Probe is relatively fast and safe; it checks for KWin availability without
    // interfering with the session. This ensures CLI and daemon report consistent
    // window tracking capability.
    let has_window_tracker = KwinWindowTracker::probe().is_ok();
    let capabilities = Capabilities {
        has_input_method_v2,
        has_virtual_keyboard,
        has_direct_libei_socket,
        has_dev_input,
        has_window_tracker,
        compositor: Compositor::detect(),
    };
    debug!(?capabilities, "probed backend capabilities");
    capabilities
}

/// Pure policy: select a compatible source/backend pair from capabilities.
pub fn select_backend(
    capabilities: &Capabilities,
    explicit_source: Option<&str>,
    explicit_backend: Option<&str>,
) -> Result<BackendSelection, BackendSelectionError> {
    let pair = if let (Some(source), Some(backend)) = (explicit_source, explicit_backend) {
        match source {
            "input-method" if backend == "none" && capabilities.has_input_method_v2 => {
                ResolvedBackendPair::InputMethod
            }
            "input-method" if backend == "none" => {
                return Err(BackendSelectionError::UnavailableSource {
                    source: "input-method".into(),
                    detail: "input-method-v2 manager and seat probe failed".into(),
                })
            }
            "input-method" => {
                return Err(BackendSelectionError::Incompatible {
                    source: source.to_string(),
                    backend: backend.to_string(),
                })
            }
            "evdev" => {
                let backend = parse_backend(backend)?;
                if capabilities.has_dev_input {
                    return Ok(BackendSelection {
                        pair: ResolvedBackendPair::Evdev(backend),
                        reason: "explicit evdev source requested".into(),
                        capabilities: capabilities.clone(),
                    });
                }
                return Err(BackendSelectionError::UnavailableSource {
                    source: "evdev".into(),
                    detail: "/dev/input has no readable keyboard devices".into(),
                });
            }
            "stdin" => ResolvedBackendPair::Stdin(parse_backend(backend)?),
            other => return Err(BackendSelectionError::UnknownSource(other.to_string())),
        }
    } else if let Some(source) = explicit_source {
        match source {
            "input-method" if capabilities.has_input_method_v2 => ResolvedBackendPair::InputMethod,
            "input-method" => {
                return Err(BackendSelectionError::UnavailableSource {
                    source: "input-method".into(),
                    detail: "input-method-v2 manager and seat probe failed".into(),
                });
            }
            "evdev" if capabilities.has_dev_input => {
                ResolvedBackendPair::Evdev(InjectorBackend::Libei)
            }
            "evdev" => {
                return Err(BackendSelectionError::UnavailableSource {
                    source: "evdev".into(),
                    detail: "/dev/input has no readable keyboard devices".into(),
                });
            }
            "stdin" => ResolvedBackendPair::Stdin(InjectorBackend::Libei),
            other => return Err(BackendSelectionError::UnknownSource(other.to_string())),
        }
    } else if let Some(backend) = explicit_backend {
        let backend = parse_backend(backend)?;
        match backend {
            InjectorBackend::None => ResolvedBackendPair::Stdin(InjectorBackend::None),
            // Choosing an output backend does not acknowledge global raw
            // keyboard capture. Keep the complementary source on stdin
            // unless the user explicitly requested --source=evdev.
            InjectorBackend::Libei | InjectorBackend::Wlroots => {
                ResolvedBackendPair::Stdin(backend)
            }
        }
    } else {
        if capabilities.has_dev_input {
            warn!(
                "stdin-only mode: keyboard input is NOT being monitored. \
                To capture keyboard, run with --source=evdev or --source=input-method. \
                Readable evdev devices detected but not used without explicit request."
            );
        } else if capabilities.has_input_method_v2 {
            warn!(
                "stdin-only mode: keyboard input is NOT being monitored. \
                To enable text expansion, run with --source=input-method. \
                No /dev/input devices readable in this session."
            );
        } else {
            warn!(
                "stdin-only mode: keyboard input is NOT being monitored. \
                Text expansion must be triggered via stdin or --pipe. \
                Run `wayexpand doctor` to see available input methods."
            );
        }
        ResolvedBackendPair::Stdin(InjectorBackend::Libei)
    };

    let reason = selection_reason(capabilities, pair, explicit_source, explicit_backend);
    Ok(BackendSelection {
        pair,
        reason,
        capabilities: capabilities.clone(),
    })
}

fn selection_reason(
    capabilities: &Capabilities,
    pair: ResolvedBackendPair,
    explicit_source: Option<&str>,
    explicit_backend: Option<&str>,
) -> String {
    if explicit_source.is_some() && explicit_backend.is_some() {
        return "user-specified source and backend".to_string();
    }
    if let Some(source) = explicit_source {
        if source == "evdev" && !capabilities.has_dev_input {
            return "evdev requested but /dev/input not readable; using stdin instead".to_string();
        }
        return format!("user-specified source: {source}");
    }
    if let Some(backend) = explicit_backend {
        return format!(
            "user-specified backend: {backend}; stdin remains the source unless --source=evdev is also specified"
        );
    }
    match pair {
        ResolvedBackendPair::Stdin(InjectorBackend::Libei) => {
            let availability = if capabilities.has_dev_input {
                "readable evdev is available but disabled by default; use --source=evdev to acknowledge global keyboard capture"
            } else {
                "no readable evdev device is available"
            };
            format!("conservative default: stdin + libei ({availability})")
        }
        _ => format!("selected {} + {}", pair.source(), pair.backend()),
    }
}

fn parse_backend(backend: &str) -> Result<InjectorBackend, BackendSelectionError> {
    match backend {
        "none" => Ok(InjectorBackend::None),
        "libei" => Ok(InjectorBackend::Libei),
        "wlroots" => Ok(InjectorBackend::Wlroots),
        other => Err(BackendSelectionError::UnknownBackend(other.to_string())),
    }
}

/// Resolve automatic and explicit selection using live session capabilities.
pub fn auto_select(
    explicit_source: Option<&str>,
    explicit_backend: Option<&str>,
) -> Result<BackendSelection, BackendSelectionError> {
    let capabilities = probe_capabilities();
    select_backend(&capabilities, explicit_source, explicit_backend)
}

/// Render the same selection result used by the daemon for operators.
pub fn explain_auto_selection() -> Result<String, BackendSelectionError> {
    let selection = auto_select(None, None)?;
    Ok(selection.explanation())
}

impl BackendSelection {
    pub fn explanation(&self) -> String {
        let capabilities = &self.capabilities;
        let tracking = if capabilities.has_window_tracker {
            match capabilities.compositor {
                Compositor::KdePlasma => "kwin (best-effort)",
                _ => "not shipped",
            }
        } else {
            "none"
        };
        let desktop = capabilities.compositor.name();
        let mut output = String::new();
        use fmt::Write as _;
        writeln!(output, "Selected:").unwrap();
        writeln!(output, "  capture: {}", self.pair.capture_label()).unwrap();
        writeln!(output, "  injection: {}", self.pair.injection_label()).unwrap();
        writeln!(output, "  window tracking: {tracking}").unwrap();
        writeln!(output, "\nReason:").unwrap();
        writeln!(output, "  {}", self.reason).unwrap();
        writeln!(output, "  compositor: {desktop}").unwrap();
        writeln!(
            output,
            "  /dev/input readable: {}",
            if capabilities.has_dev_input {
                "yes"
            } else {
                "no"
            }
        )
        .unwrap();
        if capabilities.has_dev_input && self.pair.source() != "evdev" {
            writeln!(
                output,
                "  evdev automatic use: disabled; pass --source=evdev to acknowledge global keyboard capture"
            )
            .unwrap();
        }
        writeln!(
            output,
            "  direct LIBEI_SOCKET: {}",
            if capabilities.has_direct_libei_socket {
                "yes"
            } else {
                "no"
            }
        )
        .unwrap();
        writeln!(output, "\nSecurity tradeoffs:").unwrap();
        match self.pair {
            ResolvedBackendPair::Evdev(_) => {
                writeln!(output, "  password-field detection unavailable with evdev").unwrap();
                writeln!(
                    output,
                    "  global keyboard visibility requires explicit input permissions"
                )
                .unwrap();
            }
            ResolvedBackendPair::InputMethod => {
                writeln!(
                    output,
                    "  experimental opt-in only: libei key pass-through exists but still requires compositor certification"
                )
                .unwrap();
                writeln!(
                    output,
                    "  keyboard fidelity remains unvalidated for modifiers, repeats, and shortcuts"
                )
                .unwrap();
            }
            ResolvedBackendPair::Stdin(_) => {
                writeln!(output, "  no automatic input path is selected").unwrap();
            }
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capabilities(
        has_input_method_v2: bool,
        has_dev_input: bool,
        has_direct_libei_socket: bool,
    ) -> Capabilities {
        Capabilities {
            has_input_method_v2,
            has_virtual_keyboard: true,
            has_direct_libei_socket,
            has_dev_input,
            has_window_tracker: false,
            compositor: Compositor::Gnome,
        }
    }

    #[test]
    fn automatic_resolution_does_not_enable_raw_evdev() {
        let selection = select_backend(&capabilities(true, true, false), None, None).unwrap();
        assert_eq!(
            selection.pair,
            ResolvedBackendPair::Stdin(InjectorBackend::Libei)
        );
        assert!(selection.reason.contains("disabled by default"));
        assert!(selection.explanation().contains("capture: stdin"));
        assert!(selection.explanation().contains("injection: libei"));
        assert!(selection
            .explanation()
            .contains("evdev automatic use: disabled"));
    }

    #[test]
    fn input_method_remains_available_as_an_explicit_opt_in() {
        let selection =
            select_backend(&capabilities(true, true, false), Some("input-method"), None).unwrap();
        assert_eq!(selection.pair, ResolvedBackendPair::InputMethod);
        assert!(selection.explanation().contains("experimental opt-in only"));
    }

    #[test]
    fn explicit_input_method_fails_closed_when_probe_is_unavailable() {
        let result = select_backend(
            &capabilities(false, true, false),
            Some("input-method"),
            None,
        );
        assert!(matches!(
            result,
            Err(BackendSelectionError::UnavailableSource { source, .. })
                if source == "input-method"
        ));

        let result = select_backend(
            &capabilities(false, true, false),
            Some("input-method"),
            Some("none"),
        );
        assert!(matches!(
            result,
            Err(BackendSelectionError::UnavailableSource { source, .. })
                if source == "input-method"
        ));
    }

    #[test]
    fn automatic_fallback_also_avoids_input_method_without_evdev() {
        let selection = select_backend(&capabilities(true, false, false), None, None).unwrap();
        assert_eq!(
            selection.pair,
            ResolvedBackendPair::Stdin(InjectorBackend::Libei)
        );
        assert!(selection.reason.contains("no readable evdev device"));
    }

    #[test]
    fn explicit_injector_does_not_imply_raw_capture() {
        let result = select_backend(&capabilities(true, true, false), None, Some("libei")).unwrap();
        assert_eq!(
            result.pair,
            ResolvedBackendPair::Stdin(InjectorBackend::Libei)
        );
        assert!(result.reason.contains("--source=evdev"));
    }

    #[test]
    fn input_method_cannot_be_combined_with_an_output_backend() {
        let result = select_backend(
            &Capabilities::default(),
            Some("input-method"),
            Some("libei"),
        );
        assert!(matches!(
            result,
            Err(BackendSelectionError::Incompatible { .. })
        ));
    }

    #[test]
    fn explicit_and_partial_requests_are_resolved_once() {
        assert_eq!(
            select_backend(&capabilities(false, true, false), Some("evdev"), None)
                .unwrap()
                .pair,
            ResolvedBackendPair::Evdev(InjectorBackend::Libei)
        );
        assert_eq!(
            select_backend(&capabilities(false, true, false), None, Some("wlroots"))
                .unwrap()
                .pair,
            ResolvedBackendPair::Stdin(InjectorBackend::Wlroots)
        );
    }

    #[test]
    fn explicit_evdev_fails_closed_when_devices_are_unreadable() {
        let result = select_backend(&capabilities(false, false, false), Some("evdev"), None);
        assert!(matches!(
            result,
            Err(BackendSelectionError::UnavailableSource { source, .. }) if source == "evdev"
        ));
        let result = select_backend(
            &capabilities(false, false, false),
            Some("evdev"),
            Some("libei"),
        );
        assert!(matches!(
            result,
            Err(BackendSelectionError::UnavailableSource { source, .. }) if source == "evdev"
        ));
    }

    #[test]
    fn fallback_selection_is_explained_by_the_shared_result() {
        let selection = select_backend(&capabilities(false, false, false), None, None).unwrap();
        assert_eq!(
            selection.pair,
            ResolvedBackendPair::Stdin(InjectorBackend::Libei)
        );
        assert!(selection.explanation().contains("conservative default"));
    }

    #[test]
    fn recommended_route_is_shared_and_policy_aware() {
        let capabilities = capabilities(false, true, true);
        assert_eq!(
            recommended_route(&capabilities, true, true),
            Some(RecommendedRoute::IBus)
        );
        assert_eq!(recommended_route(&capabilities, true, false), None);
        assert_eq!(recommended_route(&capabilities, false, true), None);
    }

    #[test]
    fn recommended_route_never_selects_raw_input() {
        let capabilities = capabilities(true, true, true);
        assert_eq!(recommended_route(&capabilities, false, false), None);
    }

    #[test]
    fn recommended_routes_have_complete_operator_contracts() {
        let route = RecommendedRoute::IBus;
        let contract = route.contract();
        assert!(!contract.id.is_empty());
        assert!(!contract.capture.is_empty());
        assert!(!contract.injection.is_empty());
        assert!(!contract.setup_backend.is_empty());
        assert!(!contract.setup_detail.is_empty());
        assert_eq!(route.setup_backend(), contract.setup_backend);
        let description = recommended_mode_description();
        assert!(description.contains("IBus"));
        assert!(description.contains("evdev"));
        assert!(description.contains("never automatically"));
    }

    #[test]
    fn daemon_route_status_resolves_through_the_shared_contract() {
        let contract = route_contract_for("input-method", "input-method-v2").unwrap();
        assert_eq!(contract.id, "input-method-v2");
        assert!(route_contract_for("unknown", "backend").is_none());
    }

    #[test]
    fn every_resolved_pair_status_resolves_to_a_route_contract() {
        // The exact (source, backend) values the resolver hands the daemon,
        // plus their canonical status labels, must all find a contract.
        let input_method = ResolvedBackendPair::InputMethod;
        assert_eq!(
            route_contract_for(input_method.source(), input_method.backend())
                .unwrap()
                .id,
            "input-method-v2"
        );
        assert_eq!(
            route_contract_for("input-method", "none").unwrap().id,
            "input-method-v2"
        );
        assert_eq!(
            route_contract_for("input-method", "libei").unwrap().id,
            "input-method-v2"
        );
        for (pair, id) in [
            (
                ResolvedBackendPair::Evdev(InjectorBackend::Libei),
                "kde-evdev-libei",
            ),
            (
                ResolvedBackendPair::Evdev(InjectorBackend::Wlroots),
                "sway-evdev-wlroots",
            ),
        ] {
            assert_eq!(
                route_contract_for(pair.source(), pair.backend())
                    .unwrap()
                    .id,
                id
            );
            assert_eq!(
                route_contract_for(pair.source(), injection_status_label(pair.backend()))
                    .unwrap()
                    .id,
                id
            );
        }
        assert_eq!(
            route_contract_for("evdev", "wlroots-virtual-keyboard")
                .unwrap()
                .id,
            "sway-evdev-wlroots"
        );
        assert!(route_contract_for("evdev", "none").is_none());
        assert!(route_contract_for("stdin", "none").is_none());
    }
}
