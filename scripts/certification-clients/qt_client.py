#!/usr/bin/env python3
"""Qt 6 client for the KWin certification driver.

    qt_client.py APP_ID STATE_FILE

Shows one real Qt QPlainTextEdit and records what the toolkit delivered to
it in STATE_FILE as JSON: {"app_id", "focused", "text", "cursor"}. The
cursor is a character offset. Only run inside the private certification
session.
"""

import json
import os
import sys

from PyQt6.QtWidgets import QApplication, QPlainTextEdit


def main():
    app_id, state_file = sys.argv[1], sys.argv[2]
    application = QApplication([sys.argv[0]])
    # On Wayland the desktop file name becomes the xdg_toplevel app_id.
    application.setDesktopFileName(app_id)
    editor = QPlainTextEdit()
    editor.setWindowTitle(app_id)
    editor.resize(640, 400)

    def write(*_):
        text = editor.toPlainText()
        # Qt positions count UTF-16 units; report characters like the
        # other clients.
        prefix = text.encode("utf-16-le")[: 2 * editor.textCursor().position()]
        temporary = state_file + ".tmp"
        with open(temporary, "w", encoding="utf-8") as handle:
            json.dump(
                {
                    "app_id": app_id,
                    "focused": editor.isActiveWindow(),
                    "text": text,
                    "cursor": len(prefix.decode("utf-16-le")),
                },
                handle,
            )
        os.replace(temporary, state_file)

    editor.textChanged.connect(write)
    editor.cursorPositionChanged.connect(write)
    application.applicationStateChanged.connect(write)
    editor.show()
    editor.setFocus()
    write()
    sys.exit(application.exec())


if __name__ == "__main__":
    main()
