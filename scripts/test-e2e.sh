#!/bin/sh
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
runtime_dir=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-e2e.XXXXXX")
daemon_pid=
config_path="$runtime_dir/expansions.toml"

cleanup() {
    if [ -n "$daemon_pid" ] && kill -0 "$daemon_pid" 2>/dev/null; then
        kill "$daemon_pid" 2>/dev/null || true
        wait "$daemon_pid" 2>/dev/null || true
    fi
    rm -rf "$runtime_dir"
}
trap cleanup EXIT INT TERM

cat >"$config_path" <<'EOF'
[[expansion]]
trigger = ":approved"
replacement = "approved expansion"
app_filter = ["org.example.App"]

[[expansion]]
trigger = ":command"
replacement = "fallback"
[expansion.command]
program = "/bin/sh"
args = ["-c", "printf e2e-command"]
EOF
chmod 0600 "$config_path"
cp "$config_path" "$runtime_dir/valid.toml"

daemon_binary="$project_dir/target/debug/wayexpand-daemon"
cli_binary="$project_dir/target/debug/wayexpand"

XDG_RUNTIME_DIR="$runtime_dir" \
WAYEXPAND_CONFIG="$config_path" \
WAYEXPAND_SOURCE=stdin \
WAYEXPAND_BACKEND=none \
"$daemon_binary" </dev/null >"$runtime_dir/daemon.log" 2>&1 &
daemon_pid=$!

status=
for _attempt in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do
    if status=$(XDG_RUNTIME_DIR="$runtime_dir" "$cli_binary" status 2>/dev/null); then
        case "$status" in
            *"source=stdin"*"backend=none"*"config_state=ok"*) break ;;
        esac
    fi
    sleep 0.1
done
case "$status" in
    *"source=stdin"*"backend=none"*"config_state=ok"*) ;;
    *)
        printf '%s\n' "daemon did not become ready:" "$status" >&2
        exit 1
        ;;
esac

# Scenario 1: startup and control-socket status.
printf '%s\n' "$status" | grep -F 'state=running' >/dev/null

# Scenario 2: invalid then valid configuration reload.
printf '%s\n' '[[expansion]]' 'trigger = ' >"$config_path"
[ "$(XDG_RUNTIME_DIR="$runtime_dir" "$cli_binary" reload)" = "reload scheduled" ]
reload_state=
for _attempt in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do
    reload_state=$(XDG_RUNTIME_DIR="$runtime_dir" "$cli_binary" status 2>/dev/null || true)
    case "$reload_state" in
        *"config_state=reload-rejected"*) break ;;
    esac
    sleep 0.1
done
printf '%s\n' "$reload_state" | grep -F 'config_state=reload-rejected' >/dev/null
cp "$runtime_dir/valid.toml" "$config_path"
[ "$(XDG_RUNTIME_DIR="$runtime_dir" "$cli_binary" reload)" = "reload scheduled" ]
reload_state=
for _attempt in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do
    reload_state=$(XDG_RUNTIME_DIR="$runtime_dir" "$cli_binary" status 2>/dev/null || true)
    case "$reload_state" in
        *"config_state=ok"*) break ;;
    esac
    sleep 0.1
done
printf '%s\n' "$reload_state" | grep -F 'config_state=ok' >/dev/null

# Scenario 3: pause/resume through the daemon control socket.
[ "$(XDG_RUNTIME_DIR="$runtime_dir" "$cli_binary" pause)" = "paused" ]
pause_status=
for _attempt in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do
    pause_status=$(XDG_RUNTIME_DIR="$runtime_dir" "$cli_binary" status 2>/dev/null || true)
    printf '%s\n' "$pause_status" | grep -F 'paused=true' >/dev/null && break
    sleep 0.1
done
printf '%s\n' "$pause_status" | grep -F 'paused=true' >/dev/null
[ "$(XDG_RUNTIME_DIR="$runtime_dir" "$cli_binary" resume)" = "resumed" ]
resume_status=
for _attempt in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do
    resume_status=$(XDG_RUNTIME_DIR="$runtime_dir" "$cli_binary" status 2>/dev/null || true)
    printf '%s\n' "$resume_status" | grep -F 'paused=false' >/dev/null && break
    sleep 0.1
done
printf '%s\n' "$resume_status" | grep -F 'paused=false' >/dev/null

# Scenario 4: app-switch matching. The preview command supplies the focused
# window context and verifies fail-closed matching for the other application.
preview=$("$cli_binary" preview :approved --preview-app=org.example.App "$config_path")
printf '%s\n' "$preview" | grep -F 'approved expansion' >/dev/null
if "$cli_binary" preview :approved --preview-app=org.other.App "$config_path" \
    | grep -F 'no expansion matched' >/dev/null; then
    :
else
    printf '%s\n' 'app-filter expansion matched the wrong application' >&2
    exit 1
fi

# Scenario 5: command-backed expansion execution through the same validated
# configuration used by the daemon.
command_output=$("$cli_binary" test :command "$config_path")
[ "$command_output" = "e2e-command" ]

[ "$(XDG_RUNTIME_DIR="$runtime_dir" "$cli_binary" stop)" = "stopping" ]
wait "$daemon_pid"
daemon_pid=
[ ! -e "$runtime_dir/wayexpand.sock" ]

printf '%s\n' 'daemon E2E scaffold passed (startup, reload, app switch, pause/resume, command)'
