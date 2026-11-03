#!/usr/bin/env python3
"""Validate reviewed pack metadata and exact generated-output fixtures."""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path


def fail(pack: Path, detail: str) -> None:
    raise SystemExit(f"FAIL {pack}: {detail}")


def run(cli: str, args: list[str], pack: Path) -> subprocess.CompletedProcess[str]:
    result = subprocess.run([cli, *args], check=False, capture_output=True, text=True)
    if result.returncode:
        fail(pack, result.stderr.strip() or result.stdout.strip() or "command failed")
    return result


def main() -> None:
    cli = sys.argv[1] if len(sys.argv) > 1 else "target/debug/wayexpand"
    root = Path(__file__).resolve().parents[1]
    packs = root / "packs"
    checked = 0
    for pack in sorted(path for path in packs.iterdir() if (path / "wayexpand-pack.toml").is_file()):
        manifest = tomllib.loads((pack / "wayexpand-pack.toml").read_text())
        for field in ("maintainer", "compatibility", "risk", "review", "verification"):
            if not manifest.get(field):
                fail(pack.name, f"missing {field} metadata")
        compatibility = manifest["compatibility"]
        if not all(compatibility.get(field) for field in ("shells", "platforms")) or "tools" not in compatibility:
            fail(pack.name, "compatibility must list shells, platforms, and tools (which may be empty)")
        if manifest["risk"].get("level") not in {"low", "moderate", "high", "critical"}:
            fail(pack.name, "risk level must be low, moderate, high, or critical")
        cases_path = pack / manifest["verification"]["test_cases"]
        if not cases_path.is_file() or not cases_path.resolve().is_relative_to(pack.resolve()):
            fail(pack.name, "verification cases must be a regular file inside the pack")
        cases = tomllib.loads(cases_path.read_text())
        if cases.get("format_version") != 1 or not cases.get("case"):
            fail(pack.name, "test cases must declare format_version=1 and at least one case")

        run(cli, ["pack", "inspect", str(pack)], pack)
        imported = run(cli, ["pack", "import", str(pack)], pack)
        with tempfile.TemporaryDirectory(prefix="wayexpand-pack-case-") as temp:
            config = Path(temp) / "merged.toml"
            config.write_text(imported.stdout)
            config.chmod(0o600)
            for case in cases["case"]:
                trigger = case.get("trigger", "")
                output_name = case.get("expected_output", "")
                expected_path = pack / output_name
                if (
                    not trigger
                    or not output_name.startswith("examples/")
                    or not expected_path.is_file()
                    or not expected_path.resolve().is_relative_to(pack.resolve())
                ):
                    fail(pack.name, f"invalid example case {case!r}")
                preview = run(cli, ["preview", trigger, "--json", str(config)], pack)
                try:
                    result = json.loads(preview.stdout)
                except json.JSONDecodeError as error:
                    fail(pack.name, f"preview returned invalid JSON: {error}")
                expected = expected_path.read_text()
                if result.get("matched") is not True or result.get("replacement") != expected:
                    fail(
                        pack.name,
                        f"{trigger} output differs from {expected_path.relative_to(pack)}",
                    )
        checked += 1
        print(f"ok   {pack.name}: metadata and {len(cases['case'])} generated-output case(s)")
    if checked == 0:
        raise SystemExit("no reviewed packs found")
    print(f"checked {checked} reviewed packs")


if __name__ == "__main__":
    main()
