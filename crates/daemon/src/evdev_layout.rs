//! Compositor-backed XKB layout state for raw evdev capture.
//!
//! KDE exposes the active XKB group and layout list over session D-Bus. The
//! provider cross-checks them against KWin's config, or systemd-localed's
//! keyboard properties when KWin has no per-user config. It listens for
//! KWin layout signals and reconciles with a slower fallback poll; snapshots
//! are published from a worker so the input reactor never performs I/O.

use std::{
    collections::hash_map::DefaultHasher,
    env, fs,
    hash::{Hash, Hasher},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use futures_lite::StreamExt;
use wayexpand_backend_evdev::{KeyboardLayoutProvider, KeyboardLayoutSnapshot};
#[cfg(test)]
use zbus::blocking::{connection::Builder, Connection, Proxy};

const DBUS_TIMEOUT: Duration = Duration::from_millis(250);
const LAYOUT_FALLBACK_POLL_INTERVAL: Duration = Duration::from_secs(1);
const LAYOUT_STOP_CHECK_INTERVAL: Duration = Duration::from_millis(100);
const UNAVAILABLE_RETRY_INTERVAL: Duration = Duration::from_millis(500);
const INITIAL_SNAPSHOT_WAIT: Duration = Duration::from_millis(300);

pub(crate) fn provider() -> Box<dyn KeyboardLayoutProvider> {
    Box::new(KwinKeyboardLayoutProvider::new())
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
    zbus::block_on(layout_worker_async(shared, stopping));
}

async fn layout_worker_async(shared: Arc<SharedSnapshot>, stopping: Arc<AtomicBool>) {
    while !stopping.load(Ordering::Acquire) {
        match connect_kwin_async().await {
            Ok(connection) => {
                let proxy = match zbus::Proxy::new(
                    &connection,
                    "org.kde.keyboard",
                    "/Layouts",
                    "org.kde.KeyboardLayouts",
                )
                .await
                {
                    Ok(proxy) => proxy,
                    Err(error) => {
                        tracing::debug!(%error, "KWin keyboard-layout interface unavailable");
                        publish_snapshot(&shared, None);
                        wait_or_stop(&stopping, UNAVAILABLE_RETRY_INTERVAL).await;
                        continue;
                    }
                };
                let config = match read_layout_config_async().await {
                    Ok(config) => config,
                    Err(error) => {
                        tracing::debug!(%error, "no trustworthy KDE keyboard layout configuration is available");
                        publish_snapshot(&shared, None);
                        wait_or_stop(&stopping, UNAVAILABLE_RETRY_INTERVAL).await;
                        continue;
                    }
                };
                let mut layout_changed = match proxy.receive_signal("layoutChanged").await {
                    Ok(stream) => Some(stream),
                    Err(error) => {
                        tracing::debug!(%error, "KWin layoutChanged notification unavailable; using fallback polling");
                        None
                    }
                };
                let mut layout_list_changed = match proxy.receive_signal("layoutListChanged").await
                {
                    Ok(stream) => Some(stream),
                    Err(error) => {
                        tracing::debug!(%error, "KWin layoutListChanged notification unavailable; using fallback polling");
                        None
                    }
                };
                let mut refresh_layout_list = true;
                loop {
                    if stopping.load(Ordering::Acquire) {
                        return;
                    }
                    match query_snapshot_async(&proxy, &config, refresh_layout_list).await {
                        Ok(snapshot) => {
                            publish_snapshot(&shared, Some(snapshot));
                        }
                        Err(error) => {
                            tracing::debug!(%error, "KWin keyboard layout is unavailable or inconsistent");
                            publish_snapshot(&shared, None);
                            break;
                        }
                    }
                    let next_fallback = std::time::Instant::now() + LAYOUT_FALLBACK_POLL_INTERVAL;
                    loop {
                        if stopping.load(Ordering::Acquire) {
                            return;
                        }
                        let left =
                            next_fallback.saturating_duration_since(std::time::Instant::now());
                        let wait = left.min(LAYOUT_STOP_CHECK_INTERVAL);
                        let outcome = match (layout_changed.as_mut(), layout_list_changed.as_mut())
                        {
                            (Some(layout), Some(list)) => {
                                futures_lite::future::race(
                                    async {
                                        layout
                                            .next()
                                            .await
                                            .map(|_| SignalResult::ActiveGroupChanged)
                                            .unwrap_or(SignalResult::LayoutClosed)
                                    },
                                    futures_lite::future::race(
                                        async {
                                            list.next()
                                                .await
                                                .map(|_| SignalResult::LayoutListChanged)
                                                .unwrap_or(SignalResult::ListClosed)
                                        },
                                        async {
                                            async_io::Timer::after(wait).await;
                                            SignalResult::Timeout
                                        },
                                    ),
                                )
                                .await
                            }
                            (Some(layout), None) => {
                                futures_lite::future::race(
                                    async {
                                        layout
                                            .next()
                                            .await
                                            .map(|_| SignalResult::ActiveGroupChanged)
                                            .unwrap_or(SignalResult::LayoutClosed)
                                    },
                                    async {
                                        async_io::Timer::after(wait).await;
                                        SignalResult::Timeout
                                    },
                                )
                                .await
                            }
                            (None, Some(list)) => {
                                futures_lite::future::race(
                                    async {
                                        list.next()
                                            .await
                                            .map(|_| SignalResult::LayoutListChanged)
                                            .unwrap_or(SignalResult::ListClosed)
                                    },
                                    async {
                                        async_io::Timer::after(wait).await;
                                        SignalResult::Timeout
                                    },
                                )
                                .await
                            }
                            (None, None) => {
                                async_io::Timer::after(wait).await;
                                SignalResult::Timeout
                            }
                        };
                        match outcome {
                            SignalResult::ActiveGroupChanged => {
                                refresh_layout_list = false;
                                break;
                            }
                            SignalResult::LayoutListChanged => {
                                refresh_layout_list = true;
                                break;
                            }
                            SignalResult::LayoutClosed => {
                                layout_changed = None;
                                tracing::debug!(
                                    "KWin layoutChanged stream closed; using fallback polling"
                                );
                            }
                            SignalResult::ListClosed => {
                                layout_list_changed = None;
                                tracing::debug!(
                                    "KWin layoutListChanged stream closed; using fallback polling"
                                );
                            }
                            SignalResult::Timeout if std::time::Instant::now() >= next_fallback => {
                                refresh_layout_list = true;
                                break;
                            }
                            SignalResult::Timeout => {}
                        }
                    }
                }
                wait_or_stop(&stopping, UNAVAILABLE_RETRY_INTERVAL).await;
            }
            Err(error) => {
                tracing::debug!(%error, "could not connect to KWin keyboard-layout service");
                publish_snapshot(&shared, None);
                wait_or_stop(&stopping, UNAVAILABLE_RETRY_INTERVAL).await;
            }
        }
    }
}

#[derive(Clone, Copy)]
enum SignalResult {
    ActiveGroupChanged,
    LayoutListChanged,
    LayoutClosed,
    ListClosed,
    Timeout,
}

async fn wait_or_stop(stopping: &AtomicBool, duration: Duration) {
    let deadline = std::time::Instant::now() + duration;
    while !stopping.load(Ordering::Acquire) {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            return;
        }
        async_io::Timer::after(left.min(LAYOUT_STOP_CHECK_INTERVAL)).await;
    }
}

fn publish_snapshot(shared: &SharedSnapshot, value: Option<KeyboardLayoutSnapshot>) {
    if let Ok(mut current) = shared.value.lock() {
        if *current != value {
            *current = value;
        }
    }
    if let Ok(mut attempted) = shared.first_attempt.lock() {
        *attempted = true;
        shared.ready.notify_all();
    }
}

#[cfg(test)]
fn connect_kwin() -> zbus::Result<Connection> {
    Builder::session()?.method_timeout(DBUS_TIMEOUT).build()
}

async fn connect_kwin_async() -> zbus::Result<zbus::Connection> {
    zbus::connection::Builder::session()?
        .method_timeout(DBUS_TIMEOUT)
        .build()
        .await
}

async fn read_layout_config_async() -> anyhow::Result<KwinLayoutConfig> {
    let config_path = kde_keyboard_config_path()?;
    match fs::read_to_string(&config_path) {
        Ok(contents) => parse_kwin_layout_config(&contents)
            .ok_or_else(|| anyhow::anyhow!("KWin layout config is incomplete or unsupported")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let connection = zbus::connection::Builder::system()?
                .method_timeout(DBUS_TIMEOUT)
                .build()
                .await?;
            let proxy = zbus::Proxy::new(
                &connection,
                "org.freedesktop.locale1",
                "/org/freedesktop/locale1",
                "org.freedesktop.locale1",
            )
            .await?;
            let model: String = proxy.get_property("X11Model").await?;
            let layouts: String = proxy.get_property("X11Layout").await?;
            let variants: String = proxy.get_property("X11Variant").await?;
            let options: String = proxy.get_property("X11Options").await?;
            locale1_layout_config_values(&model, &layouts, &variants, &options)
        }
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
fn connect_locale1() -> zbus::Result<Connection> {
    Builder::system()?.method_timeout(DBUS_TIMEOUT).build()
}

#[cfg(test)]
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

#[cfg(test)]
fn locale1_layout_config(proxy: &Proxy<'_>) -> anyhow::Result<KwinLayoutConfig> {
    let model: String = proxy.get_property("X11Model")?;
    let layout_list: String = proxy.get_property("X11Layout")?;
    let variant_list: String = proxy.get_property("X11Variant")?;
    let options_list: String = proxy.get_property("X11Options")?;
    locale1_layout_config_values(&model, &layout_list, &variant_list, &options_list)
}

fn locale1_layout_config_values(
    model: &str,
    layout_list: &str,
    variant_list: &str,
    options_list: &str,
) -> anyhow::Result<KwinLayoutConfig> {
    let layouts = layout_list
        .split(',')
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let variants = if variant_list.is_empty() {
        vec![String::new(); layouts.len()]
    } else {
        variant_list.split(',').map(str::to_owned).collect()
    };
    let model = if model.is_empty() { "pc105" } else { model }.to_owned();
    if layouts.is_empty()
        || variants.len() != layouts.len()
        || !valid_xkb_name(&model)
        || layouts.iter().any(|layout| !valid_xkb_name(layout))
        || variants.iter().any(|variant| !valid_variant(variant))
        || !valid_options(options_list)
    {
        anyhow::bail!("systemd-localed keyboard properties are incomplete or unsupported");
    }
    Ok(KwinLayoutConfig {
        model,
        layouts,
        variants,
        options: (!options_list.is_empty()).then_some(options_list.to_owned()),
    })
}

#[cfg(test)]
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

async fn query_snapshot_async(
    proxy: &zbus::Proxy<'_>,
    config: &KwinLayoutConfig,
    refresh_layout_list: bool,
) -> anyhow::Result<KeyboardLayoutSnapshot> {
    let current_group: u32 = proxy.call("getLayout", &()).await?;
    let effective_layouts: Vec<(String, String)> = if refresh_layout_list {
        let layouts: Vec<(String, String, String)> = proxy.call("getLayoutsList", &()).await?;
        layouts
            .into_iter()
            .map(|(layout, variant, _)| (layout, variant))
            .collect()
    } else {
        config
            .layouts
            .iter()
            .cloned()
            .zip(config.variants.iter().cloned())
            .collect()
    };
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

    #[test]
    #[ignore = "manual five-second idle measurement; observe with dbus-monitor and pidstat"]
    fn live_kwin_layout_provider_idle_measurement() {
        let mut provider = KwinKeyboardLayoutProvider::new();
        assert!(provider.snapshot().is_some());
        thread::sleep(Duration::from_secs(5));
        assert!(provider.snapshot().is_some());
    }
}
