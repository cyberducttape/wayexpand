#!/usr/bin/env python3
"""Fail closed unless production-required compositor routes have valid evidence."""

from __future__ import annotations

import importlib.util
import json
import sys
from pathlib import Path
from typing import Any


def load_evidence_validator(project_root: Path):
    sys.dont_write_bytecode = True
    module_path = project_root / "scripts/generate-desktop-matrix.py"
    spec = importlib.util.spec_from_file_location("desktop_matrix", module_path)
    if spec is None or spec.loader is None:
        raise RuntimeError("could not load the certification evidence validator")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.is_production_evidence


def validate_matrix(
    matrix: Any,
    project_root: Path,
    route_catalog: Any | None = None,
    expected_commit: str | None = None,
) -> list[str]:
    errors: list[str] = []
    if not isinstance(matrix, dict):
        return ["certification matrix must be a JSON object"]
    targets = matrix.get("targets")
    scenarios = matrix.get("required_scenarios")
    layouts = matrix.get("required_layout_profiles")
    if not isinstance(targets, list):
        return ["certification matrix targets must be an array"]
    if not any(
        isinstance(target, dict)
        and target.get("production_certification_required") is True
        for target in targets
    ):
        return ["no compositor route is marked production-required"]
    if not isinstance(scenarios, list) or not scenarios or not all(
        isinstance(value, str) and value for value in scenarios
    ):
        return ["required_scenarios must be a non-empty string array"]
    if not isinstance(layouts, list) or not layouts or not all(
        isinstance(value, str) and value for value in layouts
    ):
        return ["required_layout_profiles must be a non-empty string array"]

    try:
        is_production_evidence = load_evidence_validator(project_root)
    except Exception as error:  # noqa: BLE001 - diagnostic CLI must fail closed
        return [str(error)]
    if route_catalog is None:
        try:
            route_catalog = json.loads(
                (project_root / "crates/backend-selection/routes.json").read_text(
                    encoding="utf-8"
                )
            )
        except (OSError, UnicodeError, json.JSONDecodeError) as error:
            return [f"could not read backend route catalog: {error}"]
    routes = route_catalog.get("routes") if isinstance(route_catalog, dict) else None
    if not isinstance(routes, list):
        return ["backend route catalog must contain a routes array"]

    for target in targets:
        if not isinstance(target, dict) or target.get("production_certification_required") is not True:
            continue
        target_id = target.get("id")
        label = target_id if isinstance(target_id, str) and target_id else "<unknown>"
        if target.get("certification_status") != "certified":
            errors.append(f"{label}: certification_status is not certified")
            continue
        if target.get("sensitive_field_observation") != "observed":
            errors.append(f"{label}: sensitive-field behavior has not been observed")
            continue
        evidence_path = target.get("certification_evidence")
        if not isinstance(evidence_path, str) or not evidence_path:
            errors.append(f"{label}: certification_evidence is missing")
            continue
        resolved_evidence = (project_root / evidence_path).resolve()
        try:
            resolved_evidence.relative_to(project_root.resolve())
        except ValueError:
            errors.append(f"{label}: evidence path must remain inside the repository")
            continue
        if not resolved_evidence.is_file():
            errors.append(f"{label}: evidence file does not exist: {evidence_path}")
            continue
        try:
            evidence = json.loads(resolved_evidence.read_text(encoding="utf-8"))
        except (OSError, UnicodeError, json.JSONDecodeError) as error:
            errors.append(f"{label}: could not read evidence JSON: {error}")
            continue
        evidence_commit = evidence.get("wayexpand_commit") if isinstance(evidence, dict) else None
        if expected_commit is not None and (
            not isinstance(evidence_commit, str)
            or not expected_commit.startswith(evidence_commit)
        ):
            errors.append(
                f"{label}: evidence commit {evidence_commit!r} does not match "
                f"the checked-out commit {expected_commit}"
            )
            continue

        required = (
            isinstance(target_id, str)
            and isinstance(target.get("input_paths"), list)
            and isinstance(target.get("certification_backend"), str)
            and isinstance(target.get("certification_session"), str)
            and isinstance(target.get("required_client_markers"), list)
        )
        route = next(
            (
                item
                for item in routes
                if isinstance(item, dict)
                and item.get("path") == target.get("certification_backend")
                and isinstance(item.get("certification_targets"), list)
                and target_id in item["certification_targets"]
            ),
            None,
        )
        support = route.get("sensitive_field_support") if route else None
        if not (
            route
            and route.get("status") == "supported"
            and route.get("atomic_replace") is True
            and route.get("output_transaction") == "atomic_protocol_batch"
            and route.get("trigger_precondition") == "surrounding_text_matches_trigger"
            and route.get("application_result") == "unobservable"
            and route.get("sensitive_fields") is True
            and isinstance(support, dict)
            and support.get("implemented") is True
            and support.get("certified") is True
        ):
            errors.append(
                f"{label}: the declared route contract does not provide certified "
                "sensitive-field handling and atomic verified replacement"
            )
            continue
        if not required or not is_production_evidence(
            evidence,
            target_id,
            target["input_paths"],
            target["certification_backend"],
            target.get("detected_desktop", ""),
            target["certification_session"],
            scenarios,
            layouts,
            target["required_client_markers"],
        ):
            errors.append(
                f"{label}: evidence is not a complete, passing production report "
                "for the declared route and required scenario matrix"
            )

    return errors


def main() -> int:
    if len(sys.argv) != 3:
        print("usage: validate-production-certification.py MATRIX.json PROJECT_ROOT", file=sys.stderr)
        return 2
    matrix_path = Path(sys.argv[1]).resolve()
    project_root = Path(sys.argv[2]).resolve()
    import subprocess

    try:
        expected_commit = subprocess.check_output(
            ["git", "-C", str(project_root), "rev-parse", "HEAD"],
            text=True,
            stderr=subprocess.DEVNULL,
        ).strip()
    except (OSError, subprocess.CalledProcessError):
        print("error: could not determine the checked-out source commit", file=sys.stderr)
        return 2
    try:
        matrix = json.loads(matrix_path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        print(f"error: could not read certification matrix {matrix_path}: {error}", file=sys.stderr)
        return 2
    errors = validate_matrix(matrix, project_root, expected_commit=expected_commit)
    if errors:
        for error in errors:
            print(f"production certification blocked: {error}", file=sys.stderr)
        return 1
    print("All production-required compositor routes have complete passing evidence.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
