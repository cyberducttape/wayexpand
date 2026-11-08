#!/usr/bin/env python3
"""Exercise GTK 3/4 input contexts in the private GNOME session.

This drives GTK's focused test widget, so key events traverse GTK's actual
IMContext. The IBus mode also traverses the IBus module; the simple mode is an
XKB control. It does not simulate physical keyboard delivery. The only
entered content is fixed, harmless test text.
"""

import os
import sys
import argparse

import gi


DEFAULT_APP_ID = "org.wayexpand.GnomeGtkIbus"
TIMEOUT_MS = 15000
FOCUS_STABLE_FOR_US = 250_000
FOCUS_RETRY_INTERVAL_US = 250_000


def update_focus_stability(
    mutter_focus_confirmed, window_active, widget_has_keyboard_focus, focused_since, now
):
    """Track real GTK focus after Mutter confirms the target window active."""
    if not (mutter_focus_confirmed and window_active and widget_has_keyboard_focus):
        return None, False
    if focused_since is None:
        return now, False
    return focused_since, now - focused_since >= FOCUS_STABLE_FOR_US


def update_readiness_marker(path, ready):
    """Publish current focus readiness, removing stale readiness on focus loss."""
    if ready:
        try:
            descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        except FileExistsError:
            return
        try:
            os.write(descriptor, b"1")
        finally:
            os.close(descriptor)
    else:
        try:
            os.unlink(path)
        except FileNotFoundError:
            pass


def focus_request_due(window_active, widget_has_keyboard_focus, now, next_request_at):
    """Throttle focus requests until GTK and Mutter agree on the target."""
    if window_active and widget_has_keyboard_focus:
        return False, next_request_at
    if next_request_at is None or now >= next_request_at:
        return True, now + FOCUS_RETRY_INTERVAL_US
    return False, next_request_at


def input_mode_environment_error(im_mode, environ):
    """Check only the environment contract required by the selected IM mode."""
    if im_mode == "ibus" and environ.get("GTK_IM_MODULE") != "ibus":
        return "GTK_IM_MODULE must be set to ibus"
    if im_mode == "simple" and environ.get("GTK_IM_MODULE") != "gtk-im-context-simple":
        return "GTK_IM_MODULE must be set to gtk-im-context-simple"
    if im_mode == "simple" and environ.get("XMODIFIERS"):
        return "XMODIFIERS must be unset for the GTK simple input path"
    if im_mode == "simple" and environ.get("IBUS_ADDRESS"):
        return "IBUS_ADDRESS must be unset for the GTK simple input path"
    if im_mode == "ibus" and not environ.get("IBUS_ADDRESS"):
        return "IBUS_ADDRESS must identify the private IBus daemon"
    return None


def display_backend_environment_error(environ):
    """Require the display variables for the backend the fixture claims to test."""
    backend = environ.get("GDK_BACKEND")
    if backend == "wayland":
        if not environ.get("WAYLAND_DISPLAY"):
            return "WAYLAND_DISPLAY must identify GNOME's private Wayland compositor"
        if environ.get("DISPLAY") or environ.get("XAUTHORITY"):
            return "X11 display variables must be unset for the native Wayland test"
        return None
    if backend == "x11":
        if not environ.get("DISPLAY"):
            return "DISPLAY must identify GNOME's private XWayland server"
        return None
    return "GDK_BACKEND must explicitly select wayland or x11"


def configure_widget_im_module(widget, gtk_version, im_mode):
    """Pin the GTK 3 XKB control to its requested module on the widget."""
    if gtk_version != "3":
        return None
    if im_mode == "simple":
        widget.set_property("im-module", "gtk-im-context-simple")
    return widget.get_property("im-module")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--expected", required=True)
    parser.add_argument("--password", action="store_true")
    parser.add_argument("--gtk-version", choices=("3", "4"), default="3")
    parser.add_argument("--im-mode", choices=("ibus", "simple"), default="ibus")
    args = parser.parse_args()
    gi.require_version("Gdk", "4.0" if args.gtk_version == "4" else "3.0")
    gi.require_version("Gtk", "4.0" if args.gtk_version == "4" else "3.0")
    from gi.repository import Gdk, Gio, GLib, Gtk
    environment_error = input_mode_environment_error(args.im_mode, os.environ)
    if environment_error:
        raise SystemExit(environment_error)
    backend_error = display_backend_environment_error(os.environ)
    if backend_error:
        raise SystemExit(backend_error)
    focus_marker = os.environ.get("GTK_E2E_FOCUS_MARKER")
    if not focus_marker:
        raise SystemExit("GTK_E2E_FOCUS_MARKER must be set by the Mutter focus test")
    ready_marker = os.environ.get("GTK_E2E_READY_MARKER")
    if not ready_marker:
        raise SystemExit("GTK_E2E_READY_MARKER must be set by the Mutter focus test")
    # A readiness marker is a live focus lease. Do not inherit one left by a
    # stale test process, even when the caller reuses a work directory.
    update_readiness_marker(ready_marker, False)

    if args.gtk_version == "4":
        Gtk.init()
    else:
        Gtk.init(sys.argv)
    gtk_im_module_setting = None
    if args.im_mode == "simple":
        # Record desktop settings too, but GTK_IM_MODULE explicitly selects
        # GtkIMContextSimple so this XKB control cannot attach to IBus.
        settings = Gtk.Settings.get_default()
        if settings is not None:
            try:
                gtk_im_module_setting = settings.get_property("gtk-im-module")
            except (TypeError, ValueError):
                gtk_im_module_setting = "unavailable"
    application_id = os.environ.get("GTK_E2E_APP_ID", DEFAULT_APP_ID)
    application = Gtk.Application(
        application_id=application_id,
        flags=Gio.ApplicationFlags.NON_UNIQUE,
    )
    application.register(None)
    window = Gtk.ApplicationWindow(application=application)
    window.set_title(f"WayExpand GTK IBus acceptance {application_id}")
    window.set_default_size(500, 80)
    window.set_decorated(False)
    widget_im_module = None
    if args.gtk_version == "4" and args.password:
        focus_widget = Gtk.PasswordEntry()

        def current_text():
            return focus_widget.get_text()
    elif args.gtk_version == "4":
        focus_widget = Gtk.TextView()
        text_buffer = focus_widget.get_buffer()

        def current_text():
            start, end = text_buffer.get_bounds()
            return text_buffer.get_text(start, end, True)
    else:
        focus_widget = Gtk.Entry()
        # GtkSettings can carry an input-module choice from the desktop and
        # override the process environment. Pin the XKB control at the widget
        # boundary so it cannot attach to IBus through that setting.
        widget_im_module = configure_widget_im_module(
            focus_widget, args.gtk_version, args.im_mode
        )

        def current_text():
            return focus_widget.get_text()

    if args.password and args.gtk_version == "3":
        focus_widget.set_visibility(False)
        focus_widget.set_input_purpose(Gtk.InputPurpose.PASSWORD)
    focus_widget.set_hexpand(True)
    focus_widget.set_vexpand(True)
    if args.gtk_version == "4":
        window.set_child(focus_widget)
    else:
        window.add(focus_widget)
        window.show_all()
    window.present()
    focus_widget.grab_focus()

    failed = False
    started_at = GLib.get_monotonic_time()
    focused_since = None
    gtk_key_events = 0
    gtk_gdk_key_events = 0
    gtk_pointer_events = 0
    gtk_gdk_pointer_events = 0
    focus_diagnostic_reported = False
    next_focus_request_at = None

    if args.gtk_version == "3" and args.im_mode == "simple":
        # Count key events as GDK first receives them, before GTK routes them
        # to the focused widget. This distinguishes an X11/GDK delivery loss
        # from a GTK focus/dispatch issue without retaining key values.
        def dispatch_gdk_event(event, _data):
            nonlocal gtk_gdk_key_events, gtk_gdk_pointer_events
            if event.type == Gdk.EventType.KEY_PRESS:
                gtk_gdk_key_events += 1
            elif event.type == Gdk.EventType.BUTTON_PRESS:
                gtk_gdk_pointer_events += 1
            Gtk.main_do_event(event)

        Gdk.event_handler_set(dispatch_gdk_event, None)
    loop = GLib.MainLoop() if args.gtk_version == "4" else None

    def count_key_event(*_event):
        # Count only. Never retain or print keyvals, text, or modifier state.
        nonlocal gtk_key_events
        gtk_key_events += 1
        return False

    def count_pointer_event(*_event):
        # Count only. Never retain coordinates, button state, or other input.
        nonlocal gtk_pointer_events
        gtk_pointer_events += 1
        return False

    if args.gtk_version == "4":
        key_controller = Gtk.EventControllerKey.new()
        key_controller.set_propagation_phase(Gtk.PropagationPhase.CAPTURE)
        key_controller.connect("key-pressed", count_key_event)
        focus_widget.add_controller(key_controller)
        click_controller = Gtk.GestureClick.new()
        click_controller.connect("pressed", count_pointer_event)
        focus_widget.add_controller(click_controller)
    else:
        # Connect before GtkEntry's default handler so this observes the
        # widget delivery boundary even when IBus consumes the key event.
        focus_widget.connect("key-press-event", count_key_event)
        focus_widget.add_events(Gdk.EventMask.BUTTON_PRESS_MASK)
        focus_widget.connect("button-press-event", count_pointer_event)

    def focused_widget_has_keyboard_focus():
        focused = window.get_focus()
        is_within_test_widget = False
        while focused is not None:
            if focused == focus_widget:
                is_within_test_widget = True
                break
            focused = focused.get_parent()
        if not is_within_test_widget:
            return False
        focused = window.get_focus()
        return focused is not None and focused.has_focus()

    def quit_loop():
        if loop is not None:
            loop.quit()
        else:
            Gtk.main_quit()

    def wait_for_expansion():
        nonlocal failed, focused_since, focus_diagnostic_reported
        nonlocal next_focus_request_at
        if not os.path.exists(focus_marker):
            if GLib.get_monotonic_time() - started_at >= TIMEOUT_MS * 1000:
                print("Mutter did not activate the GTK acceptance window", file=sys.stderr)
                failed = True
                quit_loop()
                return False
            return True
        # The marker is written only after the Shell-side controller confirms
        # global.display.get_focus_window() is this window. Also require GTK
        # to report the active toplevel and actual keyboard focus on the entry
        # (or its internal focused child). get_focus() alone only reports the
        # designated focus widget and can be non-NULL while the widget is not
        # receiving keyboard events.
        # GTK 4 controls such as PasswordEntry can delegate focus to an
        # internal child, so accept any focused descendant of the test widget.
        # GTK 3 must pass the same check; treating the version as proof of focus
        # can send synthetic input before the entry owns keyboard focus.
        widget_has_keyboard_focus = focused_widget_has_keyboard_focus()
        window_active = window.is_active()
        focused_since, child_focus_stable = update_focus_stability(
            True,
            window_active,
            widget_has_keyboard_focus,
            focused_since,
            GLib.get_monotonic_time(),
        )
        if (
            not child_focus_stable
            and not focus_diagnostic_reported
            and GLib.get_monotonic_time() - started_at >= 1_000_000
        ):
            # The shell harness may stop a client that never becomes ready.
            # Emit one content-free snapshot early enough to survive teardown.
            print(
                "GTK focus readiness diagnostic: "
                "mutter_focus_confirmed=True "
                f"active={window_active} "
                f"widget_focused={focus_widget.has_focus()} "
                f"focus_within_widget={widget_has_keyboard_focus} "
                f"focused_widget_type={type(window.get_focus()).__name__ if window.get_focus() else 'none'} "
                f"GDK_BACKEND={os.environ.get('GDK_BACKEND')!r} "
                f"WAYLAND_DISPLAY={os.environ.get('WAYLAND_DISPLAY')!r}",
                file=sys.stderr,
            )
            focus_diagnostic_reported = True
        update_readiness_marker(ready_marker, child_focus_stable)
        request_focus, next_focus_request_at = focus_request_due(
            window_active,
            widget_has_keyboard_focus,
            GLib.get_monotonic_time(),
            next_focus_request_at,
        )
        if request_focus:
            # The compositor can confirm its target before GTK has processed
            # the corresponding activation. Keep requesting focus at a
            # bounded cadence while the focus lease is active; the readiness
            # marker remains closed until GTK confirms stable active focus.
            window.present()
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
            mismatch_kind = (
                "GTK simple-input result mismatch"
                if args.im_mode == "simple"
                else "GTK IBus result mismatch"
            )
            print(
                f"{mismatch_kind}: expected {args.expected!r}, got {actual!r}",
                file=sys.stderr,
            )
            print(
                f"GTK focus diagnostic: mutter_focus_confirmed=True "
                f"active={window.is_active()} "
                f"widget_focused={focus_widget.has_focus()} "
                f"focus_within_widget={focused_widget_has_keyboard_focus()} "
                f"focused_widget_type={type(window.get_focus()).__name__ if window.get_focus() else 'none'}",
                file=sys.stderr,
            )
            print(
                f"GTK key events observed at target widget: {gtk_key_events}",
                file=sys.stderr,
            )
            print(
                f"GDK key events received by GTK process: {gtk_gdk_key_events}",
                file=sys.stderr,
            )
            print(
                f"GTK pointer press events observed at target widget: {gtk_pointer_events}",
                file=sys.stderr,
            )
            print(
                f"GDK pointer press events received by GTK process: {gtk_gdk_pointer_events}",
                file=sys.stderr,
            )
            print(
                "GTK input diagnostics: "
                f"version={args.gtk_version} "
                f"im_mode={args.im_mode} "
                f"gtk_im_module_setting={gtk_im_module_setting!r} "
                f"widget_im_module={widget_im_module!r} "
                f"GTK_IM_MODULE={os.environ.get('GTK_IM_MODULE')!r} "
                f"XMODIFIERS={os.environ.get('XMODIFIERS')!r} "
                f"GTK_APPLICATION_ID={application.get_application_id()!r} "
                f"GDK_CORE_DEVICE_EVENTS={os.environ.get('GDK_CORE_DEVICE_EVENTS')!r} "
                f"GDK_BACKEND={os.environ.get('GDK_BACKEND')!r}",
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
    if args.im_mode == "simple":
        result = "received ordinary XKB input through GtkIMContextSimple"
    elif args.password:
        result = "preserved the trigger in a password-purpose field"
    elif args.expected.startswith(";"):
        result = "passed the trigger through unchanged"
    else:
        result = "replaced the trigger"
    print(f"pass GTK {args.gtk_version} IBus input context {result} in Gtk.Entry")


if __name__ == "__main__":
    main()
