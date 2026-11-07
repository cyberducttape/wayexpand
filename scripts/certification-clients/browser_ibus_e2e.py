#!/usr/bin/env python3
"""Verify WayExpand IBus replacement in a real Chromium text field.

The browser is isolated in a temporary profile and opened on a local page.
The fixed test trigger is sent by XTEST only after Mutter confirms the browser
window is focused. This exercises Chromium's input-method integration and DOM
text field, but does not certify native Wayland or physical keyboard delivery.
"""

import argparse
import json
import os
import signal
import shutil
import subprocess
import time
import urllib.error
import urllib.request
from pathlib import Path

import websocket


APP_ID = "org.wayexpand.GnomeBrowserIbus"
TITLE = f"WayExpand browser IBus acceptance {APP_ID}"
TRIGGER = ";e2e"
EXPECTED = "expanded"
TIMEOUT_SECONDS = 20


def fail(message):
    raise SystemExit(message)


def wait_for(predicate, description, timeout=TIMEOUT_SECONDS):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(0.05)
    fail(f"timed out waiting for {description}")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--browser", required=True)
    parser.add_argument("--profile", required=True)
    parser.add_argument("--page", required=True)
    parser.add_argument("--ready-marker", required=True)
    parser.add_argument("--focus-marker", required=True)
    parser.add_argument("--status-file", required=True)
    parser.add_argument("--log", required=True)
    args = parser.parse_args()

    for name in ("DISPLAY", "XAUTHORITY", "IBUS_ADDRESS"):
        if not os.environ.get(name):
            fail(f"{name} must identify GNOME's private XWayland/IBus session")
    if os.environ.get("GTK_IM_MODULE") != "ibus":
        fail("GTK_IM_MODULE must be set to ibus for the browser process")

    page = Path(args.page)
    page.write_text(
        "<!doctype html><meta charset=utf-8>"
        f"<title>{TITLE}</title>"
        "<label>WayExpand IBus acceptance "
        "<textarea id=target autofocus></textarea></label>",
        encoding="utf-8",
    )
    profile = Path(args.profile)
    profile.mkdir(mode=0o700, parents=True, exist_ok=True)
    log_file = open(args.log, "wb")
    process = subprocess.Popen(
        [
            args.browser,
            f"--user-data-dir={profile}",
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-background-networking",
            "--disable-component-update",
            "--disable-sync",
            "--disable-dev-shm-usage",
            "--ozone-platform=x11",
            "--remote-debugging-port=0",
            "--remote-allow-origins=*",
            page.as_uri(),
        ],
        stdout=log_file,
        stderr=subprocess.STDOUT,
        env=os.environ.copy(),
        start_new_session=True,
    )

    def terminate(signum, _frame):
        raise SystemExit(f"received signal {signum}")

    signal.signal(signal.SIGTERM, terminate)
    signal.signal(signal.SIGINT, terminate)
    connection = None
    try:
        active_port = profile / "DevToolsActivePort"

        def devtools_url():
            if process.poll() is not None:
                fail(f"browser exited during startup ({process.returncode}); see {args.log}")
            if not active_port.is_file():
                return None
            lines = active_port.read_text(encoding="utf-8").splitlines()
            if len(lines) < 2:
                return None
            port = int(lines[0])
            try:
                with urllib.request.urlopen(
                    f"http://127.0.0.1:{port}/json/list", timeout=1
                ) as response:
                    targets = json.load(response)
            except (OSError, urllib.error.URLError):
                return None
            for target in targets:
                if target.get("type") == "page" and target.get("webSocketDebuggerUrl"):
                    return target["webSocketDebuggerUrl"]
            return None

        ws_url = wait_for(devtools_url, "Chromium DevTools page")
        connection = websocket.create_connection(ws_url, timeout=2)
        next_id = 0

        def evaluate(expression):
            nonlocal next_id
            next_id += 1
            request_id = next_id
            connection.send(
                json.dumps(
                    {
                        "id": request_id,
                        "method": "Runtime.evaluate",
                        "params": {"expression": expression, "returnByValue": True},
                    }
                )
            )
            deadline = time.monotonic() + 3
            while time.monotonic() < deadline:
                response = json.loads(connection.recv())
                if response.get("id") != request_id:
                    continue
                if "error" in response:
                    fail(f"browser DOM inspection failed: {response['error']}")
                result = response.get("result", {}).get("result", {})
                return result.get("value")
            fail("timed out waiting for browser DOM inspection")

        wait_for(lambda: evaluate("document.readyState") == "complete", "browser page load")
        if evaluate("document.title") != TITLE:
            fail("unexpected browser title; refusing to activate an unrelated window")
        evaluate("document.querySelector('#target').focus()")
        status_file = Path(args.status_file)

        def status_snapshot():
            try:
                return json.loads(status_file.read_text(encoding="utf-8"))
            except (OSError, ValueError, TypeError):
                return {}

        baseline = status_snapshot()
        baseline_refusals = int(
            baseline.get("totals", {}).get("refusals", {}).get(
                "no_surrounding_capability", 0
            )
        )
        baseline_keys = int(baseline.get("totals", {}).get("keys", 0))
        Path(args.ready_marker).touch(mode=0o600)
        wait_for(lambda: Path(args.focus_marker).exists(), "Mutter-confirmed browser focus")

        deadline = time.monotonic() + TIMEOUT_SECONDS
        actual = ""
        while time.monotonic() < deadline:
            actual = evaluate("document.querySelector('#target').value") or ""
            if actual == EXPECTED:
                print("pass Chromium IBus input context replaced the test trigger in a textarea")
                return
            if actual == TRIGGER:
                status = status_snapshot()
                totals = status.get("totals", {})
                refusals = totals.get("refusals", {})
                fields = status.get("recent_fields", [])
                latest_field = fields[-1] if fields else {}
                if (
                    int(totals.get("keys", 0)) > baseline_keys
                    and latest_field.get("surrounding_capability") is False
                    and int(refusals.get("no_surrounding_capability", 0))
                    > baseline_refusals
                ):
                    print(
                        "pass Chromium input reached WayExpand; IBus did not advertise "
                        "surrounding-text support, the replacement was refused, and the trigger remained intact"
                    )
                    return
            time.sleep(0.05)
        fail(
            "Chromium did not insert the expected text or record a safe refusal "
            "for missing IBus surrounding-text support"
        )
    finally:
        if connection is not None:
            try:
                connection.close()
            except Exception:
                pass
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait(timeout=5)
        log_file.close()
        shutil.rmtree(profile, ignore_errors=True)


if __name__ == "__main__":
    main()
