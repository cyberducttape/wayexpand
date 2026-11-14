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

sink_text_starts_with() {
    jq -e --arg prefix "$2" '(.text | type == "string") and (.text | startswith($prefix))' \
        "$case_dir/$1.json" >/dev/null 2>&1
}

sink_text_contains() {
    jq -e --arg needle "$2" '(.text | type == "string") and (.text | contains($needle))' \
        "$case_dir/$1.json" >/dev/null 2>&1
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

output_interruption_reported() {
    status=$(daemon_status) || return 1
    case "$status" in
        *"output_uncertain_total="[1-9]*) return 0 ;;
        *) return 1 ;;
    esac
}

output_limit_refusal_reported() {
    status=$(daemon_status) || return 1
    case "$status" in
        *"output_last_unsupported=too-long"*) return 0 ;;
        *) return 1 ;;
    esac
}

daemon_ready() {
    status=$(daemon_status) || return 1
    route_ready "$status"
}

write_config() {
    umask 077
    {
        printf '%s\n' '[settings]' "compatibility_mode = \"${e2e_compatibility_mode:?}\"" ''
        if [ -n "${e2e_keystroke_max_chars:-}" ]; then
            printf 'keystroke_max_chars = %s\n' "$e2e_keystroke_max_chars"
        fi
        if [ -n "${e2e_extra_settings:-}" ]; then
            printf '%s\n' "$e2e_extra_settings"
        fi
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
    unset WAYEXPAND_FORM_HELPER
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
trigger = ";s"
replacement = "Thanks"
EOF
    start_sink e2e-a && start_daemon || return 2
    expand_in e2e-a "" ";s" || return 2
    wait_until 30 sink_text_is e2e-a "Thanks"
    result=$?
    describe e2e-a
    return "$result"
}

scenario_batched_stdin_lines_are_not_quarantined() {
    # The stdin reader reads ahead. A line that was already queued before the
    # previous one started is not typing during output and must still expand.
    [ "$e2e_input_source" = stdin ] || {
        printf 'stdin-only scenario'
        return 2
    }
    write_config <<'EOF'
[[expansion]]
trigger = ";s"
replacement = "Thanks"
EOF
    start_sink e2e-a && start_daemon || return 2
    user_types e2e-a "x;s" "x;s" || return 2
    printf '%s\n%s\n' "x" ";s" >&3
    wait_until 50 sink_text_is e2e-a "xThanks"
    result=$?
    describe e2e-a
    return "$result"
}

scenario_output_latency() {
    status=''
    profiles=''
    latency_profiles=''
    small_profile=''
    small_replacement=x
    write_config <<EOF
[[expansion]]
trigger = ";s"
replacement = "$small_replacement"
EOF
    start_sink e2e-a && start_daemon || return 2
    latency_expected=''
    sample=0
    while [ "$sample" -lt 20 ]; do
        user_types e2e-a "${latency_expected};s" ";s" || return 1
        daemon_sees ";s"
        latency_expected="${latency_expected}${small_replacement}"
        wait_until 150 sink_text_is e2e-a "$latency_expected" || return 1
        sample=$((sample + 1))
    done
    attempts=0
    latency_profiles='[]'
    while [ "$attempts" -lt 50 ]; do
        status=$(daemon_status) || return 2
        profiles=$(printf '%s\n' "$status" | sed -n 's/^injection_latency_profiles=//p')
        latency_profiles=$(printf '%s' "$profiles" | jq -ce '
        [to_entries[] as $mode
         | "small" as $size
         | select($mode.value[$size].sample_count >= 20)
         | {
             backend_mode: $mode.key,
             size: $size,
             sample_count: $mode.value[$size].sample_count,
             window_count: $mode.value[$size].window_count,
             p50_us: $mode.value[$size].p50_us,
             p95_us: $mode.value[$size].p95_us,
             p99_us: $mode.value[$size].p99_us
           }]
        | sort_by(.size)
        ' 2>/dev/null) || latency_profiles='[]'
        if [ "$(printf '%s' "$latency_profiles" | jq 'length')" -ge 1 ]; then
            break
        fi
        sleep 0.1
        attempts=$((attempts + 1))
    done
    if [ "$(printf '%s' "$latency_profiles" | jq 'length')" -lt 1 ]; then
        printf 'small-output latency profile incomplete after %s polls: %s' "$attempts" "${profiles:-unavailable}"
        return 2
    fi
    small_profile=$(printf '%s' "$latency_profiles" | jq -ce '.[0]') || return 2
    printf 'small-output-latency-profile=%s ' "$small_profile"
    printf 'text_chars=%s cursor=%s' \
        "$(sink_field e2e-a text | wc -m | tr -d ' ')" \
        "$(sink_field e2e-a cursor)"
}

scenario_paced_output_within_raised_limit_is_typed() {
    # Raising keystroke_max_chars is explicit consent to long paced output;
    # no fixed duration budget may refuse it.
    saved_keystroke_max_chars=${e2e_keystroke_max_chars:-}
    e2e_keystroke_max_chars=250
    replacement=$(head -c 128 /dev/zero | tr '\000' A)
    write_config <<EOF
[[expansion]]
trigger = ";long"
replacement = "$replacement"
EOF
    e2e_keystroke_max_chars=$saved_keystroke_max_chars
    start_sink e2e-a && start_daemon || return 2
    expand_in e2e-a "" ";long" || return 2
    wait_until 100 sink_text_is e2e-a "$replacement"
    result=$?
    describe e2e-a
    return "$result"
}

scenario_paced_output_over_limit_refuses_before_erasing() {
    replacement=$(head -c 128 /dev/zero | tr '\000' A)
    # Use the production default limit of 8 characters.
    saved_keystroke_max_chars=${e2e_keystroke_max_chars:-}
    e2e_keystroke_max_chars=8
    write_config <<EOF
[[expansion]]
trigger = ";long"
replacement = "$replacement"
EOF
    e2e_keystroke_max_chars=$saved_keystroke_max_chars
    start_sink e2e-a && start_daemon || return 2
    status=$(daemon_status) || return 2
    case "$status" in
        *"inject_insertion_mode=libei keysym fallback"*) ;;
        *)
            printf 'route does not negotiate libei keysym fallback'
            return 2
            ;;
    esac

    expand_in e2e-a "" ";long" || return 2
    attempts=0
    while [ "$attempts" -lt 30 ]; do
        status=$(daemon_status) || return 2
        case "$status" in
            *"output_last_unsupported=too-long"*) break ;;
        esac
        sleep 0.1
        attempts=$((attempts + 1))
    done
    if [ "$attempts" -ge 30 ]; then
        printf 'paced output over keystroke_max_chars was not refused with the too-long diagnostic'
        return 1
    fi
    printf '%s\n' "$status" >"$case_dir/status-after-refusal.txt"
    uncertain_total=$(printf '%s\n' "$status" | sed -n 's/^output_uncertain_total=//p')
    partial_total=$(printf '%s\n' "$status" | sed -n 's/^output_partial_total=//p')
    if [ "${uncertain_total:-0}" -ne 0 ] || [ "${partial_total:-0}" -ne 0 ]; then
        printf 'preflight refusal unexpectedly recorded output as attempted or partial'
        return 1
    fi
    if ! sink_text_is e2e-a ";long"; then
        printf 'preflight refusal changed the target before output; '
        describe e2e-a
        return 1
    fi
    describe e2e-a
}

scenario_paced_refusal_preserves_follow_up_input() {
    # The default keystroke_max_chars (8) must refuse this long paced
    # replacement before deleting the trigger. Physical-keyboard overlap
    # remains a separate certification requirement; this headless test proves
    # only the preflight refusal and preservation of subsequent client input.
    replacement=$(head -c 128 /dev/zero | tr '\000' A)
    # Use the production default limit of 8 characters.
    saved_keystroke_max_chars=${e2e_keystroke_max_chars:-}
    e2e_keystroke_max_chars=8
    write_config <<EOF
[[expansion]]
trigger = ";long"
replacement = "$replacement"
[[expansion]]
trigger = ";next"
replacement = "SECOND-EXPANSION-MUST-NOT-RUN"
EOF
    e2e_keystroke_max_chars=$saved_keystroke_max_chars
    start_sink e2e-a && start_daemon || return 2
    status=$(daemon_status) || return 2
    case "$status" in
        *"inject_insertion_mode=libei keysym fallback"*) ;;
        *)
            printf 'route does not negotiate non-atomic libei keysym fallback'
            return 2
            ;;
    esac

    expand_in e2e-a "" ";long" || return 2
    wait_until 30 output_limit_refusal_reported || {
        printf 'long paced output was not refused before erasing; '
        describe e2e-a
        return 1
    }
    status=$(daemon_status) || return 2
    printf '%s\n' "$status" >"$case_dir/status-after-limit-refusal.txt"
    uncertain_total=$(printf '%s\n' "$status" | sed -n 's/^output_uncertain_total=//p')
    partial_total=$(printf '%s\n' "$status" | sed -n 's/^output_partial_total=//p')
    unsupported=$(printf '%s\n' "$status" | sed -n 's/^output_last_unsupported=//p')
    if [ "$unsupported" != too-long ] ||
        [ "${uncertain_total:-0}" -ne 0 ] || [ "${partial_total:-0}" -ne 0 ]; then
        printf 'preflight refusal did not preserve a certain, unmodified outcome; '
        printf 'reason=%s uncertain=%s partial=%s ' "$unsupported" \
            "${uncertain_total:-unavailable}" "${partial_total:-unavailable}"
        return 1
    fi
    if ! sink_text_is e2e-a ";long"; then
        printf 'limit refusal changed the target before output; '
        describe e2e-a
        return 1
    fi

    "$bin/wx-e2e-type" text x >>"$case_dir/type.log" 2>&1 || return 2
    daemon_sees x || return 2
    if ! sink_text_is e2e-a ";longx"; then
        printf 'input after a refused replacement was not preserved; '
        describe e2e-a
        return 1
    fi
    final_status=$(daemon_status) || return 2
    final_uncertain=$(printf '%s\n' "$final_status" | sed -n 's/^output_uncertain_total=//p')
    final_partial=$(printf '%s\n' "$final_status" | sed -n 's/^output_partial_total=//p')
    if [ "${final_uncertain:-0}" -ne 0 ] || [ "${final_partial:-0}" -ne 0 ]; then
        printf 'typing after a preflight refusal unexpectedly changed output integrity counters'
        return 1
    fi
    describe e2e-a
}

scenario_keystroke_route_refuses_without_consent() {
    previous_mode=$e2e_compatibility_mode
    e2e_compatibility_mode=off
    # Read by route_ready in the compositor session driver.
    # shellcheck disable=SC2034
    e2e_expect_blocked_route=1
    write_config <<'EOF'
[[expansion]]
trigger = ";s"
replacement = "no"
EOF
    result=2
    if start_sink e2e-a && start_daemon && expand_in e2e-a "" ";s"; then
        if wait_until 30 refusal_is_reported; then
            status=$(daemon_status)
            case "$status" in
                *"output_uncertain_total=0"*"insertion_assurance=blocked"*"output_refused_total="[1-9]*"output_last_refusal=compatibility-mode-off"*)
                    if sink_text_is e2e-a ";s"; then
                        result=0
                    else
                        result=1
                    fi
                    ;;
                *) result=1 ;;
            esac
        else
            daemon_status >"$case_dir/status-after-refusal.txt" 2>&1 || true
            result=1
        fi
    fi
    e2e_compatibility_mode=$previous_mode
    unset e2e_expect_blocked_route
    describe e2e-a
    return "$result"
}

refusal_is_reported() {
    status=$(daemon_status) || return 1
    case "$status" in
        *"output_uncertain_total=0"*"output_refused_total="[1-9]*"output_last_refusal=compatibility-mode-off"*) return 0 ;;
        *) return 1 ;;
    esac
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
replacement = "OK"
match_mode = "word-boundary"
EOF
    start_sink e2e-a && start_daemon || return 2
    expand_in e2e-a "" "so btw " || return 2
    wait_until 30 sink_text_is e2e-a "so OK "
    result=$?
    describe e2e-a
    return "$result"
}

scenario_cursor_placement() {
    write_config <<'EOF'
[[expansion]]
trigger = ";t"
replacement = "a{{cursor}}b"
EOF
    start_sink e2e-a && start_daemon || return 2
    expand_in e2e-a "" ";t" || return 2
    wait_until 30 sink_text_is e2e-a "ab" &&
        wait_until 10 sink_cursor_is e2e-a 1
    result=$?
    describe e2e-a
    return "$result"
}

scenario_multiline_replacement() {
    write_config <<'EOF'
[[expansion]]
trigger = ";m"
replacement = "x\ny"
EOF
    start_sink e2e-a && start_daemon || return 2
    expand_in e2e-a "" ";m" || return 2
    expected=$(printf 'x\ny')
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
trigger = ";o"
replacement = "A"
app_filter = ["e2e-a"]
EOF
    start_sink e2e-a && start_sink e2e-b && start_daemon || return 2
    activate e2e-b || return 2
    sleep 0.5
    expand_in e2e-b "" ";o" || return 2
    sleep 1.5
    if ! sink_text_is e2e-b ";o"; then
        printf 'expanded in B: '
        describe e2e-b
        return 1
    fi
    activate e2e-a || return 2
    sleep 0.5
    expand_in e2e-a "" ";o" || return 2
    wait_until 30 sink_text_is e2e-a "A"
    result=$?
    printf 'A: %s; B: %s' "$(describe e2e-a)" "$(describe e2e-b)"
    return "$result"
}

scenario_app_filter_fails_closed() {
    # The filter names another application. The focused app must not receive
    # the expansion, even when the window tracker is connected and reports it.
    # This exercises the production fail-closed filter decision rather than
    # relying on a test-only absence of the tracker.
    write_config <<'EOF'
[[expansion]]
trigger = ";o"
replacement = "A"
app_filter = ["e2e-b"]
EOF
    start_sink e2e-a && start_daemon || return 2
    activate e2e-a || return 2
    expand_in e2e-a "" ";o" || return 2
    sleep 1.5
    sink_text_is e2e-a ";o"
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
args = ["-c", "sleep 1.5; printf ok"]
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
    wait_until 50 sink_text_is e2e-a "ok"
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
trigger = ";r"
replacement = "a"
EOF
    start_sink e2e-a && start_daemon || return 2
    expand_in e2e-a "" ";r" || return 2
    wait_until 30 sink_text_is e2e-a "a" || {
        describe e2e-a
        return 1
    }
    write_config <<'EOF'
[[expansion]]
trigger = ";r"
replacement = "b"
EOF
    # The watcher debounces for 100 ms and rebuilds off the reactor.
    sleep 1.5
    expand_in e2e-a "a" ";r" || return 2
    wait_until 30 sink_text_is e2e-a "ab"
    result=$?
    describe e2e-a
    return "$result"
}

scenario_daemon_restart() {
    write_config <<'EOF'
[[expansion]]
trigger = ";s"
replacement = "Thanks"
EOF
    start_sink e2e-a && start_daemon || return 2
    expand_in e2e-a "" ";s" || return 2
    wait_until 30 sink_text_is e2e-a "Thanks" || {
        describe e2e-a
        return 1
    }
    kill -9 "$daemon_pid" 2>/dev/null
    wait "$daemon_pid" 2>/dev/null
    exec 3>&-
    daemon_pid=
    start_daemon || return 2
    expand_in e2e-a "Thanks" ";s" || return 2
    wait_until 30 sink_text_is e2e-a "ThanksThanks"
    result=$?
    describe e2e-a
    return "$result"
}

# --- clipboard and forms -----------------------------------------------

clipboard_config() {
    e2e_extra_settings='allow_clipboard = true
clipboard_mode = "fresh"'
    write_config <<'EOF'
[[expansion]]
trigger = ";q"
replacement = "{{clipboard}}"
EOF
    e2e_extra_settings=
}

session_clipboard_is() {
    [ "$(wl-paste --type='text/plain;charset=utf-8' --no-newline 2>/dev/null)" = "$1" ]
}

scenario_clipboard_variable() {
    # The daemon reads the private session's clipboard with wl-paste at
    # render time and types it in place of the trigger.
    command -v wl-copy >/dev/null 2>&1 || {
        printf 'wl-copy is not installed'
        return 2
    }
    clipboard_config
    start_sink e2e-a && start_daemon || return 2
    activate e2e-a || return 2
    printf 'hello' | wl-copy --type='text/plain;charset=utf-8' \
        2>>"$case_dir/clipboard.log" || {
        printf 'wl-copy could not set the session clipboard'
        return 2
    }
    wait_until 30 session_clipboard_is "hello" || {
        printf 'the session clipboard did not report the copied text'
        return 2
    }
    activate e2e-a || return 2
    expand_in e2e-a "" ";q" || return 2
    wait_until 50 sink_text_is e2e-a "hello"
    result=$?
    if [ "$result" -ne 0 ]; then
        # Capture counters and outcome categories without collecting clipboard
        # or snippet contents. This distinguishes a clipboard read failure
        # from a trigger that never reached the matcher.
        daemon_status >"$case_dir/daemon-status-after.txt" 2>&1 || true
    fi
    describe e2e-a
    return "$result"
}

scenario_clipboard_empty_fails_closed() {
    # With nothing on the clipboard the snippet must not expand: the trigger
    # stays and nothing is erased or half-typed.
    command -v wl-copy >/dev/null 2>&1 || {
        printf 'wl-copy is not installed'
        return 2
    }
    clipboard_config
    start_sink e2e-a && start_daemon || return 2
    wl-copy --clear 2>>"$case_dir/clipboard.log" || true
    activate e2e-a || return 2
    expand_in e2e-a "" ";q" || return 2
    sleep 1.5
    sink_text_is e2e-a ";q"
    result=$?
    describe e2e-a
    return "$result"
}

# A form helper that behaves like the real dialog towards the compositor: it
# maps its own toplevel, which takes focus from the origin window, then
# answers and closes. While form-release exists in the case directory it
# waits for that file to say "go" before closing, so a scenario can move
# focus while the form is open.
write_form_helper() {
    helper="$case_dir/form-helper"
    cat >"$helper" <<EOF
#!/bin/sh
"$bin/wx-e2e-sink" e2e-form "$case_dir/e2e-form.json" >>"$case_dir/e2e-form.log" 2>&1 &
form_pid=\$!
tries=50
while [ "\$tries" -gt 0 ]; do
    [ "\$(jq -r .focused "$case_dir/e2e-form.json" 2>/dev/null)" = true ] && break
    sleep 0.1
    tries=\$((tries - 1))
done
printf '%s\n' "\$tries" >"$case_dir/form-focus-tries"
if [ -e "$case_dir/form-release" ]; then
    tries=100
    while [ "\$tries" -gt 0 ] && [ "\$(cat "$case_dir/form-release")" != go ]; do
        sleep 0.1
        tries=\$((tries - 1))
    done
fi
kill "\$form_pid" 2>/dev/null
wait "\$form_pid" 2>/dev/null
printf '%s' '{"field:name":"Ada"}'
EOF
    chmod 700 "$helper"
    export WAYEXPAND_FORM_HELPER="$helper"
}

form_helper_took_focus() {
    [ -s "$case_dir/form-focus-tries" ] && [ "$(cat "$case_dir/form-focus-tries")" -gt 0 ]
}

form_config() {
    write_config <<'EOF'
[[expansion]]
trigger = "frm"
replacement = "Hello {{field:name|required}}"
EOF
}

scenario_form_returns_to_origin() {
    # Forms open only on routes that verify the trigger at the cursor, so
    # this runs on the input-method route. The form window takes focus; when
    # it closes the compositor returns focus to the origin toplevel, and the
    # rendered form must replace the trigger there.
    write_form_helper
    form_config
    start_sink e2e-a && start_daemon || return 2
    wait_until 50 sink_text_input_active e2e-a || return 2
    expand_in e2e-a "" "frm" || return 2
    wait_until 100 sink_text_is e2e-a "Hello Ada"
    result=$?
    if ! form_helper_took_focus; then
        printf 'form window never took focus; '
        describe e2e-a
        return 2
    fi
    describe e2e-a
    return "$result"
}

scenario_form_focus_moved_fails_closed() {
    # Focus goes to another window while the form is open. The completed
    # form must not be typed into that window, nor into the origin later.
    write_form_helper
    printf wait >"$case_dir/form-release"
    form_config
    start_sink e2e-b && start_sink e2e-a && start_daemon || return 2
    activate e2e-a || return 2
    wait_until 50 sink_text_input_active e2e-a || return 2
    expand_in e2e-a "" "frm" || return 2
    if ! wait_until 80 form_helper_took_focus; then
        printf go >"$case_dir/form-release"
        printf 'form window never took focus; '
        describe e2e-a
        return 2
    fi
    if ! activate e2e-b; then
        printf go >"$case_dir/form-release"
        printf 'could not move focus to B'
        return 2
    fi
    printf go >"$case_dir/form-release"
    sleep 2
    result=0
    sink_text_contains e2e-a "Ada" && result=1
    sink_text_contains e2e-b "Ada" && result=1
    printf 'A: %s; B: %s' "$(describe e2e-a)" "$(describe e2e-b)"
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
        if "$function_name" >"$case_dir/detail" 2>>"$case_dir/scenario.log"; then
            code=0
        else
            code=$?
        fi
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
