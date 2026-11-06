#!/usr/bin/python3
"""Drive a real GTK3 entry through its IBus input-method context.

This acceptance test runs inside the isolated GNOME/IBus CI session. GTK's
test event helper sends keys through the widget's ordinary key and IM-context
path; the active GNOME-selected WayExpand IBus engine performs the expansion
and GTK receives the committed replacement.
"""

import sys
import time

import gi
from evdev import UInput, ecodes as E

gi.require_version("Gtk", "3.0")
from gi.repository import GLib, Gtk

TRIGGER = ";e2e"
EXPECTED = "expanded"
TIMEOUT_MS = 5000
KEYS = {
    ";": E.KEY_SEMICOLON,
    "e": E.KEY_E,
    "2": E.KEY_2,
}


def main():
    window = Gtk.Window(title="WayExpand GTK insertion acceptance")
    entry = Gtk.Entry()
    entry.set_input_purpose(Gtk.InputPurpose.FREE_FORM)
    window.add(entry)
    window.show_all()
    entry.grab_focus()

    def send_trigger():
        if not window.is_active() or not entry.is_focus():
            print("fail gtk-ibus-insertion GTK test window did not acquire focus")
            Gtk.main_quit()
            return GLib.SOURCE_REMOVE
        try:
            # GTK cannot synthesize compositor input on Wayland. Use an
            # isolated CI uinput keyboard instead, so Mutter routes the same
            # key events through GNOME's active IBus source and GTK's normal
            # input-method context.
            with UInput(
                events={E.EV_KEY: sorted(set(KEYS.values()))},
                name="WayExpand GNOME acceptance keyboard",
            ) as keyboard:
                time.sleep(0.25)  # Let Mutter discover the new virtual device.
                for character in TRIGGER:
                    key = KEYS[character]
                    keyboard.write(E.EV_KEY, key, 1)
                    keyboard.syn()
                    time.sleep(0.025)
                    keyboard.write(E.EV_KEY, key, 0)
                    keyboard.syn()
                    time.sleep(0.025)
        except (OSError, KeyError) as error:
            print(f"fail gtk-ibus-insertion virtual keyboard unavailable: {error}")
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
