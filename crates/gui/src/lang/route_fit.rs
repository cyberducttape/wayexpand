//! Snippet-list notes about whether the current output route can type a
//! snippet. Hints never repeat snippet text.

use super::{Language, Strings};

crate::texts! {
    route_badge_too_long => "too long", "zu lang", "terlalu panjang";
    route_badge_cannot_type => "won't type", "nicht tippbar", "tak bisa diketik";
    route_badge_cursor => "no cursor", "kein Cursor", "tanpa kursor";
    route_badge_layout => "layout", "Layout", "tata letak";
    route_fit_layout_hint =>
        "Some characters are typed only if your active keyboard layout has them. If it does not, WayExpand refuses this snippet before changing anything; use Quick Picker to paste it instead.",
        "Einige Zeichen werden nur getippt, wenn Ihr aktives Tastaturlayout sie enthält. Andernfalls lehnt WayExpand dieses Snippet ab, bevor etwas geändert wird; fügen Sie es dann mit der Schnellauswahl ein.",
        "Beberapa karakter hanya diketik jika tata letak papan ketik aktif Anda memilikinya. Jika tidak, WayExpand menolak cuplikan ini sebelum mengubah apa pun; gunakan Pemilih Cepat untuk menempelkannya.";
    route_fit_cursor_hint =>
        "This snippet moves the cursor after typing, which your current typing route cannot do. WayExpand refuses it before changing anything.",
        "Dieses Snippet bewegt nach dem Tippen den Cursor, was Ihr aktueller Eingabeweg nicht kann. WayExpand lehnt es ab, bevor etwas geändert wird.",
        "Cuplikan ini memindahkan kursor setelah mengetik, yang tidak dapat dilakukan jalur pengetikan Anda saat ini. WayExpand menolaknya sebelum mengubah apa pun.";
}

impl Strings {
    /// The snippet is longer than the route accepts.
    pub fn route_fit_too_long_hint(&self, chars: usize, max: usize) -> String {
        match self.lang {
            Language::English => format!(
                "This snippet types {chars} characters; your current typing route accepts at most {max}. WayExpand refuses it before changing anything. Shorten it, raise keystroke_max_chars in Settings, or use Quick Picker to paste it."
            ),
            Language::German => format!(
                "Dieses Snippet tippt {chars} Zeichen; Ihr aktueller Eingabeweg akzeptiert höchstens {max}. WayExpand lehnt es ab, bevor etwas geändert wird. Kürzen Sie es, erhöhen Sie keystroke_max_chars in den Einstellungen oder fügen Sie es mit der Schnellauswahl ein."
            ),
            Language::Indonesian => format!(
                "Cuplikan ini mengetik {chars} karakter; jalur pengetikan Anda saat ini menerima paling banyak {max}. WayExpand menolaknya sebelum mengubah apa pun. Persingkat, naikkan keystroke_max_chars di Pengaturan, atau gunakan Pemilih Cepat untuk menempelkannya."
            ),
        }
    }

    /// A character the route cannot type, by position only.
    pub fn route_fit_cannot_type_hint(&self, position: usize) -> String {
        match self.lang {
            Language::English => format!(
                "Your current typing route cannot type character {position} of this snippet. WayExpand refuses it before changing anything; use Quick Picker to paste it, or set up a route with full Unicode support."
            ),
            Language::German => format!(
                "Ihr aktueller Eingabeweg kann Zeichen {position} dieses Snippets nicht tippen. WayExpand lehnt es ab, bevor etwas geändert wird; fügen Sie es mit der Schnellauswahl ein oder richten Sie einen Eingabeweg mit voller Unicode-Unterstützung ein."
            ),
            Language::Indonesian => format!(
                "Jalur pengetikan Anda saat ini tidak dapat mengetik karakter ke-{position} cuplikan ini. WayExpand menolaknya sebelum mengubah apa pun; gunakan Pemilih Cepat untuk menempelkannya, atau siapkan jalur dengan dukungan Unicode penuh."
            ),
        }
    }
}
