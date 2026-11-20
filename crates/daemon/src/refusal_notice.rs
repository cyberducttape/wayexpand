//! Desktop notification when the output route refuses a snippet.
//!
//! A snippet the route cannot type here (too long, a character the layout
//! lacks, a cursor move it cannot make) is refused before anything is typed,
//! so the trigger simply stays. Without a notice that looks like WayExpand
//! ignored it. The notification names only the reason, never the trigger or
//! the text, because notifications are often visible while screen sharing
//! and are kept in history. It offers one action, "Copy snippet": only when
//! that is pressed is the text put on the clipboard, for the user to paste.
//! Nothing is pasted automatically.
//!
//! All D-Bus work happens on worker threads; the reactor only hands over the
//! reason and text with a non-blocking send.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use tracing::{debug, warn};
use wayexpand_core::RouteLimit;
use zbus::blocking::{connection::Builder, Connection, Proxy};
use zbus::zvariant::Value;
use zeroize::Zeroizing;

const DESTINATION: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
const INTERFACE: &str = "org.freedesktop.Notifications";
const COPY_ACTION: &str = "copy";
/// How long the refused text stays available to the copy action. It is
/// zeroized afterwards, or as soon as a newer refusal replaces it.
const COPY_WINDOW: Duration = Duration::from_secs(120);
const DBUS_TIMEOUT: Duration = Duration::from_secs(2);

/// Why the route refused a snippet, as the notification explains it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reason {
    TooLong,
    Character,
    CursorMove,
    LayoutUnsettled,
    TooSlow,
}

impl Reason {
    pub(crate) fn from_route_limit(limit: RouteLimit) -> Self {
        match limit {
            RouteLimit::Unicode { .. } => Self::Character,
            RouteLimit::TooLong { .. } => Self::TooLong,
            RouteLimit::CursorMove => Self::CursorMove,
        }
    }

    /// Map a backend refusal category (see `backend_refusal_category`).
    pub(crate) fn from_backend_category(category: &str) -> Option<Self> {
        match category {
            "unicode-keymap" => Some(Self::Character),
            "fallback-too-slow" => Some(Self::TooSlow),
            "layout-mismatch" | "active-layout-unavailable" | "keyboard-state-busy" => {
                Some(Self::LayoutUnsettled)
            }
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Language {
    English,
    German,
    Indonesian,
}

impl Language {
    fn from_env() -> Self {
        let value = ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .find_map(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
            .unwrap_or_default();
        Self::from_locale(&value)
    }

    fn from_locale(value: &str) -> Self {
        if value.starts_with("de") {
            Self::German
        } else if value.starts_with("id") {
            Self::Indonesian
        } else {
            Self::English
        }
    }
}

/// Summary, body, and copy-button label. No snippet content.
fn texts(language: Language, reason: Reason) -> (&'static str, String, &'static str) {
    use Language::{English as En, German as De, Indonesian as Id};
    let summary = match language {
        En => "WayExpand didn't type a snippet",
        De => "WayExpand hat ein Snippet nicht eingegeben",
        Id => "WayExpand tidak mengetik cuplikan",
    };
    let why = match (reason, language) {
        (Reason::TooLong, En) => "It is longer than this typing route types safely.",
        (Reason::TooLong, De) => "Es ist länger, als dieser Eingabeweg sicher tippen kann.",
        (Reason::TooLong, Id) => {
            "Cuplikan lebih panjang dari yang dapat diketik dengan aman oleh rute ini."
        }
        (Reason::Character, En) => {
            "It contains a character the current keyboard layout cannot type."
        }
        (Reason::Character, De) => {
            "Es enthält ein Zeichen, das die aktuelle Tastaturbelegung nicht tippen kann."
        }
        (Reason::Character, Id) => {
            "Cuplikan berisi karakter yang tidak dapat diketik oleh tata letak papan ketik saat ini."
        }
        (Reason::CursorMove, En) => "It places the cursor, which this typing route cannot do.",
        (Reason::CursorMove, De) => "Es setzt den Cursor, was dieser Eingabeweg nicht kann.",
        (Reason::CursorMove, Id) => {
            "Cuplikan menempatkan kursor, yang tidak dapat dilakukan rute ini."
        }
        (Reason::LayoutUnsettled, En) => {
            "The keyboard layout was changing or could not be confirmed. Try again."
        }
        (Reason::LayoutUnsettled, De) => {
            "Die Tastaturbelegung änderte sich gerade oder ließ sich nicht bestätigen. Versuchen Sie es erneut."
        }
        (Reason::LayoutUnsettled, Id) => {
            "Tata letak papan ketik sedang berubah atau tidak dapat dipastikan. Coba lagi."
        }
        (Reason::TooSlow, En) => "Typing it with this keyboard layout would be too slow.",
        (Reason::TooSlow, De) => "Mit dieser Tastaturbelegung wäre das Tippen zu langsam.",
        (Reason::TooSlow, Id) => "Mengetiknya dengan tata letak papan ketik ini akan terlalu lambat.",
    };
    let unchanged = match language {
        En => "Nothing was changed; your trigger is still there.",
        De => "Es wurde nichts geändert; Ihr Kürzel steht noch da.",
        Id => "Tidak ada yang diubah; pemicu Anda masih ada.",
    };
    let copy = match language {
        En => "Copy snippet",
        De => "Snippet kopieren",
        Id => "Salin cuplikan",
    };
    (summary, format!("{why} {unchanged}"), copy)
}

struct Notice {
    reason: Reason,
    text: Zeroizing<String>,
}

/// The text the copy action may still put on the clipboard.
struct Copyable {
    id: u32,
    text: Zeroizing<String>,
    until: Instant,
}

static COPYABLE: Mutex<Option<Copyable>> = Mutex::new(None);

fn copyable() -> std::sync::MutexGuard<'static, Option<Copyable>> {
    COPYABLE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn sender() -> Option<&'static SyncSender<Notice>> {
    static SENDER: OnceLock<Option<SyncSender<Notice>>> = OnceLock::new();
    SENDER
        .get_or_init(|| {
            let (sender, receiver) = mpsc::sync_channel(1);
            std::thread::Builder::new()
                .name("wayexpand-notify".into())
                .spawn(move || notifier(receiver))
                .map_err(|error| warn!(%error, "could not start the notification worker"))
                .ok()
                .map(|_| sender)
        })
        .as_ref()
}

/// Announce a refused snippet. Never blocks: when a notice is still being
/// sent, this one is dropped.
pub(crate) fn announce(reason: Reason, text: &str) {
    // Unit tests must not pop up notifications on the developer's desktop.
    if cfg!(test) {
        return;
    }
    if let Some(sender) = sender() {
        let _ = sender.try_send(Notice {
            reason,
            text: Zeroizing::new(text.to_owned()),
        });
    }
}

fn session() -> zbus::Result<Connection> {
    Builder::session()?.method_timeout(DBUS_TIMEOUT).build()
}

fn notifier(receiver: Receiver<Notice>) {
    let language = Language::from_env();
    let mut connection: Option<Connection> = None;
    let mut last_id = 0u32;
    loop {
        let notice = match receiver.recv_timeout(COPY_WINDOW) {
            Ok(notice) => notice,
            Err(RecvTimeoutError::Timeout) => {
                expire_copyable(Instant::now());
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => return,
        };
        expire_copyable(Instant::now());
        if connection.is_none() {
            match session() {
                Ok(opened) => {
                    start_listener(opened.clone());
                    connection = Some(opened);
                }
                Err(error) => {
                    debug!(%error, "no session bus for refusal notifications");
                    continue;
                }
            }
        }
        let Some(bus) = connection.as_ref() else {
            continue;
        };
        let sent = show(bus, last_id, language, notice.reason);
        match sent {
            Ok(id) => {
                last_id = id;
                *copyable() = Some(Copyable {
                    id,
                    text: notice.text,
                    until: Instant::now() + COPY_WINDOW,
                });
            }
            Err(error) => {
                debug!(%error, "refusal notification was not shown");
                // Reconnect on the next notice; the bus may have restarted.
                connection = None;
            }
        }
    }
}

/// Show (or replace `replaces`) the notice for `reason`; returns its ID.
fn show(bus: &Connection, replaces: u32, language: Language, reason: Reason) -> zbus::Result<u32> {
    let (summary, body, copy_label) = texts(language, reason);
    let mut hints: HashMap<&str, Value> = HashMap::new();
    hints.insert("desktop-entry", Value::from("wayexpand"));
    hints.insert("urgency", Value::U8(1));
    Proxy::new(bus, DESTINATION, PATH, INTERFACE)?.call(
        "Notify",
        &(
            "WayExpand",
            replaces,
            "wayexpand",
            summary,
            body.as_str(),
            vec![COPY_ACTION, copy_label],
            hints,
            -1i32,
        ),
    )
}

fn expire_copyable(now: Instant) {
    let mut slot = copyable();
    if slot.as_ref().is_some_and(|entry| now >= entry.until) {
        *slot = None;
    }
}

/// Take the text for a pressed copy action on notification `id`, if it is
/// the current notice and has not expired.
fn take_for_action(id: u32, now: Instant) -> Option<Zeroizing<String>> {
    let mut slot = copyable();
    if slot
        .as_ref()
        .is_some_and(|entry| entry.id == id && now < entry.until)
    {
        slot.take().map(|entry| entry.text)
    } else {
        None
    }
}

fn start_listener(connection: Connection) {
    let spawned = std::thread::Builder::new()
        .name("wayexpand-notify-actions".into())
        .spawn(move || {
            let signals = Proxy::new(&connection, DESTINATION, PATH, INTERFACE)
                .and_then(|proxy| proxy.receive_signal("ActionInvoked"));
            let signals = match signals {
                Ok(signals) => signals,
                Err(error) => {
                    debug!(%error, "cannot watch notification actions");
                    return;
                }
            };
            for message in signals {
                let Ok((id, action)) = message.body().deserialize::<(u32, String)>() else {
                    continue;
                };
                if action != COPY_ACTION {
                    continue;
                }
                if let Some(text) = take_for_action(id, Instant::now()) {
                    crate::clipboard::copy_text(text);
                }
            }
        });
    if let Err(error) = spawned {
        warn!(%error, "could not start the notification action listener");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_reason_has_text_in_every_language_and_none_contains_content() {
        for language in [Language::English, Language::German, Language::Indonesian] {
            for reason in [
                Reason::TooLong,
                Reason::Character,
                Reason::CursorMove,
                Reason::LayoutUnsettled,
                Reason::TooSlow,
            ] {
                let (summary, body, copy) = texts(language, reason);
                assert!(!summary.is_empty() && !body.is_empty() && !copy.is_empty());
                assert!(!body.contains('{'), "no placeholders: {body}");
            }
        }
    }

    #[test]
    fn locale_selects_language() {
        assert_eq!(Language::from_locale("de_DE.UTF-8"), Language::German);
        assert_eq!(Language::from_locale("id_ID.UTF-8"), Language::Indonesian);
        assert_eq!(Language::from_locale("en_GB.UTF-8"), Language::English);
        assert_eq!(Language::from_locale(""), Language::English);
    }

    #[test]
    fn backend_categories_map_to_reasons() {
        assert_eq!(
            Reason::from_backend_category("unicode-keymap"),
            Some(Reason::Character)
        );
        assert_eq!(
            Reason::from_backend_category("layout-mismatch"),
            Some(Reason::LayoutUnsettled)
        );
        assert_eq!(Reason::from_backend_category("something-else"), None);
    }

    /// Shows one notice on the real session bus, replaces it, and closes it.
    /// Run by hand: `cargo test -p wayexpand-daemon live_refusal_notice -- --ignored`.
    #[test]
    #[ignore = "shows a notification on the desktop"]
    fn live_refusal_notice_is_shown_replaced_and_closed() {
        let bus = session().expect("session bus");
        let first = show(&bus, 0, Language::English, Reason::TooLong).expect("Notify");
        assert_ne!(first, 0);
        let replaced = show(&bus, first, Language::German, Reason::Character).expect("Notify");
        assert_eq!(
            replaced, first,
            "a newer refusal replaces the visible notice"
        );
        std::thread::sleep(Duration::from_secs(2));
        Proxy::new(&bus, DESTINATION, PATH, INTERFACE)
            .unwrap()
            .call::<_, _, ()>("CloseNotification", &(first,))
            .expect("CloseNotification");
    }

    #[test]
    fn copy_action_takes_only_the_current_unexpired_text_once() {
        let now = Instant::now();
        *copyable() = Some(Copyable {
            id: 7,
            text: Zeroizing::new("snippet".into()),
            until: now + COPY_WINDOW,
        });
        assert!(take_for_action(6, now).is_none(), "an older notification");
        assert_eq!(
            take_for_action(7, now).as_deref().map(String::as_str),
            Some("snippet")
        );
        assert!(take_for_action(7, now).is_none(), "copied once");

        *copyable() = Some(Copyable {
            id: 8,
            text: Zeroizing::new("snippet".into()),
            until: now,
        });
        assert!(take_for_action(8, now).is_none(), "expired");
        expire_copyable(now);
        assert!(copyable().is_none(), "expired text is dropped");
    }
}
