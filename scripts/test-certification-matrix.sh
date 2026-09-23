#!/bin/sh
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
matrix="$project_dir/tests/certification/compositor-matrix.json"

command -v jq >/dev/null 2>&1

jq -e '
  .schema == 1 and
  ([.targets[].id] | sort == ["gnome", "hyprland", "kde", "sway"]) and
  ([.targets[] | select((.input_paths | length) > 0 and (.toolkits | sort == ["GTK", "Qt"]) and (.required_client_markers | sort == ["gtk", "password", "qt"]) and (.requires_password_field_check == true))] | length == 4) and
  ([.targets[] | select((.id == "kde" or .id == "gnome") and (.input_paths | index("ibus")) and (.input_paths | index("evdev+libei")))] | length == 2) and
  ([.targets[] | select((.id == "kde" or .id == "gnome") and (.input_paths | index("input-method-v2")))] | length == 2) and
  ([.targets[] | select((.id == "sway" or .id == "hyprland") and (.input_paths | index("evdev+wlroots")))] | length == 2) and
  ([.required_scenarios | length] | all(. == 12)) and
  ([.required_scenarios[]] | unique | length == 12)
' "$matrix" >/dev/null

workflow="$project_dir/.github/workflows/certification.yml"
[ -f "$workflow" ]
for compositor in kde gnome sway hyprland; do
    grep -F -- "compositor: $compositor" "$workflow" >/dev/null
done
grep -F -- 'scripts/run-certification-driver.sh' "$workflow" >/dev/null
grep -F -- 'scripts/certify-compositor.sh' "$workflow" >/dev/null
grep -F -- 'WAYEXPAND_CERTIFICATION_DRIVER' "$workflow" >/dev/null
grep -F -- 'WAYEXPAND_CERTIFICATION_VERSION' "$workflow" >/dev/null
grep -F -- 'WAYEXPAND_CERTIFICATION_LAYOUT' "$workflow" >/dev/null
grep -F -- 'WAYEXPAND_CERTIFICATION_TARGET_APPS' "$workflow" >/dev/null

printf '%s\n' 'certification matrix contract passed'
