#!/bin/sh
# Session script for the KWin certification driver.
#
# Runs inside the private session that scripts/kwin-certification-prepare.sh
# starts (dbus-run-session + kwin_wayland --virtual, one session per keyboard
# layout profile); never run it in a real desktop session. It reuses the
# helpers in scripts/e2e-lib.sh, with the same daemon route as
# scripts/kwin-e2e-session.sh:
#   - capture: --source=stdin (the harness feeds what the "user" typed;
#     physical keyboard capture is not exercised),
#   - output: the production libei backend through wx-e2e-eis-relay,
#   - focus: the production KWin window tracker on the private bus.
# Unlike the e2e harness, the target is a real toolkit client or
# application (see app_info), whose received text is read back from the
# client itself or over AT-SPI.
#
# Results go to $E2E_RESULTS, one line per cell:
#   "<outcome> <scenario> <app> <detail>"
# where outcome is pass, fail, unverified, or unsupported-by-design.
set -u

# shellcheck source=scripts/e2e-lib.sh
. "$(dirname -- "$0")/e2e-lib.sh"

export LIBEI_SOCKET="$XDG_RUNTIME_DIR/eis.sock"
export WX_E2E_TYPIST=libei
e2e_output_backend=libei
e2e_compatibility_mode=tracked-focus
clients=${CERT_CLIENTS:?}
layout_profile=${CERT_LAYOUT:?}
cert_apps=${CERT_APPS:?}
cert_scenarios=${CERT_SCENARIOS:?}

# Activate the window whose resourceClass is $1 through KWin scripting.
activate_class() {
    script="$case_dir/activate-$1.js"
    cat >"$script" <<EOF
const target = workspace.windowList().find((window) => window.resourceClass === "$1");
if (target) {
    workspace.activeWindow = target;
}
EOF
    plugin="wx-cert-activate-$$-$1"
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
}

# e2e-lib.sh's sinks are activated by app ID, which is their resourceClass.
activate() {
    activate_class "$1" || return 1
    wait_until 30 sink_focused "$1"
}

route_ready() {
    case "$1" in
        *"window_tracker_connected=true"*) ;;
        *) return 1 ;;
    esac
    case "$1" in
        *"insertion_assurance=compatibility"*) return 0 ;;
    esac
    return 1
}

# --- target clients -----------------------------------------------------

# app_info LABEL: set how the client is started and read.
#   app_kind  json (writes its own state file), tty (a recorder inside the
#             terminal), or atspi (read over the accessibility bus)
#   app_class KWin resourceClass, used for activation and app_filter
app_info() {
    app_kind=
    app_class=
    app_atspi=
    app_single_line=false
    case "$1" in
        gtk3-textview) app_kind=json app_class=wx-cert-gtk ;;
        password-field) app_kind=json app_class=wx-cert-password app_single_line=true ;;
        qt6-plaintextedit) app_kind=json app_class=wx-cert-qt ;;
        terminal-konsole) app_kind=tty app_class=org.kde.konsole ;;
        text-editor-gedit) app_kind=atspi app_class=gedit app_atspi=gedit ;;
        qt6-editor-kate) app_kind=atspi app_class=org.kde.kate app_atspi=kate ;;
        *) return 1 ;;
    esac
}

client_state_file() {
    printf '%s/%s.json' "$case_dir" "$1"
}

# Refresh a client's state file; json and tty clients write their own.
client_refresh() {
    app_info "$1" || return 1
    [ "$app_kind" = atspi ] || return 0
    python3 -I "$clients/atspi_text.py" "$app_atspi" "$(client_state_file "$1")" \
        2>>"$case_dir/atspi.log"
}

client_field() {
    client_refresh "$1" || return 1
    jq -r ".$2" "$(client_state_file "$1")" 2>/dev/null
}

client_text_is() {
    [ "$(client_field "$1" text)" = "$2" ]
}

client_cursor_is() {
    [ "$(client_field "$1" cursor)" = "$2" ]
}

client_focused() {
    [ "$(client_field "$1" focused)" = true ]
}

client_describe() {
    printf 'text=%s cursor=%s' "$(client_field "$1" text | sed -n l | tr -d '\n')" \
        "$(client_field "$1" cursor)"
}

client_pid=

# Kate detaches from the shell that starts it, so $client_pid can miss the
# real client. Every client of a case runs with that case's XDG_CONFIG_HOME;
# stop whatever still carries it.
# Editors with an unsaved document can outlive SIGTERM, so survivors get
# SIGKILL.
case_client_pids() {
    for pid in $(ps -u "$(id -u)" -o pid=); do
        if { tr '\0' '\n' <"/proc/$pid/environ"; } 2>/dev/null |
            grep -Fqx "XDG_CONFIG_HOME=$case_dir/xdg/config"; then
            printf '%s\n' "$pid"
        fi
    done
}

no_case_clients() {
    [ -z "$(case_client_pids)" ]
}

reap_case_clients() {
    for pid in $(case_client_pids); do
        kill "$pid" 2>/dev/null || true
    done
    wait_until 10 no_case_clients && return 0
    for pid in $(case_client_pids); do
        kill -KILL "$pid" 2>/dev/null || true
    done
}

start_client() {
    app_info "$1" || return 1
    state=$(client_state_file "$1")
    document="$case_dir/document.txt"
    : >"$document"
    # Applications restore earlier documents and sessions; each case starts
    # from empty per-case settings.
    mkdir -p "$case_dir/xdg/config" "$case_dir/xdg/data" "$case_dir/xdg/cache"
    run_client "$1" "$state" "$document" >>"$case_dir/$1.log" 2>&1 &
    client_pid=$!
    sink_pids="$sink_pids $client_pid"
    wait_until 150 client_refresh "$1" || return 1
    wait_until 50 test -s "$state" || return 1
    activate_client "$1"
}

# Runs in the background subshell; exec keeps $client_pid the client's.
run_client() {
    state=$2
    document=$3
    export XDG_CONFIG_HOME="$case_dir/xdg/config" XDG_DATA_HOME="$case_dir/xdg/data" \
        XDG_CACHE_HOME="$case_dir/xdg/cache"
    case "$1" in
        gtk3-textview)
            exec python3 -I "$clients/gtk_client.py" "$app_class" "$state" ;;
        password-field)
            exec python3 -I "$clients/gtk_client.py" "$app_class" "$state" --password ;;
        qt6-plaintextedit)
            QT_QPA_PLATFORM=wayland exec python3 -I "$clients/qt_client.py" "$app_class" "$state" ;;
        terminal-konsole)
            QT_QPA_PLATFORM=wayland exec konsole --separate --nofork --hide-menubar --hide-tabbar \
                -e python3 -I "$clients/tty_client.py" "$app_class" "$state" ;;
        text-editor-gedit)
            exec gedit --standalone "$document" ;;
        qt6-editor-kate)
            QT_QPA_PLATFORM=wayland QT_LINUX_ACCESSIBILITY_ALWAYS_ON=1 exec kate --new "$document" ;;
    esac
}

activate_client() {
    app_info "$1" || return 1
    activate_class "$app_class" || return 1
    if [ "$app_kind" = tty ]; then
        # A terminal application cannot observe window focus.
        sleep 0.7
        return 0
    fi
    wait_until 50 client_focused "$1"
}

# The "user" types TEXT into the focused client; wait for EXPECTED.
client_types() {
    "$bin/wx-e2e-type" text "$3" >>"$case_dir/type.log" 2>&1 &&
        wait_until 50 client_text_is "$1" "$2"
}

expand_in_client() {
    client_types "$1" "$2$3" "$3" || return 1
    daemon_sees "$3"
}

# The multi-layout profile switches the active layout between the phases of
# a scenario, so the second half runs with a keymap the daemon's output
# connection did not start with.
between_phases() {
    [ "$layout_profile" = multi-layout-switching ] || return 0
    gdbus call --session --dest org.kde.keyboard --object-path /Layouts \
        --method org.kde.KeyboardLayouts.switchToNextLayout >/dev/null 2>>"$case_dir/layout.log" ||
        return 1
    sleep 0.5
    printf 'layout switched; '
}

# --- scenarios ----------------------------------------------------------
# Each takes the app label, prints a detail line and returns 0 (pass),
# 1 (fail), 2 (unverified) or 3 (unsupported by design). The replacement
# text mixes letters that move between US, German and French layouts.

basic_config() {
    write_config <<'EOF'
[[expansion]]
trigger = ";zq"
replacement = "zany quiz"
EOF
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

cert_multiline_rapid() {
    app_info "$1"
    if [ "$app_single_line" = true ]; then
        printf 'single-line client cannot hold a multi-line replacement'
        return 2
    fi
    write_config <<'EOF'
[[expansion]]
trigger = ";ml"
replacement = "zany\nquiz"
EOF
    start_client "$1" && start_daemon || return 2
    expand_in_client "$1" "" ";ml" || return 2
    expected=$(printf 'zany\nquiz')
    wait_until 50 client_text_is "$1" "$expected" || {
        client_describe "$1"
        return 1
    }
    # Typing continues immediately after the replacement lands.
    client_types "$1" "${expected}ok" "ok"
    result=$?
    client_describe "$1"
    return "$result"
}

cert_focus_cross_window() {
    app_info "$1"
    target_class=$app_class
    write_config <<EOF
[[expansion]]
trigger = ";only"
replacement = "only here"
app_filter = ["$target_class"]
EOF
    start_client "$1" && start_sink e2e-b && start_daemon || return 2
    activate e2e-b || return 2
    sleep 0.5
    expand_in e2e-b "" ";only" || return 2
    sleep 1.5
    if ! sink_text_is e2e-b ";only"; then
        printf 'expanded in the other window: '
        describe e2e-b
        return 1
    fi
    activate_client "$1" || return 2
    sleep 0.5
    expand_in_client "$1" "" ";only" || return 2
    wait_until 50 client_text_is "$1" "only here"
    result=$?
    printf 'target: %s; other: %s' "$(client_describe "$1")" "$(describe e2e-b)"
    return "$result"
}

cert_focus_change_during_expansion() {
    slow_command_config
    start_client "$1" && start_sink e2e-b && start_daemon || return 2
    activate_client "$1" || return 2
    sleep 0.5
    expand_in_client "$1" "" ";slow" || return 2
    activate e2e-b || return 2
    sleep 3
    if client_text_is "$1" ";slow" && sink_text_is e2e-b ""; then
        result=0
    else
        result=1
    fi
    printf 'target: %s; other: %s' "$(client_describe "$1")" "$(describe e2e-b)"
    return "$result"
}

cert_target_closes_during_expansion() {
    slow_command_config
    start_sink e2e-b && start_client "$1" && start_daemon || return 2
    activate_client "$1" || return 2
    sleep 0.5
    expand_in_client "$1" "" ";slow" || return 2
    kill "$client_pid" 2>/dev/null
    wait_until 30 sink_focused e2e-b || activate e2e-b || true
    sleep 3
    if ! daemon_alive; then
        printf 'daemon exited'
        return 1
    fi
    sink_text_is e2e-b ""
    result=$?
    printf 'other: %s' "$(describe e2e-b)"
    return "$result"
}

cert_config_reload() {
    write_config <<'EOF'
[[expansion]]
trigger = ";rel"
replacement = "zany"
EOF
    start_client "$1" && start_daemon || return 2
    expand_in_client "$1" "" ";rel" || return 2
    wait_until 50 client_text_is "$1" "zany" || {
        client_describe "$1"
        return 1
    }
    write_config <<'EOF'
[[expansion]]
trigger = ";rel"
replacement = " quiz"
EOF
    # The watcher debounces for 100 ms and rebuilds off the reactor.
    sleep 1.5
    between_phases || return 2
    expand_in_client "$1" "zany" ";rel" || return 2
    wait_until 50 client_text_is "$1" "zany quiz"
    result=$?
    client_describe "$1"
    return "$result"
}

cert_daemon_restart() {
    basic_config
    start_client "$1" && start_daemon || return 2
    expand_in_client "$1" "" ";zq" || return 2
    wait_until 50 client_text_is "$1" "zany quiz" || {
        client_describe "$1"
        return 1
    }
    kill -9 "$daemon_pid" 2>/dev/null
    wait "$daemon_pid" 2>/dev/null
    exec 3>&-
    stdin_open=0
    daemon_pid=
    between_phases || return 2
    start_daemon || return 2
    expand_in_client "$1" "zany quiz" ";zq" || return 2
    wait_until 50 client_text_is "$1" "zany quizzany quiz"
    result=$?
    client_describe "$1"
    return "$result"
}

cert_daemon_restart_during_typing() {
    # Half a trigger is typed, the daemon dies, and the rest is typed after
    # it restarts. The new daemon never saw the first half, so it must not
    # expand; a complete trigger afterwards must.
    basic_config
    start_client "$1" && start_daemon || return 2
    client_types "$1" ";z" ";z" || return 2
    daemon_sees ";z"
    kill -9 "$daemon_pid" 2>/dev/null
    wait "$daemon_pid" 2>/dev/null
    exec 3>&-
    stdin_open=0
    daemon_pid=
    start_daemon || return 2
    expand_in_client "$1" ";z" "q" || return 2
    sleep 1.5
    if ! client_text_is "$1" ";zq"; then
        printf 'expanded a trigger split across the restart: '
        client_describe "$1"
        return 1
    fi
    expand_in_client "$1" ";zq" ";zq" || return 2
    wait_until 50 client_text_is "$1" ";zqzany quiz"
    result=$?
    client_describe "$1"
    return "$result"
}

# Exact text or an untouched trigger are acceptable; anything else is not.
# Returns 0 for exact text, 4 for a clean refusal, 1 otherwise.
insert_or_refuse() {
    expand_in_client "$1" "" "$2" || return 2
    if wait_until 50 client_text_is "$1" "$3"; then
        return 0
    fi
    client_text_is "$1" "$2" && return 4
    return 1
}

cert_failed_insertion() {
    # A route that cannot type the replacement must refuse before it erases
    # the trigger; it must never type something else.
    write_config <<'EOF'
[[expansion]]
trigger = ";far"
replacement = "東京"
EOF
    start_client "$1" && start_daemon || return 2
    insert_or_refuse "$1" ";far" "東京"
    result=$?
    case "$result" in
        0) printf 'route inserted the text exactly; ' ;;
        4)
            printf 'route refused and left the trigger intact; '
            result=0
            ;;
    esac
    client_describe "$1"
    return "$result"
}

cert_unicode_combining() {
    write_config <<'EOF'
[[expansion]]
trigger = ";acc"
replacement = "café ñ"
EOF
    start_client "$1" && start_daemon || return 2
    expected=$(printf 'cafe\314\201 n\314\203')
    insert_or_refuse "$1" ";acc" "$expected"
    result=$?
    if [ "$result" -eq 4 ]; then
        printf 'route refused combining marks cleanly; insertion not demonstrated; '
        result=2
    fi
    client_describe "$1"
    return "$result"
}

cert_command_timeout() {
    write_config <<'EOF'
[[expansion]]
trigger = ";hang"
replacement = ""
[expansion.command]
program = "/bin/sh"
args = ["-c", "sleep 5; printf late"]
timeout_ms = 500

[[expansion]]
trigger = ";zq"
replacement = "zany quiz"
EOF
    start_client "$1" && start_daemon || return 2
    expand_in_client "$1" "" ";hang" || return 2
    sleep 2
    if ! client_text_is "$1" ";hang" || ! daemon_alive; then
        printf 'timed-out command changed the text or stopped the daemon: '
        client_describe "$1"
        return 1
    fi
    # The daemon keeps expanding after the timeout.
    expand_in_client "$1" ";hang" ";zq" || return 2
    wait_until 50 client_text_is "$1" ";hangzany quiz"
    result=$?
    client_describe "$1"
    return "$result"
}

cert_expansion_cancellation() {
    # The user keeps typing while a command expansion is pending. The text
    # has moved on, so the pending expansion must be dropped, not replayed.
    slow_command_config
    start_client "$1" && start_daemon || return 2
    expand_in_client "$1" "" ";slow" || return 2
    # Capture sees the key as it is typed, so the daemon is told at once,
    # not after the client confirms it (AT-SPI readback can take longer
    # than the command runs).
    "$bin/wx-e2e-type" text "x" >>"$case_dir/type.log" 2>&1 || return 2
    daemon_sees "x"
    wait_until 50 client_text_is "$1" ";slowx" || {
        client_describe "$1"
        return 1
    }
    sleep 3
    client_text_is "$1" ";slowx"
    result=$?
    client_describe "$1"
    return "$result"
}

cert_text_editor_client() {
    # Expansion in the middle of existing text, after the caret was moved.
    basic_config
    start_client "$1" && start_daemon || return 2
    client_types "$1" "ab" "ab" || return 2
    "$bin/wx-e2e-type" key Left >>"$case_dir/type.log" 2>&1 || return 2
    wait_until 30 client_cursor_is "$1" 1 || {
        printf 'caret did not move: '
        client_describe "$1"
        return 2
    }
    client_types "$1" "a;zqb" ";zq" || return 2
    daemon_sees ";zq"
    wait_until 50 client_text_is "$1" "azany quizb"
    result=$?
    if [ "$result" -eq 0 ] && ! wait_until 10 client_cursor_is "$1" 10; then
        printf 'caret not after the replacement; '
        result=1
    fi
    client_describe "$1"
    return "$result"
}

# --- runner -------------------------------------------------------------

record() {
    printf '%s %s %s %s\n' "$1" "$2" "$3" "$4" >>"$results"
}

if ! wait_until 50 gdbus introspect --session --dest org.kde.KWin --object-path /Scripting >/dev/null 2>&1; then
    record unverified session - 'KWin did not appear on the private bus'
    exit 0
fi
# Applications only expose their widgets once accessibility is enabled.
gdbus call --session --dest org.a11y.Bus --object-path /org/a11y/bus \
    --method org.freedesktop.DBus.Properties.Set org.a11y.Status IsEnabled '<true>' \
    >/dev/null 2>>"$work/session.log" || log 'could not enable accessibility'
relay_pid=
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
    record unverified session - 'the EIS relay did not start'
    exit 0
fi
gdbus call --session --dest org.kde.keyboard --object-path /Layouts \
    --method org.kde.KeyboardLayouts.getLayoutsList >"$work/layouts.txt" 2>&1 || true

old_ifs=$IFS
IFS=,
# shellcheck disable=SC2086 # Split the validated CSV on its explicit comma IFS.
set -- $cert_apps
IFS=$old_ifs
for app do
    for name in $cert_scenarios; do
        function_name="cert_$(printf '%s' "$name" | tr '-' '_')"
        if ! command -v "$function_name" >/dev/null 2>&1; then
            record unverified "$name" "$app" 'scenario is not driven by this session'
            continue
        fi
        if ! app_info "$app"; then
            record unverified "$name" "$app" 'client is not driven by this session'
            continue
        fi
        begin_case "$name-$app"
        "$function_name" "$app" >"$case_dir/detail" 2>>"$case_dir/scenario.log"
        code=$?
        end_case
        reap_case_clients
        detail=$(tr '\n' ' ' <"$case_dir/detail" | cut -c 1-400)
        case "$code" in
            0) outcome=pass ;;
            1) outcome=fail ;;
            3) outcome=unsupported-by-design ;;
            *) outcome=unverified ;;
        esac
        record "$outcome" "$name" "$app" "$detail"
    done
done
