#!/usr/bin/env python3
"""Exercise the GTK 3 or GTK 4 IBus module in the private GNOME session.

This drives GTK's focused Entry test widget, so key events traverse GTK's
actual IMContext and IBus module. It does not simulate compositor or physical
keyboard delivery. The only entered content is a fixed, harmless test trigger.
"""

import os
import sys
import argparse

import gi


APP_ID = "org.wayexpand.GnomeGtkIbus"
TIMEOUT_MS = 15000


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--expected", required=True)
    parser.add_argument("--password", action="store_true")
    parser.add_argument("--gtk-version", choices=("3", "4"), default="3")
    args = parser.parse_args()
    gi.require_version("Gtk", "4.0" if args.gtk_version == "4" else "3.0")
    from gi.repository import GLib, Gtk
    if os.environ.get("GTK_IM_MODULE") != "ibus":
        raise SystemExit("GTK_IM_MODULE must be set to ibus")
    if not os.environ.get("IBUS_ADDRESS"):
        raise SystemExit("IBUS_ADDRESS must identify the private IBus daemon")
    if not os.environ.get("DISPLAY"):
        raise SystemExit("DISPLAY must identify GNOME's private XWayland server")
    focus_marker = os.environ.get("GTK_E2E_FOCUS_MARKER")
    if not focus_marker:
        raise SystemExit("GTK_E2E_FOCUS_MARKER must be set by the Mutter focus test")

    if args.gtk_version == "4":
        Gtk.init()
    else:
        Gtk.init(sys.argv)
    application = None
    if args.gtk_version == "4":
        application = Gtk.Application(application_id=APP_ID)
        application.register(None)
        window = Gtk.ApplicationWindow(
            application=application,
            title=f"WayExpand GTK IBus acceptance {APP_ID}",
        )
    else:
        window = Gtk.Window(title=f"WayExpand GTK IBus acceptance {APP_ID}")
    window.set_default_size(500, 80)
    if args.gtk_version == "4" and not args.password:
        focus_widget = Gtk.TextView()
        text_buffer = focus_widget.get_buffer()

        def current_text():
            start, end = text_buffer.get_bounds()
            return text_buffer.get_text(start, end, True)
    else:
        focus_widget = Gtk.Entry()

        def current_text():
            return focus_widget.get_text()

    if args.password:
        focus_widget.set_visibility(False)
        focus_widget.set_input_purpose(Gtk.InputPurpose.PASSWORD)
    if args.gtk_version == "4":
        window.set_child(focus_widget)
    else:
        window.add(focus_widget)
        window.show_all()
    window.present()
    focus_widget.grab_focus()
    if application is not None:
        def activate_window(_application):
            window.present()
            window.set_focus(focus_widget)

        application.connect("activate", activate_window)
        application.activate()

    failed = False
    started_at = GLib.get_monotonic_time()
    loop = GLib.MainLoop() if args.gtk_version == "4" else None

    def quit_loop():
        if loop is not None:
            loop.quit()
        else:
            Gtk.main_quit()

    def wait_for_expansion():
        nonlocal failed
        if not os.path.exists(focus_marker):
            if GLib.get_monotonic_time() - started_at >= TIMEOUT_MS * 1000:
                print("Mutter did not activate the GTK acceptance window", file=sys.stderr)
                failed = True
                quit_loop()
                return False
            return True
        if not focus_widget.has_focus():
            # In GTK 4, grabbing focus before Mutter activates the toplevel
            # may be ignored. Retry only after the Shell focus controller has
            # confirmed this acceptance window is active.
            if args.gtk_version == "4":
                window.set_focus(focus_widget)
            else:
                focus_widget.grab_focus()
        actual = current_text()
        if actual == args.expected:
            quit_loop()
            return False
        if GLib.get_monotonic_time() - started_at >= TIMEOUT_MS * 1000:
            # The test text is fixed and deliberately non-sensitive.
            print(
                f"GTK IBus result mismatch: expected {args.expected!r}, got {actual!r}",
                file=sys.stderr,
            )
            print(
                f"GTK focus diagnostic: active={window.is_active()} "
                f"widget_focused={window.get_focus() is focus_widget}",
                file=sys.stderr,
            )
            failed = True
            quit_loop()
            return False
        return True

    GLib.timeout_add(20, wait_for_expansion)
    if loop is not None:
        loop.run()
    else:
        Gtk.main()
    if args.gtk_version == "4":
        window.close()
    else:
        window.destroy()
    if failed:
        raise SystemExit(1)
    if args.password:
        result = "preserved the trigger in a password-purpose field"
    elif args.expected.startswith(";"):
        result = "passed the trigger through unchanged"
    else:
        result = "replaced the trigger"
    print(f"pass GTK {args.gtk_version} IBus input context {result} in Gtk.Entry")


if __name__ == "__main__":
    main()
