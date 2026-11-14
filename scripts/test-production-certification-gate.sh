#!/bin/sh
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
test_root=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-production-gate.XXXXXX")
gate_evidence_dir=$(mktemp -d "$(dirname -- "$0")/../docs/evidence/.production-gate-test.XXXXXX")
trap 'rm -rf -- "$test_root" "$gate_evidence_dir"' EXIT INT TERM
script="$project_dir/scripts/check-production-certification.sh"
matrix="$project_dir/tests/certification/compositor-matrix.json"

python3 - "$matrix" "$project_dir" <<'PY'
import importlib.util
import json
import sys
from pathlib import Path

matrix_path, project_root = Path(sys.argv[1]), Path(sys.argv[2])
spec = importlib.util.spec_from_file_location(
    "production_validator", project_root / "scripts/validate-production-certification.py"
)
validator = importlib.util.module_from_spec(spec)
spec.loader.exec_module(validator)
matrix = json.loads(matrix_path.read_text(encoding="utf-8"))
errors = validator.validate_matrix(matrix, project_root)
if errors != ["no compositor route is marked production-required"]:
    raise SystemExit(f"empty production scope was not rejected: {errors!r}")
PY

blocked_summary="$test_root/blocked.md"
if GITHUB_STEP_SUMMARY="$blocked_summary" "$script" true "$matrix" \
    >"$test_root/blocked.out" 2>"$test_root/blocked.err"; then
    printf '%s\n' 'production gate accepted a matrix with no eligible route' >&2
    exit 1
fi
grep -F 'No declared compositor route currently meets' "$test_root/blocked.err" >/dev/null
grep -F 'Compatibility jobs cannot satisfy this gate' "$blocked_summary" >/dev/null

compatibility_summary="$test_root/compatibility.md"
GITHUB_STEP_SUMMARY="$compatibility_summary" "$script" false "$matrix" \
    >"$test_root/compatibility.out"
grep -F 'This run gathers compatibility evidence only' "$compatibility_summary" >/dev/null
grep -F '::warning::' "$test_root/compatibility.out" >/dev/null

eligible_matrix="$test_root/eligible.json"
jq '.targets[0].production_certification_required = true' "$matrix" >"$eligible_matrix"
eligible_summary="$test_root/eligible.md"
GITHUB_STEP_SUMMARY="$eligible_summary" "$script" true "$eligible_matrix" \
    >"$test_root/eligible.out" 2>"$test_root/eligible.err" && {
    printf '%s\n' 'production gate accepted a target flag without certification evidence' >&2
    exit 1
}
grep -F 'kde: certification_status is not certified' "$test_root/eligible.err" >/dev/null
grep -F 'At least one production-required route has missing, invalid, or incompatible real-session evidence.' \
    "$eligible_summary" >/dev/null

valid_matrix="$test_root/valid.json"
python3 - "$matrix" "$valid_matrix" "$gate_evidence_dir" "$project_dir" <<'PY'
import itertools
import json
import subprocess
import sys
from pathlib import Path

source, matrix_output, evidence_dir, project_root = sys.argv[1:]
matrix = json.loads(Path(source).read_text(encoding="utf-8"))
target = next(item for item in matrix["targets"] if item["id"] == "sway")
target["production_certification_required"] = True
target["certification_status"] = "certified"
target["sensitive_field_observation"] = "observed"
evidence_path = (Path(evidence_dir) / "report.json").resolve()
target["certification_evidence"] = evidence_path.relative_to(Path(project_root).resolve()).as_posix()
apps = target["required_client_markers"]
evidence = {
    "schema": 5,
    "wayexpand_version": "1.3.3",
    "wayexpand_commit": subprocess.check_output(
        ["git", "-C", project_root, "rev-parse", "--short=12", "HEAD"], text=True
    ).strip(),
    "compositor_version": "1.0",
    "kernel_release": "6.1.0-test",
    "cli_binary_sha256": "a" * 64,
    "keyboard_layout": "us,de,fr,altgr,multi-layout-switching",
    "expected_desktop": "Sway/wlroots",
    "detected_desktop": "Sway/wlroots",
    "recorded_at_utc": "2026-11-11T00:00:00Z",
    "certified": True,
    "status": "certified",
    "certification_scope": "production",
    "production_eligible": True,
    "production_certification_required": True,
    "compositor": "sway",
    "backend": "input-method-v2",
    "expected_session": "wayland",
    "session": "wayland",
    "session_kind": "physical-wayland-session",
    "physical_keyboard": "USB keyboard model recorded by reviewer",
    "physical_evidence_ref": "reviewed://CERT-123/evidence",
    "physical_input_observed": True,
    "physical_session_attestation_valid": True,
    "reviewed_by": "release-reviewer",
    "session_probe_valid": True,
    "backend_certification_eligible": True,
    "doctor_probe_valid": True,
    "desktop_probe_valid": True,
    "backend_probe_valid": True,
    "status_required": True,
    "status_probe_valid": True,
    "client_versions_valid": True,
    "target_apps": apps,
    "client_versions": {
        app: {
            "application_version": "1",
            "toolkit_name": "test",
            "toolkit_version": "1",
        }
        for app in apps
    },
    "required_layout_profiles": matrix["required_layout_profiles"],
    "required_client_markers": apps,
    "scenarios": [
        {"name": scenario, "layout_profile": layout, "target_app": app, "result": "pass"}
        for scenario, layout, app in itertools.product(
            matrix["required_scenarios"], matrix["required_layout_profiles"], apps
        )
    ],
}
evidence_path.write_text(json.dumps(evidence), encoding="utf-8")
Path(matrix_output).write_text(json.dumps(matrix), encoding="utf-8")
PY
if "$script" true "$valid_matrix" >"$test_root/valid.out" 2>"$test_root/valid.err"; then
    printf '%s\n' 'production gate accepted evidence while the shipped route contract is experimental' >&2
    exit 1
fi
grep -F 'the declared route contract does not provide certified sensitive-field handling and atomic verified replacement' \
    "$test_root/valid.err" >/dev/null

python3 - "$valid_matrix" "$project_dir" <<'PY'
import importlib.util
import json
import sys
from pathlib import Path

sys.dont_write_bytecode = True
matrix_path, project_root = Path(sys.argv[1]), Path(sys.argv[2])
spec = importlib.util.spec_from_file_location(
    "production_validator", project_root / "scripts/validate-production-certification.py"
)
validator = importlib.util.module_from_spec(spec)
spec.loader.exec_module(validator)
matrix = json.loads(matrix_path.read_text(encoding="utf-8"))
target = next(item for item in matrix["targets"] if item["id"] == "sway")
routes = json.loads(
    (project_root / "crates/backend-selection/routes.json").read_text(encoding="utf-8")
)
route = next(item for item in routes["routes"] if item["path"] == "input-method-v2")
route["status"] = "supported"
route["atomic_replace"] = True
route["trigger_precondition"] = "surrounding_text_matches_trigger"
route["output_transaction"] = "atomic_protocol_batch"
route["application_result"] = "unobservable"
route["sensitive_fields"] = True
route["sensitive_field_support"]["implemented"] = True
route["sensitive_field_support"]["certified"] = True
evidence_path = project_root / target["certification_evidence"]
evidence = json.loads(evidence_path.read_text(encoding="utf-8"))
errors = validator.validate_matrix(matrix, project_root, routes)
if errors:
    raise SystemExit("valid fixture rejected: " + "; ".join(errors))
headless = dict(evidence, session_kind="headless-wayland-session")
evidence_path.write_text(json.dumps(headless), encoding="utf-8")
errors = validator.validate_matrix(matrix, project_root, routes)
if not any("complete, passing production report" in error for error in errors):
    raise SystemExit("headless Wayland evidence was accepted for production")
evidence_path.write_text(json.dumps(evidence), encoding="utf-8")
stale = validator.validate_matrix(
    matrix, project_root, routes, expected_commit="ffffffffffffffffffffffffffffffffffffffff"
)
if not any("does not match the checked-out commit" in error for error in stale):
    raise SystemExit("stale source evidence was accepted")
PY

invalid_matrix="$test_root/invalid.json"
printf '%s\n' '{"targets":[{"id":"kde","production_certification_required":"true"}]}' \
    >"$invalid_matrix"
if "$script" false "$invalid_matrix" >"$test_root/invalid.out" 2>"$test_root/invalid.err"; then
    printf '%s\n' 'production gate accepted a non-boolean target declaration' >&2
    exit 1
fi
grep -F 'invalid certification target declarations' "$test_root/invalid.err" >/dev/null

printf '%s\n' 'production certification release gate tests passed'
