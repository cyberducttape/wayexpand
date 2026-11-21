#!/usr/bin/env python3
"""Create an empty, explicitly unverified GNOME physical-session report."""

import argparse
import datetime as dt
import json
import os
import shlex
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
PLAN_PATH = ROOT / "tests/certification/gnome-ibus-matrix.json"


def shell_version():
    try:
        result = subprocess.run(
            ["gnome-shell", "--version"],
            check=True,
            capture_output=True,
            text=True,
            timeout=5,
        )
    except (FileNotFoundError, subprocess.CalledProcessError, subprocess.TimeoutExpired):
        return ""
    return result.stdout.strip() or result.stderr.strip()


def main():
    parser = argparse.ArgumentParser(
        description="Create a blank GNOME real-desktop acceptance report template."
    )
    parser.add_argument("--platform", required=True, help="platform ID from the GNOME matrix")
    parser.add_argument("--output", required=True, type=Path, help="new JSON report path")
    parser.add_argument("--package-identity", required=True, help="package name/version or archive digest")
    parser.add_argument("--physical-keyboard", required=True, help="keyboard model or asset identifier")
    parser.add_argument("--reviewed-by", required=True, help="reviewer name or team")
    parser.add_argument(
        "--wayexpand-commit",
        default="",
        help="tested source commit (defaults to this checkout's HEAD)",
    )
    parser.add_argument(
        "--gnome-shell-version",
        default="",
        help="exact GNOME Shell version (auto-detected when possible)",
    )
    parser.add_argument(
        "--confirm-physical-session",
        action="store_true",
        help="attest that this is a physical Wayland desktop session with a physical keyboard",
    )
    args = parser.parse_args()

    if not args.confirm_physical_session:
        parser.error("report creation requires --confirm-physical-session")
    if os.environ.get("XDG_SESSION_TYPE", "").lower() != "wayland" or not os.environ.get(
        "WAYLAND_DISPLAY"
    ):
        parser.error("run this command inside the physical Wayland session being documented")
    for field, value in (
        ("--package-identity", args.package_identity),
        ("--physical-keyboard", args.physical_keyboard),
        ("--reviewed-by", args.reviewed_by),
    ):
        if not value.strip():
            parser.error(f"{field} must not be blank")

    with PLAN_PATH.open(encoding="utf-8") as stream:
        plan = json.load(stream)
    platform = next((item for item in plan["platforms"] if item["id"] == args.platform), None)
    if platform is None:
        parser.error(f"unknown platform {args.platform!r}; choose from the matrix platform IDs")

    commit = args.wayexpand_commit.strip()
    if not commit:
        try:
            commit = subprocess.run(
                ["git", "-C", str(ROOT), "rev-parse", "HEAD"],
                check=True,
                capture_output=True,
                text=True,
                timeout=5,
            ).stdout.strip()
        except (FileNotFoundError, subprocess.CalledProcessError, subprocess.TimeoutExpired):
            parser.error("cannot determine WayExpand commit; pass --wayexpand-commit")

    version = args.gnome_shell_version.strip() or shell_version()
    if not version:
        parser.error("cannot detect GNOME Shell version; pass --gnome-shell-version")

    applications = [
        {
            "application_class": app,
            "scenario": scenario,
            "application_id": "",
            "application_version": "",
            "input_route": "",
            "keyboard_profile": profile,
            "result": "not-tested",
            "evidence_ref": "",
            **(
                {
                    "trigger_captured": None,
                    "original_trigger_removed": None,
                    "replacement_occurrences": None,
                    "intended_field_retained_focus": None,
                }
                if scenario == "real-trigger-replacement"
                else {}
            ),
        }
        for app in plan["application_classes"]
        for scenario in plan["required_application_scenarios"]
        for profile in plan["required_keyboard_profiles"]
    ]
    report = {
        "schema": 1,
        "status": "incomplete",
        "platform_id": args.platform,
        "wayexpand_commit": commit,
        "package_identity": args.package_identity.strip(),
        "distribution_release": platform["release"],
        "gnome_shell_version": version,
        "session_kind": "physical-wayland-session",
        "physical_keyboard": args.physical_keyboard.strip(),
        "test_date": dt.date.today().isoformat(),
        "reviewed_by": args.reviewed_by.strip(),
        "application_results": applications,
        "lifecycle_results": [
            {"scenario": scenario, "result": "not-tested", "evidence_ref": ""}
            for scenario in plan["required_lifecycle_scenarios"]
        ],
        "attestation_note": (
            "Physical-session status was attested by the report creator. Environment checks "
            "cannot prove that the keyboard is physical; reviewer must confirm evidence."
        ),
    }

    output = args.output.expanduser().resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    try:
        descriptor = os.open(output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    except FileExistsError:
        parser.error(f"refusing to overwrite existing report: {output}")
    with os.fdopen(descriptor, "w", encoding="utf-8") as stream:
        json.dump(report, stream, indent=2)
        stream.write("\n")

    print(f"Created incomplete template: {output}")
    print(f"Cells to record: {len(applications)} application/scenario/layout results; "
          f"{len(report['lifecycle_results'])} lifecycle results")
    print("Fill every result and evidence_ref, then validate with:")
    print("  " + shlex.join([sys.executable, str(ROOT / "scripts/check-gnome-acceptance-matrix.py"), "--report", str(output)]))
    print("This template is not test evidence or a certification result.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
