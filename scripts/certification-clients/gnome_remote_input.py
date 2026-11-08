#!/usr/bin/env python3
"""Send fixed ASCII test keysyms through GNOME Mutter's private test interface.

This is only for the isolated GNOME acceptance suite. It uses Mutter's
RemoteDesktop D-Bus interface so test input follows the compositor's keyboard
path instead of XTEST's XWayland-only path. It does not read keyboard input or
log the test text.
"""

import argparse
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


def key_event(bus, path, keysym, pressed):
    bus.call_sync(
        BUS_NAME,
        path,
        SESSION_INTERFACE,
        "NotifyKeyboardKeysym",
        GLib.Variant("(ub)", (keysym, pressed)),
        GLib.VariantType.new("()"),
        Gio.DBusCallFlags.NONE,
        5000,
        None,
    )


def send_text(bus, path, text, delay_seconds):
    """Inject keysyms so test text remains stable across XKB layout switches."""
    for character in text:
        keysym = ord(character)
        if not 0x20 <= keysym <= 0x7E:
            raise ValueError("test input must use printable ASCII keysyms")
        try:
            key_event(bus, path, keysym, True)
            time.sleep(delay_seconds)
        finally:
            # A failed reply can leave it uncertain whether Mutter accepted
            # the press, so always attempt the matching release.
            key_event(bus, path, keysym, False)
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


def wait_for_gtk_input_ready(path, timeout_seconds):
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        if os.path.isfile(path):
            return True
        time.sleep(0.02)
    return os.path.isfile(path)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--delay-ms", type=int, default=60)
    parser.add_argument("--text", required=True)
    parser.add_argument("--app-id", required=True)
    parser.add_argument("--focus-marker", required=True)
    parser.add_argument("--ready-marker", required=True)
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
        # Ask Shell to activate the mapped window before creating Mutter's
        # virtual keyboard. In headless sessions, creating the keyboard first
        # can leave Mutter's MetaWindow focus ahead of the Wayland client's
        # activated state. The modifier-only event below creates the device
        # after focus is assigned; GTK readiness is still required before any
        # text is sent.
        activate_test_window(bus, args.app_id)
        prime_keyboard_device(bus, session_path)
        publish_focus_confirmation(args.focus_marker)
        if not wait_for_gtk_input_ready(args.ready_marker, 13.0):
            raise RuntimeError(
                "GTK did not confirm stable active input focus; test text was not sent"
            )
        send_text(bus, session_path, args.text, args.delay_ms / 1000)
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
    print("Mutter RemoteDesktop delivered the fixed ASCII test input")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
