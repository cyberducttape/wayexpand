#!/usr/bin/env python3
"""Capture fixed acceptance input in raw terminal mode without executing it."""

import argparse
import os
import select
import sys
import termios
import time
import tty


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--expected", required=True)
    parser.add_argument("--ready-marker", required=True)
    args = parser.parse_args()
    fd = sys.stdin.fileno()
    original = termios.tcgetattr(fd)
    expected = args.expected.encode("utf-8")
    received = bytearray()
    try:
        tty.setraw(fd)
        with open(args.ready_marker, "w", encoding="ascii") as marker:
            marker.write("ready\n")
        deadline = time.monotonic() + 15
        while len(received) < len(expected):
            remaining = deadline - time.monotonic()
            if remaining <= 0 or not select.select([fd], [], [], remaining)[0]:
                print("fail terminal XIM input timed out", file=sys.stderr)
                return 1
            received.extend(os.read(fd, len(expected) - len(received)))
    finally:
        termios.tcsetattr(fd, termios.TCSADRAIN, original)
    if bytes(received) != expected:
        print(
            f"fail terminal XIM changed or lost input ({len(received)} UTF-8 bytes)",
            file=sys.stderr,
        )
        return 1
    print(f"pass terminal XIM preserved {len(expected)} UTF-8 bytes without execution")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
