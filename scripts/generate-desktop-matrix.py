#!/usr/bin/env python3
"""Generate the human desktop support tables from the certification matrix."""

from __future__ import annotations

import argparse
from datetime import datetime
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "tests/certification/compositor-matrix.json"
ROUTE_SOURCE = ROOT / "crates/backend-selection/routes.json"
DOCUMENTS = (ROOT / "docs/SUPPORT_MATRIX.md", ROOT / "docs/CERTIFICATION_MATRIX.md")
START = "<!-- generated:desktop-certification-matrix:start -->"
END = "<!-- generated:desktop-certification-matrix:end -->"
ROUTE_START = "<!-- generated:route-contract:start -->"
ROUTE_END = "<!-- generated:route-contract:end -->"
VALID_CERTIFICATION = {"not-certified", "certified"}
VALID_SENSITIVE_OBSERVATION = {"not-observed", "observed", "unavailable"}


def is_production_evidence(
    evidence: object,
    target_id: str,
    declared_paths: list[str],
    target_backend: str,
    expected_desktop: str,
    expected_session: str,
    required_scenarios: list[str],
    required_layouts: list[str],
    required_client_markers: list[str],
) -> bool:
    if not isinstance(evidence, dict):
        return False
    scenarios = evidence.get("scenarios")
    if not isinstance(scenarios, list) or not scenarios:
        return False
    if not all(
        isinstance(scenario, dict)
        and isinstance(scenario.get("name"), str)
        and isinstance(scenario.get("layout_profile"), str)
        and isinstance(scenario.get("target_app"), str)
        and scenario.get("result") == "pass"
        for scenario in scenarios
    ):
        return False
    scenario_names = {scenario["name"] for scenario in scenarios}
    target_apps = evidence.get("target_apps")
    client_versions = evidence.get("client_versions")
    evidence_layouts = evidence.get("required_layout_profiles")
    evidence_markers = evidence.get("required_client_markers")
    if not isinstance(target_apps, list) or not target_apps or not all(
        isinstance(app, str) and app for app in target_apps
    ):
        return False
    if len(set(target_apps)) != len(target_apps):
        return False
    if not isinstance(client_versions, dict) or set(client_versions) != set(target_apps):
        return False
    if not all(
        isinstance(version, dict)
        and all(
            isinstance(version.get(field), str) and version[field]
            for field in ("application_version", "toolkit_name", "toolkit_version")
        )
        for version in client_versions.values()
    ):
        return False
    if evidence_layouts != required_layouts or evidence_markers != required_client_markers:
        return False
    identity_fields = (
        "wayexpand_version",
        "compositor_version",
        "kernel_release",
        "cli_binary_sha256",
        "keyboard_layout",
        "expected_desktop",
        "detected_desktop",
        "recorded_at_utc",
    )
    if not all(
        isinstance(evidence.get(field), str)
        and evidence[field]
        and evidence[field] != "unknown"
        for field in identity_fields
    ):
        return False
    # XDG_SESSION_TYPE=wayland is also true in headless compositor fixtures.
    # Production evidence therefore requires a distinct operator attestation
    # identifying the physical keyboard and an accountable reviewer.
    if not (
        evidence.get("session_kind") == "physical-wayland-session"
        and isinstance(evidence.get("physical_keyboard"), str)
        and evidence["physical_keyboard"].strip()
        and isinstance(evidence.get("physical_evidence_ref"), str)
        and evidence["physical_evidence_ref"].strip()
        and evidence.get("physical_input_observed") is True
        and evidence.get("physical_session_attestation_valid") is True
        and isinstance(evidence.get("reviewed_by"), str)
        and evidence["reviewed_by"].strip()
    ):
        return False
    commit = evidence.get("wayexpand_commit")
    binary_digest = evidence.get("cli_binary_sha256")
    if not isinstance(commit, str) or not re.fullmatch(r"[0-9a-f]{12,40}", commit):
        return False
    if ".dirty" in evidence["wayexpand_version"].lower():
        return False
    if not isinstance(binary_digest, str) or not re.fullmatch(r"[0-9a-f]{64}", binary_digest):
        return False
    try:
        datetime.fromisoformat(evidence["recorded_at_utc"].replace("Z", "+00:00"))
    except ValueError:
        return False
    if not all(
        any(
            re.search(
                rf"(^|[._+-]){re.escape(marker)}([0-9._+-]|$)", app, re.IGNORECASE
            )
            for app in target_apps
        )
        for marker in required_client_markers
    ):
        return False
    observed_cells = [
        (scenario["name"], scenario.get("layout_profile"), scenario.get("target_app"))
        for scenario in scenarios
    ]
    expected_cells = {
        (name, layout, app)
        for name in required_scenarios
        for layout in required_layouts
        for app in target_apps
    }
    if len(observed_cells) != len(expected_cells) or set(observed_cells) != expected_cells:
        return False
    return (
        evidence.get("schema") == 5
        and evidence.get("certified") is True
        and evidence.get("status") == "certified"
        and evidence.get("certification_scope") == "production"
        and evidence.get("production_eligible") is True
        and evidence.get("production_certification_required") is True
        and evidence.get("compositor") == target_id
        and evidence.get("backend") == target_backend == "input-method-v2"
        and evidence.get("backend") in declared_paths
        and evidence.get("expected_desktop") == expected_desktop
        and evidence.get("detected_desktop") == expected_desktop
        and evidence.get("expected_session") == expected_session
        and evidence.get("session") == expected_session
        and evidence.get("session_probe_valid") is True
        and evidence.get("backend_certification_eligible") is True
        and evidence.get("doctor_probe_valid") is True
        and evidence.get("desktop_probe_valid") is True
        and evidence.get("backend_probe_valid") is True
        and evidence.get("status_required") is True
        and evidence.get("status_probe_valid") is True
        and evidence.get("client_versions_valid") is True
        and set(required_scenarios).issubset(scenario_names)
    )


def render(matrix: dict[str, object], route_catalog: dict[str, object]) -> str:
    targets = matrix.get("targets")
    routes = route_catalog.get("routes")
    if not isinstance(targets, list) or not targets:
        raise ValueError("certification matrix must contain target entries")
    if not isinstance(routes, list) or not routes:
        raise ValueError("certification matrix must contain route entries")
    route_paths: dict[str, list[str]] = {}
    for route in routes:
        if not isinstance(route, dict) or not isinstance(route.get("path"), str):
            raise ValueError("every route contract must define a path")
        path = route["path"]
        targets_for_route = route.get("certification_targets")
        if not isinstance(targets_for_route, list) or not all(
            isinstance(target_id, str) for target_id in targets_for_route
        ):
            raise ValueError("every route contract must define string certification targets")
        for target_id in targets_for_route:
            route_paths.setdefault(target_id, []).append(path)
    rows = [
        "| Target | Desktop/session | Declared test paths | Window tracking | App filters | Sensitive fields observed | Production certification |",
        "| --- | --- | --- | --- | --- | --- | --- |",
    ]
    seen: set[str] = set()
    for target in targets:
        if not isinstance(target, dict):
            raise ValueError("certification target must be an object")
        target_id = target.get("id")
        display = target.get("display")
        detected_desktop = target.get("detected_desktop")
        certification_session = target.get("certification_session")
        paths = target.get("input_paths")
        certification_backend = target.get("certification_backend")
        production_required = target.get("production_certification_required")
        required_client_markers = target.get("required_client_markers")
        tracker = target.get("window_tracker")
        app_filter = target.get("application_filter")
        certification = target.get("certification_status")
        sensitive_observation = target.get("sensitive_field_observation")
        evidence_path = target.get("certification_evidence")
        if not isinstance(target_id, str) or not target_id or target_id in seen:
            raise ValueError("certification target IDs must be unique non-empty strings")
        seen.add(target_id)
        if not isinstance(display, str) or not display:
            raise ValueError(f"{target_id}: display must be a non-empty string")
        if not isinstance(detected_desktop, str) or not detected_desktop:
            raise ValueError(f"{target_id}: detected_desktop must be a non-empty string")
        if certification_session != "wayland":
            raise ValueError(f"{target_id}: production certification requires a Wayland session")
        if not isinstance(paths, list) or not paths or not all(
            isinstance(path, str) and path for path in paths
        ):
            raise ValueError(f"{target_id}: input_paths must be a non-empty string list")
        if not isinstance(certification_backend, str) or certification_backend not in paths:
            raise ValueError(f"{target_id}: certification_backend must name a declared input path")
        if not isinstance(production_required, bool):
            raise ValueError(f"{target_id}: production_certification_required must be boolean")
        if not isinstance(required_client_markers, list) or not required_client_markers or not all(
            isinstance(marker, str) and marker for marker in required_client_markers
        ):
            raise ValueError(f"{target_id}: required_client_markers must be a non-empty string list")
        if paths != route_paths.get(target_id):
            raise ValueError(
                f"{target_id}: input_paths must match route certification_targets"
            )
        if not isinstance(tracker, str) or not tracker:
            raise ValueError(f"{target_id}: window_tracker must be a non-empty string")
        # "implemented": the tracker ships and is tested headlessly, but no
        # real-session evidence exists yet.
        if app_filter not in {"supported", "implemented", "unavailable"}:
            raise ValueError(f"{target_id}: invalid application_filter status")
        if certification not in VALID_CERTIFICATION:
            raise ValueError(f"{target_id}: invalid certification_status")
        if sensitive_observation not in VALID_SENSITIVE_OBSERVATION:
            raise ValueError(f"{target_id}: invalid sensitive_field_observation")
        if certification == "certified":
            if not production_required:
                raise ValueError(
                    f"{target_id}: certified status requires production certification to be required"
                )
            if sensitive_observation != "observed":
                raise ValueError(
                    f"{target_id}: certified status requires observed sensitive-field handling"
                )
            if not isinstance(evidence_path, str) or not evidence_path:
                raise ValueError(f"{target_id}: certified status requires certification_evidence")
            evidence_file = (ROOT / evidence_path).resolve()
            if not evidence_file.is_relative_to(ROOT.resolve()) or not evidence_file.is_file():
                raise ValueError(f"{target_id}: certification evidence must be a checked-in file")
            evidence = json.loads(evidence_file.read_text(encoding="utf-8"))
            required_scenarios = matrix.get("required_scenarios")
            required_layouts = matrix.get("required_layout_profiles")
            if not isinstance(required_scenarios, list) or not required_scenarios or not all(
                isinstance(scenario, str) and scenario
                for scenario in required_scenarios
            ):
                raise ValueError("required_scenarios must be a non-empty string list")
            if not isinstance(required_layouts, list) or not required_layouts or not all(
                isinstance(layout, str) and layout for layout in required_layouts
            ):
                raise ValueError("required_layout_profiles must be a non-empty string list")
            if not is_production_evidence(
                evidence,
                target_id,
                paths,
                certification_backend,
                detected_desktop,
                certification_session,
                required_scenarios,
                required_layouts,
                required_client_markers,
            ):
                raise ValueError(f"{target_id}: evidence does not certify a declared backend")
        elif evidence_path is not None:
            raise ValueError(f"{target_id}: not-certified targets must not claim an evidence artifact")
        app_label = {
            "supported": "Available in declared path",
            "implemented": "Implemented; awaiting real-session evidence",
            "unavailable": "Unavailable",
        }[app_filter]
        observation_label = {
            "not-observed": "Not observed",
            "observed": "Observed",
            "unavailable": "Unavailable",
        }[sensitive_observation]
        certification_label = "Certified" if certification == "certified" else "Not certified"
        rows.append(
            f"| `{target_id}` | {display} ({certification_session}) | {', '.join(paths)} | {tracker} | "
            f"{app_label} | {observation_label} | **{certification_label}** |"
        )
    return "\n".join([START, *rows, END])


def render_routes(matrix: dict[str, object], route_catalog: dict[str, object]) -> str:
    routes = route_catalog.get("routes")
    targets = matrix.get("targets")
    if not isinstance(routes, list) or not routes:
        raise ValueError("certification matrix must contain route entries")
    if not isinstance(targets, list):
        raise ValueError("certification matrix targets must be an array")
    target_ids = {target.get("id") for target in targets if isinstance(target, dict)}
    rows = [
        "| Route | Capture | Injection | Trigger precondition | Output transaction | Application result | Focus tracking | Sensitive fields (implementation) | Protocol signal | Compositor observation | Certification | App identity | Status |",
        "| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |",
    ]
    seen: set[str] = set()
    for route in routes:
        if not isinstance(route, dict):
            raise ValueError("route contract must be an object")
        required = (
            "id", "label", "path", "capture", "injection", "sensitive_fields", "sensitive_field_support",
            "trigger_precondition", "output_transaction", "application_result", "atomic_replace",
            "app_identity", "focus_tracking", "status",
            "setup_backend", "setup_detail", "certification_targets",
        )
        if any(key not in route for key in required):
            raise ValueError("route contract is missing a required field")
        route_id = route["id"]
        if not isinstance(route_id, str) or not route_id or route_id in seen:
            raise ValueError("route IDs must be unique non-empty strings")
        for key in ("label", "path", "capture", "injection", "app_identity", "status", "setup_backend", "setup_detail"):
            if not isinstance(route[key], str) or not route[key]:
                raise ValueError(f"{route_id}: {key} must be a non-empty string")
        for key in ("sensitive_fields", "atomic_replace", "focus_tracking"):
            if not isinstance(route[key], bool):
                raise ValueError(f"{route_id}: {key} must be boolean")
        if route["trigger_precondition"] not in {
            "surrounding_text_matches_trigger",
            "focus_checked_with_compatibility_consent",
            "none",
        }:
            raise ValueError(f"{route_id}: invalid trigger_precondition")
        if route["output_transaction"] not in {
            "separate_delete_and_commit",
            "separate_delete_and_key_events",
            "atomic_protocol_batch",
        }:
            raise ValueError(f"{route_id}: invalid output_transaction")
        if route["application_result"] != "unobservable":
            raise ValueError(f"{route_id}: application_result must remain unobservable")
        if route["atomic_replace"] != (route["output_transaction"] == "atomic_protocol_batch"):
            raise ValueError(f"{route_id}: atomic_replace disagrees with output_transaction")
        support = route["sensitive_field_support"]
        if not isinstance(support, dict):
            raise ValueError(f"{route_id}: sensitive_field_support must be an object")
        expected_support = {"implemented", "protocol_signal", "compositor_observation", "certified"}
        if set(support) != expected_support:
            raise ValueError(f"{route_id}: sensitive_field_support has unexpected fields")
        if not isinstance(support["implemented"], bool) or not isinstance(support["certified"], bool):
            raise ValueError(f"{route_id}: sensitive field implementation/certification must be boolean")
        for key in ("protocol_signal", "compositor_observation"):
            if not isinstance(support[key], str) or not support[key]:
                raise ValueError(f"{route_id}: {key} must be a non-empty string")
        if support["certified"] and route["status"] != "supported":
            raise ValueError(f"{route_id}: certified sensitive-field support requires a supported route")
        if support["implemented"] != route["sensitive_fields"]:
            raise ValueError(f"{route_id}: sensitive_fields must mirror sensitive_field_support.implemented")
        seen.add(route_id)
        if not isinstance(route["certification_targets"], list) or not all(
            isinstance(target, str) and target in target_ids
            for target in route["certification_targets"]
        ):
            raise ValueError(f"{route_id}: certification targets must name known targets")
        rows.append(
            f"| `{route_id}` ({route['label']}) | {route['capture']} | {route['injection']} | "
            f"{route['trigger_precondition']} | {route['output_transaction']} | "
            f"{route['application_result']} | {'available' if route['focus_tracking'] else 'unavailable'} | "
            f"{'implemented' if support['implemented'] else 'not implemented'} | "
            f"{support['protocol_signal']} | {support['compositor_observation']} | "
            f"{'certified' if support['certified'] else 'not certified'} | "
            f"{'yes' if route['atomic_replace'] else 'no'} | {route['app_identity']} | "
            f"{route['status']} |"
        )
    return "\n".join([ROUTE_START, *rows, ROUTE_END])


def update(document: Path, expected: str, write: bool, start: str, end: str) -> bool:
    text = document.read_text(encoding="utf-8")
    if text.count(start) != 1 or text.count(end) != 1:
        raise ValueError(f"{document.relative_to(ROOT)} must have exactly one generated-block marker pair")
    before, remainder = text.split(start, 1)
    _, after = remainder.split(end, 1)
    actual = start + remainder.split(end, 1)[0] + end
    if actual == expected:
        return True
    if write:
        document.write_text(before + expected + after, encoding="utf-8")
        print(f"generated {document.relative_to(ROOT)}")
        return True
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
        route_catalog = json.loads(ROUTE_SOURCE.read_text(encoding="utf-8"))
        if not isinstance(matrix, dict):
            raise ValueError("certification matrix root must be an object")
        if not isinstance(route_catalog, dict):
            raise ValueError("route catalog root must be an object")
        expected = render(matrix, route_catalog)
        expected_routes = render_routes(matrix, route_catalog)
        results = []
        for document in DOCUMENTS:
            desktop_ok = update(document, expected, arguments.write, START, END)
            routes_ok = update(document, expected_routes, arguments.write, ROUTE_START, ROUTE_END)
            results.append(desktop_ok and routes_ok)
        current = all(results)
    except (OSError, json.JSONDecodeError, ValueError) as error:
        print(f"desktop matrix generation failed: {error}", file=sys.stderr)
        return 1
    if not arguments.write and current:
        print("desktop support documentation is synchronized")
    return 0 if current or arguments.write else 1


if __name__ == "__main__":
    raise SystemExit(main())
