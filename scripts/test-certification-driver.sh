#!/bin/sh
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
matrix="$project_dir/tests/certification/compositor-matrix.json"
test_root=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-certification-driver-test.XXXXXX")
trap 'rm -rf "$test_root"' EXIT INT TERM
driver="$test_root/driver"
logs="$test_root/logs"
cat >"$driver" <<'EOF'
#!/bin/sh
case "$1" in
    failed-insertion) exit 1 ;;
    *)
        [ "$WAYEXPAND_CERTIFICATION_SCENARIO" = "$1" ]
        [ -n "$WAYEXPAND_CERTIFICATION_LAYOUT" ]
        [ "$WAYEXPAND_CERTIFICATION_LAYOUT_PROFILES" = "us,de,fr,altgr,multi-layout-switching" ]
        [ -n "$WAYEXPAND_CERTIFICATION_TARGET_APP" ]
        [ "$WAYEXPAND_CERTIFICATION_TARGET_APPS" = "gtk4-demo,qt6-demo,browser-firefox,terminal-konsole,password-field,electron-vscode,text-editor-gedit" ]
        printf '%s\n' "$1|$WAYEXPAND_CERTIFICATION_LAYOUT|$WAYEXPAND_CERTIFICATION_TARGET_APP"
        ;;
esac
EOF
chmod 0755 "$driver"

results="$test_root/results.txt"
if "$project_dir/scripts/run-certification-driver.sh" \
    --driver "$driver" --compositor kde --version 6.6.2 \
    --backend ibus --layout us,de,fr,altgr,multi-layout-switching \
    --target-apps gtk4-demo,qt6-demo,browser-firefox,terminal-konsole,password-field,electron-vscode,text-editor-gedit \
    --output "$results" --log-dir "$logs"; then
    printf '%s\n' 'driver accepted failed scenarios' >&2
    exit 1
fi

test -s "$logs/printable-press-release_us_gtk4-demo.log"
grep -F -- 'printable-press-release|us|gtk4-demo' "$logs/printable-press-release_us_gtk4-demo.log" >/dev/null

sorted_results="$test_root/results.sorted"
sort "$results" >"$sorted_results"
expected_scenarios=$(jq '.required_scenarios | length' "$matrix")
expected_layouts=$(jq '.required_layout_profiles | length' "$matrix")
target_apps=gtk4-demo,qt6-demo,browser-firefox,terminal-konsole,password-field,electron-vscode,text-editor-gedit
expected_apps=$(jq -nr --arg apps "$target_apps" '$apps | split(",") | length')
expected_cases=$((expected_scenarios * expected_layouts * expected_apps))
jq -R -e -s --argjson expected "$expected_cases" --argjson per_scenario "$((expected_layouts * expected_apps))" '
    (split("\n") | map(select(length > 0))) as $results |
    ($results | length == $expected) and
    ($results | map(select(startswith("printable-press-release|") and endswith("=pass"))) | length == $per_scenario) and
    ($results | map(select(startswith("failed-insertion|") and endswith("=fail"))) | length == $per_scenario) and
    ($results | map(select(startswith("expansion-after-committed-composition|") and endswith("=pass"))) | length == $per_scenario)
' "$sorted_results" >/dev/null

printf '%s\n' 'certification driver contract test passed'
