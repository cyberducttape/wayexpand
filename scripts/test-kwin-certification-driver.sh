#!/bin/sh
# Contract tests for scripts/kwin-certification-driver.sh: cells are answered
# from a store only when the store matches the build, KWin version and route
# under certification, and capture/hardware cells are never passed.
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
driver="$project_dir/scripts/kwin-certification-driver.sh"
test_root=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-kwin-driver-test.XXXXXX")
trap 'rm -rf "$test_root"' EXIT INT TERM

cli="$test_root/wayexpand"
cat >"$cli" <<'EOF'
#!/bin/sh
printf '%s\n' "wayexpand 1.3.3 (commit ${FAKE_COMMIT:-abc123def456})"
EOF
chmod 0755 "$cli"

store="$test_root/store.txt"
write_store() {
    cat >"$store" <<EOF
# wayexpand KWin certification store
schema=1
commit=abc123def456
source_tree_state=${1:-clean}
kwin_version=6.6.6
capture=stdin
output_backend=libei
backend_mode=libei_keysym_fallback
generated_at_unix=0
cell config-reload|de|gtk3-textview pass text=zany quiz cursor=9
cell failed-insertion|de|gtk3-textview fail text=garbage cursor=7
cell unicode-combining|de|gtk3-textview unverified route refused combining marks cleanly
cell printable-press-release|de|gtk3-textview pass forged capture result
cell expansion-cancellation|de|gtk3-textview pass stdin-path observation
EOF
}

# expect EXIT_CODE SCENARIO [ENV=VALUE...]
expect() {
    expected=$1
    scenario=$2
    shift 2
    status=0
    env WAYEXPAND_CLI="$cli" \
        WAYEXPAND_KWIN_CERTIFICATION_STORE="$store" \
        WAYEXPAND_CERTIFICATION_COMPOSITOR=kde \
        WAYEXPAND_CERTIFICATION_BACKEND=evdev+libei \
        WAYEXPAND_CERTIFICATION_VERSION=6.6.6 \
        WAYEXPAND_CERTIFICATION_LAYOUT=de \
        WAYEXPAND_CERTIFICATION_TARGET_APP=gtk3-textview \
        "$@" "$driver" "$scenario" >"$test_root/out" 2>&1 || status=$?
    if [ "$status" -ne "$expected" ]; then
        printf 'expected exit %s for %s (%s), got %s:\n' "$expected" "$scenario" "$*" "$status" >&2
        cat "$test_root/out" >&2
        exit 1
    fi
}

write_store
expect 0 config-reload
grep -F 'zany quiz' "$test_root/out" >/dev/null
expect 1 failed-insertion
expect 2 unicode-combining
# Capture cells stay unverified even when a store claims a pass.
expect 2 printable-press-release
expect 2 key-pass-through-reconnect
# Pending-output scenarios observed through stdin capture do not count.
expect 2 expansion-cancellation
grep -F 'stdin-path observation' "$test_root/out" >/dev/null
expect 2 second-trigger-during-nonatomic-replacement
grep -F 'cannot exercise physical input arriving during an in-flight replacement' "$test_root/out" >/dev/null
expect 2 modifier-during-nonatomic-replacement
grep -F 'cannot exercise physical modifier or lock-key activity during replacement' "$test_root/out" >/dev/null
expect 2 keyboard-hotplug
expect 3 password-field
# Cells, clients and layouts the store does not cover.
expect 2 daemon-restart
expect 2 config-reload WAYEXPAND_CERTIFICATION_TARGET_APP=browser-firefox
expect 2 config-reload WAYEXPAND_CERTIFICATION_LAYOUT=fr
expect 2 electron-client
# The store must match the route, KWin version and build.
expect 2 config-reload WAYEXPAND_CERTIFICATION_BACKEND=ibus
expect 2 config-reload WAYEXPAND_CERTIFICATION_COMPOSITOR=gnome
expect 2 config-reload WAYEXPAND_CERTIFICATION_VERSION=6.5.0
expect 2 config-reload FAKE_COMMIT=0123456789ab
expect 2 config-reload WAYEXPAND_KWIN_CERTIFICATION_STORE="$test_root/missing"
write_store dirty
expect 2 config-reload

printf '%s\n' 'kwin certification driver contract tests passed'
