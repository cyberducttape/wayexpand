#!/usr/bin/env bash
set -euo pipefail

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
# shellcheck source=scripts/kwin-e2e-report.sh
. "$project_dir/scripts/kwin-e2e-report.sh"
[[ "$(kwin_e2e_capture_label stdin)" == stdin-simulated ]]
[[ "$(kwin_e2e_capture_label input-method)" == input-method-v2-requested ]]
if kwin_e2e_capture_label unknown >/dev/null 2>&1; then
    printf '%s\n' 'unknown KWin capture source received a report label' >&2
    exit 1
fi
test_root=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-kwin-timeout-test.XXXXXX")
trap 'rm -rf "$test_root"' EXIT
fake_bin="$test_root/bin"
helpers="$test_root/helpers"
apps="$test_root/apps"
mkdir -p "$fake_bin" "$helpers" "$apps"

for executable in wx-e2e-sink wx-e2e-type wx-e2e-eis-relay; do
    : >"$helpers/$executable"
    chmod 0755 "$helpers/$executable"
done
for executable in wayexpand wayexpand-daemon; do
    cat >"$apps/$executable" <<'EOF'
#!/bin/sh
printf '%s 1.3.3 (commit timeout-test-build)\n' "${0##*/}"
EOF
    chmod 0755 "$apps/$executable"
done

cat >"$fake_bin/timeout" <<'EOF'
#!/bin/sh
[ "${1-}" = 900 ] || exit 90
[ "${E2E_CONFIG%/*}" = "$XDG_RUNTIME_DIR" ] || {
    printf 'certification config is outside the private runtime directory: %s\n' \
        "${E2E_CONFIG-}" >&2
    exit 91
}
printf '%s\n' 'synthetic KWin timeout after scenario output' >&2
for scenario in ${E2E_SCENARIOS-}; do
    printf 'pass %s synthetic result\n' "$scenario" >>"$E2E_RESULTS"
done
exit 124
EOF
cat >"$fake_bin/kwin_wayland" <<'EOF'
#!/bin/sh
printf '%s\n' 'kwin 6.6.6'
EOF
chmod 0755 "$fake_bin/timeout" "$fake_bin/kwin_wayland"

output="$test_root/output.log"
kept_work=

fail_test() {
    printf 'KWin timeout contract test failed: %s\n' "$1" >&2
    if [ -f "$output" ]; then
        printf '%s\n' '--- test output ---' >&2
        cat "$output" >&2
    fi
    if [ -n "$kept_work" ]; then
        for file in "$kept_work/results.txt" "$kept_work/kwin.log"; do
            if [ -f "$file" ]; then
                printf '%s\n' "--- $file ---" >&2
                cat "$file" >&2
            fi
        done
    fi
    exit 1
}

mkdir -m 700 "$test_root/runtime" "$test_root/cache"
chmod 0775 "$test_root/cache"
if PATH="$fake_bin:$PATH" \
    WAYEXPAND_E2E_PREBUILT_BIN="$helpers" \
    WAYEXPAND_E2E_APP_BIN="$apps" \
    XDG_CACHE_HOME="$test_root/cache" \
    XDG_RUNTIME_DIR="$test_root/runtime" \
    "$project_dir/scripts/test-kwin-e2e.sh" --keep expansion-replaces-trigger \
    >"$output" 2>&1
then
    fail_test 'the session timeout was accepted after passing scenario output'
else
    session_status=$?
fi
if [ "${session_status:-0}" -ne 2 ]; then
    fail_test "unexpected harness exit status ${session_status:-unset}; expected 2 for an unverified timeout"
fi

if ! grep -Fq 'private KWin session exceeded its 900s timeout' "$output"; then
    fail_test 'the harness did not report its bounded KWin session timeout'
fi
if ! grep -Fq 'synthetic KWin timeout after scenario output' "$output"; then
    fail_test 'the synthetic session did not produce its timeout marker'
fi
kept_work=$(sed -n 's/^Kept //p' "$output")
if [ -z "$kept_work" ]; then
    fail_test 'the --keep cleanup did not report its evidence directory'
fi
if ! grep -Fq 'pass expansion-replaces-trigger synthetic result' "$kept_work/results.txt"; then
    fail_test 'the fake session did not record a scenario result before timing out'
fi
if ! grep -Fq 'synthetic KWin timeout after scenario output' "$kept_work/kwin.log"; then
    fail_test 'the retained compositor log does not contain the synthetic timeout marker'
fi

printf '%s\n' 'KWin E2E timeout contract test passed'
