# shellcheck shell=sh
# Shared helpers and scenarios for the headless compositor end-to-end
# harnesses (scripts/kwin-e2e-session.sh, scripts/sway-e2e-session.sh).
#
# Sourced by a session script, which runs inside a private compositor that
# its outer runner (scripts/test-*-e2e.sh) started; never run any of it in a
# real desktop session. Each scenario starts fresh e2e sink windows and a
# WayExpand daemon whose capture is --source=stdin (the harness feeds what
# the "user" typed); the session script picks the production output backend
# and focus mode. The "user" types into the focused sink with wx-e2e-type,
# and the sinks record exactly the text the compositor delivered to them.
#
# The session script must define, before calling run_scenarios:
#   e2e_output_backend      --backend= value for the daemon
#   e2e_compatibility_mode  [settings] compatibility_mode for the config
#   activate APP_ID         focus the sink window APP_ID
#   route_ready STATUS      true once the daemon status shows its route up
#
# Results go to $E2E_RESULTS, one line per scenario: "<result> <name>
# <detail>", where result is pass, fail, or unverified.

bin=${E2E_BIN:?}
app_bin=${E2E_APP_BIN:-$bin}
work=${E2E_WORK:?}
results=${E2E_RESULTS:?}
scenarios=${E2E_SCENARIOS:-all}
e2e_input_source=${E2E_INPUT_SOURCE:-stdin}
stdin_open=0

export WAYEXPAND_SOCKET="$XDG_RUNTIME_DIR/wayexpand.sock"
# Keep the active configuration under the private runtime directory when the
# installed harness provides one. Retained logs may live in XDG_CACHE_HOME,
# whose ancestors are not necessarily trusted for the daemon's config policy.
export WAYEXPAND_CONFIG="${E2E_CONFIG:-$work/expansions.toml}"

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

sink_text_input_active() {
    [ "$(sink_field "$1" text_input_active)" = true ]
}

start_sink() {
    if [ "$e2e_input_source" = input-method ]; then
        "$bin/wx-e2e-sink" "$1" "$case_dir/$1.json" --text-input >>"$case_dir/$1.log" 2>&1 &
    else
        "$bin/wx-e2e-sink" "$1" "$case_dir/$1.json" >>"$case_dir/$1.log" 2>&1 &
    fi
    sink_pids="$sink_pids $!"
    eval "pid_$(printf '%s' "$1" | tr -c 'a-z0-9\n' _)=$!"
    wait_until 50 test -s "$case_dir/$1.json"
}

start_password_sink() {
    "$bin/wx-e2e-sink" "$1" "$case_dir/$1.json" --password \
        >>"$case_dir/$1.log" 2>&1 &
    sink_pids="$sink_pids $!"
    eval "pid_$(printf '%s' "$1" | tr -c 'a-z0-9\n' _)=$!"
    wait_until 50 test -s "$case_dir/$1.json"
}

sink_pid() {
    eval "printf '%s' \"\$pid_$(printf '%s' "$1" | tr -c 'a-z0-9\n' _)\""
}

# Type as the user into the focused window, and wait until $1 shows $2.
user_types() {
    sink=$1
    expected=$2
    text=$3
    if [ "$e2e_input_source" = input-method ]; then
        "$bin/wx-e2e-type" keys "$text" >>"$case_dir/type.log" 2>&1
    else
        "$bin/wx-e2e-type" text "$text" >>"$case_dir/type.log" 2>&1 &&
            wait_until 30 sink_text_is "$sink" "$expected"
    fi
}

daemon_status() {
    "$app_bin/wayexpand" status 2>/dev/null
}

daemon_ready() {
    status=$(daemon_status) || return 1
    route_ready "$status"
}

write_config() {
    umask 077
    {
        printf '%s\n' '[settings]' "compatibility_mode = \"${e2e_compatibility_mode:?}\"" ''
        cat
    } >"$WAYEXPAND_CONFIG.tmp"
    mv "$WAYEXPAND_CONFIG.tmp" "$WAYEXPAND_CONFIG"
}

start_daemon() {
    if [ "$e2e_input_source" = stdin ]; then
        rm -f "$case_dir/stdin"
        mkfifo -m 600 "$case_dir/stdin"
        "$app_bin/wayexpand-daemon" --source=stdin --backend="${e2e_output_backend:?}" \
            <"$case_dir/stdin" >>"$case_dir/daemon.log" 2>&1 &
        daemon_pid=$!
        exec 3>"$case_dir/stdin"
        stdin_open=1
    else
        "$app_bin/wayexpand-daemon" --source=input-method \
            >>"$case_dir/daemon.log" 2>&1 &
        daemon_pid=$!
    fi
    if ! wait_until 100 daemon_ready; then
        status=$(daemon_status | tr '\n' ' ' | cut -c 1-512)
        [ -n "$status" ] || status=unavailable
        reason='daemon did not report a ready route'
        if [ "$e2e_input_source" = input-method ] &&
            grep -q 'input-method-v2 manager and seat probe failed' "$case_dir/daemon.log"; then
            reason='compositor input-method-v2 manager/seat probe unavailable'
        fi
        log "daemon never became ready: $status"
        printf 'daemon readiness timed out: %s; status=%s\n' "$reason" "$status"
        return 1
    fi
    daemon_status >"$work/runtime-status"
}

stop_daemon() {
    if [ "$stdin_open" -eq 1 ]; then
        exec 3>&-
        stdin_open=0
    fi
    if [ -n "$daemon_pid" ]; then
        kill "$daemon_pid" 2>/dev/null || true
        wait "$daemon_pid" 2>/dev/null || true
    fi
    daemon_pid=
}

# What the daemon's capture would have seen: one stdin line.
daemon_sees() {
    if [ "$e2e_input_source" = stdin ]; then
        printf '%s\n' "$1" >&3
    fi
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

scenario_output_latency() {
    latency_expected=''
    sample=0
    status=''
    profiles=''
    profile=''
    write_config <<'EOF'
[[expansion]]
trigger = ";lat"
replacement = "x"
EOF
    start_sink e2e-a && start_daemon || return 2
    while [ "$sample" -lt 20 ]; do
        user_types e2e-a "${latency_expected};lat" ";lat" || return 1
        daemon_sees ";lat"
        latency_expected="${latency_expected}x"
        wait_until 50 sink_text_is e2e-a "$latency_expected" || return 1
        sample=$((sample + 1))
    done
    status=$(daemon_status) || return 2
    profiles=$(printf '%s\n' "$status" | sed -n 's/^injection_latency_profiles=//p')
    profile=$(printf '%s' "$profiles" | jq -ce '
        to_entries[]
        | select(.value.small.sample_count >= 20)
        | {
            backend_mode: .key,
            size: "small",
            sample_count: .value.small.sample_count,
            window_count: .value.small.window_count,
            p50_us: .value.small.p50_us,
            p95_us: .value.small.p95_us,
            p99_us: .value.small.p99_us
          }
    ' | head -n 1) || return 2
    [ -n "$profile" ] || return 2
    printf 'latency-profile=%s ' "$profile"
    describe e2e-a
}

scenario_input_method_expansion() {
    write_config <<'EOF'
[[expansion]]
trigger = "sig"
replacement = "signature"
EOF
    start_sink e2e-a && start_daemon || return 2
    wait_until 50 sink_text_input_active e2e-a || return 2
    expand_in e2e-a "" "sig" || return 2
    wait_until 50 sink_text_is e2e-a "signature"
    result=$?
    describe e2e-a
    return "$result"
}

scenario_input_method_password_field() {
    write_config <<'EOF'
[[expansion]]
trigger = "sig"
replacement = "signature"
EOF
    start_password_sink e2e-a && start_daemon || return 2
    wait_until 50 sink_text_input_active e2e-a || return 2
    expand_in e2e-a "" "sig" || return 2
    wait_until 50 sink_text_is e2e-a "sig"
    result=$?
    describe e2e-a
    return "$result"
}

scenario_unicode_insertion() {
    # Only claim Unicode insertion when the negotiated production route
    # promises it (direct UTF-8 text or full Unicode). Other routes are
    # reported as unverified; their refusal has its own scenario.
    write_config <<'EOF'
[[expansion]]
trigger = ";unicode"
replacement = "café 東京"
EOF
    start_sink e2e-a && start_daemon || return 2
    status=$(daemon_status)
    mode=$(printf '%s\n' "$status" | sed -n 's/^inject_insertion_mode=//p')
    case "$mode:$status" in
        *ei_text*:* | *:*"inject_full_unicode=true"*) ;;
        *)
            printf 'route=%s; full Unicode output not offered' "${mode:-unknown}"
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
    # Neither headless route can guarantee "é": KWin's EIS offers no
    # ei_text on its default US layout, and the wlroots route refuses text
    # outside its guarantee. WayExpand must refuse before it erases anything
    # rather than type something else.
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

scenario_app_filter_fails_closed() {
    # Without a window tracker the daemon cannot know which app is focused,
    # so an app_filter snippet must not expand even in its own app.
    # expansion-replaces-trigger is the control: plain snippets do expand.
    write_config <<'EOF'
[[expansion]]
trigger = ";only"
replacement = "only in A"
app_filter = ["e2e-a"]
EOF
    start_sink e2e-a && start_daemon || return 2
    activate e2e-a || return 2
    expand_in e2e-a "" ";only" || return 2
    sleep 1.5
    sink_text_is e2e-a ";only"
    result=$?
    describe e2e-a
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

# --- runner -------------------------------------------------------------

# run_scenarios ALL: run $E2E_SCENARIOS, or every scenario in ALL.
run_scenarios() {
    [ "$scenarios" = all ] && scenarios=$1
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
}
