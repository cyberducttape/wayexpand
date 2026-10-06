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
audit_path = "$test_root/action-audit.jsonl"

[actions."integration-echo"]
program = "/bin/sh"
args = ["-c", "sleep 0.2; printf broker-e2e-pass"]
timeout_ms = 2000
enabled = true
EOF
chmod 0600 "$config"

"$project_dir/target/debug/wayexpand-action-broker" \
    --config "$config" --socket "$socket" >"$test_root/broker.log" 2>&1 &
broker_pid=$!

SOCKET="$socket" BROKER_PID="$broker_pid" python3 - <<'PY'
import json
import os
import signal
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
# Exercise graceful shutdown while the connection task still owns the audit
# logger and is waiting for the action subprocess. The broker must await that
# task before dropping the logger and exiting.
time.sleep(0.05)
os.kill(int(os.environ["BROKER_PID"]), signal.SIGTERM)
response = json.loads(client.makefile("rb").readline())
client.close()
assert response["Success"]["stdout"] == "broker-e2e-pass", response
assert response["Success"]["exit_code"] == 0, response
PY

wait "$broker_pid"
broker_pid=0

AUDIT_FILE="$test_root/action-audit.jsonl" python3 - <<'PY'
import json
import os

with open(os.environ["AUDIT_FILE"], encoding="utf-8") as audit:
    events = [json.loads(line) for line in audit if line.strip()]
assert any(event["action_id"] == "integration-echo" for event in events), events
PY

HEALTH_FILE="$test_root/wayexpand-broker-health.json" python3 - <<'PY'
import json
import os

with open(os.environ["HEALTH_FILE"], encoding="utf-8") as health_file:
    health = json.load(health_file)
assert health["audit_enabled"] is True, health
assert health["audit_queue_dropped_total"] == 0, health
assert health["audit_write_failures_total"] == 0, health
assert health["audit_healthy"] is True, health
assert health["running"] is False, health
PY

# A client that connects but never sends a request must not hold shutdown
# for the 30 s socket read timeout: the broker disconnects it on SIGTERM.
"$project_dir/target/debug/wayexpand-action-broker" \
    --config "$config" --socket "$socket" >"$test_root/broker-idle.log" 2>&1 &
broker_pid=$!

SOCKET="$socket" BROKER_PID="$broker_pid" python3 - <<'PY'
import os
import signal
import socket
import time

path = os.environ["SOCKET"]
pid = int(os.environ["BROKER_PID"])
deadline = time.time() + 5
while time.time() < deadline:
    try:
        idle = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        idle.connect(path)
        break
    except OSError:
        time.sleep(0.05)
else:
    raise SystemExit("broker socket did not become ready")

time.sleep(0.2)  # let the broker accept and start reading
started = time.time()
os.kill(pid, signal.SIGTERM)
while time.time() - started < 5:
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        break
    with open(f"/proc/{pid}/stat", encoding="ascii") as stat:
        if stat.read().split(") ", 1)[1].startswith("Z"):
            break
    time.sleep(0.05)
else:
    raise SystemExit("broker shutdown waited on an idle client")
idle.close()
PY

wait "$broker_pid"
broker_pid=0

printf '%s\n' "Action Broker end-to-end test passed"
