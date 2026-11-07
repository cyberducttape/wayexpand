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
from gi.repository import GLib, Gtk  # noqa: E402


EXPECTED = "expanded"
APP_ID = "org.wayexpand.GnomeGtkIbus"
TIMEOUT_MS = 15000


def main():
    if os.environ.get("GTK_IM_MODULE") != "ibus":
        raise SystemExit("GTK_IM_MODULE must be set to ibus")
    if not os.environ.get("IBUS_ADDRESS"):
        raise SystemExit("IBUS_ADDRESS must identify the private IBus daemon")
    if not os.environ.get("DISPLAY"):
        raise SystemExit("DISPLAY must identify GNOME's private XWayland server")
    focus_marker = os.environ.get("GTK_E2E_FOCUS_MARKER")
    if not focus_marker:
        raise SystemExit("GTK_E2E_FOCUS_MARKER must be set by the Mutter focus test")

    Gtk.init(sys.argv)
    window = Gtk.Window(title=f"WayExpand GTK IBus acceptance {APP_ID}")
    window.set_default_size(500, 80)
    entry = Gtk.Entry()
    window.add(entry)
    window.show_all()
    window.present()
    entry.grab_focus()

    failed = False
    started_at = GLib.get_monotonic_time()

    def wait_for_expansion():
        nonlocal failed
        if not os.path.exists(focus_marker):
            if GLib.get_monotonic_time() - started_at >= TIMEOUT_MS * 1000:
                print("Mutter did not activate the GTK acceptance window", file=sys.stderr)
                failed = True
                Gtk.main_quit()
                return False
            return True
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

    GLib.timeout_add(20, wait_for_expansion)
    Gtk.main()
    window.destroy()
    if failed:
        raise SystemExit(1)
    print("pass GTK 3 IBus input context replaced the test trigger in Gtk.Entry")


if __name__ == "__main__":
    main()
