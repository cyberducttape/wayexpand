#!/usr/bin/env python3
"""Render reviewed GNOME physical-session reports as a public Markdown table."""

import argparse
import json
import runpy
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
PLAN_PATH = ROOT / "tests/certification/gnome-ibus-matrix.json"
CHECKER_PATH = ROOT / "scripts/check-gnome-acceptance-matrix.py"
APP_LABELS = {
    "gtk3": "GTK 3",
    "gtk4": "GTK 4",
    "qt": "Qt",
    "browser-firefox": "Firefox",
    "browser-chromium": "Chromium",
    "electron": "Electron",
    "terminal": "Terminal",
}
PROFILE_LABELS = {
    "us": "US",
    "de": "German",
    "fr": "French",
    "altgr": "AltGr",
    "multi-layout-switching": "Layout switching",
}
SCENARIO_LABELS = {
    "real-trigger-replacement": "trigger",
    "unicode-replacement": "Unicode",
    "multiple-layout-switching": "layout switching",
    "password-field-refusal": "password refusal",
    "focus-change-during-replacement": "focus change",
}


def markdown_cell(report, app, profile):
    if report is None:
        return "Not tested"
    rows = {
        (item["application_class"], item["scenario"], item["keyboard_profile"]): item
        for item in report["application_results"]
    }
    rendered = []
    for scenario, label in SCENARIO_LABELS.items():
        item = rows[(app, scenario, profile)]
        result = item["result"].replace("-", " ")
        if scenario == "real-trigger-replacement":
            captured = observation(item.get("trigger_captured"))
            removed = observation(item.get("original_trigger_removed"))
            count = item.get("replacement_occurrences")
            count = "unknown" if count is None else str(count)
            focus = observation(item.get("intended_field_retained_focus"))
            detail = (
                f"captured {captured}; removed {removed}; replacement count {count}; "
                f"focus retained {focus}"
            )
            rendered.append(f"trigger {result} ({detail}){evidence_reference(item)}")
        else:
            rendered.append(f"{label} {result}{evidence_reference(item)}")
    return "<br>".join(rendered)


def evidence_reference(item):
    reference = item.get("evidence_ref", "").replace("\n", " ")
    if not reference:
        return ""
    if Path(reference).is_absolute():
        reference = Path(reference).name
    reference = reference.replace("|", "\\|").replace("`", "'")
    return f"; evidence: `{reference}`"


def observation(value):
    if value is True:
        return "yes"
    if value is False:
        return "no"
    return "unknown"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--report",
        type=Path,
        action="append",
        default=[],
        help="reviewed per-platform report; may be supplied once per platform",
    )
    parser.add_argument(
        "--output",
        type=Path,
        help="write Markdown to this path instead of stdout",
    )
    args = parser.parse_args()

    with PLAN_PATH.open(encoding="utf-8") as stream:
        plan = json.load(stream)
    checker = runpy.run_path(str(CHECKER_PATH))
    errors = checker["validate_plan"](plan)
    reports = {}
    for report_path in args.report:
        with report_path.open(encoding="utf-8") as stream:
            report = json.load(stream)
        if report["platform_id"] in reports:
            errors.append(f"duplicate report for platform {report['platform_id']}")
        errors.extend(checker["validate_report"](plan, report))
        reports[report["platform_id"]] = report
    if errors:
        for error in errors:
            print(f"FAIL: {error}", file=sys.stderr)
        return 1

    lines = [
        "# GNOME application compatibility",
        "",
        "This table reports physical-session evidence only. `Not tested` means no",
        "reviewed report has supplied a result; headless CI does not change that",
        "status. Each reported cell lists the five required scenario results.",
        "Trigger cells",
        "also show whether capture, removal, single replacement, and focus retention",
        "were observed. See [the certification matrix](CERTIFICATION_MATRIX.md) for",
        "the test procedure, report requirements, and known limits.",
        "",
    ]
    for platform in plan["platforms"]:
        report = reports.get(platform["id"])
        if report:
            heading = (
                f"{platform['distribution']} {platform['release']} / GNOME "
                f"{report['gnome_shell_version']}"
            )
            lines.extend([f"## {heading}", "", f"WayExpand commit: `{report['wayexpand_commit']}`; report status: `{report['status']}`.", ""])
        else:
            shell_version = platform["gnome_shell"]
            if shell_version.startswith("record "):
                shell_version = "version to be recorded"
            lines.extend([
                f"## {platform['distribution']} {platform['release']} / GNOME Shell {shell_version}",
                "",
                "No reviewed physical-session report is recorded for this platform.",
                "",
            ])
        columns = [PROFILE_LABELS[profile] for profile in plan["required_keyboard_profiles"]]
        lines.append("| Application | " + " | ".join(columns) + " |")
        lines.append("| --- | " + " | ".join("---" for _ in columns) + " |")
        for app in plan["application_classes"]:
            cells = [markdown_cell(report, app, profile) for profile in plan["required_keyboard_profiles"]]
            lines.append(f"| {APP_LABELS.get(app, app)} | " + " | ".join(cells) + " |")
        lines.append("")

    output = "\n".join(lines)
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(output, encoding="utf-8")
    else:
        print(output, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
