#!/bin/sh
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
runtime_dir=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}
config_dir=${XDG_CONFIG_HOME:-"$HOME/.config"}/wayexpand
state_root=${XDG_STATE_HOME:-"$HOME/.local/state"}/wayexpand
test_id=$$
test_root="$runtime_dir/wayexpand-action-broker-audit-test.$test_id"
config_dir="$test_root/config"
state_root="$test_root/state"
state_dir="$state_root/ci-audit-test.$test_id"
unit_name=wayexpand-action-broker-audit-test-$test_id
socket="$runtime_dir/wayexpand-broker-audit-test.$test_id.sock"
config="$config_dir/broker-ci-audit-test.$test_id.toml"
binary="$project_dir/target/debug/wayexpand-action-broker"

[ -x "$binary" ] || {
    printf '%s\n' "missing broker binary: $binary" >&2
    exit 1
}

cleanup() {
    systemctl --user stop "$unit_name.service" >/dev/null 2>&1 || true
    rm -f "$socket" "$config"
    rm -rf "$test_root"
}
trap cleanup EXIT INT TERM

install -d -m 0700 "$config_dir" "$state_root" "$state_dir"
chmod 0700 "$test_root" "$config_dir" "$state_root" "$state_dir"
rm -f "$socket"
cat >"$config" <<EOF
[broker]
require_absolute_paths = true
strict_env = true
audit_path = "$state_dir/action-audit.jsonl"

[actions."systemd-audit-test"]
program = "/bin/printf"
args = ["systemd-audit-pass"]
timeout_ms = 2000
enabled = true
EOF
chmod 0600 "$config"

systemd-run --user \
    --unit="$unit_name" \
    --collect \
    --property=Type=simple \
    --property=NoNewPrivileges=yes \
    --property=UMask=0077 \
    --property=PrivateTmp=yes \
    --property=ProtectSystem=strict \
    --property=ProtectHome=read-only \
    --property=ReadWritePaths="$runtime_dir $test_root" \
    -- "$binary" --config "$config" --socket "$socket" \
    >"$state_dir/systemd-run.log" 2>&1 &

if ! SOCKET="$socket" python3 - <<'PY'
import json
import os
import socket
import time

path = os.environ["SOCKET"]
deadline = time.time() + 10
while time.time() < deadline:
    try:
        client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        client.connect(path)
        break
    except OSError:
        time.sleep(0.05)
else:
    raise SystemExit("sandboxed broker socket did not become ready")

request = {
    "action_id": "systemd-audit-test",
    "timeout_ms": 2000,
    "inherit_env": False,
    "env_vars": [],
    "stdout_capture": True,
}
client.sendall((json.dumps(request) + "\n").encode())
response = json.loads(client.makefile("rb").readline())
client.close()
assert response["Success"]["stdout"] == "systemd-audit-pass", response
assert response["Success"]["exit_code"] == 0, response
PY
then
    cat "$state_dir/systemd-run.log" >&2 || true
    systemctl --user status "$unit_name.service" --no-pager >&2 || true
    journalctl --user -u "$unit_name.service" -n 30 --no-pager >&2 || true
    exit 1
fi

audit_file="$state_dir/action-audit.jsonl"
for _ in $(seq 1 50); do
    if [ -s "$audit_file" ]; then
        break
    fi
    sleep 0.1
done
test -s "$audit_file"
AUDIT_FILE="$audit_file" python3 - <<'PY'
import json
import os

with open(os.environ["AUDIT_FILE"], encoding="utf-8") as audit:
    events = [json.loads(line) for line in audit if line.strip()]
assert any(event.get("action_id") == "systemd-audit-test" for event in events), events
PY
printf '%s\n' "Action Broker systemd audit sandbox test passed"
