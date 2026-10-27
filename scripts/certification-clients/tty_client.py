#!/usr/bin/env python3
"""Terminal-side recorder for the KWin certification driver.

    tty_client.py APP_ID STATE_FILE

Runs inside a terminal emulator with its terminal in raw mode, so it sees
exactly the bytes the terminal sent for each key. It applies a minimal line
editor (printable UTF-8, Backspace, Return, Left/Right/Home/End/Delete) and
records the result in STATE_FILE as JSON: {"app_id", "focused", "text",
"cursor", "bytes"}. "focused" is null: a terminal application cannot observe
window focus. "bytes" is the hex of everything received. Only run inside the
private certification session.
"""

import codecs
import json
import os
import sys
import termios
import tty

SEQUENCES = {
    b"\x1b[D": "left",
    b"\x1b[C": "right",
    b"\x1b[H": "home",
    b"\x1bOH": "home",
    b"\x1b[F": "end",
    b"\x1bOF": "end",
    b"\x1b[3~": "delete",
}


def main():
    app_id, state_file = sys.argv[1], sys.argv[2]
    descriptor = sys.stdin.fileno()
    saved = termios.tcgetattr(descriptor)
    tty.setraw(descriptor)
    text, cursor, received, pending = [], 0, bytearray(), b""
    decoder = codecs.getincrementaldecoder("utf-8")("replace")

    def write():
        temporary = state_file + ".tmp"
        with open(temporary, "w", encoding="utf-8") as handle:
            json.dump(
                {
                    "app_id": app_id,
                    "focused": None,
                    "text": "".join(text),
                    "cursor": cursor,
                    "bytes": received.hex(),
                },
                handle,
            )
        os.replace(temporary, state_file)

    try:
        write()
        while True:
            chunk = os.read(descriptor, 4096)
            if not chunk:
                break
            received.extend(chunk)
            pending += chunk
            while pending:
                if pending.startswith(b"\x1b"):
                    match = next((s for s in SEQUENCES if pending.startswith(s)), None)
                    if match is None:
                        if any(s.startswith(pending) for s in SEQUENCES):
                            break  # wait for the rest of the sequence
                        pending = pending[1:]  # unknown escape: drop ESC
                        continue
                    pending = pending[len(match):]
                    action = SEQUENCES[match]
                    if action == "left":
                        cursor = max(0, cursor - 1)
                    elif action == "right":
                        cursor = min(len(text), cursor + 1)
                    elif action == "home":
                        cursor = 0
                    elif action == "end":
                        cursor = len(text)
                    elif action == "delete" and cursor < len(text):
                        del text[cursor]
                    continue
                byte, pending = pending[:1], pending[1:]
                if byte in (b"\x7f", b"\x08"):
                    if cursor > 0:
                        cursor -= 1
                        del text[cursor]
                    continue
                if byte in (b"\r", b"\n"):
                    character = "\n"
                elif byte[0] < 0x20:
                    continue
                else:
                    character = decoder.decode(byte)
                    if not character:
                        continue
                text.insert(cursor, character)
                cursor += 1
            write()
    finally:
        termios.tcsetattr(descriptor, termios.TCSADRAIN, saved)


if __name__ == "__main__":
    main()
