#!/bin/sh
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
test_root=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-certification-test.XXXXXX")
trap 'rm -rf "$test_root"' EXIT INT TERM
command -v jq >/dev/null 2>&1
matrix="$project_dir/tests/certification/compositor-matrix.json"
scenarios=$(jq -r '.required_scenarios[]' "$matrix" | tr '\n' ' ')

results="$test_root/results.txt"
output="$test_root/certification.md"
for scenario in $scenarios; do
    printf '%s\n' "$scenario=pass"
done >"$results"

certification_cli="$test_root/certification-cli"
cat >"$certification_cli" <<EOF
#!/bin/sh
case "\${1-} \${2-}" in
    "doctor --json") printf '%s\\n' '{"healthy":true,"ibus":{"installed":true},"capture_readiness":{"end_to_end_verified":true}}' ;;
    "status --json") printf '%s\\n' '{"response":"running"}' ;;
    *) exec "$project_dir/target/debug/wayexpand" "\$@" ;;
esac
EOF
chmod 0755 "$certification_cli"

run_certification() {
    PATH="$project_dir/target/debug:$PATH" \
        "$project_dir/scripts/certify-compositor.sh" \
        --compositor kde --version 6.6.2 --backend ibus --layout us \
        --target-apps gtk4-demo,qt6-demo,password-field \
        --cli "$certification_cli" "$@"
}

run_certification --results "$results" --output "$output"
grep -F -- '- keyboard_layout: us' "$output" >/dev/null
    grep -F -- '- target_apps: gtk4-demo,qt6-demo,password-field' "$output" >/dev/null

json_output="$test_root/certification.json"
run_certification --format json --results "$results" --output "$json_output"
jq -e '
    .schema == 1 and .certified == true and .status == "certified" and
    .compositor == "kde" and .backend == "ibus" and
    .keyboard_layout == "us" and
    .required_client_markers == ["gtk", "qt", "password"] and
    .doctor_probe_valid == true and (.doctor_exit | type == "number") and
    .status_probe_valid == true and
    .status_required == false and .backend_probe_valid == true and
    .target_apps == ["gtk4-demo", "qt6-demo", "password-field"] and
    ([.scenarios[] | select(.result == "pass")] | length == 12)
' "$json_output" >/dev/null

spaced_cli_dir="$test_root/cli with spaces"
mkdir -p "$spaced_cli_dir"
cp "$certification_cli" "$spaced_cli_dir/wayexpand"
chmod 0755 "$spaced_cli_dir/wayexpand"
spaced_json="$test_root/spaced-cli.json"
"$project_dir/scripts/certify-compositor.sh" --format json \
    --compositor kde --version 6.6.2 --backend ibus --layout us \
    --target-apps gtk4-demo,qt6-demo,password-field --results "$results" \
    --cli "$spaced_cli_dir/wayexpand" --output "$spaced_json" >/dev/null
jq -e '.certified == true and .doctor_probe_valid == true' "$spaced_json" >/dev/null

daemon_cli="$test_root/daemon-cli"
cat >"$daemon_cli" <<'EOF'
#!/bin/sh
case "${1-} ${2-}" in
    "doctor --json") printf '%s\n' '{"healthy":true}' ;;
    "status --json") printf '%s\n' '{"response":"running","source":"evdev","backend":"libei"}' ;;
esac
EOF
chmod 0755 "$daemon_cli"
daemon_json="$test_root/daemon-certification.json"
"$project_dir/scripts/certify-compositor.sh" --format json \
    --compositor kde --version 6.6.2 --backend evdev+libei --layout us \
    --target-apps gtk4-demo,qt6-demo,password-field --results "$results" \
    --cli "$daemon_cli" --output "$daemon_json" >/dev/null
jq -e '.certified == true and .status_required == true and .backend_probe_valid == true' "$daemon_json" >/dev/null

invalid_probe_bin="$test_root/invalid-probe-bin"
mkdir -p "$invalid_probe_bin"
cat >"$invalid_probe_bin/wayexpand" <<'EOF'
#!/bin/sh
printf '%s\n' 'not-json'
exit 1
EOF
chmod 0755 "$invalid_probe_bin/wayexpand"
invalid_probe_json="$test_root/invalid-probe.json"
if PATH="$invalid_probe_bin:$project_dir/target/debug:$PATH" \
    "$project_dir/scripts/certify-compositor.sh" --format json \
    --compositor kde --version 6.6.2 --backend ibus --layout us \
    --target-apps gtk4-demo,qt6-demo,password-field --results "$results" \
    --cli "$invalid_probe_bin/wayexpand" \
    --output "$invalid_probe_json"; then
    printf '%s\n' 'certification accepted an invalid doctor probe' >&2
    exit 1
fi
jq -e '.certified == false and .doctor_probe_valid == false' "$invalid_probe_json" >/dev/null

unhealthy_probe="$test_root/unhealthy-probe"
cat >"$unhealthy_probe" <<'EOF'
#!/bin/sh
case "${1-} ${2-}" in
    "doctor --json") printf '%s\n' '{"healthy":false}' ;;
    "status --json") printf '%s\n' '{"response":"running"}' ;;
esac
EOF
chmod 0755 "$unhealthy_probe"
unhealthy_json="$test_root/unhealthy.json"
if "$project_dir/scripts/certify-compositor.sh" --format json \
    --compositor kde --version 6.6.2 --backend ibus --layout us \
    --target-apps gtk4-demo,qt6-demo,password-field --results "$results" \
    --cli "$unhealthy_probe" --output "$unhealthy_json"; then
    printf '%s\n' 'certification accepted an unhealthy doctor probe' >&2
    exit 1
fi
jq -e '.certified == false and .doctor_probe_valid == false' "$unhealthy_json" >/dev/null

if "$project_dir/scripts/certify-compositor.sh" --format json \
    --compositor kde --version 6.6.2 --backend ibus --layout us \
    --target-apps gtk4-demo,qt6-demo --results "$results" \
    --cli "$project_dir/target/debug/wayexpand" \
    --output "$test_root/missing-password-client.json"; then
    printf '%s\n' 'certification accepted missing password-field coverage' >&2
    exit 1
fi

missing="$test_root/missing.txt"
sed '$d' "$results" >"$missing"
if run_certification --results "$missing" --output "$test_root/missing.md"; then
    printf '%s\n' 'certification accepted an incomplete results file' >&2
    exit 1
fi
missing_json="$test_root/missing.json"
if run_certification --format json --results "$missing" --output "$missing_json"; then
    printf '%s\n' 'JSON certification accepted an incomplete results file' >&2
    exit 1
fi
jq -e '.schema == 1 and .certified == false and .status == "incomplete" and ([.scenarios[] | select(.result == "UNVERIFIED")] | length == 1)' "$missing_json" >/dev/null

failed="$test_root/failed.txt"
sed '1s/=pass$/=fail/' "$results" >"$failed"
if run_certification --results "$failed" --output "$test_root/failed.md"; then
    printf '%s\n' 'certification accepted an explicit failed scenario' >&2
    exit 1
fi
failed_json="$test_root/failed.json"
if run_certification --format json --results "$failed" --output "$failed_json"; then
    printf '%s\n' 'JSON certification accepted an explicit failed scenario' >&2
    exit 1
fi
jq -e '.certified == false and .status == "failed"' "$failed_json" >/dev/null

unknown="$test_root/unknown.txt"
cp "$results" "$unknown"
printf '%s\n' 'not-a-scenario=pass' >>"$unknown"
if run_certification --results "$unknown" --output "$test_root/unknown.md"; then
    printf '%s\n' 'certification accepted an unknown scenario' >&2
    exit 1
fi

duplicate="$test_root/duplicate.txt"
cp "$results" "$duplicate"
printf '%s\n' 'ime-preedit=fail' >>"$duplicate"
if run_certification --results "$duplicate" --output "$test_root/duplicate.md"; then
    printf '%s\n' 'certification accepted a duplicate scenario' >&2
    exit 1
fi

malformed="$test_root/malformed.txt"
cp "$results" "$malformed"
printf '%s\n' 'ime-preedit=maybe' >>"$malformed"
if run_certification --results "$malformed" --output "$test_root/malformed.md"; then
    printf '%s\n' 'certification accepted a malformed result' >&2
    exit 1
fi

if "$project_dir/scripts/certify-compositor.sh" \
    --compositor kde --version 6.6.2 --backend ibus \
    --results "$results" --output "$test_root/no-metadata.md"; then
    printf '%s\n' 'certification accepted missing reproducibility metadata' >&2
    exit 1
fi

if "$project_dir/scripts/certify-compositor.sh" \
    --compositor sway --version 1.10 --backend ibus \
    --layout us --target-apps gtk >/dev/null 2>&1; then
    printf '%s\n' 'certification accepted an incompatible compositor/backend path' >&2
    exit 1
fi

printf '%s\n' 'certification contract test passed'
