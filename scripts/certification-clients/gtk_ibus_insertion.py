#!/usr/bin/python3
"""Type a harmless trigger into a focused GTK3 IBus context via XTest."""

import subprocess
import sys

import gi
gi.require_version("Gtk", "3.0")
from gi.repository import GLib, Gtk

APP_ID = "org.wayexpand.GtkIbusInsertion"
TRIGGER = ";e2e"
EXPECTED = "expanded"
TIMEOUT_MS = 5000


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
            # The headless Mutter backend does not consume hot-plugged evdev
            # devices. Use XTest against a real GTK Xwayland window so the
            # input still traverses GTK's IBus context and the selected IBus
            # engine; the compositor/input-method integration is checked by
            # the source activation and service diagnostics alongside this.
            window_ids = subprocess.run(
                ["xdotool", "search", "--sync", "--name", "^WayExpand GTK insertion acceptance$"],
                check=True,
                timeout=3,
                capture_output=True,
                text=True,
            ).stdout.splitlines()
            if not window_ids:
                raise RuntimeError("GTK test window was not found by Xwayland")
            window_id = window_ids[-1]
            subprocess.run(
                ["xdotool", "windowactivate", "--sync", window_id],
                check=True,
                timeout=3,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.PIPE,
                text=True,
            )
            subprocess.run(
                ["xdotool", "windowfocus", "--sync", window_id],
                check=True,
                timeout=3,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.PIPE,
                text=True,
            )
            subprocess.run(
                ["xdotool", "key", "--clearmodifiers", "--delay", "50",
                 "semicolon", "e", "2", "e"],
                check=True,
                timeout=3,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.PIPE,
                text=True,
            )
        except (OSError, RuntimeError, subprocess.SubprocessError) as error:
            print(f"fail gtk-ibus-insertion XTest keyboard input unavailable: {error}", flush=True)
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
