//! Live keyboard-layout snapshots from KWin's documented session D-Bus API.
//!
//! The D-Bus calls run on a bounded polling thread. The evdev capture loop
//! reads only a fresh in-memory snapshot, so neither input matching nor a
//! layout query waits on the compositor.

use std::{
    env, fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use tracing::{debug, warn};
use wayexpand_backend_evdev::{KeyboardLayoutProvider, KeyboardLayoutSnapshot};
use zbus::blocking::{connection::Builder as BlockingConnectionBuilder, Proxy};

const POLL_INTERVAL: Duration = Duration::from_millis(200);
const METHOD_TIMEOUT: Duration = Duration::from_millis(500);
const MAX_SNAPSHOT_AGE: Duration = Duration::from_secs(2);
const MAX_KXKBRC_BYTES: u64 = 64 * 1024;

#[derive(Default)]
struct MonitorState {
    snapshot: Option<KeyboardLayoutSnapshot>,
    refreshed_at: Option<Instant>,
}

type LayoutSignature = (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// Snapshot-backed provider used by raw evdev capture on KDE Plasma.
pub(crate) struct KwinKeyboardLayoutProvider {
    state: Arc<Mutex<MonitorState>>,
    stop: Arc<AtomicBool>,
}

impl KwinKeyboardLayoutProvider {
    #[cfg(test)]
    pub(crate) fn wait_for_snapshot(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if self.fresh_snapshot().is_some() {
                return true;
            }
            thread::sleep(Duration::from_millis(20));
        }
        self.fresh_snapshot().is_some()
    }

    pub(crate) fn start() -> Self {
        let state = Arc::new(Mutex::new(MonitorState::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let worker_state = Arc::clone(&state);
        let worker_stop = Arc::clone(&stop);
        if let Err(error) = thread::Builder::new()
            .name("wayexpand-keyboard-layout".into())
            .spawn(move || monitor_layout(worker_state, worker_stop))
        {
            warn!(%error, "could not start KWin keyboard-layout monitor; evdev matching will remain disabled");
        }
        Self { state, stop }
    }
}

impl KeyboardLayoutProvider for KwinKeyboardLayoutProvider {
    fn snapshot(&mut self) -> Option<KeyboardLayoutSnapshot> {
        self.fresh_snapshot()
    }

    fn is_available(&self) -> bool {
        self.fresh_snapshot().is_some()
    }
}

impl KwinKeyboardLayoutProvider {
    fn fresh_snapshot(&self) -> Option<KeyboardLayoutSnapshot> {
        let state = self.state.lock().ok()?;
        if state.refreshed_at?.elapsed() > MAX_SNAPSHOT_AGE {
            return None;
        }
        state.snapshot.clone()
    }
}

impl Drop for KwinKeyboardLayoutProvider {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

fn monitor_layout(state: Arc<Mutex<MonitorState>>, stop: Arc<AtomicBool>) {
    let mut previous: Option<LayoutSignature> = None;
    let mut generation = 0u64;
    while !stop.load(Ordering::Acquire) {
        let connection = match BlockingConnectionBuilder::session()
            .and_then(|builder| builder.method_timeout(METHOD_TIMEOUT).build())
        {
            Ok(connection) => connection,
            Err(error) => {
                set_unavailable(&state, &error.to_string());
                wait_before_retry(&stop);
                continue;
            }
        };
        let proxy = match Proxy::new(
            &connection,
            "org.kde.keyboard",
            "/Layouts",
            "org.kde.KeyboardLayouts",
        ) {
            Ok(proxy) => proxy,
            Err(error) => {
                set_unavailable(&state, &error.to_string());
                wait_before_retry(&stop);
                continue;
            }
        };
        while !stop.load(Ordering::Acquire) {
            match query_kwin_layout(&proxy) {
                Ok(mut snapshot) => {
                    let signature = (
                        snapshot.rules.clone(),
                        snapshot.model.clone(),
                        snapshot.layout.clone(),
                        snapshot.variant.clone(),
                        snapshot.options.clone(),
                    );
                    if previous.as_ref() != Some(&signature) {
                        generation = generation.wrapping_add(1).max(1);
                        previous = Some(signature);
                        debug!(generation, layout = ?snapshot.layout, "KWin keyboard layout changed");
                    }
                    snapshot.generation = generation;
                    if let Ok(mut current) = state.lock() {
                        current.snapshot = Some(snapshot);
                        current.refreshed_at = Some(Instant::now());
                    }
                }
                Err(error) => {
                    set_unavailable(&state, &error.to_string());
                    break;
                }
            }
            wait_poll_interval(&stop);
        }
    }
}

fn set_unavailable(state: &Mutex<MonitorState>, error: &str) {
    debug!(%error, "KWin keyboard layout is unavailable; evdev matching is paused");
    if let Ok(mut current) = state.lock() {
        current.snapshot = None;
        current.refreshed_at = Some(Instant::now());
    }
}

fn wait_before_retry(stop: &AtomicBool) {
    let until = Instant::now() + Duration::from_secs(1);
    while !stop.load(Ordering::Acquire) && Instant::now() < until {
        thread::sleep(Duration::from_millis(20));
    }
}

fn wait_poll_interval(stop: &AtomicBool) {
    let until = Instant::now() + POLL_INTERVAL;
    while !stop.load(Ordering::Acquire) && Instant::now() < until {
        thread::sleep(Duration::from_millis(20));
    }
}

fn query_kwin_layout(proxy: &Proxy<'_>) -> anyhow::Result<KeyboardLayoutSnapshot> {
    let current: u32 = proxy.call("getLayout", &())?;
    let layouts: Vec<(String, String, String)> = proxy.call("getLayoutsList", &())?;
    let layout = layouts
        .get(current as usize)
        .map(|entry| entry.0.clone())
        .filter(|value| valid_xkb_token(value))
        .ok_or_else(|| anyhow::anyhow!("KWin returned an unknown active layout index"))?;
    let (model, variant, options) = read_xkb_settings(current as usize);
    Ok(KeyboardLayoutSnapshot {
        generation: 0,
        rules: env::var("XKB_DEFAULT_RULES")
            .ok()
            .filter(|value| valid_xkb_token(value)),
        model,
        layout: Some(layout),
        variant,
        options,
    })
}

#[cfg(test)]
fn query_live_kwin_layout() -> anyhow::Result<KeyboardLayoutSnapshot> {
    let connection = BlockingConnectionBuilder::session()?
        .method_timeout(METHOD_TIMEOUT)
        .build()?;
    let proxy = Proxy::new(
        &connection,
        "org.kde.keyboard",
        "/Layouts",
        "org.kde.KeyboardLayouts",
    )?;
    query_kwin_layout(&proxy)
}

fn read_xkb_settings(active_index: usize) -> (Option<String>, Option<String>, Option<String>) {
    let settings = kxkbrc_section().unwrap_or_default();
    let model = settings
        .get("Model")
        .cloned()
        .or_else(|| Some("pc104".into()))
        .filter(|value| valid_xkb_token(value));

    // KWin applies XKB_DEFAULT_VARIANT together with XKB_DEFAULT_LAYOUT. If
    // that override is present it takes precedence over the KConfig list.
    let variants = if env::var_os("XKB_DEFAULT_LAYOUT").is_some() {
        env::var("XKB_DEFAULT_VARIANT").ok()
    } else {
        settings
            .get("VariantList")
            .and_then(|value| split_kconfig_list(value).get(active_index).cloned())
            .filter(|value| !value.is_empty())
    };
    let variant = variants.filter(|value| valid_xkb_token(value));

    let reset_old_options = settings
        .get("ResetOldOptions")
        .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "True"));
    let options = if reset_old_options {
        settings.get("Options").cloned()
    } else {
        env::var("XKB_DEFAULT_OPTIONS").ok()
    }
    .filter(|value| valid_xkb_token(value));
    (model, variant, options)
}

fn kxkbrc_section() -> Option<std::collections::HashMap<String, String>> {
    let path = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?
        .join("kxkbrc");
    let metadata = fs::metadata(&path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_KXKBRC_BYTES {
        return None;
    }
    let contents = fs::read_to_string(path).ok()?;
    let mut in_layout = false;
    let mut result = std::collections::HashMap::new();
    for raw in contents.lines() {
        let line = raw.trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_layout = line == "[Layout]";
            continue;
        }
        if !in_layout || line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            result.insert(key.trim().to_owned(), value.trim().to_owned());
        }
    }
    Some(result)
}

fn split_kconfig_list(value: &str) -> Vec<String> {
    value.split(',').map(str::to_owned).collect()
}

fn valid_xkb_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-,:+".contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xkb_settings_keep_empty_variant_positions() {
        assert_eq!(split_kconfig_list(",nodeadkeys"), ["", "nodeadkeys"]);
    }

    #[test]
    fn xkb_tokens_reject_control_and_config_syntax() {
        assert!(valid_xkb_token("pc105"));
        assert!(valid_xkb_token("grp:alt_shift_toggle"));
        assert!(!valid_xkb_token("us\n[Group]"));
    }

    #[test]
    fn kwin_active_layout_probe_is_safe_when_the_service_is_absent() {
        if let Ok(snapshot) = query_live_kwin_layout() {
            assert!(snapshot.layout.as_deref().is_some_and(valid_xkb_token));
            assert!(snapshot.model.as_deref().is_some_and(valid_xkb_token));
        }
    }

    #[test]
    fn kde_sessions_publish_a_fresh_layout_snapshot() {
        let is_kde = env::var("XDG_CURRENT_DESKTOP").is_ok_and(|desktop| {
            desktop
                .split(':')
                .any(|name| name.eq_ignore_ascii_case("KDE"))
        });
        if !is_kde {
            return;
        }
        let provider = KwinKeyboardLayoutProvider::start();
        assert!(
            provider.wait_for_snapshot(Duration::from_secs(2)),
            "KDE session did not provide an active keyboard layout"
        );
    }
}
