#!/usr/bin/env python3
"""Exercise the GTK 3 IBus module in the private GNOME acceptance session.

This drives GTK's focused Entry test widget, so key events traverse GTK's
actual IMContext and IBus module. It does not simulate compositor or physical
keyboard delivery. The only entered content is a fixed, harmless test trigger.
"""

import os
import sys

import gi

gi.require_version("Gtk", "3.0")
gi.require_version("Gdk", "3.0")
from gi.repository import Gdk, GLib, Gtk  # noqa: E402


TRIGGER = ";e2e"
EXPECTED = "expanded"
APP_ID = "org.wayexpand.GnomeGtkIbus"
TIMEOUT_MS = 10000


def main():
    if os.environ.get("GTK_IM_MODULE") != "ibus":
        raise SystemExit("GTK_IM_MODULE must be set to ibus")
    if not os.environ.get("IBUS_ADDRESS"):
        raise SystemExit("IBUS_ADDRESS must identify the private IBus daemon")

    Gtk.init(sys.argv)
    window = Gtk.Window(title=f"WayExpand GTK IBus acceptance {APP_ID}")
    window.set_default_size(500, 80)
    entry = Gtk.Entry()
    window.add(entry)
    window.show_all()
    window.present()
    entry.grab_focus()

    sent = 0
    failed = False

    started_at = GLib.get_monotonic_time()

    def send_next():
        nonlocal sent, failed
        if not entry.has_focus():
            entry.grab_focus()
            if GLib.get_monotonic_time() - started_at >= TIMEOUT_MS * 1000:
                print("GTK Entry did not receive focus", file=sys.stderr)
                failed = True
                Gtk.main_quit()
                return False
            return True
        if sent >= len(TRIGGER):
            return False
        character = TRIGGER[sent]
        keyval = Gdk.unicode_to_keyval(ord(character))
        # The return value reports whether the widget handled the key event;
        # an input method may consume it before Gtk.Entry's own handler. The
        # authoritative assertion is the final widget text below.
        Gtk.test_widget_send_key(entry, keyval, 0)
        sent += 1
        if sent < len(TRIGGER):
            return True
        GLib.timeout_add(25, wait_for_expansion)
        return False

    def wait_for_expansion():
        nonlocal failed
        actual = entry.get_text()
        if actual == EXPECTED:
            Gtk.main_quit()
            return False
        if GLib.get_monotonic_time() - started_at >= TIMEOUT_MS * 1000:
            # The test text is fixed and deliberately non-sensitive.
            print(
                f"GTK IBus insertion mismatch: expected {EXPECTED!r}, got {actual!r}",
                file=sys.stderr,
            )
            failed = True
            Gtk.main_quit()
            return False
        return True

    GLib.timeout_add(20, send_next)
    Gtk.main()
    window.destroy()
    if failed:
        raise SystemExit(1)
    print("pass GTK 3 IBus input context replaced the test trigger in Gtk.Entry")


if __name__ == "__main__":
    main()
