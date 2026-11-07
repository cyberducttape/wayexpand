#!/usr/bin/env python3
"""Validate GNOME application/lifecycle evidence without promoting gaps.

The plan check is suitable for CI. A report check is intended for reviewed
real-session evidence collected separately on each supported distribution.
"""

import argparse
import json
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
MATRIX = ROOT / "tests/certification/gnome-ibus-matrix.json"
APPLICATION_STATUSES = {"pass", "fail", "unsupported", "not-tested"}
REQUIRED_APPLICATIONS = {
    "gtk3",
    "gtk4",
    "qt",
    "browser-firefox",
    "browser-chromium",
    "electron",
    "terminal",
}
REQUIRED_SCENARIOS = {
    "real-trigger-replacement",
    "unicode-replacement",
    "multiple-layout-switching",
    "password-field-refusal",
    "focus-change-during-replacement",
}
REQUIRED_KEYBOARD_PROFILES = {"us", "de", "fr", "altgr", "multi-layout-switching"}
REQUIRED_LIFECYCLE = {
    "clean-user-install",
    "component-registration",
    "gnome-source-registration-and-activation",
    "real-trigger-replacement",
    "unicode-replacement",
    "multiple-layout-switching",
    "password-field-refusal",
    "switch-away-and-back",
    "logout-login-persistence",
    "shell-restart-and-extension-reload",
    "package-upgrade-preserves-input-sources",
    "uninstall-preserves-original-desktop-settings",
    "uninstall-removes-wayexpand-components",
}
REQUIRED_PLATFORMS = {
    "fedora-45-gnome51",
    "ubuntu-24.04-gnome",
    "ubuntu-26.04-gnome",
}
IDENTITY_FIELDS = {
    "wayexpand_commit",
    "package_identity",
    "distribution_release",
    "gnome_shell_version",
    "session_kind",
    "physical_keyboard",
    "test_date",
    "reviewed_by",
}


def load(path):
    with path.open(encoding="utf-8") as stream:
        return json.load(stream)


def validate_plan(plan):
    errors = []
    applications = set(plan.get("application_classes", []))
    lifecycle = set(plan.get("required_lifecycle_scenarios", []))
    scenarios = set(plan.get("required_application_scenarios", []))
    keyboard_profiles = set(plan.get("required_keyboard_profiles", []))
    platforms = {item.get("id") for item in plan.get("platforms", [])}
    if missing := REQUIRED_APPLICATIONS - applications:
        errors.append(f"matrix omits application classes: {', '.join(sorted(missing))}")
    if missing := REQUIRED_LIFECYCLE - lifecycle:
        errors.append(f"matrix omits lifecycle scenarios: {', '.join(sorted(missing))}")
    if missing := REQUIRED_SCENARIOS - scenarios:
        errors.append(f"matrix omits application scenarios: {', '.join(sorted(missing))}")
    if missing := REQUIRED_KEYBOARD_PROFILES - keyboard_profiles:
        errors.append(f"matrix omits keyboard profiles: {', '.join(sorted(missing))}")
    if missing := REQUIRED_PLATFORMS - platforms:
        errors.append(f"matrix omits supported platforms: {', '.join(sorted(missing))}")
    if plan.get("initial_status") != "not-tested":
        errors.append("coverage plan must not claim test results")
    return errors


def validate_report(plan, report):
    errors = []
    platform_id = report.get("platform_id")
    platforms = {item["id"]: item for item in plan["platforms"]}
    platform = platforms.get(platform_id)
    if platform is None:
        return [f"unknown platform_id: {platform_id!r}"]
    for field in IDENTITY_FIELDS:
        if not isinstance(report.get(field), str) or not report[field].strip():
            errors.append(f"missing evidence identity: {field}")
    if report.get("distribution_release") != platform["release"]:
        errors.append("reported distribution release does not match the matrix")
    if platform_id == "fedora-45-gnome51" and not report.get("gnome_shell_version", "").startswith("51."):
        errors.append("Fedora 45 report must identify GNOME Shell 51.x")
    if report.get("session_kind") != "physical-wayland-session":
        errors.append("evidence must come from a real Wayland session with a physical keyboard")

    expected = {
        (app, scenario, profile)
        for app in plan["application_classes"]
        for scenario in plan["required_application_scenarios"]
        for profile in plan["required_keyboard_profiles"]
    }
    observed = set()
    for item in report.get("application_results", []):
        key = (item.get("application_class"), item.get("scenario"), item.get("keyboard_profile"))
        if key in observed:
            errors.append(f"duplicate application result: {key}")
        observed.add(key)
        if key not in expected:
            errors.append(f"unexpected application result: {key}")
        if item.get("result") not in APPLICATION_STATUSES:
            errors.append(f"invalid result for {key}: {item.get('result')!r}")
        for field in ("application_id", "application_version", "input_route", "evidence_ref"):
            value = item.get(field)
            if not value:
                errors.append(f"{key} lacks {field}")
    if missing := expected - observed:
        errors.append(f"missing application cells: {len(missing)}")

    lifecycle_seen = set()
    for item in report.get("lifecycle_results", []):
        scenario = item.get("scenario")
        if scenario in lifecycle_seen:
            errors.append(f"duplicate lifecycle result: {scenario}")
        lifecycle_seen.add(scenario)
        if scenario not in REQUIRED_LIFECYCLE:
            errors.append(f"unexpected lifecycle scenario: {scenario}")
        if item.get("result") not in APPLICATION_STATUSES:
            errors.append(f"invalid lifecycle result for {scenario}: {item.get('result')!r}")
        if not item.get("evidence_ref"):
            errors.append(f"{scenario} lacks an evidence_ref")
    if missing := REQUIRED_LIFECYCLE - lifecycle_seen:
        errors.append(f"missing lifecycle cells: {', '.join(sorted(missing))}")

    if not errors:
        every_passed = all(item["result"] == "pass" for item in report["application_results"])
        every_passed &= all(item["result"] == "pass" for item in report["lifecycle_results"])
        declared = report.get("status")
        expected_status = "passed" if every_passed else "incomplete"
        if declared != expected_status:
            errors.append(f"status must be {expected_status!r} for these results")
    return errors


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--plan", type=Path, default=MATRIX)
    parser.add_argument("--report", type=Path, help="reviewed per-platform evidence JSON")
    args = parser.parse_args()
    plan = load(args.plan)
    errors = validate_plan(plan)
    if not errors and args.report:
        errors = validate_report(plan, load(args.report))
    if errors:
        for error in errors:
            print(f"FAIL: {error}", file=sys.stderr)
        return 1
    if args.report:
        print(f"PASS: reviewed GNOME evidence report is complete for {load(args.report)['platform_id']}")
    else:
        print("PASS: GNOME acceptance plan covers Fedora 45/GNOME 51 and Ubuntu 24.04/26.04")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
