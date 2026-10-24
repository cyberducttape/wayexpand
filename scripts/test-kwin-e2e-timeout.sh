#!/usr/bin/env bash
set -euo pipefail

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
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
    : >"$apps/$executable"
    chmod 0755 "$apps/$executable"
done

cat >"$fake_bin/timeout" <<'EOF'
#!/bin/sh
[ "${1-}" = 900 ] || exit 90
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
if PATH="$fake_bin:$PATH" \
    WAYEXPAND_E2E_PREBUILT_BIN="$helpers" \
    WAYEXPAND_E2E_APP_BIN="$apps" \
    XDG_CACHE_HOME="$test_root/cache" \
    "$project_dir/scripts/test-kwin-e2e.sh" --keep expansion-replaces-trigger \
    >"$output" 2>&1
then
    printf '%s\n' 'KWin session timeout was incorrectly accepted after passing scenario output' >&2
    exit 1
fi

grep -F 'private KWin session exceeded its 900s timeout' "$output" >/dev/null
grep -F 'synthetic KWin timeout after scenario output' "$output" >/dev/null
kept_work=$(sed -n 's/^Kept //p' "$output")
[ -n "$kept_work" ]
grep -F 'synthetic KWin timeout after scenario output' "$kept_work/kwin.log" >/dev/null

printf '%s\n' 'KWin E2E timeout contract test passed'
