#!/usr/bin/env python3
"""Interactively record reviewed GNOME application and lifecycle results."""

import argparse
import json
import os
import shlex
import stat
import sys
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
RESULTS = {"p": "pass", "f": "fail", "u": "unsupported"}
APPLICATION_EXERCISES = {
    "real-trigger-replacement": "In an ordinary empty text field, type the configured test trigger once. Check capture, removal, one replacement, and retained focus below.",
    "unicode-replacement": "Expand a harmless test snippet containing non-ASCII text and compare the full output with its expected value.",
    "multiple-layout-switching": "Switch among the installed US, German, and French layouts; verify the expected trigger behavior after each switch and on return.",
    "password-field-refusal": "In a password-purpose field, type only a harmless test trigger. Confirm it passes through unchanged and no replacement occurs.",
    "focus-change-during-replacement": "Using two empty scratch fields, start a harmless test replacement and change focus during output. Record where text appears and whether output stops.",
}
LIFECYCLE_EXERCISES = {
    "clean-user-install": "Install for a fresh user and confirm setup reports the actual desktop route and next action.",
    "component-registration": "Check that the installed IBus component is registered and resolves to the intended package files.",
    "gnome-source-registration-and-activation": "Register WayExpand in GNOME input sources, activate it, and confirm live source state.",
    "real-trigger-replacement": "Run one harmless trigger replacement in a real application and record its evidence reference.",
    "unicode-replacement": "Replace a trigger with a harmless Unicode test snippet and record exact output evidence.",
    "multiple-layout-switching": "Switch between the installed layouts and back to WayExpand; record each observed state.",
    "password-field-refusal": "Confirm a harmless trigger passes unchanged in a password-purpose field.",
    "switch-away-and-back": "Select another input source, type normally, then return to WayExpand and verify the source and test behavior.",
    "logout-login-persistence": "Log out and back in; verify registration, selected source, and setup state persist as expected.",
    "shell-restart-and-extension-reload": "Restart GNOME Shell or reload the extension using the supported session procedure; verify route recovery.",
    "package-upgrade-preserves-input-sources": "Upgrade the package and verify GNOME input-source settings and component registration remain valid.",
    "uninstall-preserves-original-desktop-settings": "Uninstall WayExpand and confirm unrelated input-source settings remain unchanged.",
    "uninstall-removes-wayexpand-components": "Uninstall WayExpand and confirm its component and extension files are removed.",
}


def ask_required(label, default=""):
    while True:
        suffix = f" [{default}]" if default else ""
        value = input(f"{label}{suffix}: ").strip()
        value = value or default
        if value:
            return value
        print("This field is required.")


def ask_result():
    while True:
        value = input("Result: [p]ass, [f]ail, [u]nsupported, [s]kip, [q]uit: ").strip().lower()
        if value == "q":
            return "quit"
        if value == "s":
            return "skip"
        if value in RESULTS:
            return RESULTS[value]
        print("Choose p, f, u, s, or q.")


def ask_observation(label, numeric=False):
    choices = "0, 1, 2… or ?" if numeric else "y, n, or ?"
    while True:
        value = input(f"  {label} ({choices}): ").strip().lower()
        if value == "?":
            return None
        if numeric:
            try:
                count = int(value)
            except ValueError:
                count = -1
            if count >= 0:
                return count
        elif value in {"y", "n"}:
            return value == "y"
        print("Enter a listed value; use ? when the observation could not be made.")


def save_report(path, report):
    all_passed = all(
        item["result"] == "pass" for item in report["application_results"]
    ) and all(item["result"] == "pass" for item in report["lifecycle_results"])
    report["status"] = "passed" if all_passed else "incomplete"
    descriptor, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        os.fchmod(descriptor, 0o600)
        with os.fdopen(descriptor, "w", encoding="utf-8") as stream:
            json.dump(report, stream, indent=2)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    except BaseException:
        try:
            os.unlink(temporary)
        except FileNotFoundError:
            pass
        raise


def main():
    parser = argparse.ArgumentParser(
        description="Record real-session results in an existing GNOME acceptance report."
    )
    parser.add_argument("--report", required=True, type=Path, help="report created by the template generator")
    args = parser.parse_args()

    if os.environ.get("XDG_SESSION_TYPE", "").lower() != "wayland" or not os.environ.get(
        "WAYLAND_DISPLAY"
    ):
        parser.error("run this recorder inside the physical Wayland session being documented")
    path = args.report.expanduser().resolve()
    try:
        metadata = path.lstat()
    except OSError as error:
        parser.error(f"cannot read report {path}: {error}")
    if not stat.S_ISREG(metadata.st_mode):
        parser.error("report must be a regular file")
    with path.open(encoding="utf-8") as stream:
        report = json.load(stream)
    if report.get("session_kind") != "physical-wayland-session":
        parser.error("report must come from new-gnome-acceptance-report.py")

    skipped = set()
    print(f"Recording {report['platform_id']} · {report['gnome_shell_version']}")
    print("Run the displayed test in the named application/layout before entering its result.")
    print("Each saved cell is written immediately; q exits and the next run resumes.\n")

    while True:
        item = next(
            (
                result
                for result in report["application_results"]
                if result["result"] == "not-tested"
                and (result["application_class"], result["scenario"], result["keyboard_profile"])
                not in skipped
            ),
            None,
        )
        lifecycle = False
        if item is None:
            item = next(
                (
                    result
                    for result in report["lifecycle_results"]
                    if result["result"] == "not-tested" and result["scenario"] not in skipped
                ),
                None,
            )
            lifecycle = item is not None
        if item is None:
            if skipped:
                print(f"Skipped {len(skipped)} cells for now. Rerun to record them later:")
                print("  " + shlex.join([sys.executable, str(Path(__file__).resolve()), "--report", str(path)]))
                return 0
            print("No not-tested cells remain. Validate the report with:")
            print("  " + shlex.join([sys.executable, str(ROOT / "scripts/check-gnome-acceptance-matrix.py"), "--report", str(path)]))
            return 0

        if lifecycle:
            key = item["scenario"]
            print(f"Lifecycle: {key}")
            print(f"  {LIFECYCLE_EXERCISES[key]}")
        else:
            key = (item["application_class"], item["scenario"], item["keyboard_profile"])
            print(f"Application: {key[0]} · scenario: {key[1]} · keyboard: {key[2]}")
            if key[2] == "us":
                print("  Select the US keyboard layout in GNOME.")
            elif key[2] == "de":
                print("  Select the German keyboard layout in GNOME.")
            elif key[2] == "fr":
                print("  Select the French keyboard layout in GNOME.")
            elif key[2] == "altgr":
                print("  Use a layout with AltGr and verify the trigger/replacement around AltGr input.")
            else:
                print("  Exercise switching among the configured layouts and return to WayExpand.")
            print(f"  {APPLICATION_EXERCISES[key[1]]}")
        result = ask_result()
        if result == "quit":
            print(f"Saved progress in {path}")
            return 0
        if result == "skip":
            skipped.add(key)
            continue

        item["result"] = result
        if lifecycle:
            item["evidence_ref"] = ask_required("Evidence reference (repo-relative path or reviewed artifact ID)")
        else:
            prior = next(
                (
                    row
                    for row in report["application_results"]
                    if row is not item
                    and row["application_class"] == item["application_class"]
                    and row.get("application_id")
                ),
                {},
            )
            item["application_id"] = ask_required("Exact application ID", prior.get("application_id", ""))
            item["application_version"] = ask_required(
                "Exact application version", prior.get("application_version", "")
            )
            item["input_route"] = ask_required("Input route", prior.get("input_route", ""))
            item["evidence_ref"] = ask_required("Evidence reference (repo-relative path or reviewed artifact ID)")
            if item["scenario"] == "real-trigger-replacement":
                item["trigger_captured"] = ask_observation("Trigger captured")
                item["original_trigger_removed"] = ask_observation("Original trigger removed")
                item["replacement_occurrences"] = ask_observation(
                    "Replacement occurrence count", numeric=True
                )
                item["intended_field_retained_focus"] = ask_observation(
                    "Intended field retained focus"
                )

        save_report(path, report)
        print(f"Saved {item['result']} result.\n")


if __name__ == "__main__":
    raise SystemExit(main())
