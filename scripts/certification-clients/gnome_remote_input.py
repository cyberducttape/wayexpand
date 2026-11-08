#!/usr/bin/env python3
"""Send fixed ASCII test keys through GNOME Mutter's private test interface.

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

# Linux evdev keycodes for a US keyboard. Shift is represented separately so
# Mutter resolves the code using the active XKB layout, as a keyboard would.
LETTERS = {
    "a": 30, "b": 48, "c": 46, "d": 32, "e": 18, "f": 33,
    "g": 34, "h": 35, "i": 23, "j": 36, "k": 37, "l": 38,
    "m": 50, "n": 49, "o": 24, "p": 25, "q": 16, "r": 19,
    "s": 31, "t": 20, "u": 22, "v": 47, "w": 17, "x": 45,
    "y": 21, "z": 44,
}
DIGITS = {"1": 2, "2": 3, "3": 4, "4": 5, "5": 6,
          "6": 7, "7": 8, "8": 9, "9": 10, "0": 11}
PUNCTUATION = {
    ";": (39, False), ":": (39, True),
    "'": (40, False), '"': (40, True),
    ",": (51, False), "<": (51, True),
    ".": (52, False), ">": (52, True),
    "/": (53, False), "?": (53, True),
    "-": (12, False), "_": (12, True),
    "=": (13, False), "+": (13, True),
    " ": (57, False),
}
SHIFT = 42


def key_event(bus, path, code, pressed):
    bus.call_sync(
        BUS_NAME,
        path,
        SESSION_INTERFACE,
        "NotifyKeyboardKeycode",
        GLib.Variant("(ub)", (code, pressed)),
        GLib.VariantType.new("()"),
        Gio.DBusCallFlags.NONE,
        5000,
        None,
    )


def send_text(bus, path, text, delay_seconds):
    for character in text:
        shifted = character.isalpha() and character.isupper()
        code = LETTERS.get(character.lower()) if character.isalpha() else DIGITS.get(character)
        if code is None and character in PUNCTUATION:
            code, shifted = PUNCTUATION[character]
        if code is None:
            raise ValueError("test input contains a character not supported by the US keymap")
        if shifted:
            key_event(bus, path, SHIFT, True)
        try:
            key_event(bus, path, code, True)
            time.sleep(delay_seconds)
        finally:
            # A failed reply can leave it uncertain whether Mutter accepted
            # the press, so always attempt the matching release.
            try:
                key_event(bus, path, code, False)
            finally:
                if shifted:
                    key_event(bus, path, SHIFT, False)
        time.sleep(delay_seconds)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--delay-ms", type=int, default=60)
    parser.add_argument("--text", required=True)
    args = parser.parse_args()
    if args.delay_ms < 0 or args.delay_ms > 1000:
        parser.error("--delay-ms must be between 0 and 1000")
    if not args.text or not args.text.isascii():
        parser.error("--text must contain non-empty ASCII acceptance-test input")

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
