//! Unix-only text expansion engine and secure configuration loader.

/// Version of the daemon's stable key/value control-socket status contract.
/// Increment when a consumer must distinguish a newer required field set.
pub const CONTROL_STATUS_SCHEMA: u32 = 44;

pub use limits::{CommandLimits, COMMAND_LIMITS, CONTROL_MAX_RESPONSE_BYTES};
pub use wayexpand_process_supervisor::containment_mode as command_containment_mode;
pub use wayexpand_process_supervisor::containment_policy_mode as command_containment_policy_mode;

#[cfg(not(unix))]
compile_error!("wayexpand-core currently requires a Unix target");

mod autokey_import;
mod backend;
mod capabilities;
mod config;
mod daemon_client;
mod daemon_status;
mod engine;
mod fleet;
#[cfg(any(test, feature = "fuzzing"))]
#[doc(hidden)]
pub mod fuzzing;
mod insertion_plan;
mod keys;
pub mod limits;
mod matcher;
mod migration;
mod pack;
mod paths;
mod policy;
mod profiles;
mod session_context;
mod setup_resume;
mod store;
mod template;
mod template_dynamic;
mod usage;

pub use autokey_import::{
    import_autokey, AutoKeyImport, AutoKeyImportError, AutoKeyImportMode, AutoKeyImportReport,
    AutoKeyImportWarning,
};
pub use backend::{
    discover_backends, keystroke_max_chars, set_keystroke_max_chars, window_update_channel,
    BackendKind, BackendState, BackendStatus, FocusCheck, FocusStamp, FocusWitness,
    InjectorCapabilities, InjectorError, InjectorErrorKind, InputSource, InputSourceCapabilities,
    InputSourceError, KeyEventState, OutputCheckpoint, OutputProgress, ProcessIdentityState,
    ReplacementAdmission, ReplacementGuarantee, TextInjector, WindowIdentitySource, WindowTracker,
    WindowTrackerError, WindowUpdateReceiver, WindowUpdateSendError, WindowUpdateSender,
};
pub use capabilities::{all_capabilities, Capabilities, TextMethod};
pub use config::{
    is_reserved_process_app_id, validate_command_config, AppFilter, ClipboardMode, CommandConfig,
    CommandEnvironment, CompatibilityMode, Config, ConfigError, ConfigRevision, ExpansionConfig,
    ExpansionDelivery, FontScale, HotkeyConfig, LoadedConfig, MatchMode, OrganizationPolicy,
    Settings, TerminalPaste, ValidatedConfig, DEFAULT_KEYSTROKE_MAX_CHARS, MAX_KEYSTROKE_MAX_CHARS,
    PROCESS_EXECUTABLE_APP_ID_PREFIX, PROCESS_NAME_APP_ID_PREFIX,
};
pub use daemon_client::{DaemonClient, DaemonClientError, DaemonFocus, DaemonOperation};
pub use daemon_status::{DaemonRouteState, DaemonStatus};
pub use engine::route_limit;
pub use engine::{
    command_preview_sandboxed, run_command, run_command_cancellable, run_command_preview,
    BrokerOperation, BrokerProtocolFailure, BrokerUnavailableReason, CheckStatus, CommandError,
    CommandErrorKind, CommandMetrics, CompletionNotifier, ExpansionEngine, ExpansionError,
    ExpansionResult, ExplainCheck, Explanation, HotkeyError, HotkeyResult, InputEvent, InsertError,
    PendingExpansionDispatch, PendingExpansionResult, ProcessWaitFailure, ProcessWaitOperation,
    RouteLimit, SensitiveText, TransactionOutcome, TransactionPhase, TransactionReport,
    WindowContext, MAX_WINDOW_INSTANCE_ID_BYTES,
};
pub use fleet::{FleetConfig, FleetError, Layer, MergeStats, Provenance};
pub use insertion_plan::{
    paced_output_duration_ms, paced_replacement_event_count, plan_insertion, plan_replacement,
    InsertionRouteCapabilities, InsertionStrategy,
};
pub use keys::{KeyChord, KeyChordError, Modifiers};
pub use matcher::{Matcher, MatcherBuildError};
pub use migration::{
    import_espanso, import_espanso_with_mode, EspansoImport, EspansoImportMode,
    EspansoImportReport, EspansoImportWarning, EspansoUnsupportedMatch, MigrationError,
};
pub use pack::{
    import_pack, inspect_pack, is_valid_pack_id, pack_digest, pack_fingerprint, sign_pack,
    trusted_signers_file, verify_pack_signature, PackCompatibility, PackError, PackExpansionReview,
    PackInspection, PackManifest, PackReview, PackRisk, PackVerification, SIGNATURE_FILE,
};
pub use paths::default_config_path;
pub use policy::{
    load_organization_policy, load_organization_policy_from_paths, parse_organization_policy,
    policy_backend_name, pre_flight_check, validate_organization_policy_directory,
    validate_organization_policy_file, MAX_ORGANIZATION_POLICY_BYTES, ORGANIZATION_POLICY_DIR,
    ORGANIZATION_POLICY_PATH,
};
pub use profiles::{ProfileAssignments, ProfileAssignmentsError};
pub use session_context::SessionContextReport;
pub use setup_resume::{
    clear_gnome_ibus_setup_resume, gnome_ibus_setup_resume_marker_present,
    gnome_ibus_setup_resume_pending, gnome_ibus_setup_resume_pending_for_session,
    gnome_ibus_setup_resume_transaction_pending, mark_gnome_ibus_setup_resume,
};
pub use store::{ConfigSnapshot, ConfigStore, ConfigStoreStatus};
pub use template::{
    form_fields, render_template, render_template_preview, render_template_with_cursor,
    template_variables, today_local_date, ClipboardPrefetch, ClipboardReader, CompiledTemplate,
    DateError, FormField, FormFieldKind, FormFieldValidation, FormValueError, FormValues,
    SnippetLibrary, TemplateContext, TemplateError,
};
pub use usage::{
    latest_usage_event_sequence, trigger_risks, usage_stats_path, SnippetUsage, TriggerRisk,
    UsageEvent, UsageLine, UsageReport, UsageStats,
};
pub use wayexpand_broker_client::ActionError;
