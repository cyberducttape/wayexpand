//! Compositor-backed XKB layout state for raw evdev capture.
//!
//! KDE exposes the active XKB group over its session D-Bus interface. The
//! provider cross-checks that group and layout list against KWin's xkb config
//! and publishes snapshots from a worker so the input reactor never performs
//! D-Bus or filesystem I/O.

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

use wayexpand_backend_evdev::{KeyboardLayoutProvider, KeyboardLayoutSnapshot};
use zbus::blocking::{connection::Builder, Connection, Proxy};

const DBUS_TIMEOUT: Duration = Duration::from_millis(250);
const LAYOUT_POLL_INTERVAL: Duration = Duration::from_millis(20);
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
                loop {
                    if stopping.load(Ordering::Acquire) {
                        return;
                    }
                    match query_snapshot(&proxy) {
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

fn query_snapshot(proxy: &Proxy<'_>) -> anyhow::Result<KeyboardLayoutSnapshot> {
    let config_path = kde_keyboard_config_path()?;
    let config_contents = fs::read_to_string(&config_path)?;
    let config = parse_kwin_layout_config(&config_contents)
        .ok_or_else(|| anyhow::anyhow!("KWin layout config is incomplete or unsupported"))?;
    let current_group: u32 = proxy.call("getLayout", &())?;
    let effective_layouts: Vec<(String, String, String)> = proxy.call("getLayoutsList", &())?;
    let effective_layouts = effective_layouts
        .into_iter()
        .map(|(layout, _, _)| layout)
        .collect::<Vec<_>>();
    snapshot_for_config(
        config,
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
    effective_layouts: Vec<String>,
    current_group: u32,
    rules: Option<String>,
) -> anyhow::Result<KeyboardLayoutSnapshot> {
    if effective_layouts != config.layouts || current_group as usize >= config.layouts.len() {
        anyhow::bail!("KWin's active layout list differs from kxkbrc");
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
    layouts: &[String],
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
        assert!(snapshot_for_config(config.clone(), vec!["us".into()], 0, None).is_err());
        assert!(snapshot_for_config(config, vec!["us".into(), "de".into()], 2, None).is_err());
    }

    #[test]
    fn active_group_changes_snapshot_generation() {
        let config = parse_kwin_layout_config(
            "[Layout]\nModel=pc105\nLayoutList=us,de\nVariantList=,nodeadkeys\n",
        )
        .unwrap();
        let layouts = vec!["us".into(), "de".into()];
        let first = snapshot_for_config(config.clone(), layouts.clone(), 0, None).unwrap();
        let second = snapshot_for_config(config, layouts, 1, None).unwrap();
        assert_ne!(first.generation, second.generation);
        assert_eq!(second.group, 1);
    }
}
