#!/usr/bin/env python3
"""Check exact pass-through behavior in a Qt XIM text field."""

import os
import sys
import argparse
import time

from PyQt6.QtCore import QElapsedTimer, QTimer
from PyQt6.QtWidgets import QApplication, QLineEdit, QVBoxLayout, QWidget


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--expected", required=True)
    parser.add_argument("--password", action="store_true")
    args = parser.parse_args()
    if os.environ.get("QT_IM_MODULE") != "xim":
        raise SystemExit("QT_IM_MODULE must be xim for this Qt/XIM acceptance check")
    if os.environ.get("XMODIFIERS") != "@im=ibus":
        raise SystemExit("XMODIFIERS must select the IBus XIM server")
    marker = os.environ.get("QT_E2E_FOCUS_MARKER")
    if not marker:
        raise SystemExit("QT_E2E_FOCUS_MARKER must be set by the focus controller")
    ready_marker = os.environ.get("QT_E2E_READY_MARKER")
    if not ready_marker:
        raise SystemExit("QT_E2E_READY_MARKER must be set by the focus controller")

    app = QApplication(sys.argv)
    window = QWidget()
    window.setWindowTitle("WayExpand Qt acceptance org.wayexpand.GnomeQtIbus")
    layout = QVBoxLayout(window)
    field = QLineEdit()
    if args.password:
        field.setEchoMode(QLineEdit.EchoMode.Password)
    layout.addWidget(field)
    window.resize(500, 80)
    window.show()
    window.activateWindow()
    field.setFocus()

    failed = False
    elapsed = QElapsedTimer()
    ready_since = None

    def check_result():
        nonlocal failed, ready_since
        if not os.path.exists(marker):
            return True
        # A mapped Qt window is not necessarily ready to receive XIM input.
        # Require stable activation and child focus before the caller types.
        active = field.hasFocus()
        if active:
            if ready_since is None:
                ready_since = time.monotonic()
            elif time.monotonic() - ready_since >= 0.25:
                try:
                    fd = os.open(ready_marker, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
                except FileExistsError:
                    pass
                else:
                    os.write(fd, b"1")
                    os.close(fd)
        else:
            ready_since = None
        if not os.path.exists(ready_marker):
            return True
        field.setFocus()
        if field.text() == args.expected:
            app.quit()
            return False
        if elapsed.elapsed() >= 15_000:
            print(
                f"Qt XIM result mismatch: expected {args.expected!r}, got {field.text()!r}",
                file=sys.stderr,
            )
            failed = True
            app.quit()
            return False
        return True

    timer = QTimer()
    timer.timeout.connect(check_result)

    def start_check():
        if os.path.exists(marker):
            if not elapsed.isValid():
                elapsed.start()
            timer.start(20)
        else:
            QTimer.singleShot(20, start_check)

    QTimer.singleShot(20, start_check)
    app.exec()
    window.close()
    if failed:
        raise SystemExit(1)
    result = "preserved the trigger in a password field" if args.password else "passed input through unchanged"
    print(f"pass Qt XIM {result}")


if __name__ == "__main__":
    main()
