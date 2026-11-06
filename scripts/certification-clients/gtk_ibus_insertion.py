#!/usr/bin/python3
"""Drive a real GTK3 entry through its IBus input-method context.

This acceptance test runs inside the isolated GNOME/IBus CI session. GTK's
test event helper sends keys through the widget's ordinary key and IM-context
path; the active GNOME-selected WayExpand IBus engine performs the expansion
and GTK receives the committed replacement.
"""

import sys

import gi

gi.require_version("Gdk", "3.0")
gi.require_version("Gtk", "3.0")
from gi.repository import Gdk, GLib, Gtk

TRIGGER = ";e2e"
EXPECTED = "expanded"
TIMEOUT_MS = 5000


def main():
    window = Gtk.Window(title="WayExpand GTK insertion acceptance")
    entry = Gtk.Entry()
    entry.set_input_purpose(Gtk.InputPurpose.FREE_FORM)
    window.add(entry)
    window.show_all()
    entry.grab_focus()

    def send_trigger():
        for character in TRIGGER:
            keyval = Gdk.unicode_to_keyval(ord(character))
            if not Gtk.test_widget_send_key(entry, keyval, Gdk.ModifierType(0)):
                print("fail gtk-ibus-insertion GTK rejected a synthetic test key")
                Gtk.main_quit()
                return GLib.SOURCE_REMOVE
        GLib.timeout_add(50, check_result)
        return GLib.SOURCE_REMOVE

    def check_result():
        actual = entry.get_text()
        if actual == EXPECTED:
            print("pass gtk-ibus-insertion GTK entry contains the committed replacement")
            Gtk.main_quit()
            return GLib.SOURCE_REMOVE
        if check_result.attempts >= TIMEOUT_MS // 50:
            print(
                "fail gtk-ibus-insertion GTK entry did not receive the expected "
                f"replacement (length={len(actual)})"
            )
            Gtk.main_quit()
            return GLib.SOURCE_REMOVE
        check_result.attempts += 1
        return GLib.SOURCE_CONTINUE

    check_result.attempts = 0
    GLib.timeout_add(250, send_trigger)
    Gtk.main()
    window.destroy()
    return 0 if entry.get_text() == EXPECTED else 1


if __name__ == "__main__":
    sys.exit(main())
