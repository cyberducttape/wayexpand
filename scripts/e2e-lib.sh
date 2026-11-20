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
    # The lifecycle soak closes and reopens these sinks in the same case
    # directory. Remove the previous instance's state before launching so the
    # readiness wait below cannot pass on stale JSON before the new window
    # exists.
    rm -f "$case_dir/$1.json"
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

expansion_rejection_reported() {
    status=$(daemon_status) || return 1
    rejected=$(printf '%s\n' "$status" | sed -n 's/^expansions_rejected_total=//p')
    [ "${rejected:-0}" -gt 0 ]
}

daemon_ready() {
    status=$(daemon_status) || return 1
    route_ready "$status"
}

terminal_paste_active() {
    status=$(daemon_status) || return 1
    printf '%s\n' "$status" | grep -Fxq 'terminal_paste=multiline'
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

file_has_at_least() {
    [ "$(wc -c <"$1")" -ge "$2" ]
}

scenario_terminal_paste_multiline() {
    # Opt-in: needs Konsole. A multi-line replacement in a listed terminal
    # arrives through the clipboard and Ctrl+Shift+V, byte for byte, and the
    # previous clipboard text is restored afterwards.
    if ! command -v konsole >/dev/null 2>&1 || ! command -v wl-copy >/dev/null 2>&1; then
        printf 'konsole or wl-copy is not installed'
        return 2
    fi
    e2e_extra_settings='terminal_apps = ["org.kde.konsole"]
terminal_paste = "multiline"'
    write_config <<'EOF'
[[expansion]]
trigger = ";ml"
replacement = "first line\nsecond line ünïcode"
EOF
    e2e_extra_settings=''
    out="$case_dir/terminal.bin"
    : >"$out"
    recorder="$case_dir/record.sh"
    printf '#!/bin/sh\nstty -icanon -echo min 1\nexec cat > "%s"\n' "$out" >"$recorder"
    chmod +x "$recorder"
    QT_QPA_PLATFORM=wayland konsole --nofork --separate -e "$recorder" \
        >>"$case_dir/konsole.log" 2>&1 &
    konsole_pid=$!
    sink_pids="$sink_pids $konsole_pid"
    sleep 4
    kill -0 "$konsole_pid" 2>/dev/null || {
        printf 'Konsole did not start: %s' "$(tail -n 1 "$case_dir/konsole.log" | cut -c 1-200)"
        return 2
    }
    printf 'keep-me' | wl-copy --type='text/plain;charset=utf-8' 2>>"$case_dir/clipboard.log"
    wait_until 30 session_clipboard_is "keep-me" || {
        printf 'could not seed the session clipboard'
        return 2
    }
    # activate() confirms focus through the test sink, which Konsole is
    # not; Konsole is the only window, so its focus is best effort here.
    activate org.kde.konsole || :
    start_daemon || return 2
    if ! wait_until 50 terminal_paste_active; then
        status=$(daemon_status 2>/dev/null || true)
        printf '%s\n' "$status" >"$case_dir/status-before-input.txt"
        printf 'WayExpand did not confirm Konsole paste mode before input'
        return 2
    fi
    "$bin/wx-e2e-type" text ";ml" >>"$case_dir/type.log" 2>&1 || return 2
    sleep 0.3
    daemon_sees ";ml"
    expected="first line
second line ünïcode"
    # Trigger, three DEL bytes erasing it, then the pasted text.
    expected_len=$(( $(printf '%s' "$expected" | wc -c) + 6 ))
    wait_until 50 file_has_at_least "$out" "$expected_len"
    sleep 0.5
    tail -c +7 "$out" >"$case_dir/got.txt"
    printf '%s' "$expected" >"$case_dir/expected.txt"
    status=$(daemon_status 2>/dev/null || true)
    printf '%s\n' "$status" >"$case_dir/status-after-input.txt"
    if ! cmp -s "$case_dir/expected.txt" "$case_dir/got.txt"; then
        paste_mode=$(printf '%s\n' "$status" | sed -n 's/^terminal_paste=//p')
        pasted=$(printf '%s\n' "$status" | sed -n 's/^terminal_paste_total=//p')
        fallbacks=$(printf '%s\n' "$status" | sed -n 's/^terminal_paste_fallbacks_total=//p')
        unread=$(printf '%s\n' "$status" | sed -n 's/^terminal_paste_unread_total=//p')
        printf 'pasted bytes differ: expected=%s got=%s (paste_mode=%s pasted=%s fallbacks=%s unread=%s)' \
            "$(wc -c <"$case_dir/expected.txt")" "$(wc -c <"$case_dir/got.txt")" \
            "${paste_mode:-unknown}" "${pasted:-unknown}" "${fallbacks:-unknown}" "${unread:-unknown}"
        return 1
    fi
    wait_until 30 session_clipboard_is "keep-me" || {
        printf 'the previous clipboard text was not restored'
        return 1
    }
    printf 'pasted=%s bytes; clipboard restored' "$(wc -c <"$case_dir/got.txt")"
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

scenario_paced_output_raised_limit_applies() {
    # An explicitly raised limit allows paced replacement after the normal
    # compatibility and focus checks. This headless sink verifies final text;
    # it does not certify physical-keyboard overlap behavior.
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
    wait_until 100 sink_text_is e2e-a "$replacement" || {
        printf 'configured paced replacement did not reach the sink; '
        describe e2e-a
        return 1
    }
    status=$(daemon_status) || return 2
    uncertain_total=$(printf '%s\n' "$status" | sed -n 's/^output_uncertain_total=//p')
    partial_total=$(printf '%s\n' "$status" | sed -n 's/^output_partial_total=//p')
    if [ "${uncertain_total:-0}" -ne 0 ] || [ "${partial_total:-0}" -ne 0 ]; then
        printf 'completed sink insertion reported an uncertain or partial outcome'
        return 1
    fi
    describe e2e-a
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
    wait_until 30 expansion_rejection_reported || {
        printf 'replacement over the configured non-atomic limit was not rejected'
        return 1
    }
    status=$(daemon_status) || return 2
    printf '%s\n' "$status" >"$case_dir/status-after-refusal.txt"
    uncertain_total=$(printf '%s\n' "$status" | sed -n 's/^output_uncertain_total=//p')
    partial_total=$(printf '%s\n' "$status" | sed -n 's/^output_partial_total=//p')
    rejected=$(printf '%s\n' "$status" | sed -n 's/^expansions_rejected_total=//p')
    if [ "${rejected:-0}" -lt 1 ] ||
        [ "${uncertain_total:-0}" -ne 0 ] || [ "${partial_total:-0}" -ne 0 ]; then
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
    wait_until 30 expansion_rejection_reported || {
        printf 'long paced output was not refused before erasing; '
        describe e2e-a
        return 1
    }
    status=$(daemon_status) || return 2
    printf '%s\n' "$status" >"$case_dir/status-after-limit-refusal.txt"
    uncertain_total=$(printf '%s\n' "$status" | sed -n 's/^output_uncertain_total=//p')
    partial_total=$(printf '%s\n' "$status" | sed -n 's/^output_partial_total=//p')
    rejected=$(printf '%s\n' "$status" | sed -n 's/^expansions_rejected_total=//p')
    if [ "${rejected:-0}" -lt 1 ] ||
        [ "${uncertain_total:-0}" -ne 0 ] || [ "${partial_total:-0}" -ne 0 ]; then
        printf 'preflight refusal did not preserve a certain, unmodified outcome; '
        printf 'rejected=%s uncertain=%s partial=%s ' "${rejected:-unavailable}" \
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
    if ! wait_until 30 sink_text_is e2e-a ";longx"; then
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
    # KWin's EIS offers no ei_text, and its default US layout has no key for
    # "é". WayExpand must refuse before it erases anything rather than type
    # something else. (The wlroots route types it through its own keymap, so
    # Sway runs unicode-insertion instead.)
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

# --- keyboard layouts ---------------------------------------------------
# Each runs in a session started with that layout (E2E_LAYOUT, as
# LAYOUTS[:VARIANTS]); the session driver defines compositor_layouts_are
# LAYOUTS (the compositor reports exactly these layouts) and switch_layout
# (advance to the next configured layout). Each expects exact output: a
# layout-dependent route must type the replacement with the active keymap
# or refuse before erasing anything; anything else is a failure.

# require_layout LAYOUTS[:VARIANTS]
require_layout() {
    if [ "${E2E_LAYOUT:-}" != "$1" ]; then
        printf 'needs a session started with --layout %s' "$1"
        return 1
    fi
    command -v compositor_layouts_are >/dev/null 2>&1 || {
        printf 'this session cannot confirm the compositor keyboard layout'
        return 1
    }
    wait_until 50 compositor_layouts_are "${1%%:*}" || {
        printf 'the compositor did not report the %s layout' "${1%%:*}"
        return 1
    }
}

# expect_layout_output SINK TEXT_BEFORE_EXPANSION EXPECTED: the replacement
# appeared exactly (0), the route refused and left the text untouched (2),
# or anything else (1).
expect_layout_output() {
    if wait_until 80 sink_text_is "$1" "$3"; then
        return 0
    fi
    if sink_text_is "$1" "$2" && expansion_rejection_reported; then
        # Both labels are content-free status values.
        status=$(daemon_status)
        printf 'route refused (%s/%s); ' \
            "$(printf '%s\n' "$status" | sed -n 's/^output_last_unsupported=//p')" \
            "$(printf '%s\n' "$status" | sed -n 's/^output_last_refusal=//p')"
        return 2
    fi
    return 1
}

# toml_string TEXT: TEXT as a TOML basic string.
toml_string() {
    printf '"%s"' "$(printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g')"
}

# layout_case LAYOUTS[:VARIANTS] REPLACEMENT [TRIGGER]
layout_case() {
    require_layout "$1" || return 2
    case_trigger=${3:-";lt"}
    printf '[[expansion]]\ntrigger = %s\nreplacement = %s\n' \
        "$(toml_string "$case_trigger")" "$(toml_string "$2")" | write_config
    start_sink e2e-a && start_daemon || return 2
    expand_in e2e-a "" "$case_trigger" || {
        printf 'the typist could not type the trigger on %s; ' "$1"
        describe e2e-a
        return 2
    }
    expect_layout_output e2e-a "$case_trigger" "$2"
    result=$?
    printf 'layout=%s ' "$1"
    describe e2e-a
    return "$result"
}

# expect_layout_refusal SINK TEXT_BEFORE_EXPANSION: the route refused and
# left the text exactly as it was.
expect_layout_refusal() {
    sleep 1.5
    if sink_text_is "$1" "$2" && expansion_rejection_reported; then
        return 0
    fi
    return 1
}

# layout_refusal_case LAYOUTS[:VARIANTS] REPLACEMENT [TRIGGER]
layout_refusal_case() {
    require_layout "$1" || return 2
    case_trigger=${3:-";lt"}
    printf '[[expansion]]\ntrigger = %s\nreplacement = %s\n' \
        "$(toml_string "$case_trigger")" "$(toml_string "$2")" | write_config
    start_sink e2e-a && start_daemon || return 2
    expand_in e2e-a "" "$case_trigger" || {
        printf 'the typist could not type the trigger on %s; ' "$1"
        describe e2e-a
        return 2
    }
    expect_layout_refusal e2e-a "$case_trigger"
    result=$?
    printf 'layout=%s refusal expected; ' "$1"
    describe e2e-a
    return "$result"
}

# Letters that move between US and German, and ASCII symbols that German
# reaches only through AltGr (level 3).
scenario_layout_de_altgr_ascii() {
    layout_case de 'zy@{}[]|~\ Zy'
}

# "^" and "`" exist only as dead keys on the German layout, so a keystroke
# route cannot type them without composing; it must refuse rather than leave
# a stray accent. (If a later route composes them exactly, change this to
# layout_case and update docs/SUPPORT_MATRIX.md.)
scenario_layout_de_dead_key_only_ascii_is_refused() {
    layout_refusal_case de 'a^b`c'
}

# Non-ASCII letters and symbols the active layout reaches directly, including
# through AltGr (€), are typed exactly on a layout-checked keystroke route.
scenario_layout_de_umlauts() {
    layout_case de "Grüße, äöü ÄÖÜ ß €"
}

# "ê" is reachable only through a dead key on the German layout; the route
# refuses it rather than composing.
scenario_layout_de_dead_key_letter_is_refused() {
    layout_refusal_case de "rêve"
}

# AZERTY moves letters and puts digits on the shifted level.
scenario_layout_fr_azerty() {
    layout_case fr "azqwm AZQWM 1234 !?;: éèàçù"
}

# us(altgr-intl) makes ' " ` ~ and ^ dead keys on the base level.
scenario_layout_altgr_intl_dead_keys() {
    layout_case us:altgr-intl 'don'"'"'t "say" ~^`'
}

# us(altgr-intl) reaches accented letters through AltGr.
scenario_layout_altgr_intl_accents() {
    layout_case us:altgr-intl "äéñßøå ¡¿"
}

# us(intl), "US International with dead keys", is the common choice for
# typing accents on a US keyboard. Its base-level ' " ` ~ and ^ are dead
# keys, so everyday text such as "don't" must still arrive exactly.
scenario_layout_us_intl_punctuation() {
    layout_case us:intl 'don'"'"'t "say" ~^`'
}

# us(intl) reaches common accented letters through AltGr.
scenario_layout_us_intl_accents() {
    layout_case us:intl "äéñßøå ¡¿"
}

# A non-Latin layout types its own script and the punctuation and digits it
# has. The trigger avoids Latin letters, which this layout cannot type.
scenario_layout_ru_cyrillic() {
    layout_case ru "Привет, мир 1;2" ";12"
}

# ...and refuses Latin letters instead of typing whatever shares the key.
scenario_layout_ru_latin_is_refused() {
    layout_refusal_case ru "hello" ";12"
}

# Two configured layouts: the route must learn the active group from the
# compositor, type correctly in it, and follow a switch while running.
scenario_layout_multi_layout_switch() {
    require_layout us,de:, || return 2
    printf '[[expansion]]\ntrigger = ";zq"\nreplacement = "zany quiz"\n' | write_config
    start_sink e2e-a && start_daemon || return 2
    expand_in e2e-a "" ";zq" || return 2
    expect_layout_output e2e-a ";zq" "zany quiz"
    first=$?
    printf 'us: '
    describe e2e-a
    [ "$first" -eq 0 ] || return "$first"
    switch_layout || {
        printf '; could not switch to the second layout'
        return 2
    }
    expand_in e2e-a "zany quiz" " ;zq" || {
        printf '; the typist could not type after the switch; '
        describe e2e-a
        return 2
    }
    expect_layout_output e2e-a "zany quiz ;zq" "zany quiz zany quiz"
    second=$?
    printf '; after switch to de: '
    describe e2e-a
    return "$second"
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

# --- lifecycle soak -----------------------------------------------------
# Repeats lifecycle events against the real output route for
# E2E_SOAK_SECONDS (default 90) and checks the exact target text after each
# one: no lost or extra output, no stale expansion replayed after a
# disconnect, and no stuck modifier (an ordinary key typed after every
# expansion must arrive unchanged). Output revocation needs a session
# driver that defines revoke_output and restore_output.
#
# E2E_SOAK_RESTART_EVERY (default 1) restarts the daemon every Nth cycle;
# 0 never restarts it, so one daemon runs the whole soak and its memory
# growth is measured: resident memory may grow by at most
# E2E_SOAK_MAX_RSS_GROWTH_KIB (default 65536) after the first cycle.

status_field() {
    printf '%s\n' "$1" | sed -n "s/^$2=//p"
}

# soak_check SINK EXPECTED EVENT: wait for SINK to show EXPECTED exactly.
soak_check() {
    if wait_until 50 sink_text_is "$1" "$2"; then
        return 0
    fi
    printf 'cycle %s, after %s: expected %s; ' "$soak_cycle" "$3" \
        "$(printf '%s' "$2" | sed -n l | tr -d '\n')"
    describe "$1"
    return 1
}

# soak_expand SINK EVENT: type the trigger in SINK, expect the current
# replacement, then type an ordinary key and expect it unchanged.
soak_expand() {
    var=soak_text_$(printf '%s' "$1" | tr - _)
    eval "before=\$$var"
    expand_in "$1" "$before" ";s" || {
        printf 'cycle %s, %s: the typist could not type the trigger; ' "$soak_cycle" "$2"
        describe "$1"
        return 2
    }
    soak_check "$1" "$before$soak_replacement" "$2" || return 1
    user_types "$1" "$before${soak_replacement}k" "k" || {
        printf 'cycle %s, after %s: an ordinary key did not arrive unchanged (stuck key?); ' \
            "$soak_cycle" "$2"
        describe "$1"
        return 1
    }
    daemon_sees "k"
    eval "$var=\"\$before\${soak_replacement}k\""
    soak_events=$((soak_events + 1))
}

soak_config() {
    printf '[[expansion]]\ntrigger = ";s"\nreplacement = "%s"\n' "$soak_replacement" | write_config
}

# Close the target windows but keep the daemon running.
soak_close_sinks() {
    for pid in $sink_pids; do
        kill "$pid" 2>/dev/null || true
    done
    for pid in $sink_pids; do
        wait "$pid" 2>/dev/null || true
    done
    sink_pids=
}

config_generation_changed_from() {
    [ "$(status_field "$(daemon_status)" config_generation)" != "$1" ]
}

output_reconnected_since() {
    [ "$(status_field "$(daemon_status)" output_reconnect_total)" -gt "$1" ] && daemon_ready
}

soak_open_sinks() {
    start_sink e2e-a && start_sink e2e-b || return 1
    # Read through eval in soak_expand.
    # shellcheck disable=SC2034
    soak_text_e2e_a=
    soak_text_e2e_b=
}

scenario_lifecycle_soak() {
    seconds=${E2E_SOAK_SECONDS:-90}
    restart_every=${E2E_SOAK_RESTART_EVERY:-1}
    max_rss_growth=${E2E_SOAK_MAX_RSS_GROWTH_KIB:-65536}
    rss_baseline=
    soak_cycle=0
    soak_events=0
    soak_replacement=R0
    soak_config
    soak_open_sinks && start_daemon || return 2
    revocation=unsupported
    command -v revoke_output >/dev/null 2>&1 && revocation=0
    deadline=$(($(date +%s) + seconds))
    while [ "$(date +%s)" -lt "$deadline" ]; do
        soak_cycle=$((soak_cycle + 1))

        activate e2e-a || { printf 'cycle %s: could not focus A' "$soak_cycle"; return 2; }
        soak_expand e2e-a expansion || return $?

        # Focus moves to another window between expansions.
        activate e2e-b || { printf 'cycle %s: could not focus B' "$soak_cycle"; return 2; }
        soak_expand e2e-b focus-change || return $?

        # A reload takes effect without a restart.
        soak_replacement=R$soak_cycle
        generation=$(status_field "$(daemon_status)" config_generation)
        soak_config
        wait_until 50 config_generation_changed_from "$generation" || {
            printf 'cycle %s: the daemon did not load the edited configuration' "$soak_cycle"
            return 1
        }
        soak_expand e2e-b reload || return $?

        # The daemon is killed and started again.
        if [ "$restart_every" -gt 0 ] && [ $((soak_cycle % restart_every)) -eq 0 ]; then
            kill -9 "$daemon_pid" 2>/dev/null
            wait "$daemon_pid" 2>/dev/null
            exec 3>&-
            stdin_open=0
            daemon_pid=
            start_daemon || { printf 'cycle %s: the daemon did not restart' "$soak_cycle"; return 2; }
            soak_expand e2e-b daemon-restart || return $?
        fi

        # Output access is revoked while a trigger is in flight. Nothing may
        # be typed (the trigger never reached the window, so erasing it would
        # delete real text), and nothing stale may be replayed after the
        # route reconnects.
        if [ "$revocation" != unsupported ]; then
            reconnects=$(status_field "$(daemon_status)" output_reconnect_total)
            revoke_output || { printf 'cycle %s: could not revoke output' "$soak_cycle"; return 2; }
            daemon_sees ";s"
            sleep 0.5
            restore_output || { printf 'cycle %s: could not restore output' "$soak_cycle"; return 2; }
            # The failed attempt must have made the daemon reconnect.
            wait_until 150 output_reconnected_since "${reconnects:-0}" || {
                printf 'cycle %s: output did not reconnect after revocation; ' "$soak_cycle"
                printf 'output_connection_state=%s' \
                    "$(status_field "$(daemon_status)" output_connection_state)"
                return 1
            }
            sleep 1.5
            soak_check e2e-b "$soak_text_e2e_b" output-revocation || return 1
            # Whatever the engine buffered before the disconnect must not
            # complete a trigger now: a delimiter clears it.
            daemon_sees " "
            user_types e2e-b "$soak_text_e2e_b " " " || return 2
            soak_text_e2e_b="$soak_text_e2e_b "
            soak_expand e2e-b after-revocation || return $?
            revocation=$((revocation + 1))
        fi

        # The target windows close and new ones open.
        soak_close_sinks
        soak_open_sinks || { printf 'cycle %s: could not reopen the windows' "$soak_cycle"; return 2; }

        # One cycle has warmed up every path; later growth is measured
        # against that.
        if [ -z "$rss_baseline" ]; then
            rss_baseline=$(status_field "$(daemon_status)" memory_rss_kib)
        fi
    done
    status=$(daemon_status)
    rss=$(status_field "$status" memory_rss_kib)
    printf '%s cycles, %s verified expansions in %ss, %s output revocations; rss_kib=%s high_water_kib=%s' \
        "$soak_cycle" "$soak_events" "$seconds" "$revocation" \
        "$rss" "$(status_field "$status" memory_high_water_kib)"
    [ "$soak_cycle" -gt 0 ] || return 1
    if [ "$restart_every" -eq 0 ] && [ -n "$rss_baseline" ] && [ -n "$rss" ]; then
        growth=$((rss - rss_baseline))
        printf '; one daemon throughout, rss growth after cycle 1: %s KiB' "$growth"
        [ "$growth" -le "$max_rss_growth" ] || {
            printf ' (limit %s KiB)' "$max_rss_growth"
            return 1
        }
    fi
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
