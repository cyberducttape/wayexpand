#!/bin/sh
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
matrix="$project_dir/tests/certification/compositor-matrix.json"
gnome_ibus_matrix="$project_dir/tests/certification/gnome-ibus-matrix.json"

command -v jq >/dev/null 2>&1

jq -e '
  .schema == 1 and
  (.outcome_types == ["pass", "fail", "unsupported-by-design", "UNVERIFIED"]) and
  ([.out_of_scope_capabilities[] | select(.id == "active-ime-preedit" and .status == "unsupported-by-design")] | length == 1) and
  ([.targets[].id] | sort == ["gnome", "hyprland", "kde", "sway"]) and
  ([.targets[] | select((.input_paths | length) > 0 and (.toolkits | sort == ["GTK", "Qt"]) and (.requires_password_field_check == true))] | length == 4) and
  ([.targets[] | select(.certification_session == "wayland")] | length == 4) and
  ([.targets[] | select(.id != "gnome" and (.required_client_markers | sort == ["browser", "editor", "electron", "gtk", "password", "qt", "terminal"]))] | length == 3) and
  ([.targets[] | select(.id == "gnome" and (.required_client_markers | sort == ["browser", "editor", "electron", "gtk3", "gtk4", "password", "qt", "terminal"]))] | length == 1) and
  ([.targets[] | select((.id == "kde" or .id == "gnome") and (.input_paths | index("ibus")) and (.input_paths | index("evdev+libei")))] | length == 2) and
  ([.targets[] | select((.certification_status == "certified" and (.certification_evidence | type == "string")) or (.certification_status == "not-certified" and .certification_evidence == null))] | length == 4) and
  ([.targets[] | select((.certification_backend as $backend | (.input_paths | index($backend)) != null) and (.production_certification_required != true or .certification_backend == "input-method-v2"))] | length == 4) and
  ([.targets[] | select((.id == "sway" or .id == "hyprland") and (.input_paths | index("evdev+wlroots")))] | length == 2) and
  ([.targets[] | select((.id == "sway" or .id == "hyprland") and (.input_paths | index("input-method-v2")))] | length == 2) and
  ([.targets[] | select((.id == "sway" or .id == "hyprland") and .certification_backend == "input-method-v2" and .production_certification_required == false)] | length == 2) and
  ([.targets[] | select(.id == "kde" and .application_filter == "supported" and .window_tracker == "KWin application tracker")] | length == 1) and
  ([.targets[] | select(.id == "gnome" and .application_filter == "implemented" and .window_tracker == "WayExpand GNOME Shell extension (headless runtime gates: GNOME 46, 50, 51)")] | length == 1) and
  ([.targets[] | select((.id == "sway" or .id == "hyprland") and .application_filter == "unavailable" and .window_tracker == "none")] | length == 2) and
  ([.required_scenarios | length] | all(. == 48)) and
  ([.required_scenarios[]] | unique | length == 48) and
  ([.required_scenarios[]] | index("backspace") != null and index("escape") != null and index("arrow-navigation") != null and index("function-keys") != null and index("held-modifiers") != null and index("repeated-keys") != null and index("ctrl-alt-super-chords") != null and index("application-shortcuts") != null and index("compositor-shortcuts") != null and index("media-keys") != null and index("held-modifier-unsupported-key") != null and index("caps-lock") != null and index("fast-typing") != null and index("second-trigger-during-nonatomic-replacement") != null and index("modifier-during-nonatomic-replacement") != null and index("focus-change-during-expansion") != null and index("target-closes-during-expansion") != null and index("bluetooth-keyboard-reconnect") != null and index("key-pass-through-injector-failure") != null and index("key-pass-through-reconnect") != null and index("key-pass-through-cancellation") != null and index("key-pass-through-daemon-crash") != null and index("key-pass-through-compositor-restart") != null and index("dead-key-committed-text") != null and index("compose-committed-text") != null and index("expansion-after-committed-composition") != null and index("multiple-keyboards") != null and index("suspend-resume") != null and index("daemon-restart-during-typing") != null and index("command-timeout") != null and index("expansion-cancellation") != null and index("keyboard-hotplug") != null and index("keyboard-hotplug-during-nonatomic-replacement") != null and index("portal-revocation") != null and index("picker-same-app-title-focus-isolation") != null and index("ime-preedit") == null) and
  ([.required_layout_profiles[]] | unique | sort == ["altgr", "de", "fr", "multi-layout-switching", "us"])
' "$matrix" >/dev/null

jq -e '
  .schema == 1 and
  .initial_status == "not-tested" and
  ([.platforms[].id] | sort == ["fedora-45-gnome51", "ubuntu-24.04-gnome", "ubuntu-26.04-gnome"]) and
  ([.platforms[] | select(.id == "fedora-45-gnome51" and .gnome_shell == "51.x" and (.automated_extension_runtime_compatibility | contains("not desktop route certification")))] | length == 1) and
  (.application_classes | sort == ["browser-chromium", "browser-firefox", "electron", "gtk3", "gtk4", "qt", "terminal"]) and
  ([.required_lifecycle_scenarios[]] | unique | sort == ["clean-user-install", "component-registration", "gnome-source-registration-and-activation", "logout-login-persistence", "multiple-layout-switching", "package-upgrade-preserves-input-sources", "password-field-refusal", "real-trigger-replacement", "shell-restart-and-extension-reload", "switch-away-and-back", "unicode-replacement", "uninstall-preserves-original-desktop-settings", "uninstall-removes-wayexpand-components"]) and
  (.evidence_requirement | contains("exact package") and contains("GNOME Shell version") and contains("physical-session evidence"))
' "$gnome_ibus_matrix" >/dev/null

workflow="$project_dir/.github/workflows/certification.yml"
gate_script="$project_dir/scripts/check-production-certification.sh"
gate_test="$project_dir/scripts/test-production-certification-gate.sh"
[ -f "$workflow" ]
[ -x "$gate_script" ]
[ -x "$gate_test" ]
grep -F -- 'runner-preflight:' "$workflow" >/dev/null
grep -F -- 'actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1' "$workflow" >/dev/null
grep -F -- "ref: \${{ inputs.checkout_ref || github.sha }}" "$workflow" >/dev/null
grep -F -- 'needs: runner-preflight' "$workflow" >/dev/null
grep -F -- 'gh api --paginate --slurp' "$workflow" >/dev/null
grep -F -- 'CERTIFICATION_RUNNER_TOKEN' "$workflow" >/dev/null
grep -F -- 'Missing CERTIFICATION_RUNNER_TOKEN.' "$workflow" >/dev/null
grep -F -- 'Could not list repository Actions runners.' "$workflow" >/dev/null
grep -F -- 'actions: read' "$workflow" >/dev/null
grep -F -- 'fromJSON(needs.runner-preflight.outputs.matrix)' "$workflow" >/dev/null
grep -F -- "targets=\$(jq -r '.targets[]" "$workflow" >/dev/null
grep -F -- 'Invalid certification backend declaration' "$workflow" >/dev/null
grep -F -- 'scripts/run-certification-driver.sh' "$workflow" >/dev/null
grep -F -- 'scripts/certify-compositor.sh' "$workflow" >/dev/null
# These are literal GitHub Actions expressions, not shell expansions.
# shellcheck disable=SC2016
grep -F -- 'EXPECTED_SESSION: ${{ matrix.certification_session }}' "$workflow" >/dev/null
# shellcheck disable=SC2016
grep -F -- 'PRODUCTION_REQUIRED: ${{ matrix.production_required }}' "$workflow" >/dev/null
grep -F -- 'XDG_SESSION_TYPE:-unknown' "$workflow" >/dev/null
grep -F -- '.certification_scope == "production"' "$workflow" >/dev/null
grep -F -- 'continue-on-error: true' "$workflow" >/dev/null
grep -F -- 'if: always()' "$workflow" >/dev/null
grep -F -- 'schedule:' "$workflow" >/dev/null
grep -F -- '30 3 * * 1' "$workflow" >/dev/null
grep -F -- 'require_production:' "$workflow" >/dev/null
grep -F -- "REQUIRE_PRODUCTION: \${{ inputs.require_production == true }}" "$workflow" >/dev/null
grep -F -- 'scripts/check-production-certification.sh' "$workflow" >/dev/null
grep -F -- 'GITHUB_STEP_SUMMARY' "$gate_script" >/dev/null
grep -F -- 'Compatibility jobs will still be selected when their real-session runners are online.' "$gate_script" >/dev/null
grep -F -- 'WAYEXPAND_CERTIFICATION_DRIVER' "$workflow" >/dev/null
grep -F -- 'WAYEXPAND_CERTIFICATION_VERSION' "$workflow" >/dev/null
grep -F -- 'WAYEXPAND_CERTIFICATION_LAYOUT' "$workflow" >/dev/null
grep -F -- 'WAYEXPAND_CERTIFICATION_TARGET_APPS' "$workflow" >/dev/null
grep -F -- 'No declared compositor route currently meets WayExpand production certification requirements.' "$gate_script" >/dev/null

python3 - "$matrix" "$workflow" <<'PY'
import json
import importlib.util
import copy
import itertools
import sys
from pathlib import Path

matrix_path, workflow_path = sys.argv[1:]
matrix = json.load(open(matrix_path, encoding="utf-8"))
project_root = Path(matrix_path).resolve().parents[2]
route_catalog = json.load(
    open(project_root / "crates/backend-selection/routes.json", encoding="utf-8")
)
workflow = open(workflow_path, encoding="utf-8").read()
if "targets=$(jq -r '.targets[]" not in workflow:
    raise SystemExit("certification workflow must derive candidates from the matrix")
workflow_backends = {
    target["id"]: target["certification_backend"] for target in matrix["targets"]
}
if "evdev+libei" in workflow_backends.values():
    raise SystemExit("certification workflow must not use raw evdev as a certification candidate")
for target in matrix["targets"]:
    if target.get("certification_session") != "wayland":
        raise SystemExit(f"production certification target {target['id']} must be Wayland-bound")
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
    "schema": 5,
    "wayexpand_version": "1.3.3",
    "wayexpand_commit": "0123456789ab",
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
}
required_scenarios = matrix["required_scenarios"]
required_layouts = matrix["required_layout_profiles"]
target = next(target for target in matrix["targets"] if target["id"] == "sway")
required_client_markers = target["required_client_markers"]
target_apps = required_client_markers
production_evidence.update(
    {
        "target_apps": target_apps,
        "client_versions": {
            app: {
                "application_version": "1.0",
                "toolkit_name": "test-toolkit",
                "toolkit_version": "1.0",
            }
            for app in target_apps
        },
        "required_layout_profiles": required_layouts,
        "required_client_markers": required_client_markers,
        "scenarios": [
            {
                "name": name,
                "layout_profile": layout,
                "target_app": app,
                "result": "pass",
            }
            for name, layout, app in itertools.product(
                required_scenarios, required_layouts, target_apps
            )
        ],
    }
)
if not generator.is_production_evidence(
    production_evidence,
    "sway",
    ["evdev+wlroots", "input-method-v2"],
    "input-method-v2",
    "Sway/wlroots",
    "wayland",
    required_scenarios,
    required_layouts,
    required_client_markers,
):
    raise SystemExit("valid production evidence was rejected")
for field, value in (
    ("certification_scope", "compatibility"),
    ("production_eligible", False),
    ("production_certification_required", False),
    ("status_probe_valid", False),
    ("backend_certification_eligible", False),
):
    compatibility_report = {**production_evidence, field: value}
    if generator.is_production_evidence(
        compatibility_report,
        "sway",
        ["evdev+wlroots", "input-method-v2"],
        "input-method-v2",
        "Sway/wlroots",
        "wayland",
        required_scenarios,
        required_layouts,
        required_client_markers,
    ):
        raise SystemExit(f"compatibility evidence was accepted as production: {field}")

for label, incomplete in (
    ("old schema", {**production_evidence, "schema": 4}),
    ("unknown build identity", {**production_evidence, "wayexpand_commit": "unknown"}),
    ("dirty build identity", {**production_evidence, "wayexpand_version": "1.3.3-dev+abc.dirty.1234"}),
    ("desktop identity mismatch", {**production_evidence, "detected_desktop": "GNOME"}),
    ("invalid binary digest", {**production_evidence, "cli_binary_sha256": "unknown"}),
    ("headless Wayland session", {**production_evidence, "session_kind": "headless-wayland-session"}),
    ("no physical keyboard identified", {**production_evidence, "physical_keyboard": ""}),
    ("no physical-session evidence reference", {**production_evidence, "physical_evidence_ref": ""}),
    ("physical input not observed", {**production_evidence, "physical_input_observed": False}),
    ("session attestation absent", {**production_evidence, "physical_session_attestation_valid": False}),
    ("unreviewed evidence", {**production_evidence, "reviewed_by": ""}),
    (
        "missing matrix cell",
        {
            **production_evidence,
            "scenarios": production_evidence["scenarios"][:-1],
        },
    ),
    (
        "failed scenario",
        {
            **production_evidence,
            "scenarios": [
                *production_evidence["scenarios"][:-1],
                {
                    **production_evidence["scenarios"][-1],
                    "result": "fail",
                },
            ],
        },
    ),
    ("unsupported route", {**production_evidence, "backend": "ibus"}),
):
    if generator.is_production_evidence(
        incomplete,
        "sway",
        ["evdev+wlroots", "input-method-v2"],
        "input-method-v2",
        "Sway/wlroots",
        "wayland",
        required_scenarios,
        required_layouts,
        required_client_markers,
    ):
        raise SystemExit(f"incomplete certification evidence was accepted: {label}")

if generator.is_production_evidence(
    production_evidence,
    "sway",
    ["evdev+wlroots", "input-method-v2"],
    "ibus",
    "Sway/wlroots",
    "wayland",
    required_scenarios,
    required_layouts,
    required_client_markers,
):
    raise SystemExit("evidence for a different backend was accepted")

for label, updates, message in (
    (
        "certification flag disagreement",
        {"production_certification_required": False, "sensitive_field_observation": "observed"},
        "certified status requires production certification to be required",
    ),
    (
        "sensitive-field evidence disagreement",
        {"production_certification_required": True, "sensitive_field_observation": "not-observed"},
        "certified status requires observed sensitive-field handling",
    ),
):
    inconsistent = copy.deepcopy(matrix)
    target = inconsistent["targets"][0]
    target.update(updates)
    target["certification_status"] = "certified"
    target["certification_evidence"] = "docs/evidence/not-created.json"
    try:
        generator.render(inconsistent, route_catalog)
    except ValueError as error:
        if message not in str(error):
            raise SystemExit(f"{label} failed for the wrong reason: {error}")
    else:
        raise SystemExit(f"inconsistent certification claim was accepted: {label}")
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
grep -F -- 'Not production eligible; release workflow requires production evidence for any target marked required' "$project_dir/docs/CERTIFICATION.md" >/dev/null
grep -F -- 'CERTIFICATION_RUNNER_TOKEN' "$project_dir/docs/CERTIFICATION.md" >/dev/null
grep -F -- 'Whether the repository currently has the required online runners' "$project_dir/docs/CERTIFICATION.md" >/dev/null

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
