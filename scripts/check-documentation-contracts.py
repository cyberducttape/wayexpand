#!/usr/bin/env python3
"""Check claims that must stay synchronized with the current source tree."""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def active_documents() -> list[Path]:
    files = [*ROOT.glob("*.md"), *(ROOT / "docs").glob("*.md")]
    return [path for path in files if path.is_file()]


def main() -> int:
    errors: list[str] = []
    cargo = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    gui_source = (ROOT / "crates/gui/src/colorpack.rs").read_text(encoding="utf-8")
    broker_source = ROOT / "crates/action-broker/src/bin/wayexpand-action-broker.rs"
    gui_doc = (ROOT / "docs/GUI.md").read_text(encoding="utf-8")
    architecture = (ROOT / "docs/ACTION_BROKER_ARCHITECTURE.md").read_text(
        encoding="utf-8"
    )

    version_match = re.search(r'^version = "([^"]+)"', cargo, re.MULTILINE)
    version = version_match.group(1) if version_match else None
    if version is None:
        errors.append("Cargo.toml has no workspace/package version")
    elif f"**Version:** {version}" not in architecture:
        errors.append("Action Broker architecture version is out of sync with Cargo.toml")

    name_method = re.search(
        r"pub fn name\(&self\).*?\n    }\n\n    pub fn description",
        gui_source,
        re.DOTALL,
    )
    names = re.findall(r'ColorPack::\w+ => "([^"]+)"', name_method.group(0)) if name_method else []
    for name in names:
        if name not in gui_doc:
            errors.append(f"GUI documentation does not mention color pack {name!r}")

    image = ROOT / "docs/images/gui-empty-library.png"
    if not image.is_file():
        errors.append("GUI documentation screenshot is missing")
    else:
        for document in (ROOT / "README.md", ROOT / "docs/GUI.md", ROOT / "docs/GETTING_STARTED.md"):
            if "gui-empty-library.png" not in document.read_text(encoding="utf-8"):
                errors.append(f"{document.relative_to(ROOT)} does not reference the current GUI screenshot")

    if not broker_source.is_file():
        errors.append("Action Broker standalone binary source is missing")
    if "wayexpand-action-broker" not in architecture or "Source implementation" not in architecture:
        errors.append("Action Broker architecture does not describe the current standalone binary")

    for document in active_documents():
        text = document.read_text(encoding="utf-8")
        relative = document.relative_to(ROOT)
        if "wayexpand-action-broker.service" in text:
            errors.append(f"{relative} mentions a nonexistent managed broker service")
        if "audit_enabled" in text or "audit_path" in text:
            errors.append(f"{relative} documents removed, inert broker audit settings")

    if errors:
        print("documentation contract failures:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1
    print("documentation contracts passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
