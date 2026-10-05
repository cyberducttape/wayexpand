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
  ([.targets[] | select((.id == "kde" or .id == "gnome") and (.input_paths | index("input-method-v2")))] | length == 2) and
  ([.targets[] | select((.certification_status == "certified" and (.certification_evidence | type == "string")) or (.certification_status == "not-certified" and .certification_evidence == null))] | length == 4) and
  ([.targets[] | select((.id == "sway" or .id == "hyprland") and (.input_paths | index("evdev+wlroots")))] | length == 2) and
  ([.targets[] | select(.id == "kde" and .application_filter == "supported" and .window_tracker == "KWin application tracker")] | length == 1) and
  ([.targets[] | select((.id == "gnome" or .id == "sway" or .id == "hyprland") and .application_filter == "unavailable" and .window_tracker == "none")] | length == 3) and
  ([.required_scenarios | length] | all(. == 36)) and
  ([.required_scenarios[]] | unique | length == 36) and
  ([.required_scenarios[]] | index("backspace") != null and index("escape") != null and index("arrow-navigation") != null and index("function-keys") != null and index("held-modifiers") != null and index("repeated-keys") != null and index("ctrl-alt-super-chords") != null and index("key-pass-through-injector-failure") != null and index("key-pass-through-reconnect") != null and index("key-pass-through-cancellation") != null and index("key-pass-through-daemon-crash") != null and index("key-pass-through-compositor-restart") != null and index("dead-key-committed-text") != null and index("compose-committed-text") != null and index("expansion-after-committed-composition") != null and index("multiple-keyboards") != null and index("suspend-resume") != null and index("daemon-restart-during-typing") != null and index("command-timeout") != null and index("expansion-cancellation") != null and index("keyboard-hotplug") != null and index("portal-revocation") != null and index("picker-same-app-title-focus-isolation") != null and index("ime-preedit") == null) and
  ([.required_layout_profiles[]] | unique | sort == ["altgr", "de", "fr", "multi-layout-switching", "us"])
' "$matrix" >/dev/null

workflow="$project_dir/.github/workflows/certification.yml"
[ -f "$workflow" ]
for compositor in kde gnome sway hyprland; do
    grep -F -- "compositor: $compositor" "$workflow" >/dev/null
done
grep -F -- 'scripts/run-certification-driver.sh' "$workflow" >/dev/null
grep -F -- 'scripts/certify-compositor.sh' "$workflow" >/dev/null
grep -F -- 'continue-on-error: true' "$workflow" >/dev/null
grep -F -- 'if: always()' "$workflow" >/dev/null
grep -F -- 'schedule:' "$workflow" >/dev/null
grep -F -- '30 3 * * 1' "$workflow" >/dev/null
grep -F -- 'WAYEXPAND_CERTIFICATION_DRIVER' "$workflow" >/dev/null
grep -F -- 'WAYEXPAND_CERTIFICATION_VERSION' "$workflow" >/dev/null
grep -F -- 'WAYEXPAND_CERTIFICATION_LAYOUT' "$workflow" >/dev/null
grep -F -- 'WAYEXPAND_CERTIFICATION_TARGET_APPS' "$workflow" >/dev/null
grep -F -- 'backend: input-method-v2' "$workflow" >/dev/null
if grep -F -- 'backend: evdev+libei' "$workflow" >/dev/null; then
    printf '%s\n' 'certification workflow must not certify raw evdev' >&2
    exit 1
fi
grep -F -- 'Require certified evidence' "$workflow" >/dev/null
grep -F -- '.certified == true and .status == "certified"' "$workflow" >/dev/null
grep -F -- "backend_certification_block_reason='IBus lacks atomic replacement, exact window identity, and composition awareness'" "$project_dir/scripts/certify-compositor.sh" >/dev/null

release_workflow="$project_dir/.github/workflows/release.yml"
grep -F -- 'uses: ./.github/workflows/certification.yml' "$release_workflow" >/dev/null
grep -F -- 'needs: [ci, certification]' "$release_workflow" >/dev/null
grep -F -- 'needs: [ci, certification, linux-aarch64]' "$release_workflow" >/dev/null
grep -F -- "checkout_ref: \${{ inputs.release_ref || github.ref }}" "$release_workflow" >/dev/null

printf '%s\n' 'certification matrix contract passed'
