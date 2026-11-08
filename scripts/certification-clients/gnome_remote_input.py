#!/usr/bin/env python3
"""Send fixed ASCII keyboard input through Mutter's private test interface.

This is only for the isolated GNOME acceptance suite. It uses Mutter's
RemoteDesktop D-Bus interface so test input follows the compositor's keyboard
path instead of XTEST's XWayland-only path. It does not read keyboard input or
log the test text.
"""

import argparse
import json
import os
import sys
import time

import gi

gi.require_version("Gio", "2.0")
from gi.repository import Gio, GLib


BUS_NAME = "org.gnome.Mutter.RemoteDesktop"
ROOT_PATH = "/org/gnome/Mutter/RemoteDesktop"
ROOT_INTERFACE = "org.gnome.Mutter.RemoteDesktop"
SESSION_INTERFACE = "org.gnome.Mutter.RemoteDesktop.Session"
FOCUS_CONTROLLER_PATH = "/org/wayexpand/GnomeE2eFocusController"
FOCUS_CONTROLLER_INTERFACE = "org.wayexpand.GnomeE2eFocusController"
FOCUS_CONTROLLER_NOT_FOUND = (
    "org.wayexpand.GnomeE2eFocusController.Error.NotFound"
)
SHIFT_L_KEYSYM = 0xFFE1
BTN_LEFT = 0x110
IBUS_FOCUS_STABLE_FOR_US = 250_000


def key_event(bus, path, key, pressed, *, keycode=False):
    bus.call_sync(
        BUS_NAME,
        path,
        SESSION_INTERFACE,
        "NotifyKeyboardKeycode" if keycode else "NotifyKeyboardKeysym",
        GLib.Variant("(ub)", (key, pressed)),
        GLib.VariantType.new("()"),
        Gio.DBusCallFlags.NONE,
        5000,
        None,
    )


def pointer_motion(bus, path, dx, dy):
    bus.call_sync(
        BUS_NAME,
        path,
        SESSION_INTERFACE,
        "NotifyPointerMotionRelative",
        GLib.Variant("(dd)", (float(dx), float(dy))),
        GLib.VariantType.new("()"),
        Gio.DBusCallFlags.NONE,
        5000,
        None,
    )


def pointer_button(bus, path, pressed):
    bus.call_sync(
        BUS_NAME,
        path,
        SESSION_INTERFACE,
        "NotifyPointerButton",
        GLib.Variant("(ib)", (BTN_LEFT, pressed)),
        GLib.VariantType.new("()"),
        Gio.DBusCallFlags.NONE,
        5000,
        None,
    )


def get_test_window_rect(bus, app_id):
    result = bus.call_sync(
        "org.gnome.Shell",
        FOCUS_CONTROLLER_PATH,
        FOCUS_CONTROLLER_INTERFACE,
        "GetWindowRect",
        GLib.Variant("(s)", (app_id,)),
        GLib.VariantType.new("(iiii)"),
        Gio.DBusCallFlags.NONE,
        5000,
        None,
    )
    return result.unpack()


def get_pointer_position(bus):
    result = bus.call_sync(
        "org.gnome.Shell",
        FOCUS_CONTROLLER_PATH,
        FOCUS_CONTROLLER_INTERFACE,
        "GetPointerPosition",
        None,
        GLib.VariantType.new("(ii)"),
        Gio.DBusCallFlags.NONE,
        5000,
        None,
    )
    return result.unpack()


def click_test_field(bus, path, window_rect):
    """Click the mapped fixture's center to establish real surface focus."""
    x, y, width, height = window_rect
    target_x = x + width / 2
    target_y = y + height / 2
    # RemoteDesktop relative motion accepts deltas, while GetWindowRect reports
    # absolute compositor coordinates. Convert using the live pointer location
    # instead of assuming an edge-clamped starting position or a zero-origin
    # monitor. Both values come from Mutter's global coordinate space.
    pointer_x, pointer_y = get_pointer_position(bus)
    start = (pointer_x, pointer_y)
    # Keep the axes in separate RemoteDesktop events. Headless Mutter has
    # occasionally applied only one component of a combined relative motion;
    # read the compositor position after each event so the click is never sent
    # on the strength of a successful D-Bus method return alone.
    for axis, current, target in (
        ("x", pointer_x, target_x),
        ("y", pointer_y, target_y),
    ):
        delta = target - current
        if abs(delta) <= 2:
            continue
        pointer_motion(
            bus,
            path,
            delta if axis == "x" else 0.0,
            delta if axis == "y" else 0.0,
        )
        time.sleep(0.1)
        actual_x, actual_y = get_pointer_position(bus)
        actual = actual_x if axis == "x" else actual_y
        if abs(actual - target) > 2:
            raise RuntimeError(
                f"Mutter pointer {axis}-axis motion was not applied: "
                f"start=({start[0]}, {start[1]}) "
                f"requested_delta={delta} target=({target_x}, {target_y}) "
                f"actual=({actual_x}, {actual_y})"
            )
        pointer_x, pointer_y = actual_x, actual_y
    actual_x, actual_y = get_pointer_position(bus)
    if abs(actual_x - target_x) > 2 or abs(actual_y - target_y) > 2:
        raise RuntimeError(
            "Mutter pointer did not reach GTK target: "
            f"start=({start[0]}, {start[1]}) "
            f"target=({target_x}, {target_y}) actual=({actual_x}, {actual_y})"
        )
    print(
        "Mutter pointer verified at GTK target: "
        f"({actual_x}, {actual_y})",
        flush=True,
    )
    try:
        pointer_button(bus, path, True)
        time.sleep(0.05)
    finally:
        pointer_button(bus, path, False)
    time.sleep(0.1)


def send_text(
    bus,
    path,
    text,
    delay_seconds,
    ready_marker,
    *,
    keycode_probe=False,
    ibus_status_path=None,
    ibus_sequence=None,
):
    """Inject fixed test text as keysyms or a single US evdev-keycode probe."""
    if keycode_probe and text != "a":
        raise ValueError("the evdev-keycode probe is limited to the fixed ASCII 'a' input")
    if (ibus_status_path is None) != (ibus_sequence is None):
        raise ValueError("IBus status path and focused sequence must be supplied together")
    sent = 0
    for character in text:
        if not os.path.isfile(ready_marker):
            raise RuntimeError(
                "GTK active focus was lost after "
                f"{sent} of {len(text)} test characters; remaining input withheld"
            )
        if ibus_status_path is not None:
            current_sequence = latest_focused_ibus_context(
                ibus_status_path, ibus_sequence - 1
            )
            if current_sequence != ibus_sequence:
                raise RuntimeError(
                    "the focused IBus context changed after "
                    f"{sent} of {len(text)} test characters; remaining input withheld"
                )
        if keycode_probe:
            # Linux input-event-codes.h defines KEY_A as evdev keycode 30.
            # The US XKB source is active for this baseline, so this exercises
            # compositor keycode-to-text translation before testing IBus.
            key = 30
        else:
            key = ord(character)
            if not 0x20 <= key <= 0x7E:
                raise ValueError("test input must use printable ASCII keysyms")
        try:
            key_event(bus, path, key, True, keycode=keycode_probe)
            time.sleep(delay_seconds)
        finally:
            # A failed reply can leave it uncertain whether Mutter accepted
            # the press, so always attempt the matching release.
            key_event(bus, path, key, False, keycode=keycode_probe)
        sent += 1
        time.sleep(delay_seconds)


def prime_keyboard_device(bus, path):
    """Create Mutter's virtual keyboard before asking it to activate GTK."""
    try:
        key_event(bus, path, SHIFT_L_KEYSYM, True)
    finally:
        # A single modifier press/release creates the virtual device without
        # inserting text into whichever application was focused beforehand.
        key_event(bus, path, SHIFT_L_KEYSYM, False)


def activate_test_window(bus, app_id):
    deadline = time.monotonic() + 5.0
    while True:
        try:
            bus.call_sync(
                "org.gnome.Shell",
                FOCUS_CONTROLLER_PATH,
                FOCUS_CONTROLLER_INTERFACE,
                "ActivateWindow",
                GLib.Variant("(s)", (app_id,)),
                GLib.VariantType.new("()"),
                Gio.DBusCallFlags.NONE,
                10000,
                None,
            )
            return
        except GLib.Error as error:
            # GTK can publish its application ID before Mutter maps the
            # corresponding toplevel. Retry only that explicit transient
            # condition; focus timeouts and authorization failures must fail
            # immediately instead of being hidden by repeated activation.
            message = getattr(error, "message", str(error))
            if FOCUS_CONTROLLER_NOT_FOUND not in message or time.monotonic() >= deadline:
                raise
            time.sleep(0.05)


def publish_focus_confirmation(path):
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        os.write(descriptor, b"1")
    finally:
        os.close(descriptor)


def latest_focused_ibus_context(status_path, sequence_baseline):
    """Return the newest newly focused IBus context sequence, if any."""
    try:
        with open(status_path, encoding="utf-8") as source:
            status = json.load(source)
        fields = status.get("recent_fields") or []
        newest = max(fields, key=lambda field: int(field.get("sequence", 0)), default=None)
        if (
            newest
            and int(newest.get("sequence", 0)) > sequence_baseline
            and newest.get("focused") is True
        ):
            return int(newest["sequence"])
        return None
    except (OSError, ValueError, TypeError, json.JSONDecodeError):
        return None


def update_ibus_focus_stability(sequence, current_sequence, focused_since, now):
    """Require the same newest focused IBus context for a stable interval."""
    if sequence is None:
        return None, None, False
    if sequence != current_sequence:
        return sequence, now, False
    return (
        current_sequence,
        focused_since,
        now - focused_since >= IBUS_FOCUS_STABLE_FOR_US,
    )


def wait_for_gtk_input_ready(
    bus,
    app_id,
    path,
    timeout_seconds,
    ibus_status_path=None,
    ibus_sequence_baseline=0,
):
    deadline = time.monotonic() + timeout_seconds
    ready_since = None
    next_activation_at = 0.0
    next_ibus_probe_at = 0.0
    ibus_context_sequence = None
    ibus_context_focused_since = None
    while time.monotonic() < deadline:
        now = time.monotonic()
        client_focus_ready = os.path.isfile(path)
        if not client_focus_ready and now >= next_activation_at:
            # Shell can report Meta.Window focus before GTK receives the
            # corresponding Wayland activation. Reissue the compositor-side
            # activation until the GTK client publishes its own active/focus
            # lease. Once GTK confirms focus, do not perturb it while waiting
            # for IBus context telemetry to stabilize.
            activate_test_window(bus, app_id)
            next_activation_at = now + 0.25
        if client_focus_ready:
            if ready_since is None:
                ready_since = now
            elif now - ready_since >= 0.25:
                if ibus_status_path is None:
                    return True
                if now >= next_ibus_probe_at:
                    focused_sequence = latest_focused_ibus_context(
                        ibus_status_path, ibus_sequence_baseline
                    )
                    (
                        ibus_context_sequence,
                        ibus_context_focused_since,
                        ibus_ready,
                    ) = update_ibus_focus_stability(
                        focused_sequence,
                        ibus_context_sequence,
                        ibus_context_focused_since,
                        int(now * 1_000_000),
                    )
                    next_ibus_probe_at = now + 0.1
                    if ibus_ready:
                        return ibus_context_sequence
        else:
            ready_since = None
            ibus_context_sequence = None
            ibus_context_focused_since = None
        time.sleep(0.02)
    return False


def wait_for_gtk_input_ready_with_pointer_fallback(
    bus,
    session_path,
    app_id,
    window_rect,
    ready_marker,
    timeout_seconds,
    ibus_status_path=None,
    ibus_sequence_baseline=0,
):
    """Wait for actual GTK focus, clicking once only if activation stalls."""
    initial_wait = min(2.0, timeout_seconds)
    ready = wait_for_gtk_input_ready(
        bus,
        app_id,
        ready_marker,
        initial_wait,
        ibus_status_path,
        ibus_sequence_baseline,
    )
    if ready:
        return ready

    click_test_field(bus, session_path, window_rect)
    # Require a fresh focus lease after the fallback click before sending any
    # test keys. The GTK client recreates this marker only after focus is
    # stable again.
    try:
        os.unlink(ready_marker)
    except FileNotFoundError:
        pass
    remaining = max(0.0, timeout_seconds - initial_wait)
    return wait_for_gtk_input_ready(
        bus,
        app_id,
        ready_marker,
        remaining,
        ibus_status_path,
        ibus_sequence_baseline,
    )


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--delay-ms", type=int, default=60)
    parser.add_argument("--text", required=True)
    parser.add_argument("--app-id", required=True)
    parser.add_argument("--focus-marker", required=True)
    parser.add_argument("--ready-marker", required=True)
    parser.add_argument("--ibus-status-path")
    parser.add_argument("--ibus-sequence-baseline", type=int, default=0)
    parser.add_argument(
        "--keycode-probe",
        action="store_true",
        help="send only the fixed US-layout 'a' baseline as evdev KEY_A",
    )
    args = parser.parse_args()
    if args.delay_ms < 0 or args.delay_ms > 1000:
        parser.error("--delay-ms must be between 0 and 1000")
    if not args.text or any(not 0x20 <= ord(character) <= 0x7E for character in args.text):
        parser.error("--text must contain non-empty printable ASCII acceptance-test input")

    bus = None
    session_path = None
    try:
        bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
        result = bus.call_sync(
            BUS_NAME,
            ROOT_PATH,
            ROOT_INTERFACE,
            "CreateSession",
            None,
            GLib.VariantType.new("(o)"),
            Gio.DBusCallFlags.NONE,
            5000,
            None,
        )
        session_path = result.unpack()[0]
        bus.call_sync(
            BUS_NAME,
            session_path,
            SESSION_INTERFACE,
            "Start",
            None,
            GLib.VariantType.new("()"),
            Gio.DBusCallFlags.NONE,
            5000,
            None,
        )
        # Ask Shell to raise the mapped window before creating Mutter's
        # virtual keyboard. The modifier-only event creates the device without
        # inserting text. Re-activate afterwards so the Wayland surface gets a
        # fresh compositor activation following that input event; Shell's
        # MetaWindow focus can otherwise get ahead of GTK's active state in a
        # headless session.
        activate_test_window(bus, args.app_id)
        prime_keyboard_device(bus, session_path)
        activate_test_window(bus, args.app_id)
        window_rect = get_test_window_rect(bus, args.app_id)
        print(f"Mutter test target geometry: window={window_rect!r}", flush=True)
        # Start GTK's bounded present()/grab_focus() retries as soon as Mutter
        # confirms its window focus. Give the client a short chance to report
        # real active-toplevel and widget focus before clicking. A click sent
        # while Mutter has only updated MetaWindow focus can be dropped by a
        # native Wayland surface. Use one click only as a bounded fallback.
        publish_focus_confirmation(args.focus_marker)
        ready = wait_for_gtk_input_ready_with_pointer_fallback(
            bus,
            session_path,
            args.app_id,
            window_rect,
            args.ready_marker,
            13.0,
            args.ibus_status_path,
            args.ibus_sequence_baseline,
        )
        if not ready:
            raise RuntimeError(
                "GTK or IBus did not confirm a stable focused input context; "
                "test text was not sent"
            )
        ibus_context_sequence = (
            ready if args.ibus_status_path is not None else None
        )
        send_text(
            bus,
            session_path,
            args.text,
            args.delay_ms / 1000,
            args.ready_marker,
            keycode_probe=args.keycode_probe,
            ibus_status_path=args.ibus_status_path,
            ibus_sequence=ibus_context_sequence,
        )
    except (GLib.Error, OSError, RuntimeError, ValueError) as error:
        print(f"Mutter RemoteDesktop keyboard input failed: {error}", file=sys.stderr)
        return 1
    finally:
        if bus is not None and session_path is not None:
            try:
                bus.call_sync(
                    BUS_NAME,
                    session_path,
                    SESSION_INTERFACE,
                    "Stop",
                    None,
                    GLib.VariantType.new("()"),
                    Gio.DBusCallFlags.NONE,
                    5000,
                    None,
                )
            except GLib.Error:
                pass
    if args.keycode_probe:
        print(
            "Mutter submitted the fixed KEY_A evdev probe; "
            f"GTK fixture will verify delivery (window={window_rect!r})"
        )
    else:
        print(
            "Mutter submitted the fixed ASCII keysym input; "
            f"GTK fixture will verify delivery (window={window_rect!r})"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
