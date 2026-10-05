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
    --output "$results" --log-dir "$logs" --timeout 120 --deadline 3600; then
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

timeout_driver="$test_root/timeout-driver"
timeout_marker="$test_root/timeout-driver-first-cell"
cat >"$timeout_driver" <<'EOF'
#!/bin/sh
if [ ! -e "${CERT_TEST_TIMEOUT_MARKER:?}" ]; then
    : >"$CERT_TEST_TIMEOUT_MARKER"
    exec sleep 10
fi
EOF
chmod 0755 "$timeout_driver"
timeout_results="$test_root/timeout-results.txt"
timeout_logs="$test_root/timeout-logs"
if CERT_TEST_TIMEOUT_MARKER="$timeout_marker" \
    "$project_dir/scripts/run-certification-driver.sh" \
        --driver "$timeout_driver" --compositor kde --version 6.6.2 \
        --backend ibus --layout us,de,fr,altgr,multi-layout-switching \
        --target-apps "$target_apps" --output "$timeout_results" --log-dir "$timeout_logs" \
        --timeout 1 --deadline 120; then
    printf '%s\n' 'driver timeout was incorrectly recorded as a pass' >&2
    exit 1
fi
grep -Fx 'printable-press-release|us|gtk4-demo=UNVERIFIED' "$timeout_results" >/dev/null
[ "$(grep -c '=UNVERIFIED$' "$timeout_results")" -eq 1 ]
grep -F 'scenario exceeded its 1s timeout' \
    "$timeout_logs/printable-press-release_us_gtk4-demo.log" >/dev/null

deadline_driver="$test_root/deadline-driver"
deadline_marker="$test_root/deadline-driver-first-cell"
deadline_calls="$test_root/deadline-driver-calls"
cat >"$deadline_driver" <<'EOF'
#!/bin/sh
printf '%s\n' "$1" >>"${CERT_TEST_DEADLINE_CALLS:?}"
if [ ! -e "${CERT_TEST_DEADLINE_MARKER:?}" ]; then
    : >"$CERT_TEST_DEADLINE_MARKER"
    exec sleep 2
fi
EOF
chmod 0755 "$deadline_driver"
deadline_results="$test_root/deadline-results.txt"
deadline_logs="$test_root/deadline-logs"
if CERT_TEST_DEADLINE_MARKER="$deadline_marker" CERT_TEST_DEADLINE_CALLS="$deadline_calls" \
    "$project_dir/scripts/run-certification-driver.sh" \
        --driver "$deadline_driver" --compositor kde --version 6.6.2 \
        --backend ibus --layout us,de,fr,altgr,multi-layout-switching \
        --target-apps "$target_apps" --output "$deadline_results" \
        --log-dir "$deadline_logs" --deadline 1 --timeout 10; then
    printf '%s\n' 'matrix deadline was incorrectly treated as a complete pass' >&2
    exit 1
fi
[ "$(wc -l <"$deadline_calls")" -eq 1 ]
grep -Fx 'printable-press-release|us|gtk4-demo=pass' "$deadline_results" >/dev/null
[ "$(grep -c '=UNVERIFIED$' "$deadline_results")" -eq $((expected_cases - 1)) ]
grep -F 'overall certification deadline (1s) reached' \
    "$deadline_logs/backspace_us_gtk4-demo.log" >/dev/null

printf '%s\n' 'certification driver contract test passed'
