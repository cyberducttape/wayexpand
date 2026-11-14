#!/usr/bin/env python3
"""Ensure every advertised GNOME Shell major has a version-pinned runtime gate."""

import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[1]
METADATA = ROOT / "gnome-extension/wayexpand-window-tracker@cyberducttape.github.io/metadata.json"
MATRIX = ROOT / "scripts/gnome-shell-runtime-matrix.json"
WORKFLOW = ROOT / ".github/workflows/ci.yml"
E2E = ROOT / "scripts/test-gnome-e2e.sh"
REQUIRED_CHECKPOINTS = {
    "gnome-ibus-source-registered",
    "ibus-engine-active",
    "gnome-ibus-source-active",
    "gnome-layout-de",
    "gnome-layout-fr",
    "extension-disabled",
    "extension-after-login",
    "gnome-ibus-source-persisted-after-login",
    "gnome-ibus-source-after-login",
}
REQUIRED_FOCUS_EVIDENCE = {
    "pass focus transition to org.wayexpand.GnomeE2eFirst",
    "pass focus transition to org.wayexpand.GnomeE2eSecond",
    "pass focus-loss transition",
}


def validate(metadata, matrix, workflow, e2e):
    errors = []
    advertised = set(metadata.get("shell-version", []))
    entries = matrix.get("shell_versions", [])
    majors = [item.get("major") for item in entries]
    if len(majors) != len(set(majors)):
        errors.append("runtime matrix contains duplicate GNOME Shell majors")
    if advertised != set(majors):
        errors.append(
            "metadata shell-version must exactly match runtime-gated majors; "
            f"advertised={sorted(advertised)}, gated={sorted(set(majors))}"
        )
    contract = set(matrix.get("required_runtime_contract", []))
    required_contract = {
        "source-registration", "engine-selection", "active-layout-tracking",
        "focus-tracking", "extension-reload", "session-persistence",
    }
    if contract != required_contract:
        errors.append("runtime matrix must retain the complete GNOME integration contract")
    if matrix.get("evidence_scope") != "headless-runtime-only":
        errors.append("headless GNOME checks must not be labeled physical certification")

    for item in entries:
        major = item.get("major")
        if item.get("runtime_test") != "scripts/test-gnome-e2e.sh":
            errors.append(f"GNOME {major} must use the integrated runtime acceptance test")
        if item.get("job") == "gnome-e2e":
            matrix_row = (
                f"- os: {item.get('runner')}\n"
                f"            shell_major: '{major}'"
            )
            if matrix_row not in workflow:
                errors.append(f"GNOME {major} runner is not pinned to its expected Shell major")
        elif item.get("job") == "gnome51-ibus-e2e":
            job_start = workflow.find("  gnome51-ibus-e2e:")
            job_end = workflow.find("\n  packaging:", job_start)
            job_text = workflow[job_start:job_end if job_end != -1 else None]
            if not job_text or item.get("runner") != "fedora:45":
                errors.append(f"GNOME {major} Fedora runtime job is absent or has the wrong runner")
            if f"WX_GNOME_EXPECTED_SHELL_MAJOR={major}" not in job_text:
                errors.append(f"GNOME {major} Fedora runtime job does not pin its actual Shell major")
        else:
            errors.append(f"GNOME {major} has no recognized version-specific CI job")

    text = e2e.read_text(encoding="utf-8")
    expected_line = next((line for line in text.splitlines() if line.startswith("expected_checks=")), "")
    checkpoints = set(expected_line.removeprefix("expected_checks='").removesuffix("'").split())
    if missing := REQUIRED_CHECKPOINTS - checkpoints:
        errors.append(f"runtime acceptance omits checkpoints: {', '.join(sorted(missing))}")
    for marker in REQUIRED_FOCUS_EVIDENCE:
        if marker not in text:
            errors.append(f"runtime acceptance omits focus checkpoint: {marker}")
    for marker in ("WX_GNOME_EXPECTED_SHELL_MAJOR", "gnome-shell-runtime-version"):
        if marker not in text:
            errors.append(f"runtime acceptance does not verify the actual Shell version ({marker})")
    return errors


def main():
    metadata = json.loads(METADATA.read_text(encoding="utf-8"))
    matrix = json.loads(MATRIX.read_text(encoding="utf-8"))
    workflow = WORKFLOW.read_text(encoding="utf-8")
    errors = validate(metadata, matrix, workflow, E2E)
    if errors:
        for error in errors:
            print(f"FAIL: {error}", file=sys.stderr)
        return 1
    print("PASS: every advertised GNOME Shell major has a version-pinned runtime integration gate")
    print("Scope: headless runtime compatibility; physical desktop certification remains separate")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
