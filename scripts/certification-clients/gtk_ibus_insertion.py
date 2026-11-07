#!/usr/bin/python3
"""Exercise GTK's IBus input-method context with a harmless test trigger."""

import json
import os
import subprocess
import sys
import time

import gi
gi.require_version("Gdk", "3.0")
gi.require_version("Gtk", "3.0")
from gi.repository import Gdk, GLib, Gtk

APP_ID = "org.wayexpand.GtkIbusInsertion"
TRIGGER = ";e2e"
EXPECTED = "expanded"
TIMEOUT_MS = 5000


def main():
    application = Gtk.Application(application_id=APP_ID)
    state = {
        "window": None,
        "entry": None,
        "im_context": None,
        "text": "",
        "focus_attempts": 0,
        "attempts": 0,
        "trigger_index": 0,
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
            # GTK can report widget focus before GNOME's newly selected IBus
            # source has finished creating its engine process. Wait for the
            # engine's content-free live status before sending the trigger so
            # the acceptance test does not race source activation.
            cli = os.path.join(os.environ["E2E_BIN_DIR"], "wayexpand")
            deadline = time.monotonic() + 8
            engine_live = False
            while time.monotonic() < deadline:
                status = subprocess.run(
                    [cli, "ibus", "status", "--json"],
                    check=False,
                    timeout=3,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.DEVNULL,
                    text=True,
                )
                try:
                    report = json.loads(status.stdout)
                    engine_live = report.get("readiness", {}).get("engine_running") is True
                except json.JSONDecodeError:
                    engine_live = False
                if engine_live:
                    break
                time.sleep(0.1)
            if not engine_live:
                raise RuntimeError("GNOME selected WayExpand but its IBus engine did not become live")
            time.sleep(0.2)
            state["trigger_index"] = 0
            GLib.timeout_add(50, send_next_key)
        except (OSError, RuntimeError, subprocess.SubprocessError) as error:
            print(f"fail gtk-ibus-insertion Wayland IBus context unavailable: {error}", flush=True)
            return finish(1)
        return GLib.SOURCE_REMOVE

    def send_next_key():
        index = state["trigger_index"]
        if index >= len(TRIGGER):
            state["attempts"] = 0
            GLib.timeout_add(50, check_result)
            return GLib.SOURCE_REMOVE

        context = state["im_context"]
        text = state["text"]
        context.set_surrounding(text, len(text), len(text))
        event = Gdk.Event.new(Gdk.EventType.KEY_PRESS)
        event.key.window = state["entry"].get_window()
        event.key.send_event = True
        event.key.time = Gdk.CURRENT_TIME
        event.key.state = Gdk.ModifierType(0)
        event.key.keyval = Gdk.unicode_to_keyval(ord(TRIGGER[index]))
        event.key.hardware_keycode = 0
        event.key.group = 0
        # Call the GTK input-method filter itself. Queuing a made-up event
        # through GDK is ignored by Mutter's Wayland backend, while calling
        # Gtk.Widget.event() skips this filter entirely.
        handled = context.filter_keypress(event)
        if not handled and state["text"] == text:
            state["text"] += TRIGGER[index]
            state["entry"].set_text(state["text"])
            state["entry"].set_position(len(state["text"]))
        state["trigger_index"] += 1
        GLib.timeout_add(50, send_next_key)
        return GLib.SOURCE_REMOVE

    def on_activate(app):
        window = Gtk.ApplicationWindow(
            application=app,
            title="WayExpand GTK insertion acceptance",
        )
        entry = Gtk.Entry()
        entry.set_input_purpose(Gtk.InputPurpose.FREE_FORM)
        window.add(entry)
        context = Gtk.IMMulticontext.new()

        def on_commit(_context, committed_text):
            state["text"] += committed_text
            entry.set_text(state["text"])
            entry.set_position(len(state["text"]))

        def on_delete_surrounding(_context, offset, count):
            end = len(state["text"])
            start = max(0, min(end, end + offset))
            stop = max(start, min(end, start + count))
            state["text"] = state["text"][:start] + state["text"][stop:]
            entry.set_text(state["text"])
            entry.set_position(len(state["text"]))
            return True

        context.connect("commit", on_commit)
        context.connect("delete-surrounding", on_delete_surrounding)
        state["im_context"] = context
        window.show_all()
        context.set_client_window(entry.get_window())
        window.present()
        context.focus_in()
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
