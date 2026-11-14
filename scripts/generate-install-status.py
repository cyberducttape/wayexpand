#!/usr/bin/env python3
"""Render public package availability statements from packaging/release-status.json."""

import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
STATUS = json.loads((ROOT / "packaging/release-status.json").read_text())
START = "<!-- generated-install-status:start -->"
END = "<!-- generated-install-status:end -->"


def validate_status():
    if not STATUS.get("audited_on"):
        raise SystemExit("release-status manifest has no audit date")
    release = STATUS.get("latest_github_release", {})
    if not release.get("version") or not isinstance(release.get("published"), bool):
        raise SystemExit("release-status manifest has invalid GitHub release metadata")
    if not isinstance(release.get("binary_assets_published"), bool):
        raise SystemExit("release-status manifest must state binary asset availability")
    ppa = STATUS.get("ppa", {})
    if not ppa.get("architectures") or not ppa.get("series"):
        raise SystemExit("release-status manifest has incomplete PPA metadata")
    allowed_states = {"current", "stale", "unavailable"}
    for series, entry in ppa["series"].items():
        state = entry.get("status")
        version = entry.get("version")
        if state not in allowed_states:
            raise SystemExit(f"invalid PPA status for {series}: {state!r}")
        if (state == "unavailable") != (version is None):
            raise SystemExit(f"PPA version/status mismatch for {series}")
        if state != "unavailable" and not version:
            raise SystemExit(f"PPA version is missing for {series}")
    for field in (
        "fedora_copr_published",
        "arch_aur_submitted",
        "physical_desktop_certification",
        "upgrade_from_previous_package_tested",
    ):
        if not isinstance(STATUS.get(field), bool):
            raise SystemExit(f"release-status manifest must define {field} as a boolean")


def render_readme():
    ppa = STATUS["ppa"]
    current = ", ".join(
        f"{series} ({ppa['architectures'][0]})"
        for series, entry in ppa["series"].items()
        if entry["status"] == "current"
    )
    stale = ", ".join(
        f"{series} ({entry['version']}, stale)"
        for series, entry in ppa["series"].items()
        if entry["status"] == "stale"
    )
    unavailable = ", ".join(
        series
        for series, entry in ppa["series"].items()
        if entry["status"] == "unavailable"
    )
    release = STATUS["latest_github_release"]
    release_text = (
        f"GitHub release {release['version']} has no uploaded binary assets."
        if release["published"] and not release["binary_assets_published"]
        else f"Latest GitHub release: {release['version']}."
    )
    details = []
    if stale:
        details.append(f"PPA packages for {stale} are stale")
    if unavailable:
        details.append(f"No PPA packages are published for {unavailable}")
    availability_details = f" {'; '.join(details)}." if details else ""
    return (
        f"**Package availability (audited {STATUS['audited_on']}):** {ppa['status']} "
        f"for {current}. {release_text}{availability_details} "
        f"Fedora Copr published: {STATUS['fedora_copr_published']}. "
        "Package availability is separate from desktop certification."
    )


def render_packaging():
    release = STATUS["latest_github_release"]
    ppa = STATUS["ppa"]
    lines = [
        f"**Publication status from the shared manifest (audited {STATUS['audited_on']}):** "
        f"GitHub {release['version']} binary assets published: {release['binary_assets_published']}. "
        f"Launchpad PPA: {ppa['status']} for {ppa['architectures']}.",
        "",
        "| Distribution | PPA package status |",
        "|---|---|",
    ]
    for series, entry in ppa["series"].items():
        version = entry["version"] or "not published"
        lines.append(f"| {series} | {version} — {entry['status']} |")
    lines.extend([
        "",
        f"Fedora Copr published: {STATUS['fedora_copr_published']}; "
        f"Arch AUR submitted: {STATUS['arch_aur_submitted']}. "
        f"Package CI targets: {', '.join(STATUS['package_ci_targets'])}. "
        f"Physical desktop certification: {STATUS['physical_desktop_certification']}. "
        f"Previous-package upgrade CI passed: {STATUS['upgrade_from_previous_package_tested']}.",
    ])
    return "\n".join(lines)


def replace(path, generated):
    text = path.read_text()
    before, start, rest = text.partition(START)
    if not start:
        raise SystemExit(f"missing generated-section marker in {path}")
    _, end, after = rest.partition(END)
    if not end:
        raise SystemExit(f"missing generated-section end marker in {path}")
    updated = before + START + "\n" + generated + "\n" + END + after
    return updated


def main():
    validate_status()
    write = "--write" in __import__("sys").argv
    stale = []
    for relative, renderer in [
        ("README.md", render_readme),
        ("docs/PACKAGING.md", render_packaging),
    ]:
        path = ROOT / relative
        updated = replace(path, renderer())
        if updated != path.read_text():
            if write:
                path.write_text(updated)
            else:
                stale.append(relative)
    if stale:
        raise SystemExit("generated install status is stale: " + ", ".join(stale))
    print("generated installation status is current")


if __name__ == "__main__":
    main()
