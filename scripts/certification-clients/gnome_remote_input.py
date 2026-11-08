#!/usr/bin/env python3
"""Send fixed ASCII test keysyms through GNOME Mutter's private test interface.

This is only for the isolated GNOME acceptance suite. It uses Mutter's
RemoteDesktop D-Bus interface so test input follows the compositor's keyboard
path instead of XTEST's XWayland-only path. It does not read keyboard input or
log the test text.
"""

import argparse
import sys
import time

import gi

gi.require_version("Gio", "2.0")
from gi.repository import Gio, GLib


BUS_NAME = "org.gnome.Mutter.RemoteDesktop"
ROOT_PATH = "/org/gnome/Mutter/RemoteDesktop"
ROOT_INTERFACE = "org.gnome.Mutter.RemoteDesktop"
SESSION_INTERFACE = "org.gnome.Mutter.RemoteDesktop.Session"


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


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--delay-ms", type=int, default=60)
    parser.add_argument("--text", required=True)
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
        send_text(bus, session_path, args.text, args.delay_ms / 1000)
    except (GLib.Error, OSError, ValueError) as error:
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
