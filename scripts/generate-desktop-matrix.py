#!/usr/bin/env python3
"""Generate the human desktop support tables from the certification matrix."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "tests/certification/compositor-matrix.json"
DOCUMENTS = (ROOT / "docs/SUPPORT_MATRIX.md", ROOT / "docs/CERTIFICATION_MATRIX.md")
START = "<!-- generated:desktop-certification-matrix:start -->"
END = "<!-- generated:desktop-certification-matrix:end -->"
VALID_CERTIFICATION = {"not-certified", "certified"}


def render(matrix: dict[str, object]) -> str:
    targets = matrix.get("targets")
    if not isinstance(targets, list) or not targets:
        raise ValueError("certification matrix must contain target entries")
    rows = [
        "| Target | Desktop/session | Declared test paths | Window tracking | App filters | E2E certification |",
        "| --- | --- | --- | --- | --- | --- |",
    ]
    seen: set[str] = set()
    for target in targets:
        if not isinstance(target, dict):
            raise ValueError("certification target must be an object")
        target_id = target.get("id")
        display = target.get("display")
        paths = target.get("input_paths")
        tracker = target.get("window_tracker")
        app_filter = target.get("application_filter")
        certification = target.get("certification_status")
        evidence_path = target.get("certification_evidence")
        if not isinstance(target_id, str) or not target_id or target_id in seen:
            raise ValueError("certification target IDs must be unique non-empty strings")
        seen.add(target_id)
        if not isinstance(display, str) or not display:
            raise ValueError(f"{target_id}: display must be a non-empty string")
        if not isinstance(paths, list) or not paths or not all(
            isinstance(path, str) and path for path in paths
        ):
            raise ValueError(f"{target_id}: input_paths must be a non-empty string list")
        if not isinstance(tracker, str) or not tracker:
            raise ValueError(f"{target_id}: window_tracker must be a non-empty string")
        if app_filter not in {"supported", "unavailable"}:
            raise ValueError(f"{target_id}: invalid application_filter status")
        if certification not in VALID_CERTIFICATION:
            raise ValueError(f"{target_id}: invalid certification_status")
        if certification == "certified":
            if not isinstance(evidence_path, str) or not evidence_path:
                raise ValueError(f"{target_id}: certified status requires certification_evidence")
            evidence_file = (ROOT / evidence_path).resolve()
            if not evidence_file.is_relative_to(ROOT.resolve()) or not evidence_file.is_file():
                raise ValueError(f"{target_id}: certification evidence must be a checked-in file")
            evidence = json.loads(evidence_file.read_text(encoding="utf-8"))
            if (
                evidence.get("schema") != 2
                or evidence.get("certified") is not True
                or evidence.get("status") != "certified"
                or evidence.get("compositor") != target_id
                or evidence.get("backend") not in paths
            ):
                raise ValueError(f"{target_id}: evidence does not certify a declared backend")
        elif evidence_path is not None:
            raise ValueError(f"{target_id}: not-certified targets must not claim an evidence artifact")
        app_label = "Available in declared path" if app_filter == "supported" else "Unavailable"
        certification_label = "Certified" if certification == "certified" else "Not certified"
        rows.append(
            f"| `{target_id}` | {display} | {', '.join(paths)} | {tracker} | "
            f"{app_label} | **{certification_label}** |"
        )
    return "\n".join([START, *rows, END])


def update(document: Path, expected: str, write: bool) -> bool:
    text = document.read_text(encoding="utf-8")
    if text.count(START) != 1 or text.count(END) != 1:
        raise ValueError(f"{document.relative_to(ROOT)} must have exactly one generated-block marker pair")
    before, remainder = text.split(START, 1)
    _, after = remainder.split(END, 1)
    actual = START + remainder.split(END, 1)[0] + END
    if actual == expected:
        return True
    if write:
        document.write_text(before + expected + after, encoding="utf-8")
        print(f"generated {document.relative_to(ROOT)}")
    else:
        print(f"{document.relative_to(ROOT)} is stale; run scripts/generate-desktop-matrix.py --write", file=sys.stderr)
    return False


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--write", action="store_true", help="rewrite generated documentation blocks")
    mode.add_argument("--check", action="store_true", help="check generated blocks without writing (default)")
    arguments = parser.parse_args()
    try:
        matrix = json.loads(SOURCE.read_text(encoding="utf-8"))
        if not isinstance(matrix, dict):
            raise ValueError("certification matrix root must be an object")
        expected = render(matrix)
        results = [update(document, expected, arguments.write) for document in DOCUMENTS]
        current = all(results)
    except (OSError, json.JSONDecodeError, ValueError) as error:
        print(f"desktop matrix generation failed: {error}", file=sys.stderr)
        return 1
    if not arguments.write and current:
        print("desktop support documentation is synchronized")
    return 0 if current or arguments.write else 1


if __name__ == "__main__":
    raise SystemExit(main())
