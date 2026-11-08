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


def find_acceptance_page(targets, expected_url):
    """Ignore Chromium startup tabs and select only the isolated test page."""
    for target in targets:
        if (
            target.get("type") == "page"
            and target.get("url") == expected_url
            and target.get("webSocketDebuggerUrl")
        ):
            return target["webSocketDebuggerUrl"]
    return None


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--browser", required=True)
    parser.add_argument("--profile", required=True)
    parser.add_argument("--page", required=True)
    parser.add_argument("--ready-marker", required=True)
    parser.add_argument("--focus-marker", required=True)
    parser.add_argument("--armed-marker", required=True)
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
    status_file = Path(args.status_file)

    def status_snapshot():
        try:
            return json.loads(status_file.read_text(encoding="utf-8"))
        except (OSError, ValueError, TypeError):
            return {}

    # Take this before Chromium creates its input context. A snapshot taken
    # after window activation can miss the FocusIn transition if the browser
    # already reported focus while Mutter was mapping its XWayland window.
    initial_status = status_snapshot()
    initial_focus_ins = int(initial_status.get("totals", {}).get("focus_ins", 0))
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

        expected_page_url = page.resolve().as_uri()

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
            return find_acceptance_page(targets, expected_page_url)

        ws_url = wait_for(devtools_url, "Chromium's isolated acceptance page")
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
        Path(args.ready_marker).touch(mode=0o600)
        wait_for(lambda: Path(args.focus_marker).exists(), "Mutter-confirmed browser focus")

        def browser_context_focused():
            status = status_snapshot()
            fields = status.get("recent_fields", [])
            latest_field = fields[-1] if fields else {}
            focus_ins = int(status.get("totals", {}).get("focus_ins", 0))
            return focus_ins > initial_focus_ins and latest_field.get("focused") is True

        try:
            wait_for(
                browser_context_focused,
                "a new focused Chromium IBus context",
            )
        except SystemExit as error:
            status = status_snapshot()
            totals = status.get("totals", {})
            fields = status.get("recent_fields", [])
            latest_field = fields[-1] if fields else {}
            fail(
                f"{error}; "
                f"focus_ins_delta={int(totals.get('focus_ins', 0)) - initial_focus_ins}, "
                f"keys={totals.get('keys', 0)}, "
                f"latest_field_focused={latest_field.get('focused')}, "
                f"latest_field_surrounding={latest_field.get('surrounding_capability')}"
            )
        baseline = status_snapshot()
        baseline_keys = int(baseline.get("totals", {}).get("keys", 0))
        baseline_uncaptured = int(
            (baseline.get("recent_fields", [])[-1] if baseline.get("recent_fields") else {})
            .get("keys_without_capture", 0)
        )
        Path(args.armed_marker).touch(mode=0o600)

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
                fields = status.get("recent_fields", [])
                latest_field = fields[-1] if fields else {}
                uncaptured = int(latest_field.get("keys_without_capture", 0))
                if (
                    int(totals.get("keys", 0)) > baseline_keys
                    and latest_field.get("surrounding_capability") is False
                    and uncaptured - baseline_uncaptured >= len(TRIGGER)
                ):
                    print(
                        "pass Chromium input reached WayExpand; without IBus "
                        "surrounding-text support, all trigger keys passed through unchanged"
                    )
                    return
            time.sleep(0.05)
        final_status = status_snapshot()
        final_totals = final_status.get("totals", {})
        final_fields = final_status.get("recent_fields", [])
        latest_field = final_fields[-1] if final_fields else {}
        final_uncaptured = int(latest_field.get("keys_without_capture", 0))
        value_state = (
            "replacement"
            if actual == EXPECTED
            else "trigger"
            if actual == TRIGGER
            else "empty"
            if not actual
            else "other"
        )
        fail(
            "Chromium did not insert the expected text or pass the trigger "
            "through unchanged without surrounding-text support; "
            f"value_state={value_state}, "
            "test_value_codepoints="
            f"{','.join(f'U+{ord(character):04X}' for character in actual) or 'empty'}, "
            f"focus_ins_delta={int(final_totals.get('focus_ins', 0)) - initial_focus_ins}, "
            f"keys_delta={int(final_totals.get('keys', 0)) - baseline_keys}, "
            f"latest_field_focused={latest_field.get('focused')}, "
            f"latest_field_keys={latest_field.get('keys', 0)}, "
            f"latest_field_surrounding={latest_field.get('surrounding_capability')}, "
            "uncaptured_keys_delta="
            f"{final_uncaptured - baseline_uncaptured}"
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
