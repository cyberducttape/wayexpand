#!/usr/bin/env bash
# Collect KWin certification results for scripts/kwin-certification-driver.sh.
#
#   scripts/kwin-certification-prepare.sh --store FILE [--keep]
#       [--layouts us,de,...] [--apps gtk3-textview,...] [--scenarios "a b"]
#
# For each keyboard layout profile this starts a private KWin
# (kwin_wayland --virtual) with its own D-Bus session, accessibility bus,
# runtime directory, home and configuration, and runs the headless-capable
# certification scenarios (scripts/kwin-certification-session.sh) against
# real GTK, Qt, terminal and editor clients. Nothing touches the running
# desktop, its daemon, or the user's settings.
#
# The runner gives each matrix cell 120 seconds and the whole matrix an
# hour, which cannot start a compositor and an application per cell; this
# script does the slow work once, and the driver answers each cell from the
# store. The store is bound to the tested commit and KWin version, and the
# driver refuses a store that does not match the build under certification.
#
# What this does and does not show: replacements are typed into real
# clients by the production libei backend, focus comes from the production
# KWin tracker, and each client reports the text it actually received. Key
# capture is simulated (the daemon reads stdin) and the session is
# headless, so capture, hardware and session-lifecycle cells stay
# UNVERIFIED in the driver.
set -euo pipefail

keep=false
store=
layouts=us,de,fr,altgr,multi-layout-switching
apps=gtk3-textview,qt6-plaintextedit,terminal-konsole,password-field,text-editor-gedit,qt6-editor-kate
scenarios="multiline-rapid focus-cross-window focus-change-during-expansion
target-closes-during-expansion config-reload daemon-restart daemon-restart-during-typing
failed-insertion unicode-combining command-timeout expansion-cancellation text-editor-client"
while [ "$#" -gt 0 ]; do
    case "$1" in
        --keep) keep=true; shift ;;
        --store) store=${2:?missing value for --store}; shift 2 ;;
        --layouts) layouts=${2:?missing value for --layouts}; shift 2 ;;
        --apps) apps=${2:?missing value for --apps}; shift 2 ;;
        --scenarios) scenarios=${2:?missing value for --scenarios}; shift 2 ;;
        --help|-h)
            sed -n '2,25p' "$0" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *) printf 'error: unknown option %s\n' "$1" >&2; exit 2 ;;
    esac
done
[ -n "$store" ] || { printf '%s\n' 'error: --store is required' >&2; exit 2; }
case "$store" in
    /*) ;;
    *) store="$PWD/$store" ;;
esac
project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
session_driver="$project_dir/scripts/kwin-certification-session.sh"
clients="$project_dir/scripts/certification-clients"
. "$project_dir/scripts/certification-processes.sh"

for tool in kwin_wayland dbus-run-session gdbus jq mkfifo timeout python3; do
    command -v "$tool" >/dev/null || {
        printf 'error: %s is required\n' "$tool" >&2
        exit 2
    }
done

# kxkbrc [Layout] entries for each matrix layout profile.
layout_config() {
    case "$1" in
        us) printf 'LayoutList=us\nVariantList=\n' ;;
        de) printf 'LayoutList=de\nVariantList=\n' ;;
        fr) printf 'LayoutList=fr\nVariantList=\n' ;;
        altgr) printf 'LayoutList=us\nVariantList=altgr-intl\n' ;;
        multi-layout-switching) printf 'LayoutList=us,de\nVariantList=,\n' ;;
        *) return 1 ;;
    esac
}
IFS=, read -r -a layout_list <<<"$layouts"
for layout in "${layout_list[@]}"; do
    layout_config "$layout" >/dev/null || {
        printf 'error: unknown layout profile %s\n' "$layout" >&2
        exit 2
    }
done

cd "$project_dir"
if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    source_tree_state=$(sh "$project_dir/scripts/source-tree-state.sh" "$project_dir" \
        Cargo.toml Cargo.lock .cargo rust-toolchain.toml crates scripts tests/certification)
else
    source_tree_state=unknown
fi
target_dir=${CARGO_TARGET_DIR:-"$project_dir/target"}
case "$target_dir" in
    /*) ;;
    *) target_dir="$project_dir/$target_dir" ;;
esac
export CARGO_TARGET_DIR="$target_dir"
cargo build --quiet --locked -p wayexpand -p wayexpand-daemon -p wayexpand-e2e
bin="$target_dir/debug"

cli_commit=$("$bin/wayexpand" --version | sed -n 's/.* (commit \([^)]*\))$/\1/p')
daemon_commit=$("$bin/wayexpand-daemon" --version | sed -n 's/.* (commit \([^)]*\))$/\1/p')
if [ -z "$cli_commit" ] || [ "$cli_commit" = unknown ] || [ "$cli_commit" != "$daemon_commit" ]; then
    printf '%s\n' 'error: tested CLI and daemon build identities do not match' >&2
    exit 2
fi
kwin_version=$(kwin_wayland --version 2>/dev/null | awk '{print $2}')

work=$(mktemp -d "$target_dir/kwin-certification.XXXXXX")
chmod 700 "$work"
runtime=
# A desktop launcher may delegate a client to the host systemd --user manager.
# Such a process can have the user's normal HOME, so also match argv paths under
# this run's unique private tree. Never match or stop the user manager itself.
reap_session_processes() {
    local pid attempt
    local pids
    pids=$(certification_private_pids "$(id -u)" "$work")
    [ -n "$pids" ] || return 0
    # TERM gives GTK/Qt clients time to close; KILL ensures no portal or
    # editor process survives after its private compositor and sockets go away.
    for pid in $pids; do kill "$pid" 2>/dev/null || true; done
    attempt=0
    while [ "$attempt" -lt 20 ]; do
        pids=$(certification_private_pids "$(id -u)" "$work")
        [ -n "$pids" ] || return 0
        sleep 0.1
        attempt=$((attempt + 1))
    done
    for pid in $pids; do kill -KILL "$pid" 2>/dev/null || true; done
}
cleanup() {
    reap_session_processes
    sleep 0.2
    [ -z "$runtime" ] || rm -rf "$runtime"
    if [ "$keep" = true ]; then
        printf 'Kept %s\n' "$work"
    else
        rm -rf "$work"
    fi
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

results="$work/results.txt"
: >"$results"
printf 'tested-build commit %s, source tree %s, KWin %s\n' \
    "$cli_commit" "$source_tree_state" "$kwin_version"
for layout in "${layout_list[@]}"; do
    layout_work="$work/$layout"
    mkdir -m 700 "$layout_work" "$layout_work/home"
    mkdir -p "$layout_work/home/.config"
    {
        printf '[Layout]\nUse=true\n'
        layout_config "$layout"
    } >"$layout_work/home/.config/kxkbrc"
    # Wayland and libei socket paths must fit in 108 bytes.
    runtime=$(mktemp -d /tmp/wxcert.XXXXXX)
    chmod 700 "$runtime"
    layout_results="$layout_work/results.txt"
    : >"$layout_results"
    printf 'layout %s: running\n' "$layout"
    session_status=0
    env -i \
        PATH="$PATH" \
        HOME="$layout_work/home" \
        LANG=C.UTF-8 \
        XDG_RUNTIME_DIR="$runtime" \
        XDG_CONFIG_HOME="$layout_work/home/.config" \
        XDG_CACHE_HOME="$layout_work/home/.cache" \
        XDG_DATA_HOME="$layout_work/home/.local/share" \
        XDG_CURRENT_DESKTOP=KDE \
        KDE_SESSION_VERSION=6 \
        QT_QPA_PLATFORM=offscreen \
        E2E_BIN="$bin" \
        E2E_WORK="$layout_work" \
        E2E_CONFIG="$runtime/expansions.toml" \
        E2E_RESULTS="$layout_results" \
        CERT_CLIENTS="$clients" \
        CERT_LAYOUT="$layout" \
        CERT_APPS="$apps" \
        CERT_SCENARIOS="$scenarios" \
        timeout 3000 dbus-run-session -- \
        kwin_wayland --virtual --no-lockscreen --no-global-shortcuts \
        --width 1280 --height 800 --socket wayexpand-cert \
        --exit-with-session "$session_driver" \
        >"$layout_work/kwin.log" 2>&1 || session_status=$?
    reap_session_processes
    rm -rf "$runtime"
    runtime=
    if [ "$session_status" -eq 124 ] || [ "$session_status" -eq 137 ]; then
        printf 'error: private KWin session for layout %s timed out\n' "$layout" >&2
        exit 2
    fi
    if ! grep -q . "$layout_results"; then
        printf 'error: the %s session produced no results; compositor log:\n' "$layout" >&2
        tail -20 "$layout_work/kwin.log" >&2
        exit 2
    fi
    while read -r outcome scenario app detail; do
        printf '  %-22s %-36s %-20s %s\n' "$outcome" "$scenario" "$app" "$detail"
        printf 'cell %s|%s|%s %s %s\n' "$scenario" "$layout" "$app" "$outcome" "$detail" >>"$results"
    done <"$layout_results"
done

route_detail=
if [ -s "$work/${layout_list[0]}/runtime-status" ]; then
    route_detail=$(sed -n 's/^backend_mode=//p' "$work/${layout_list[0]}/runtime-status")
fi
umask 077
{
    printf '# wayexpand KWin certification store\n'
    printf 'schema=1\n'
    printf 'commit=%s\n' "$cli_commit"
    printf 'source_tree_state=%s\n' "$source_tree_state"
    printf 'kwin_version=%s\n' "$kwin_version"
    printf 'capture=stdin\n'
    printf 'output_backend=libei\n'
    printf 'backend_mode=%s\n' "$route_detail"
    printf 'generated_at_unix=%s\n' "$(date +%s)"
    cat "$results"
} >"$store.tmp"
mv "$store.tmp" "$store"
printf 'wrote %s\n' "$store"
