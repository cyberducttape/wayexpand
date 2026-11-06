#!/usr/bin/python3
"""Type a harmless trigger into a focused GTK3 IBus context via CI uinput."""

import sys
import time

import gi
from evdev import UInput, ecodes as E

gi.require_version("Gtk", "3.0")
from gi.repository import GLib, Gtk

APP_ID = "org.wayexpand.GtkIbusInsertion"
TRIGGER = ";e2e"
EXPECTED = "expanded"
TIMEOUT_MS = 5000
KEYS = {
    ";": E.KEY_SEMICOLON,
    "e": E.KEY_E,
    "2": E.KEY_2,
}


def main():
    application = Gtk.Application(application_id=APP_ID)
    state = {
        "window": None,
        "entry": None,
        "focus_attempts": 0,
        "attempts": 0,
        "result": 1,
    }

    def finish(result):
        state["result"] = result
        application.quit()
        return GLib.SOURCE_REMOVE

    def check_result():
        actual = state["entry"].get_text()
        if actual == EXPECTED:
            print("pass gtk-ibus-insertion GTK entry contains the committed replacement", flush=True)
            return finish(0)
        if state["attempts"] >= TIMEOUT_MS // 50:
            print(
                "fail gtk-ibus-insertion GTK entry did not receive the expected "
                f"replacement (length={len(actual)})",
                flush=True,
            )
            return finish(1)
        state["attempts"] += 1
        return GLib.SOURCE_CONTINUE

    def send_trigger():
        entry = state["entry"]
        if not entry.is_focus():
            # A focus request made before a Wayland toplevel becomes active
            # can be ignored. Retry only after GTK has observed activation.
            entry.grab_focus()
            state["focus_attempts"] += 1
            if state["focus_attempts"] >= TIMEOUT_MS // 50:
                print(
                    "fail gtk-ibus-insertion GTK did not give the test entry "
                    "keyboard focus before input",
                    flush=True,
                )
                return finish(1)
            return GLib.SOURCE_CONTINUE

        try:
            # GTK cannot synthesize compositor input on Wayland. Use an
            # isolated CI uinput keyboard instead, so Mutter routes these
            # events through GNOME's active IBus source and GTK's normal
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
            print(f"fail gtk-ibus-insertion virtual keyboard unavailable: {error}", flush=True)
            return finish(1)
        state["attempts"] = 0
        GLib.timeout_add(50, check_result)
        return GLib.SOURCE_REMOVE

    def on_activate(app):
        window = Gtk.ApplicationWindow(
            application=app,
            title="WayExpand GTK insertion acceptance",
        )
        entry = Gtk.Entry()
        entry.set_input_purpose(Gtk.InputPurpose.FREE_FORM)
        window.add(entry)
        window.show_all()
        window.present()
        state["window"] = window
        state["entry"] = entry
        print(f"ready gtk-ibus-insertion {APP_ID}", flush=True)
        GLib.timeout_add(50, send_trigger)

    application.connect("activate", on_activate)
    application.run([])
    if state["window"] is not None:
        state["window"].destroy()
    return state["result"]


if __name__ == "__main__":
    sys.exit(main())
