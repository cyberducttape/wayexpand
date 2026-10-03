#!/usr/bin/env python3
"""Check claims that must stay synchronized with the current source tree."""

from __future__ import annotations

import json
import re
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def active_documents() -> list[Path]:
    files = [*ROOT.glob("*.md"), *(ROOT / "docs").glob("*.md")]
    return [path for path in files if path.is_file()]


def check_executable_toml(errors: list[str]) -> None:
    """Parse and validate TOML blocks explicitly marked as executable."""
    marker = re.compile(
        r"<!--\s*executable-toml:\s*config\s*-->\s*```toml\n(.*?)```",
        re.DOTALL,
    )
    cli = ROOT / "target/debug/wayexpand"
    for document in active_documents():
        text = document.read_text(encoding="utf-8")
        for block_number, source in enumerate(marker.findall(text), start=1):
            relative = document.relative_to(ROOT)
            try:
                tomllib.loads(source)
            except tomllib.TOMLDecodeError as error:
                errors.append(f"{relative} executable TOML block {block_number} is invalid: {error}")
                continue
            with tempfile.NamedTemporaryFile(
                mode="w", suffix=".toml", prefix="wayexpand-doc-", delete=False
            ) as config:
                config.write(source)
                config_path = Path(config.name)
            try:
                result = subprocess.run(
                    [str(cli), "validate", "--json", str(config_path)],
                    cwd=ROOT,
                    capture_output=True,
                    text=True,
                    check=False,
                )
            finally:
                config_path.unlink(missing_ok=True)
            if result.returncode:
                detail = (result.stderr or result.stdout).strip().splitlines()[-1]
                errors.append(
                    f"{relative} executable TOML block {block_number} failed Config validation: {detail}"
                )


def main() -> int:
    errors: list[str] = []
    check_executable_toml(errors)
    cargo = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    gui_source = (ROOT / "crates/gui/src/colorpack.rs").read_text(encoding="utf-8")
    broker_source = ROOT / "crates/action-broker/src/bin/wayexpand-action-broker.rs"
    broker_manifest = (ROOT / "crates/action-broker/Cargo.toml").read_text(encoding="utf-8")
    daemon_manifest = (ROOT / "crates/daemon/Cargo.toml").read_text(encoding="utf-8")
    core_manifest = (ROOT / "crates/core/Cargo.toml").read_text(encoding="utf-8")
    gui_doc = (ROOT / "docs/GUI.md").read_text(encoding="utf-8")
    architecture = (ROOT / "docs/ACTION_BROKER_ARCHITECTURE.md").read_text(
        encoding="utf-8"
    )
    tradeoffs = (ROOT / "docs/CAPTURE_BACKEND_TRADEOFFS.md").read_text(
        encoding="utf-8"
    )
    compositor_matrix = json.loads(
        (ROOT / "tests/certification/compositor-matrix.json").read_text(
            encoding="utf-8"
        )
    )
    certification_doc = (ROOT / "docs/CERTIFICATION_MATRIX.md").read_text(
        encoding="utf-8"
    )
    route_catalog = json.loads(
        (ROOT / "crates/backend-selection/routes.json").read_text(encoding="utf-8")
    )
    recommended_description = route_catalog.get("recommended_description")
    if not isinstance(recommended_description, str) or not recommended_description:
        errors.append("backend route catalog is missing recommended_description")
    elif recommended_description not in (ROOT / "README.md").read_text(encoding="utf-8"):
        errors.append("README Recommended-mode safety description is out of sync with routes.json")

    generated_matrix_check = subprocess.run(
        [sys.executable, str(ROOT / "scripts/generate-desktop-matrix.py")],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    if generated_matrix_check.returncode:
        errors.append(
            "desktop support documentation is out of sync with the certification source: "
            + generated_matrix_check.stderr.strip()
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

    image = ROOT / "docs/archive/images/gui-empty-library-pre-settings-consolidation.png"
    if not image.is_file():
        errors.append("archived GUI screenshot is missing")
    else:
        gui_doc = (ROOT / "docs/GUI.md").read_text(encoding="utf-8")
        if "Historical GUI screenshot" not in gui_doc or image.relative_to(ROOT / "docs").as_posix() not in gui_doc:
            errors.append("GUI documentation must label the archived screenshot as historical")
        for document in (ROOT / "README.md", ROOT / "docs/GETTING_STARTED.md"):
            if "gui-empty-library" in document.read_text(encoding="utf-8"):
                errors.append(f"{document.relative_to(ROOT)} presents the historical screenshot as current")

    # Window-tracker availability is security-sensitive: app_filter must fail
    # closed where there is no shipped tracker. Keep the certification prose
    # consistent with the machine-readable matrix used by certify.
    quick_reference = certification_doc.split("## Quick Reference", 1)[-1].split(
        "## Detailed Certification Results", 1
    )[0]
    for target in compositor_matrix["targets"]:
        desktop = target["display"].split("/", 1)[0].strip().split()[0]
        rows = [line for line in quick_reference.splitlines() if f"`{target['id']}`" in line]
        if len(rows) != 1:
            errors.append(
                f"certification quick reference must contain exactly one generated row for {target['id']}"
            )
        if target["application_filter"] == "unavailable":
            if not rows or any("| Unavailable |" not in row for row in rows):
                errors.append(
                    f"certification matrix must mark {desktop} application filtering unavailable"
                )
            if any("wlr-foreign-toplevel" in row for row in rows):
                errors.append(
                    f"certification quick reference claims an unshipped tracker for {desktop}"
                )
            section = certification_doc.split(f"### ⚠️ {desktop}", 1)
            section_text = section[1].split("\n### ", 1)[0] if len(section) == 2 else ""
            if "app_filter" not in section_text or "fail closed" not in section_text.lower():
                errors.append(
                    f"certification details must explain fail-closed app_filter behavior for {desktop}"
                )
        if "evdev+wlroots" in target["input_paths"] and any(
            "evdev+wlroots" not in row for row in rows
        ):
            errors.append(
                f"certification backend columns do not match the declared wlroots route for {desktop}"
            )

    if not broker_source.is_file():
        errors.append("Action Broker standalone binary source is missing")
    if 'name = "wayexpand-action-broker"' not in broker_manifest:
        errors.append("Action Broker standalone binary is missing from its package manifest")
    if 'required-features = ["experimental-action-broker"]' in broker_manifest:
        errors.append("Action Broker still uses the removed experimental feature gate")
    if "action-broker =" in daemon_manifest:
        errors.append("daemon should consume broker routing through the core crate")
    if "action-broker =" not in core_manifest:
        errors.append("core crate is missing the Action Broker runtime dependency")
    if "wayexpand-action-broker" not in architecture or "normal workspace builds" not in architecture:
        errors.append("Action Broker architecture does not describe the current integrated binary")

    # Keep the deployment-facing capture guide aligned with the authoritative
    # composition boundary. A broad compatibility claim here would contradict
    # SUPPORT_MATRIX.md and could cause an operator to deploy an unsupported
    # CJK/Fcitx/IBus workflow.
    if "Works universally across Wayland" in tradeoffs:
        errors.append("capture trade-off guide still claims universal Wayland support")
    if "Active external preedit" not in tradeoffs or "Not supported" not in tradeoffs:
        errors.append("capture trade-off guide must state the active external preedit limitation")
    if "Committed Unicode" not in tradeoffs:
        errors.append("capture trade-off guide must distinguish committed Unicode from active IME input")

    for document in active_documents():
        text = document.read_text(encoding="utf-8")
        relative = document.relative_to(ROOT)
        if "audit_enabled" in text:
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
