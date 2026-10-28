#!/bin/sh
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
matrix="$project_dir/tests/certification/compositor-matrix.json"

command -v jq >/dev/null 2>&1

jq -e '
  .schema == 1 and
  (.outcome_types == ["pass", "fail", "unsupported-by-design", "UNVERIFIED"]) and
  ([.out_of_scope_capabilities[] | select(.id == "active-ime-preedit" and .status == "unsupported-by-design")] | length == 1) and
  ([.targets[].id] | sort == ["gnome", "hyprland", "kde", "sway"]) and
  ([.targets[] | select((.input_paths | length) > 0 and (.toolkits | sort == ["GTK", "Qt"]) and (.required_client_markers | sort == ["browser", "editor", "electron", "gtk", "password", "qt", "terminal"]) and (.requires_password_field_check == true))] | length == 4) and
  ([.targets[] | select((.id == "kde" or .id == "gnome") and (.input_paths | index("ibus")) and (.input_paths | index("evdev+libei")))] | length == 2) and
  ([.targets[] | select((.certification_status == "certified" and (.certification_evidence | type == "string")) or (.certification_status == "not-certified" and .certification_evidence == null))] | length == 4) and
  ([.targets[] | select((.certification_backend as $backend | (.input_paths | index($backend)) != null) and (.production_certification_required != true or .certification_backend == "input-method-v2"))] | length == 4) and
  ([.targets[] | select((.id == "sway" or .id == "hyprland") and (.input_paths | index("evdev+wlroots")))] | length == 2) and
  ([.targets[] | select((.id == "sway" or .id == "hyprland") and (.input_paths | index("input-method-v2")))] | length == 2) and
  ([.targets[] | select((.id == "sway" or .id == "hyprland") and .certification_backend == "input-method-v2" and .production_certification_required == false)] | length == 2) and
  ([.targets[] | select(.id == "kde" and .application_filter == "supported" and .window_tracker == "KWin application tracker")] | length == 1) and
  ([.targets[] | select((.id == "gnome" or .id == "sway" or .id == "hyprland") and .application_filter == "unavailable" and .window_tracker == "none")] | length == 3) and
  ([.required_scenarios | length] | all(. == 45)) and
  ([.required_scenarios[]] | unique | length == 45) and
  ([.required_scenarios[]] | index("backspace") != null and index("escape") != null and index("arrow-navigation") != null and index("function-keys") != null and index("held-modifiers") != null and index("repeated-keys") != null and index("ctrl-alt-super-chords") != null and index("application-shortcuts") != null and index("compositor-shortcuts") != null and index("media-keys") != null and index("held-modifier-unsupported-key") != null and index("caps-lock") != null and index("fast-typing") != null and index("focus-change-during-expansion") != null and index("target-closes-during-expansion") != null and index("bluetooth-keyboard-reconnect") != null and index("key-pass-through-injector-failure") != null and index("key-pass-through-reconnect") != null and index("key-pass-through-cancellation") != null and index("key-pass-through-daemon-crash") != null and index("key-pass-through-compositor-restart") != null and index("dead-key-committed-text") != null and index("compose-committed-text") != null and index("expansion-after-committed-composition") != null and index("multiple-keyboards") != null and index("suspend-resume") != null and index("daemon-restart-during-typing") != null and index("command-timeout") != null and index("expansion-cancellation") != null and index("keyboard-hotplug") != null and index("portal-revocation") != null and index("picker-same-app-title-focus-isolation") != null and index("ime-preedit") == null) and
  ([.required_layout_profiles[]] | unique | sort == ["altgr", "de", "fr", "multi-layout-switching", "us"])
' "$matrix" >/dev/null

workflow="$project_dir/.github/workflows/certification.yml"
[ -f "$workflow" ]
grep -F -- 'runner-preflight:' "$workflow" >/dev/null
grep -F -- 'needs: runner-preflight' "$workflow" >/dev/null
grep -F -- 'gh api --paginate --slurp' "$workflow" >/dev/null
grep -F -- 'CERTIFICATION_RUNNER_TOKEN' "$workflow" >/dev/null
grep -F -- 'Missing CERTIFICATION_RUNNER_TOKEN.' "$workflow" >/dev/null
grep -F -- 'Could not list repository Actions runners.' "$workflow" >/dev/null
grep -F -- 'GITHUB_STEP_SUMMARY' "$workflow" >/dev/null
grep -F -- 'actions: read' "$workflow" >/dev/null
grep -F -- 'fromJSON(needs.runner-preflight.outputs.matrix)' "$workflow" >/dev/null
grep -F -- "targets=\$(jq -r '.targets[]" "$workflow" >/dev/null
grep -F -- 'Invalid certification backend declaration' "$workflow" >/dev/null
grep -F -- 'scripts/run-certification-driver.sh' "$workflow" >/dev/null
grep -F -- 'scripts/certify-compositor.sh' "$workflow" >/dev/null
grep -F -- '.certification_scope == "production"' "$workflow" >/dev/null
grep -F -- 'continue-on-error: true' "$workflow" >/dev/null
grep -F -- 'if: always()' "$workflow" >/dev/null
grep -F -- 'schedule:' "$workflow" >/dev/null
grep -F -- '30 3 * * 1' "$workflow" >/dev/null
grep -F -- 'require_production:' "$workflow" >/dev/null
grep -F -- 'REQUIRE_PRODUCTION: ${{ inputs.require_production }}' "$workflow" >/dev/null
grep -F -- 'if [ "$REQUIRE_PRODUCTION" = true ]; then' "$workflow" >/dev/null
grep -F -- 'Compatibility jobs will still be selected when their real-session runners are online.' "$workflow" >/dev/null
grep -F -- 'WAYEXPAND_CERTIFICATION_DRIVER' "$workflow" >/dev/null
grep -F -- 'WAYEXPAND_CERTIFICATION_VERSION' "$workflow" >/dev/null
grep -F -- 'WAYEXPAND_CERTIFICATION_LAYOUT' "$workflow" >/dev/null
grep -F -- 'WAYEXPAND_CERTIFICATION_TARGET_APPS' "$workflow" >/dev/null
grep -F -- 'No declared compositor route currently meets WayExpand production certification requirements.' "$workflow" >/dev/null

python3 - "$matrix" "$workflow" <<'PY'
import json
import importlib.util
import sys
from pathlib import Path

matrix_path, workflow_path = sys.argv[1:]
matrix = json.load(open(matrix_path, encoding="utf-8"))
workflow = open(workflow_path, encoding="utf-8").read()
if "targets=$(jq -r '.targets[]" not in workflow:
    raise SystemExit("certification workflow must derive candidates from the matrix")
workflow_backends = {
    target["id"]: target["certification_backend"] for target in matrix["targets"]
}
if "evdev+libei" in workflow_backends.values():
    raise SystemExit("certification workflow must not use raw evdev as a certification candidate")
for target in matrix["targets"]:
    expected = target["production_certification_required"]
    backend = target.get("certification_backend")
    if backend not in target["input_paths"]:
        raise SystemExit(f"certification backend for {target['id']} is not a declared input path")
    if expected and (
        backend != "input-method-v2" or "input-method-v2" not in target["input_paths"]
    ):
        raise SystemExit(
            f"production target {target['id']} must use its declared input-method-v2 route"
        )
    if not expected and target["id"] in ("kde", "gnome") and backend != "ibus":
        raise SystemExit(f"{target['id']} compatibility job must use the IBus route")
    if target["id"] in ("sway", "hyprland") and backend != "input-method-v2":
        raise SystemExit(f"{target['id']} compatibility job must exercise input-method-v2")

generator_path = Path(matrix_path).resolve().parents[2] / "scripts/generate-desktop-matrix.py"
sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("desktop_matrix", generator_path)
if spec is None or spec.loader is None:
    raise SystemExit("could not load desktop evidence validator")
generator = importlib.util.module_from_spec(spec)
spec.loader.exec_module(generator)
production_evidence = {
    "schema": 3,
    "certified": True,
    "status": "certified",
    "certification_scope": "production",
    "production_eligible": True,
    "production_certification_required": True,
    "compositor": "sway",
    "backend": "input-method-v2",
}
if not generator.is_production_evidence(
    production_evidence, "sway", ["evdev+wlroots", "input-method-v2"]
):
    raise SystemExit("valid production evidence was rejected")
for field, value in (
    ("certification_scope", "compatibility"),
    ("production_eligible", False),
    ("production_certification_required", False),
):
    compatibility_report = {**production_evidence, field: value}
    if generator.is_production_evidence(
        compatibility_report, "sway", ["evdev+wlroots", "input-method-v2"]
    ):
        raise SystemExit(f"compatibility evidence was accepted as production: {field}")
PY
if grep -F -- 'backend: evdev+libei' "$workflow" >/dev/null; then
    printf '%s\n' 'certification workflow must not certify raw evdev' >&2
    exit 1
fi
grep -F -- 'Require certified evidence' "$workflow" >/dev/null
grep -F -- '.certified == true and .status == "certified"' "$workflow" >/dev/null
grep -F -- "backend_certification_block_reason='IBus lacks atomic replacement, exact window identity, and composition awareness'" "$project_dir/scripts/certify-compositor.sh" >/dev/null
grep -F -- 'No declared compositor' "$project_dir/docs/CERTIFICATION.md" >/dev/null
grep -F -- 'input-method-v2 is the current compatibility workflow candidate' "$project_dir/docs/CERTIFICATION.md" >/dev/null
grep -F -- 'No production certification; the workflow does not yet require production evidence' "$project_dir/docs/CERTIFICATION.md" >/dev/null

release_workflow="$project_dir/.github/workflows/release.yml"
grep -F -- 'uses: ./.github/workflows/certification.yml' "$release_workflow" >/dev/null
grep -F -- 'require_production: true' "$release_workflow" >/dev/null
python3 - "$release_workflow" <<'PY'
import sys

body = open(sys.argv[1], encoding="utf-8").read()
start = body.index("  certification:\n")
end = body.find("\n  linux-aarch64:", start)
section = body[start:] if end == -1 else body[start:end]
if "    secrets: inherit\n" not in section:
    raise SystemExit("release certification call must inherit CERTIFICATION_RUNNER_TOKEN")
PY
grep -F -- 'needs: [ci, certification]' "$release_workflow" >/dev/null
grep -F -- 'needs: [ci, certification, linux-aarch64]' "$release_workflow" >/dev/null
grep -F -- "checkout_ref: \${{ inputs.release_ref || github.ref }}" "$release_workflow" >/dev/null

printf '%s\n' 'certification matrix contract passed'
