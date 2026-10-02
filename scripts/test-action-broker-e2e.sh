#!/bin/sh
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
runtime_dir=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}
test_root=$(mktemp -d "$runtime_dir/wayexpand-broker-e2e.XXXXXX")
broker_pid=0
trap 'if [ "$broker_pid" -ne 0 ]; then kill "$broker_pid" 2>/dev/null || true; wait "$broker_pid" 2>/dev/null || true; fi; rm -rf "$test_root"' EXIT INT TERM

config="$test_root/broker.toml"
socket="$test_root/broker.sock"
cat >"$config" <<EOF
[broker]
require_absolute_paths = true
strict_env = true

[actions."integration-echo"]
program = "/bin/printf"
args = ["broker-e2e-pass"]
timeout_ms = 2000
enabled = true
EOF
chmod 0600 "$config"

"$project_dir/target/debug/wayexpand-action-broker" \
    --config "$config" --socket "$socket" >"$test_root/broker.log" 2>&1 &
broker_pid=$!

SOCKET="$socket" python3 - <<'PY'
import json
import os
import socket
import time

path = os.environ["SOCKET"]
deadline = time.time() + 5
while time.time() < deadline:
    try:
        client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        client.connect(path)
        break
    except OSError:
        time.sleep(0.05)
else:
    raise SystemExit("broker socket did not become ready")

request = {
    "action_id": "integration-echo",
    "timeout_ms": 2000,
    "inherit_env": False,
    "env_vars": [],
    "stdout_capture": True,
}
client.sendall((json.dumps(request) + "\n").encode())
response = json.loads(client.makefile("rb").readline())
client.close()
assert response["Success"]["stdout"] == "broker-e2e-pass", response
assert response["Success"]["exit_code"] == 0, response
PY

printf '%s\n' "Action Broker end-to-end test passed"
