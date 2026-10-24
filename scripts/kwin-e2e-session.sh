#!/bin/sh
# Scenarios for the headless KWin end-to-end harness.
#
# Runs inside the private session that scripts/test-kwin-e2e.sh starts
# (dbus-run-session + kwin_wayland --virtual); never run it in a real
# desktop session. Each scenario starts fresh e2e sink windows and a
# WayExpand daemon with:
#   - capture: --source=stdin (the harness feeds what the "user" typed;
#     real keyboard capture needs /dev/input and is not exercised here),
#   - output: the production libei backend, reaching KWin's EIS through
#     wx-e2e-eis-relay instead of the RemoteDesktop portal,
#   - focus: the production KWin window tracker on the private bus, with
#     compatibility_mode = "tracked-focus".
# The "user" types into the focused sink with wx-e2e-type (also libei), and
# the sinks record exactly the text KWin delivered to them.
#
# Results go to $E2E_RESULTS, one line per scenario: "<result> <name>
# <detail>", where result is pass, fail, or unverified.
set -u

bin=${E2E_BIN:?}
app_bin=${E2E_APP_BIN:-$bin}
work=${E2E_WORK:?}
results=${E2E_RESULTS:?}
scenarios=${E2E_SCENARIOS:-all}

export LIBEI_SOCKET="$XDG_RUNTIME_DIR/eis.sock"
export WAYEXPAND_SOCKET="$XDG_RUNTIME_DIR/wayexpand.sock"
export WAYEXPAND_CONFIG="$work/expansions.toml"

sink_pids=
daemon_pid=
case_dir=

log() {
    printf '%s\n' "$*" >>"$work/session.log"
}

# --- helpers ------------------------------------------------------------

wait_until() {
    # wait_until TENTHS COMMAND...: retry COMMAND every 0.1 s.
    tries=$1
    shift
    while [ "$tries" -gt 0 ]; do
        if "$@"; then
            return 0
        fi
        sleep 0.1
        tries=$((tries - 1))
    done
    return 1
}

sink_field() {
    jq -r ".$2" "$case_dir/$1.json" 2>/dev/null
}

sink_text_is() {
    [ "$(sink_field "$1" text)" = "$2" ]
}

sink_cursor_is() {
    [ "$(sink_field "$1" cursor)" = "$2" ]
}

sink_focused() {
    [ "$(sink_field "$1" focused)" = true ]
}

start_sink() {
    "$bin/wx-e2e-sink" "$1" "$case_dir/$1.json" >>"$case_dir/$1.log" 2>&1 &
    sink_pids="$sink_pids $!"
    eval "pid_$(printf '%s' "$1" | tr -c 'a-z0-9\n' _)=$!"
    wait_until 50 test -s "$case_dir/$1.json"
}

sink_pid() {
    eval "printf '%s' \"\$pid_$(printf '%s' "$1" | tr -c 'a-z0-9\n' _)\""
}

# Activate the window whose app_id is $1 through KWin scripting.
activate() {
    script="$case_dir/activate-$1.js"
    cat >"$script" <<EOF
const target = workspace.windowList().find((window) => window.resourceClass === "$1");
if (target) {
    workspace.activeWindow = target;
}
EOF
    plugin="wx-e2e-activate-$$-$1"
    id=$(gdbus call --session --dest org.kde.KWin --object-path /Scripting \
        --method org.kde.kwin.Scripting.loadScript "$script" "$plugin" 2>>"$case_dir/kwin-script.log" |
        sed -n 's/^(\([0-9-]*\),)$/\1/p')
    if [ -z "$id" ] || [ "$id" -lt 0 ]; then
        log "could not load activation script for $1"
        return 1
    fi
    gdbus call --session --dest org.kde.KWin --object-path "/Scripting/Script$id" \
        --method org.kde.kwin.Script.run >/dev/null 2>>"$case_dir/kwin-script.log"
    gdbus call --session --dest org.kde.KWin --object-path /Scripting \
        --method org.kde.kwin.Scripting.unloadScript "$plugin" >/dev/null 2>>"$case_dir/kwin-script.log"
    wait_until 30 sink_focused "$1"
}

# Type as the user into the focused window, and wait until $1 shows $2.
user_types() {
    sink=$1
    expected=$2
    text=$3
    "$bin/wx-e2e-type" text "$text" >>"$case_dir/type.log" 2>&1 &&
        wait_until 30 sink_text_is "$sink" "$expected"
}

daemon_status() {
    "$app_bin/wayexpand" status 2>/dev/null
}

daemon_ready() {
    status=$(daemon_status) || return 1
    # Output is connected once the route reports its assurance, and focus
    # checks need the KWin tracker.
    case "$status" in
        *"window_tracker_connected=true"*) ;;
        *) return 1 ;;
    esac
    case "$status" in
        *"insertion_assurance=compatibility"*) return 0 ;;
    esac
    return 1
}

write_config() {
    umask 077
    {
        printf '%s\n' '[settings]' 'compatibility_mode = "tracked-focus"' ''
        cat
    } >"$WAYEXPAND_CONFIG.tmp"
    mv "$WAYEXPAND_CONFIG.tmp" "$WAYEXPAND_CONFIG"
}

start_daemon() {
    rm -f "$case_dir/stdin"
    mkfifo -m 600 "$case_dir/stdin"
    "$app_bin/wayexpand-daemon" --source=stdin --backend=libei <"$case_dir/stdin" \
        >>"$case_dir/daemon.log" 2>&1 &
    daemon_pid=$!
    exec 3>"$case_dir/stdin"
    if ! wait_until 100 daemon_ready; then
        log "daemon never became ready: $(daemon_status | tr '\n' ' ')"
        return 1
    fi
}

stop_daemon() {
    exec 3>&-
    if [ -n "$daemon_pid" ]; then
        kill "$daemon_pid" 2>/dev/null || true
        wait "$daemon_pid" 2>/dev/null || true
    fi
    daemon_pid=
}

# What the daemon's capture would have seen: one stdin line.
daemon_sees() {
    printf '%s\n' "$1" >&3
}

daemon_alive() {
    [ -n "$daemon_pid" ] && kill -0 "$daemon_pid" 2>/dev/null
}

begin_case() {
    case_dir="$work/$1"
    mkdir -m 700 "$case_dir"
    sink_pids=
}

end_case() {
    stop_daemon
    for pid in $sink_pids; do
        kill "$pid" 2>/dev/null || true
    done
    for pid in $sink_pids; do
        wait "$pid" 2>/dev/null || true
    done
    sink_pids=
}

# Expand TRIGGER in the focused SINK: the user types it, the daemon sees it.
expand_in() {
    sink=$1
    before=$2
    trigger=$3
    user_types "$sink" "$before$trigger" "$trigger" || return 1
    daemon_sees "$trigger"
}

describe() {
    printf 'text=%s cursor=%s' "$(sink_field "$1" text | sed -n l | tr -d '\n')" "$(sink_field "$1" cursor)"
}

# --- scenarios ----------------------------------------------------------
# Each prints a detail line and returns 0 (pass), 1 (fail) or 2 (unverified).

scenario_expansion_replaces_trigger() {
    write_config <<'EOF'
[[expansion]]
trigger = ";sig"
replacement = "Best regards"
EOF
    start_sink e2e-a && start_daemon || return 2
    expand_in e2e-a "" ";sig" || return 2
    wait_until 30 sink_text_is e2e-a "Best regards"
    result=$?
    describe e2e-a
    return "$result"
}

scenario_unicode_insertion() {
    # Only claim Unicode insertion when the negotiated production route
    # supports direct UTF-8 text. A keysym-only route is reported as
    # unverified here; its safe refusal behavior has its own scenario.
    write_config <<'EOF'
[[expansion]]
trigger = ";unicode"
replacement = "café 東京"
EOF
    start_sink e2e-a && start_daemon || return 2
    mode=$(daemon_status | sed -n 's/^inject_insertion_mode=//p')
    case "$mode" in
        *ei_text*) ;;
        *)
            printf 'route=%s; direct UTF-8 text unavailable' "${mode:-unknown}"
            return 2
            ;;
    esac
    expand_in e2e-a "" ";unicode" || return 2
    wait_until 50 sink_text_is e2e-a "café 東京"
    result=$?
    describe e2e-a
    return "$result"
}

scenario_word_boundary_reinserts_delimiter() {
    write_config <<'EOF'
[[expansion]]
trigger = "btw"
replacement = "by the way"
match_mode = "word-boundary"
EOF
    start_sink e2e-a && start_daemon || return 2
    expand_in e2e-a "" "so btw " || return 2
    wait_until 30 sink_text_is e2e-a "so by the way "
    result=$?
    describe e2e-a
    return "$result"
}

scenario_cursor_placement() {
    write_config <<'EOF'
[[expansion]]
trigger = ";tag"
replacement = "<b>{{cursor}}</b>"
EOF
    start_sink e2e-a && start_daemon || return 2
    expand_in e2e-a "" ";tag" || return 2
    wait_until 30 sink_text_is e2e-a "<b></b>" &&
        wait_until 10 sink_cursor_is e2e-a 3
    result=$?
    describe e2e-a
    return "$result"
}

scenario_multiline_replacement() {
    write_config <<'EOF'
[[expansion]]
trigger = ";addr"
replacement = "1 Example Way\nSpringfield\nExampleland"
EOF
    start_sink e2e-a && start_daemon || return 2
    expand_in e2e-a "" ";addr" || return 2
    expected=$(printf '1 Example Way\nSpringfield\nExampleland')
    wait_until 50 sink_text_is e2e-a "$expected"
    result=$?
    describe e2e-a
    return "$result"
}

scenario_unreachable_character_is_refused() {
    # The headless session uses KWin's default US layout and its EIS offers
    # no ei_text, so "é" cannot be typed. WayExpand must refuse before it
    # erases anything rather than type something else.
    write_config <<'EOF'
[[expansion]]
trigger = ";cafe"
replacement = "café"
EOF
    start_sink e2e-a && start_daemon || return 2
    expand_in e2e-a "" ";cafe" || return 2
    sleep 1.5
    sink_text_is e2e-a ";cafe"
    result=$?
    describe e2e-a
    return "$result"
}

scenario_app_filter_follows_focus() {
    write_config <<'EOF'
[[expansion]]
trigger = ";only"
replacement = "only in A"
app_filter = ["e2e-a"]
EOF
    start_sink e2e-a && start_sink e2e-b && start_daemon || return 2
    activate e2e-b || return 2
    sleep 0.5
    expand_in e2e-b "" ";only" || return 2
    sleep 1.5
    if ! sink_text_is e2e-b ";only"; then
        printf 'expanded in B: '
        describe e2e-b
        return 1
    fi
    activate e2e-a || return 2
    sleep 0.5
    expand_in e2e-a "" ";only" || return 2
    wait_until 30 sink_text_is e2e-a "only in A"
    result=$?
    printf 'A: %s; B: %s' "$(describe e2e-a)" "$(describe e2e-b)"
    return "$result"
}

slow_command_config() {
    write_config <<'EOF'
[[expansion]]
trigger = ";slow"
replacement = ""
[expansion.command]
program = "/bin/sh"
args = ["-c", "sleep 1.5; printf slow-output"]
timeout_ms = 5000
EOF
}

scenario_command_expansion_completes() {
    # Control for the two focus scenarios below: with focus left alone, the
    # same slow command does replace its trigger. Without this, "nothing
    # was typed" could pass merely because commands never type anything.
    slow_command_config
    start_sink e2e-a && start_daemon || return 2
    activate e2e-a || return 2
    sleep 0.5
    expand_in e2e-a "" ";slow" || return 2
    wait_until 50 sink_text_is e2e-a "slow-output"
    result=$?
    describe e2e-a
    return "$result"
}

scenario_focus_change_during_expansion() {
    # The trigger is typed in A and its command is still running when focus
    # moves to B. Nothing may be typed into B, and A must not be erased.
    slow_command_config
    start_sink e2e-a && start_sink e2e-b && start_daemon || return 2
    activate e2e-a || return 2
    sleep 0.5
    expand_in e2e-a "" ";slow" || return 2
    activate e2e-b || return 2
    sleep 3
    if sink_text_is e2e-a ";slow" && sink_text_is e2e-b ""; then
        result=0
    else
        result=1
    fi
    printf 'A: %s; B: %s' "$(describe e2e-a)" "$(describe e2e-b)"
    return "$result"
}

scenario_target_closes_during_expansion() {
    # A closes while its expansion's command runs, so focus falls to B.
    slow_command_config
    start_sink e2e-b && start_sink e2e-a && start_daemon || return 2
    activate e2e-a || return 2
    sleep 0.5
    expand_in e2e-a "" ";slow" || return 2
    kill "$(sink_pid e2e-a)" 2>/dev/null
    wait_until 30 sink_focused e2e-b || activate e2e-b || true
    sleep 3
    if ! daemon_alive; then
        printf 'daemon exited'
        return 1
    fi
    sink_text_is e2e-b ""
    result=$?
    printf 'B: %s' "$(describe e2e-b)"
    return "$result"
}

scenario_config_reload() {
    write_config <<'EOF'
[[expansion]]
trigger = ";rel"
replacement = "first"
EOF
    start_sink e2e-a && start_daemon || return 2
    expand_in e2e-a "" ";rel" || return 2
    wait_until 30 sink_text_is e2e-a "first" || {
        describe e2e-a
        return 1
    }
    write_config <<'EOF'
[[expansion]]
trigger = ";rel"
replacement = "second"
EOF
    # The watcher debounces for 100 ms and rebuilds off the reactor.
    sleep 1.5
    expand_in e2e-a "first" ";rel" || return 2
    wait_until 30 sink_text_is e2e-a "firstsecond"
    result=$?
    describe e2e-a
    return "$result"
}

scenario_daemon_restart() {
    write_config <<'EOF'
[[expansion]]
trigger = ";sig"
replacement = "Best regards"
EOF
    start_sink e2e-a && start_daemon || return 2
    expand_in e2e-a "" ";sig" || return 2
    wait_until 30 sink_text_is e2e-a "Best regards" || {
        describe e2e-a
        return 1
    }
    kill -9 "$daemon_pid" 2>/dev/null
    wait "$daemon_pid" 2>/dev/null
    exec 3>&-
    daemon_pid=
    start_daemon || return 2
    expand_in e2e-a "Best regards" ";sig" || return 2
    wait_until 30 sink_text_is e2e-a "Best regardsBest regards"
    result=$?
    describe e2e-a
    return "$result"
}

all_scenarios="expansion-replaces-trigger word-boundary-reinserts-delimiter cursor-placement
multiline-replacement unreachable-character-is-refused app-filter-follows-focus
command-expansion-completes focus-change-during-expansion target-closes-during-expansion config-reload daemon-restart"

# --- runner -------------------------------------------------------------

if ! wait_until 50 gdbus introspect --session --dest org.kde.KWin --object-path /Scripting >/dev/null 2>&1; then
    printf 'unverified session KWin did not appear on the private bus\n' >>"$results"
    exit 0
fi
relay_pid=
# Whatever happens, leave no process behind: KWin ends the session when
# this script exits, but background children would outlive it.
stop_everything() {
    end_case
    if [ -n "$relay_pid" ]; then
        kill "$relay_pid" 2>/dev/null || true
        wait "$relay_pid" 2>/dev/null || true
    fi
}
trap stop_everything EXIT
trap 'exit 1' INT TERM
"$bin/wx-e2e-eis-relay" "$LIBEI_SOCKET" >>"$work/relay.log" 2>&1 &
relay_pid=$!
if ! wait_until 50 test -S "$LIBEI_SOCKET"; then
    printf 'unverified session the EIS relay did not start\n' >>"$results"
    exit 0
fi

[ "$scenarios" = all ] && scenarios=$all_scenarios
for name in $scenarios; do
    function_name="scenario_$(printf '%s' "$name" | tr '-' '_')"
    if ! command -v "$function_name" >/dev/null 2>&1; then
        printf 'fail %s unknown scenario\n' "$name" >>"$results"
        continue
    fi
    begin_case "$name"
    # Run in this shell (not a $(...) subshell) so end_case can stop the
    # daemon and sinks the scenario started.
    "$function_name" >"$case_dir/detail" 2>>"$case_dir/scenario.log"
    code=$?
    end_case
    detail=$(cat "$case_dir/detail")
    case "$code" in
        0) outcome=pass ;;
        1) outcome=fail ;;
        *) outcome=unverified ;;
    esac
    printf '%s %s %s\n' "$outcome" "$name" "$detail" >>"$results"
done
