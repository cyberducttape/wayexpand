#!/usr/bin/env python3
"""Validate GNOME application/lifecycle evidence without promoting gaps.

The plan check is suitable for CI. A report check is intended for reviewed
real-session evidence collected separately on each supported distribution.
"""

import argparse
import json
import re
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
REQUIRED_TRIGGER_OBSERVATIONS = {
    "trigger_captured",
    "original_trigger_removed",
    "replacement_occurrences",
    "intended_field_retained_focus",
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
    trigger_observations = set(plan.get("real_trigger_observations", []))
    platforms = {item.get("id") for item in plan.get("platforms", [])}
    if missing := REQUIRED_APPLICATIONS - applications:
        errors.append(f"matrix omits application classes: {', '.join(sorted(missing))}")
    if missing := REQUIRED_LIFECYCLE - lifecycle:
        errors.append(f"matrix omits lifecycle scenarios: {', '.join(sorted(missing))}")
    if missing := REQUIRED_SCENARIOS - scenarios:
        errors.append(f"matrix omits application scenarios: {', '.join(sorted(missing))}")
    if missing := REQUIRED_KEYBOARD_PROFILES - keyboard_profiles:
        errors.append(f"matrix omits keyboard profiles: {', '.join(sorted(missing))}")
    if missing := REQUIRED_TRIGGER_OBSERVATIONS - trigger_observations:
        errors.append(f"matrix omits real-trigger observations: {', '.join(sorted(missing))}")
    if missing := REQUIRED_PLATFORMS - platforms:
        errors.append(f"matrix omits supported platforms: {', '.join(sorted(missing))}")
    if plan.get("initial_status") != "not-tested":
        errors.append("coverage plan must not claim test results")
    return errors


def validate_report(plan, report):
    errors = []
    if not isinstance(report, dict):
        return ["report must be a JSON object"]
    platform_id = report.get("platform_id")
    platforms = {item["id"]: item for item in plan["platforms"]}
    platform = platforms.get(platform_id) if isinstance(platform_id, str) else None
    if platform is None:
        return [f"unknown platform_id: {platform_id!r}"]
    for field in IDENTITY_FIELDS:
        if not isinstance(report.get(field), str) or not report[field].strip():
            errors.append(f"missing evidence identity: {field}")
    commit = report.get("wayexpand_commit")
    if isinstance(commit, str) and not re.fullmatch(
        r"(?:[0-9a-f]{40}|[0-9a-f]{64})", commit, re.IGNORECASE
    ):
        errors.append("wayexpand_commit must be a full 40- or 64-character hexadecimal commit ID")
    if report.get("distribution_release") != platform["release"]:
        errors.append("reported distribution release does not match the matrix")
    reported_shell_version = report.get("gnome_shell_version")
    if platform_id == "fedora-45-gnome51" and (
        not isinstance(reported_shell_version, str)
        or not reported_shell_version.startswith("51.")
    ):
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
    application_results = report.get("application_results", [])
    if not isinstance(application_results, list):
        errors.append("application_results must be an array")
        application_results = []
    for index, item in enumerate(application_results):
        if not isinstance(item, dict):
            errors.append(f"application result {index} must be a JSON object")
            continue
        fields = ("application_class", "scenario", "keyboard_profile")
        if any(not isinstance(item.get(field), str) for field in fields):
            errors.append(
                f"application result {index} must identify its app, scenario, and keyboard profile as strings"
            )
            continue
        key = tuple(item[field] for field in fields)
        if key in observed:
            errors.append(f"duplicate application result: {key}")
        observed.add(key)
        if key not in expected:
            errors.append(f"unexpected application result: {key}")
        if not isinstance(item.get("result"), str) or item["result"] not in APPLICATION_STATUSES:
            errors.append(f"invalid result for {key}: {item.get('result')!r}")
        for field in ("application_id", "application_version", "input_route", "evidence_ref"):
            value = item.get(field)
            if not isinstance(value, str) or not value.strip():
                errors.append(f"{key} lacks {field}")
        if item.get("scenario") == "real-trigger-replacement":
            observations = {
                "trigger_captured": bool,
                "original_trigger_removed": bool,
                "intended_field_retained_focus": bool,
            }
            for field, expected_type in observations.items():
                value = item.get(field)
                if value is not None and type(value) is not expected_type:
                    errors.append(f"{key} has invalid {field}: expected true, false, or null")
                if field not in item:
                    errors.append(f"{key} lacks {field}")
            replacement_count = item.get("replacement_occurrences")
            if replacement_count is not None and (
                type(replacement_count) is not int or replacement_count < 0
            ):
                errors.append(
                    f"{key} has invalid replacement_occurrences: "
                    "expected a non-negative integer or null"
                )
            if "replacement_occurrences" not in item:
                errors.append(f"{key} lacks replacement_occurrences")
            if item.get("result") == "pass" and (
                item.get("trigger_captured") is not True
                or item.get("original_trigger_removed") is not True
                or replacement_count != 1
                or item.get("intended_field_retained_focus") is not True
            ):
                errors.append(
                    f"{key} cannot pass unless the trigger was captured and removed, "
                    "replacement appeared once, and focus stayed in the intended field"
                )
    if missing := expected - observed:
        errors.append(f"missing application cells: {len(missing)}")

    lifecycle_seen = set()
    lifecycle_results = report.get("lifecycle_results", [])
    if not isinstance(lifecycle_results, list):
        errors.append("lifecycle_results must be an array")
        lifecycle_results = []
    for index, item in enumerate(lifecycle_results):
        if not isinstance(item, dict):
            errors.append(f"lifecycle result {index} must be a JSON object")
            continue
        scenario = item.get("scenario")
        if not isinstance(scenario, str):
            errors.append(f"lifecycle result {index} must identify its scenario as a string")
            continue
        if scenario in lifecycle_seen:
            errors.append(f"duplicate lifecycle result: {scenario}")
        lifecycle_seen.add(scenario)
        if scenario not in REQUIRED_LIFECYCLE:
            errors.append(f"unexpected lifecycle scenario: {scenario}")
        if not isinstance(item.get("result"), str) or item["result"] not in APPLICATION_STATUSES:
            errors.append(f"invalid lifecycle result for {scenario}: {item.get('result')!r}")
        if not isinstance(item.get("evidence_ref"), str) or not item["evidence_ref"].strip():
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
        report = load(args.report)
        print(
            "PASS: GNOME evidence coverage is structurally complete for "
            f"{report['platform_id']} (report status: {report['status']})"
        )
        if report["status"] != "passed":
            print(
                "NOTE: the report contains failed, unsupported, or untested results; "
                "this is not a compatibility pass."
            )
    else:
        print("PASS: GNOME acceptance plan covers Fedora 45/GNOME 51 and Ubuntu 24.04/26.04")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
