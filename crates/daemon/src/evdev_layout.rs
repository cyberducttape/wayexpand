//! Compositor-backed XKB layout state for raw evdev capture.
//!
//! KDE exposes the active XKB group and layout list over session D-Bus. The
//! provider cross-checks them against KWin's config, or systemd-localed's
//! keyboard properties when KWin has no per-user config, and publishes
//! snapshots from a worker so the input reactor never performs I/O.

use std::{
    collections::hash_map::DefaultHasher,
    env, fs,
    hash::{Hash, Hasher},
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use wayexpand_backend_evdev::{KeyboardLayoutProvider, KeyboardLayoutSnapshot};
use xkbcommon_rs::{xkb_keymap::RuleNames, Context, Keymap};
use zbus::blocking::{connection::Builder, Connection, Proxy};

const DBUS_TIMEOUT: Duration = Duration::from_millis(250);
const LAYOUT_POLL_INTERVAL: Duration = Duration::from_millis(20);
const UNAVAILABLE_RETRY_INTERVAL: Duration = Duration::from_millis(500);
const INITIAL_SNAPSHOT_WAIT: Duration = Duration::from_millis(300);
const HYPRLAND_SOCKET_TIMEOUT: Duration = Duration::from_millis(100);
const HYPRLAND_RESPONSE_LIMIT: usize = 256 * 1024;
const HYPRLAND_POLL_INTERVAL: Duration = Duration::from_millis(100);

pub(crate) fn provider() -> Box<dyn KeyboardLayoutProvider> {
    if env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        Box::new(HyprlandKeyboardLayoutProvider::new())
    } else if env::var_os("SWAYSOCK").is_some() {
        // Sway's get_inputs reports localized layout names, not the effective
        // XKB rules/model/variant/options. Do not infer those from localed or
        // a config file: includes, device overrides and runtime changes make
        // that an unverifiable approximation.
        Box::new(NoLayoutProvider)
    } else {
        Box::new(KwinKeyboardLayoutProvider::new())
    }
}

struct NoLayoutProvider;

impl KeyboardLayoutProvider for NoLayoutProvider {
    fn snapshot(&mut self) -> Option<KeyboardLayoutSnapshot> {
        None
    }
}

/// Tracks Hyprland's active keymap from its compositor-owned device data.
/// Evdev uses one shared XKB state, so differing per-device maps are refused.
pub(crate) struct HyprlandKeyboardLayoutProvider {
    shared: Arc<SharedSnapshot>,
    stopping: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl HyprlandKeyboardLayoutProvider {
    pub(crate) fn new() -> Self {
        let shared = Arc::new(SharedSnapshot {
            value: Mutex::new(None),
            first_attempt: Mutex::new(false),
            ready: Condvar::new(),
        });
        let stopping = Arc::new(AtomicBool::new(false));
        let worker_shared = Arc::clone(&shared);
        let worker_stopping = Arc::clone(&stopping);
        let worker = thread::Builder::new()
            .name("wayexpand-hyprland-layout".into())
            .spawn(move || hyprland_layout_worker(worker_shared, worker_stopping))
            .ok();
        if let Ok(attempted) = shared.first_attempt.lock() {
            if !*attempted {
                let _ = shared.ready.wait_timeout_while(
                    attempted,
                    INITIAL_SNAPSHOT_WAIT,
                    |attempted| !*attempted,
                );
            }
        }
        Self {
            shared,
            stopping,
            worker,
        }
    }
}

impl KeyboardLayoutProvider for HyprlandKeyboardLayoutProvider {
    fn snapshot(&mut self) -> Option<KeyboardLayoutSnapshot> {
        self.shared
            .value
            .lock()
            .ok()
            .and_then(|value| value.clone())
    }
}

impl Drop for HyprlandKeyboardLayoutProvider {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[derive(Debug, serde::Deserialize)]
struct HyprDevices {
    keyboards: Vec<HyprKeyboard>,
}

#[derive(Debug, serde::Deserialize)]
struct HyprKeyboard {
    rules: String,
    model: String,
    layout: String,
    variant: String,
    options: String,
    active_keymap: String,
}

fn hyprland_layout_worker(shared: Arc<SharedSnapshot>, stopping: Arc<AtomicBool>) {
    while !stopping.load(Ordering::Acquire) {
        match hyprland_snapshot() {
            Ok(snapshot) => publish_snapshot(&shared, Some(snapshot)),
            Err(error) => {
                tracing::debug!(%error, "Hyprland keyboard layout is unavailable or inconsistent");
                publish_snapshot(&shared, None);
            }
        }
        sleep_or_stop(&stopping, HYPRLAND_POLL_INTERVAL);
    }
}

fn hyprland_snapshot() -> anyhow::Result<KeyboardLayoutSnapshot> {
    let runtime = env::var_os("XDG_RUNTIME_DIR")
        .ok_or_else(|| anyhow::anyhow!("XDG_RUNTIME_DIR is unavailable"))?;
    let signature = env::var("HYPRLAND_INSTANCE_SIGNATURE")?;
    if signature.is_empty() || signature.contains('/') || signature.contains("..") {
        anyhow::bail!("invalid Hyprland instance signature");
    }
    let socket_path = PathBuf::from(runtime)
        .join("hypr")
        .join(signature)
        .join(".socket.sock");
    let mut socket = UnixStream::connect(socket_path)?;
    socket.set_read_timeout(Some(HYPRLAND_SOCKET_TIMEOUT))?;
    socket.set_write_timeout(Some(HYPRLAND_SOCKET_TIMEOUT))?;
    socket.write_all(b"j/devices")?;
    let deadline = Instant::now() + HYPRLAND_SOCKET_TIMEOUT;
    let mut response = Vec::new();
    let mut chunk = [0_u8; 8192];
    loop {
        if Instant::now() >= deadline {
            anyhow::bail!("timed out reading Hyprland device response");
        }
        match socket.read(&mut chunk) {
            Ok(0) => break,
            Ok(count) => {
                if response.len().saturating_add(count) > HYPRLAND_RESPONSE_LIMIT {
                    anyhow::bail!("Hyprland device response exceeds size limit");
                }
                response.extend_from_slice(&chunk[..count]);
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) && Instant::now() >= deadline =>
            {
                anyhow::bail!("timed out reading Hyprland device response");
            }
            Err(error) => return Err(error.into()),
        }
    }
    let devices: HyprDevices = serde_json::from_slice(&response)?;
    snapshot_for_hyprland_devices(devices)
}

fn snapshot_for_hyprland_devices(devices: HyprDevices) -> anyhow::Result<KeyboardLayoutSnapshot> {
    let first = devices
        .keyboards
        .first()
        .ok_or_else(|| anyhow::anyhow!("Hyprland reported no keyboards"))?;
    if first.layout.is_empty() {
        anyhow::bail!("Hyprland did not provide an XKB layout");
    }
    if devices.keyboards.iter().any(|keyboard| {
        keyboard.rules != first.rules
            || keyboard.model != first.model
            || keyboard.layout != first.layout
            || keyboard.variant != first.variant
            || keyboard.options != first.options
            || keyboard.active_keymap != first.active_keymap
    }) {
        anyhow::bail!(
            "keyboards have different effective layouts; shared evdev state is ambiguous"
        );
    }
    let context =
        Context::new(0).map_err(|error| anyhow::anyhow!("cannot initialize XKB: {error}"))?;
    let keymap = Keymap::new_from_names(
        context,
        Some(RuleNames {
            rules: Some(first.rules.clone()),
            model: Some(first.model.clone()),
            layout: Some(first.layout.clone()),
            variant: Some(first.variant.clone()),
            options: Some(first.options.clone()),
        }),
        0,
    )
    .map_err(|error| anyhow::anyhow!("cannot compile compositor XKB keymap: {error:?}"))?;
    let groups = (0..keymap.num_layouts())
        .filter(|group| keymap.layout_get_name(*group) == Some(first.active_keymap.as_str()))
        .collect::<Vec<_>>();
    let [group] = groups.as_slice() else {
        anyhow::bail!(
            "active Hyprland layout name does not uniquely match the reported XKB keymap"
        );
    };
    let mut hasher = DefaultHasher::new();
    first.rules.hash(&mut hasher);
    first.model.hash(&mut hasher);
    first.layout.hash(&mut hasher);
    first.variant.hash(&mut hasher);
    first.options.hash(&mut hasher);
    first.active_keymap.hash(&mut hasher);
    Ok(KeyboardLayoutSnapshot {
        generation: hasher.finish(),
        group: *group as u32,
        rules: Some(first.rules.clone()),
        model: Some(first.model.clone()),
        layout: Some(first.layout.clone()),
        variant: Some(first.variant.clone()),
        options: Some(first.options.clone()),
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct KwinLayoutConfig {
    model: String,
    layouts: Vec<String>,
    variants: Vec<String>,
    options: Option<String>,
}

struct SharedSnapshot {
    value: Mutex<Option<KeyboardLayoutSnapshot>>,
    first_attempt: Mutex<bool>,
    ready: Condvar,
}

/// Tracks KDE's active keyboard group without blocking the capture reactor.
pub(crate) struct KwinKeyboardLayoutProvider {
    shared: Arc<SharedSnapshot>,
    stopping: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl KwinKeyboardLayoutProvider {
    pub(crate) fn new() -> Self {
        let shared = Arc::new(SharedSnapshot {
            value: Mutex::new(None),
            first_attempt: Mutex::new(false),
            ready: Condvar::new(),
        });
        let stopping = Arc::new(AtomicBool::new(false));
        let worker_shared = Arc::clone(&shared);
        let worker_stopping = Arc::clone(&stopping);
        let worker = thread::Builder::new()
            .name("wayexpand-kwin-layout".into())
            .spawn(move || layout_worker(worker_shared, worker_stopping))
            .ok();

        // Resolve the normal startup case before evdev starts receiving keys.
        // If KWin is not available, the bounded wait ends and matching remains
        // fail-closed until a later provider refresh succeeds.
        if let Ok(attempted) = shared.first_attempt.lock() {
            if !*attempted {
                let _ = shared.ready.wait_timeout_while(
                    attempted,
                    INITIAL_SNAPSHOT_WAIT,
                    |attempted| !*attempted,
                );
            }
        }
        Self {
            shared,
            stopping,
            worker,
        }
    }
}

impl KeyboardLayoutProvider for KwinKeyboardLayoutProvider {
    fn snapshot(&mut self) -> Option<KeyboardLayoutSnapshot> {
        self.shared
            .value
            .lock()
            .ok()
            .and_then(|snapshot| snapshot.clone())
    }
}

impl Drop for KwinKeyboardLayoutProvider {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn layout_worker(shared: Arc<SharedSnapshot>, stopping: Arc<AtomicBool>) {
    while !stopping.load(Ordering::Acquire) {
        match connect_kwin() {
            Ok(connection) => {
                let proxy = match Proxy::new(
                    &connection,
                    "org.kde.keyboard",
                    "/Layouts",
                    "org.kde.KeyboardLayouts",
                ) {
                    Ok(proxy) => proxy,
                    Err(error) => {
                        tracing::debug!(%error, "KWin keyboard-layout interface unavailable");
                        publish_snapshot(&shared, None);
                        sleep_or_stop(&stopping, UNAVAILABLE_RETRY_INTERVAL);
                        continue;
                    }
                };
                let locale_connection = connect_locale1().ok();
                let locale_proxy = locale_connection.as_ref().and_then(|connection| {
                    Proxy::new(
                        connection,
                        "org.freedesktop.locale1",
                        "/org/freedesktop/locale1",
                        "org.freedesktop.locale1",
                    )
                    .ok()
                });
                let config = match read_layout_config(locale_proxy.as_ref()) {
                    Ok(config) => config,
                    Err(error) => {
                        tracing::debug!(%error, "no trustworthy KDE keyboard layout configuration is available");
                        publish_snapshot(&shared, None);
                        sleep_or_stop(&stopping, UNAVAILABLE_RETRY_INTERVAL);
                        continue;
                    }
                };
                loop {
                    if stopping.load(Ordering::Acquire) {
                        return;
                    }
                    match query_snapshot(&proxy, &config) {
                        Ok(snapshot) => {
                            publish_snapshot(&shared, Some(snapshot));
                            sleep_or_stop(&stopping, LAYOUT_POLL_INTERVAL);
                        }
                        Err(error) => {
                            tracing::debug!(%error, "KWin keyboard layout is unavailable or inconsistent");
                            publish_snapshot(&shared, None);
                            sleep_or_stop(&stopping, UNAVAILABLE_RETRY_INTERVAL);
                            break;
                        }
                    }
                }
            }
            Err(error) => {
                tracing::debug!(%error, "could not connect to KWin keyboard-layout service");
                publish_snapshot(&shared, None);
                sleep_or_stop(&stopping, UNAVAILABLE_RETRY_INTERVAL);
            }
        }
    }
}

fn sleep_or_stop(stopping: &AtomicBool, duration: Duration) {
    let step = Duration::from_millis(10);
    let mut remaining = duration;
    while !remaining.is_zero() && !stopping.load(Ordering::Acquire) {
        let sleep = remaining.min(step);
        thread::sleep(sleep);
        remaining = remaining.saturating_sub(sleep);
    }
}

fn publish_snapshot(shared: &SharedSnapshot, value: Option<KeyboardLayoutSnapshot>) {
    if let Ok(mut current) = shared.value.lock() {
        *current = value;
    }
    if let Ok(mut attempted) = shared.first_attempt.lock() {
        *attempted = true;
        shared.ready.notify_all();
    }
}

fn connect_kwin() -> zbus::Result<Connection> {
    Builder::session()?.method_timeout(DBUS_TIMEOUT).build()
}

fn connect_locale1() -> zbus::Result<Connection> {
    Builder::system()?.method_timeout(DBUS_TIMEOUT).build()
}

fn read_layout_config(locale_proxy: Option<&Proxy<'_>>) -> anyhow::Result<KwinLayoutConfig> {
    let config_path = kde_keyboard_config_path()?;
    match fs::read_to_string(&config_path) {
        Ok(contents) => parse_kwin_layout_config(&contents)
            .ok_or_else(|| anyhow::anyhow!("KWin layout config is incomplete or unsupported")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => locale_proxy
            .ok_or_else(|| {
                anyhow::anyhow!("KWin config is absent and systemd-localed is unavailable")
            })
            .and_then(locale1_layout_config),
        Err(error) => Err(error.into()),
    }
}

fn locale1_layout_config(proxy: &Proxy<'_>) -> anyhow::Result<KwinLayoutConfig> {
    let model: String = proxy.get_property("X11Model")?;
    let layout_list: String = proxy.get_property("X11Layout")?;
    let variant_list: String = proxy.get_property("X11Variant")?;
    let options_list: String = proxy.get_property("X11Options")?;
    let layouts = layout_list
        .split(',')
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let variants = if variant_list.is_empty() {
        vec![String::new(); layouts.len()]
    } else {
        variant_list.split(',').map(str::to_owned).collect()
    };
    let model = if model.is_empty() { "pc105" } else { &model }.to_owned();
    if layouts.is_empty()
        || variants.len() != layouts.len()
        || !valid_xkb_name(&model)
        || layouts.iter().any(|layout| !valid_xkb_name(layout))
        || variants.iter().any(|variant| !valid_variant(variant))
        || !valid_options(&options_list)
    {
        anyhow::bail!("systemd-localed keyboard properties are incomplete or unsupported");
    }
    Ok(KwinLayoutConfig {
        model,
        layouts,
        variants,
        options: (!options_list.is_empty()).then_some(options_list),
    })
}

fn query_snapshot(
    proxy: &Proxy<'_>,
    config: &KwinLayoutConfig,
) -> anyhow::Result<KeyboardLayoutSnapshot> {
    let current_group: u32 = proxy.call("getLayout", &())?;
    let effective_layouts: Vec<(String, String, String)> = proxy.call("getLayoutsList", &())?;
    let effective_layouts = effective_layouts
        .into_iter()
        .map(|(layout, variant, _)| (layout, variant))
        .collect::<Vec<_>>();
    snapshot_for_config(
        config.clone(),
        effective_layouts,
        current_group,
        rules_from_environment(),
    )
}

fn rules_from_environment() -> Option<String> {
    env::var("XKB_DEFAULT_RULES").ok()
}

fn snapshot_for_config(
    config: KwinLayoutConfig,
    effective_layouts: Vec<(String, String)>,
    current_group: u32,
    rules: Option<String>,
) -> anyhow::Result<KeyboardLayoutSnapshot> {
    let configured_layouts = config
        .layouts
        .iter()
        .cloned()
        .zip(config.variants.iter().cloned())
        .collect::<Vec<_>>();
    if effective_layouts != configured_layouts || current_group as usize >= config.layouts.len() {
        anyhow::bail!("KWin's active layout list differs from the trusted keyboard configuration");
    }
    let generation =
        layout_generation(&config, &effective_layouts, current_group, rules.as_deref());
    Ok(KeyboardLayoutSnapshot {
        generation,
        group: current_group,
        rules,
        model: Some(config.model),
        layout: Some(config.layouts.join(",")),
        variant: Some(config.variants.join(",")),
        options: config.options,
    })
}

fn layout_generation(
    config: &KwinLayoutConfig,
    layouts: &[(String, String)],
    active_group: u32,
    rules: Option<&str>,
) -> u64 {
    let mut hasher = DefaultHasher::new();
    config.hash(&mut hasher);
    layouts.hash(&mut hasher);
    active_group.hash(&mut hasher);
    rules.hash(&mut hasher);
    hasher.finish()
}

fn kde_keyboard_config_path() -> anyhow::Result<PathBuf> {
    if let Some(config_home) = env::var_os("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(config_home).join("kxkbrc"));
    }
    let home = env::var_os("HOME").ok_or_else(|| anyhow::anyhow!("HOME is unavailable"))?;
    Ok(PathBuf::from(home).join(".config/kxkbrc"))
}

fn parse_kwin_layout_config(contents: &str) -> Option<KwinLayoutConfig> {
    let mut in_layout = false;
    let mut model = None;
    let mut layout_list = None;
    let mut variant_list = None;
    let mut options = None;
    let mut reset_old_options = false;
    for line in contents.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_layout = &line[1..line.len() - 1] == "Layout";
            continue;
        }
        if !in_layout || line.is_empty() || line.starts_with('#') || line.starts_with('!') {
            continue;
        }
        let (key, value) = line.split_once('=')?;
        let value = decode_kconfig_value(value.trim())?;
        match key.trim() {
            "Model" => model = Some(value),
            "LayoutList" => layout_list = Some(value),
            "VariantList" => variant_list = Some(value),
            "Options" => options = Some(value),
            "ResetOldOptions" => reset_old_options = value.eq_ignore_ascii_case("true"),
            _ => {}
        }
    }

    let model = model
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "pc104".into());
    if !valid_xkb_name(&model) {
        return None;
    }
    let layouts = layout_list?
        .split(',')
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if layouts.is_empty() || layouts.iter().any(|layout| !valid_xkb_name(layout)) {
        return None;
    }
    let variants = match variant_list.unwrap_or_default() {
        value if value.is_empty() => vec![String::new(); layouts.len()],
        value => value.split(',').map(str::to_owned).collect(),
    };
    if variants.len() != layouts.len() || variants.iter().any(|variant| !valid_variant(variant)) {
        return None;
    }
    let options = reset_old_options.then(|| options.unwrap_or_default());
    if options
        .as_deref()
        .is_some_and(|value| !valid_options(value))
    {
        return None;
    }
    Some(KwinLayoutConfig {
        model,
        layouts,
        variants,
        options,
    })
}

fn decode_kconfig_value(value: &str) -> Option<String> {
    let mut decoded = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(character) = chars.next() {
        if character != '\\' {
            decoded.push(character);
            continue;
        }
        decoded.push(match chars.next()? {
            's' => ' ',
            't' => '\t',
            'n' => '\n',
            'r' => '\r',
            '\\' => '\\',
            _ => return None,
        });
    }
    Some(decoded)
}

fn valid_xkb_name(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-+".contains(&byte))
}

fn valid_variant(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b"_-+".contains(&byte))
}

fn valid_options(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b"_+:-,".contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hypr_keyboard(layout: &str, active_keymap: &str) -> HyprKeyboard {
        HyprKeyboard {
            rules: "evdev".into(),
            model: "pc105".into(),
            layout: layout.into(),
            variant: "".into(),
            options: "".into(),
            active_keymap: active_keymap.into(),
        }
    }

    #[test]
    fn hyprland_snapshot_matches_active_group_in_xkb_keymap() {
        let first = snapshot_for_hyprland_devices(HyprDevices {
            keyboards: vec![hypr_keyboard("us,de", "German")],
        })
        .unwrap();
        let second = snapshot_for_hyprland_devices(HyprDevices {
            keyboards: vec![hypr_keyboard("us,de", "English (US)")],
        })
        .unwrap();
        assert_eq!(first.group, 1);
        assert_eq!(second.group, 0);
        assert_ne!(first.generation, second.generation);
        assert_eq!(first.layout.as_deref(), Some("us,de"));
    }

    #[test]
    fn parses_hyprland_device_socket_shape() {
        let devices: HyprDevices = serde_json::from_str(
            r#"{"keyboards":[{"name":"kbd","rules":"evdev","model":"pc105","layout":"us","variant":"","options":"","active_keymap":"English (US)","main":true}]}"#,
        )
        .unwrap();
        let snapshot = snapshot_for_hyprland_devices(devices).unwrap();
        assert_eq!(snapshot.group, 0);
    }

    #[test]
    fn hyprland_snapshot_rejects_unknown_or_ambiguous_layouts() {
        assert!(snapshot_for_hyprland_devices(HyprDevices {
            keyboards: vec![hypr_keyboard("us,de", "Unknown layout")],
        })
        .is_err());
        assert!(snapshot_for_hyprland_devices(HyprDevices { keyboards: vec![] }).is_err());
    }

    #[test]
    fn hyprland_snapshot_rejects_different_keyboard_layouts() {
        assert!(snapshot_for_hyprland_devices(HyprDevices {
            keyboards: vec![
                hypr_keyboard("us", "English (US)"),
                hypr_keyboard("de", "German")
            ],
        })
        .is_err());
    }

    #[test]
    fn parses_kde_layouts_and_parallel_variants() {
        let config = parse_kwin_layout_config(
            "[Other]\nModel=ignored\n[Layout]\nLayoutList=us,de\nVariantList=,nodeadkeys\nModel=pc105\nOptions=grp:alt_shift_toggle\nResetOldOptions=true\n",
        )
        .unwrap();
        assert_eq!(config.model, "pc105");
        assert_eq!(config.layouts, ["us", "de"]);
        assert_eq!(config.variants, ["", "nodeadkeys"]);
        assert_eq!(config.options.as_deref(), Some("grp:alt_shift_toggle"));
    }

    #[test]
    fn rejects_mismatched_layout_and_variant_lists() {
        assert!(
            parse_kwin_layout_config("[Layout]\nLayoutList=us,de\nVariantList=nodeadkeys\n")
                .is_none()
        );
    }

    #[test]
    fn ignores_saved_options_when_kwin_does_not_reset_them() {
        let config = parse_kwin_layout_config(
            "[Layout]\nLayoutList=us\nOptions=grp:alt_shift_toggle\nResetOldOptions=false\n",
        )
        .unwrap();
        assert_eq!(config.options, None);
    }

    #[test]
    fn snapshot_requires_a_matching_compositor_layout_list_and_valid_group() {
        let config = parse_kwin_layout_config(
            "[Layout]\nModel=pc105\nLayoutList=us,de\nVariantList=,nodeadkeys\n",
        )
        .unwrap();
        assert!(
            snapshot_for_config(config.clone(), vec![("us".into(), "".into())], 0, None).is_err()
        );
        assert!(snapshot_for_config(
            config.clone(),
            vec![("us".into(), "".into()), ("de".into(), "".into())],
            2,
            None
        )
        .is_err());
        assert!(snapshot_for_config(
            config,
            vec![("us".into(), "".into()), ("de".into(), "".into())],
            0,
            None
        )
        .is_err());
    }

    #[test]
    fn active_group_changes_snapshot_generation() {
        let config = parse_kwin_layout_config(
            "[Layout]\nModel=pc105\nLayoutList=us,de\nVariantList=,nodeadkeys\n",
        )
        .unwrap();
        let layouts = vec![("us".into(), "".into()), ("de".into(), "nodeadkeys".into())];
        let config = KwinLayoutConfig {
            variants: vec!["".into(), "nodeadkeys".into()],
            ..config
        };
        let first = snapshot_for_config(config.clone(), layouts.clone(), 0, None).unwrap();
        let second = snapshot_for_config(config, layouts, 1, None).unwrap();
        assert_ne!(first.generation, second.generation);
        assert_eq!(second.group, 1);
    }

    #[test]
    #[ignore = "requires a live KDE session and systemd-localed"]
    fn live_kwin_layout_agrees_with_trusted_keyboard_configuration() {
        let connection = connect_kwin().unwrap();
        let proxy = Proxy::new(
            &connection,
            "org.kde.keyboard",
            "/Layouts",
            "org.kde.KeyboardLayouts",
        )
        .unwrap();
        let locale_connection = connect_locale1().ok();
        let locale_proxy = locale_connection.as_ref().and_then(|connection| {
            Proxy::new(
                connection,
                "org.freedesktop.locale1",
                "/org/freedesktop/locale1",
                "org.freedesktop.locale1",
            )
            .ok()
        });
        let config = read_layout_config(locale_proxy.as_ref()).unwrap();
        let snapshot = query_snapshot(&proxy, &config).unwrap();
        assert_eq!(snapshot.group, 0);
        assert_eq!(snapshot.layout.as_deref(), Some("us"));
    }
}
