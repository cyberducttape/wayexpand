#!/usr/bin/env python3
"""Classify changed paths for the optional, expensive PR integration jobs."""

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import PurePosixPath
from typing import Iterable


OUTPUT_NAMES = ("integration", "kwin", "sway", "ibus", "gnome_extension")


def under(path: str, *roots: str) -> bool:
    candidate = PurePosixPath(path)
    return any(candidate == PurePosixPath(root) or PurePosixPath(root) in candidate.parents for root in roots)


def is_workflow_or_classifier(path: str) -> bool:
    return under(path, ".github/workflows") or path == "scripts/classify-ci-changes.py"


def classify(paths: Iterable[str], full: bool = False) -> dict[str, bool]:
    changed = tuple(paths)
    if full or any(is_workflow_or_classifier(path) for path in changed):
        return {name: True for name in OUTPUT_NAMES}

    integration = any(
        under(
            path,
            "assets",
            "crates",
            "debian",
            "desktop",
            "examples",
            "gnome-extension",
            "launchpad",
            "packs",
            "polkit",
            "scripts",
            "systemd",
            "tests",
            "udev",
        )
        or path in {"Cargo.toml", "Cargo.lock", "PKGBUILD", "wayexpand.spec"}
        for path in changed
    )

    shared_desktop = ("crates/core", "crates/daemon", "crates/backend-selection")
    kwin = any(
        under(path, *shared_desktop, "crates/backend-evdev", "crates/backend-libei", "crates/backend-kwin-window")
        or under(path, "tests/certification")
        or path.startswith("scripts/test-kwin")
        for path in changed
    )
    sway = any(
        under(path, *shared_desktop, "crates/backend-evdev", "crates/backend-input-method", "crates/backend-wlroots")
        or under(path, "tests/certification")
        or path.startswith("scripts/test-sway")
        for path in changed
    )
    ibus = any(
        under(path, *shared_desktop, "crates/backend-ibus", "crates/backend-gnome-window", "crates/cli", "crates/gui", "gnome-extension")
        or under(path, "tests/certification")
        or path == "desktop/wayexpand-ibus.xml"
        or path.startswith("scripts/test-ibus")
        or path.startswith("scripts/test-gnome")
        for path in changed
    )
    gnome_extension = any(
        under(path, "gnome-extension", "crates/backend-gnome-window", "crates/cli", "crates/gui")
        or path.startswith("scripts/test-gnome")
        for path in changed
    )
    return {
        "integration": integration,
        "kwin": kwin,
        "sway": sway,
        "ibus": ibus,
        "gnome_extension": gnome_extension,
    }


def main() -> int:
    if len(sys.argv) != 2 or not sys.argv[1]:
        print("usage: classify-ci-changes.py BASE_SHA", file=sys.stderr)
        return 2
    base = sys.argv[1]
    try:
        result = subprocess.run(
            ["git", "diff", "--name-only", "-z", f"{base}...HEAD"],
            check=True,
            stdout=subprocess.PIPE,
        )
    except subprocess.CalledProcessError as error:
        print(f"could not compare PR changes with base {base}: {error}", file=sys.stderr)
        return 1
    paths = [part.decode("utf-8", errors="surrogateescape") for part in result.stdout.split(b"\0") if part]
    decisions = classify(paths, full=os.environ.get("WAYEXPAND_EVENT_NAME") != "pull_request")
    output_path = os.environ.get("GITHUB_OUTPUT")
    if not output_path:
        print("GITHUB_OUTPUT is not set", file=sys.stderr)
        return 1
    with open(output_path, "a", encoding="utf-8") as output:
        for name, enabled in decisions.items():
            print(f"{name}={'true' if enabled else 'false'}", file=output)
    print("CI integration scope:")
    for name, enabled in decisions.items():
        print(f"  {name}: {'run' if enabled else 'skip'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
