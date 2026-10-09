#!/bin/sh
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
test_root=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-certification-test.XXXXXX")
trap 'rm -rf "$test_root"' EXIT INT TERM
command -v jq >/dev/null 2>&1
matrix="$project_dir/tests/certification/compositor-matrix.json"
scenarios=$(jq -r '.required_scenarios[]' "$matrix" | tr '\n' ' ')
layout_profiles=us,de,fr,altgr,multi-layout-switching
target_apps=gtk4-demo,qt6-demo,browser-firefox,terminal-konsole,password-field,electron-vscode,text-editor-gedit

results="$test_root/results.txt"
output="$test_root/certification.md"
for scenario in $scenarios; do
    old_ifs=$IFS
    IFS=,
    set -- $layout_profiles
    IFS=$old_ifs
    for layout_profile do
        old_ifs=$IFS
        IFS=,
        set -- $target_apps
        IFS=$old_ifs
        for target_app do
            printf '%s\n' "$scenario|$layout_profile|$target_app=pass"
        done
    done
done >"$results"
expected_scenarios=$(jq '.required_scenarios | length' "$matrix")
expected_layouts=$(jq '.required_layout_profiles | length' "$matrix")
expected_apps=$(jq -nr --arg apps "$target_apps" '$apps | split(",") | length')
expected_cases=$((expected_scenarios * expected_layouts * expected_apps))

certification_cli="$test_root/certification-cli"
cat >"$certification_cli" <<EOF
#!/bin/sh
case "\${1-} \${2-}" in
    "doctor --json") printf '%s\\n' '{"healthy":true,"desktop":"KDE Plasma","wayexpand_commit":"test-commit","ibus":{"installed":true},"capture_readiness":{"end_to_end_verified":true}}' ;;
    "status --json") printf '%s\\n' '{"response":"running","status_schema":6,"daemon_commit":"test-commit"}' ;;
    *) exec "$project_dir/target/debug/wayexpand" "\$@" ;;
esac
EOF
chmod 0755 "$certification_cli"

run_certification() {
    PATH="$project_dir/target/debug:$PATH" \
        "$project_dir/scripts/certify-compositor.sh" \
        --compositor kde --version 6.6.2 --backend ibus --layout us,de,fr,altgr,multi-layout-switching \
        --target-apps "$target_apps" \
        --cli "$certification_cli" "$@"
}

if run_certification --results "$results" --output "$output"; then
    printf '%s\n' 'certification accepted the production-ineligible IBus route' >&2
    exit 1
fi
grep -F -- '- keyboard_layout: us,de,fr,altgr,multi-layout-switching' "$output" >/dev/null
grep -F -- "- required_layout_profiles: \`us\`, \`de\`, \`fr\`, \`altgr\`, \`multi-layout-switching\`" "$output" >/dev/null
grep -F -- "- target_apps: $target_apps" "$output" >/dev/null

json_output="$test_root/certification.json"
if run_certification --format json --results "$results" --output "$json_output"; then
    printf '%s\n' 'JSON certification accepted the production-ineligible IBus route' >&2
    exit 1
fi
jq -e --argjson expected_cases "$expected_cases" '
    .schema == 2 and .certified == false and .status == "incomplete" and
    .backend_certification_eligible == false and
    .backend_certification_block_reason == "IBus lacks atomic replacement, exact window identity, and composition awareness" and
    ([.out_of_scope_capabilities[] | select(.id == "active-ime-preedit" and .status == "unsupported-by-design")] | length == 1) and
    .compositor == "kde" and .backend == "ibus" and
    .expected_desktop == "KDE Plasma" and .detected_desktop == "KDE Plasma" and
    .desktop_probe_valid == true and
    .keyboard_layout == "us,de,fr,altgr,multi-layout-switching" and
    (.required_client_markers | sort) == ["browser", "editor", "electron", "gtk", "password", "qt", "terminal"] and
    .doctor_probe_valid == true and (.doctor_exit | type == "number") and
    .status_probe_valid == true and
    .status_required == false and .backend_probe_valid == true and
    .target_apps == ["gtk4-demo", "qt6-demo", "browser-firefox", "terminal-konsole", "password-field", "electron-vscode", "text-editor-gedit"] and
    ([.scenarios[] | select(.result == "pass")] | length == $expected_cases)
' "$json_output" >/dev/null

spaced_cli_dir="$test_root/cli with spaces"
mkdir -p "$spaced_cli_dir"
cp "$certification_cli" "$spaced_cli_dir/wayexpand"
chmod 0755 "$spaced_cli_dir/wayexpand"
spaced_json="$test_root/spaced-cli.json"
if "$project_dir/scripts/certify-compositor.sh" --format json \
    --compositor kde --version 6.6.2 --backend ibus --layout us,de,fr,altgr,multi-layout-switching \
    --target-apps "$target_apps" --results "$results" \
    --cli "$spaced_cli_dir/wayexpand" --output "$spaced_json" >/dev/null; then
    printf '%s\n' 'certification accepted the production-ineligible IBus route with a spaced CLI path' >&2
    exit 1
fi
jq -e '.certified == false and .doctor_probe_valid == true and .backend_certification_eligible == false' "$spaced_json" >/dev/null

# IBus remains ineligible regardless of its optional daemon status probe; a
# missing status snapshot is recorded without accidentally certifying it.
optional_status_cli="$test_root/optional-status-cli"
cat >"$optional_status_cli" <<'EOF'
#!/bin/sh
case "${1-} ${2-}" in
    "doctor --json") printf '%s\n' '{"healthy":true,"desktop":"KDE Plasma","ibus":{"installed":true}}' ;;
    "status --json") exit 1 ;;
esac
EOF
chmod 0755 "$optional_status_cli"
optional_status_json="$test_root/optional-status-certification.json"
if "$project_dir/scripts/certify-compositor.sh" --format json \
    --compositor kde --version 6.6.2 --backend ibus \
    --layout us,de,fr,altgr,multi-layout-switching --target-apps "$target_apps" \
    --results "$results" --cli "$optional_status_cli" --output "$optional_status_json" >/dev/null; then
    printf '%s\n' 'certification accepted IBus despite its missing required guarantees' >&2
    exit 1
fi
jq -e '.certified == false and .status == "incomplete" and .status_required == false and
    .status_probe_valid == false and .backend_certification_eligible == false' \
    "$optional_status_json" >/dev/null

daemon_cli="$test_root/daemon-cli"
cat >"$daemon_cli" <<'EOF'
#!/bin/sh
case "${1-} ${2-}" in
    "doctor --json") printf '%s\n' '{"healthy":true,"desktop":"KDE Plasma","wayexpand_commit":"test-commit"}' ;;
    "status --json") printf '%s\n' '{"response":"running","status_schema":6,"daemon_commit":"test-commit","source":"evdev","backend":"libei"}' ;;
esac
EOF
chmod 0755 "$daemon_cli"
daemon_json="$test_root/daemon-certification.json"
if "$project_dir/scripts/certify-compositor.sh" --format json \
    --compositor kde --version 6.6.2 --backend evdev+libei --layout us,de,fr,altgr,multi-layout-switching \
    --target-apps "$target_apps" --results "$results" \
    --cli "$daemon_cli" --output "$daemon_json" >/dev/null
then
    printf '%s\n' 'certification accepted the evdev compatibility route' >&2
    exit 1
fi
jq -e '.certified == false and .status_required == true and .backend_probe_valid == true and
    .backend_certification_eligible == false and
    (.backend_certification_block_reason | contains("sensitive-field"))' "$daemon_json" >/dev/null

mismatched_daemon_cli="$test_root/mismatched-daemon-cli"
cat >"$mismatched_daemon_cli" <<'EOF'
#!/bin/sh
case "${1-} ${2-}" in
    "doctor --json") printf '%s\n' '{"healthy":true,"desktop":"KDE Plasma","wayexpand_commit":"test-commit"}' ;;
    "status --json") printf '%s\n' '{"response":"running","status_schema":6,"daemon_commit":"older-commit","source":"evdev","backend":"libei"}' ;;
esac
EOF
chmod 0755 "$mismatched_daemon_cli"
mismatched_daemon_json="$test_root/mismatched-daemon-certification.json"
if "$project_dir/scripts/certify-compositor.sh" --format json \
    --compositor kde --version 6.6.2 --backend evdev+libei --layout us,de,fr,altgr,multi-layout-switching \
    --target-apps "$target_apps" --results "$results" \
    --cli "$mismatched_daemon_cli" --output "$mismatched_daemon_json" >/dev/null; then
    printf '%s\n' 'certification accepted a daemon built from a different commit' >&2
    exit 1
fi
jq -e '.certified == false and .status_probe_valid == false' "$mismatched_daemon_json" >/dev/null

# An explicitly selected, already-running daemon route can be healthy even if
# automatic selection remains conservative. Require valid underlying doctor
# checks plus a matching live route rather than the unrelated auto-select bit.
explicit_route_cli="$test_root/explicit-route-cli"
cat >"$explicit_route_cli" <<'EOF'
#!/bin/sh
case "${1-} ${2-}" in
    "doctor --json") printf '%s\n' '{"healthy":false,"desktop":"KDE Plasma","wayexpand_commit":"test-commit","wayland":true,"config":{"valid":true},"policy":{"policy":{"valid":true}},"control_socket":{"valid":true},"automatic_selection":{"ready":false}}' ;;
    "status --json") printf '%s\n' '{"response":"running","status_schema":6,"daemon_commit":"test-commit","source":"evdev","backend":"libei"}' ;;
esac
EOF
chmod 0755 "$explicit_route_cli"
explicit_route_json="$test_root/explicit-route-certification.json"
if "$project_dir/scripts/certify-compositor.sh" --format json \
    --compositor kde --version 6.6.6 --backend evdev+libei \
    --layout us,de,fr,altgr,multi-layout-switching --target-apps "$target_apps" \
    --results "$results" --cli "$explicit_route_cli" --output "$explicit_route_json" >/dev/null
then
    printf '%s\n' 'certification accepted the unhealthy evdev compatibility route' >&2
    exit 1
fi
jq -e '.certified == false and .doctor.healthy == false and .doctor_probe_valid == true and
    .backend_probe_valid == true and .backend_certification_eligible == false' \
    "$explicit_route_json" >/dev/null

input_method_cli="$test_root/input-method-cli"
cat >"$input_method_cli" <<'EOF'
#!/bin/sh
case "${1-} ${2-}" in
    "doctor --json") printf '%s\n' '{"healthy":true,"desktop":"KDE Plasma","wayexpand_commit":"test-commit"}' ;;
    "status --json") printf '%s\n' '{"response":"running","status_schema":6,"daemon_commit":"test-commit","source":"input-method","backend":"input-method-v2","capture_sensitive_focus":true,"capture_key_passthrough":true,"capture_composition_aware":false,"capture_local_compose_aware":true,"capture_layout_aware":true,"inject_atomic_replace":true,"inject_full_unicode":true,"inject_key_passthrough":true}' ;;
esac
EOF
chmod 0755 "$input_method_cli"
input_method_json="$test_root/input-method-certification.json"
"$project_dir/scripts/certify-compositor.sh" --format json \
    --compositor kde --version 6.6.2 --backend input-method-v2 --layout us,de,fr,altgr,multi-layout-switching \
    --target-apps "$target_apps" --results "$results" \
    --cli "$input_method_cli" --output "$input_method_json" >/dev/null
jq -e '.certified == true and .backend_probe_valid == true and
    .daemon_status.capture_composition_aware == false and
    .daemon_status.capture_local_compose_aware == true' "$input_method_json" >/dev/null

unhealthy_input_method_cli="$test_root/unhealthy-input-method-cli"
cat >"$unhealthy_input_method_cli" <<'EOF'
#!/bin/sh
case "${1-} ${2-}" in
    "doctor --json") printf '%s\n' '{"healthy":false,"desktop":"KDE Plasma","wayexpand_commit":"test-commit"}' ;;
    "status --json") printf '%s\n' '{"response":"running","status_schema":6,"daemon_commit":"test-commit","source":"input-method","backend":"input-method-v2","capture_sensitive_focus":true,"capture_key_passthrough":true,"capture_composition_aware":false,"capture_local_compose_aware":true,"capture_layout_aware":true,"inject_atomic_replace":true,"inject_full_unicode":true,"inject_key_passthrough":true}' ;;
esac
EOF
chmod 0755 "$unhealthy_input_method_cli"
unhealthy_input_method_json="$test_root/unhealthy-input-method-certification.json"
if "$project_dir/scripts/certify-compositor.sh" --format json \
    --compositor kde --version 6.6.2 --backend input-method-v2 \
    --layout us,de,fr,altgr,multi-layout-switching --target-apps "$target_apps" \
    --results "$results" --cli "$unhealthy_input_method_cli" \
    --output "$unhealthy_input_method_json" >/dev/null; then
    printf '%s\n' 'certification accepted an unhealthy input-method route' >&2
    exit 1
fi
jq -e '.certified == false and .doctor_probe_valid == false and .status_probe_valid == true' \
    "$unhealthy_input_method_json" >/dev/null

unsafe_input_method_cli="$test_root/unsafe-input-method-cli"
cat >"$unsafe_input_method_cli" <<'EOF'
#!/bin/sh
case "\${1-} \${2-}" in
    "doctor --json") printf '%s\n' '{"healthy":true,"desktop":"KDE Plasma","wayexpand_commit":"test-commit"}' ;;
    "status --json") printf '%s\n' '{"response":"running","status_schema":6,"daemon_commit":"test-commit","source":"input-method","backend":"input-method-v2","capture_sensitive_focus":true,"capture_key_passthrough":true,"capture_composition_aware":false,"capture_local_compose_aware":true,"capture_layout_aware":true,"inject_atomic_replace":true,"inject_full_unicode":false,"inject_key_passthrough":true}' ;;
esac
EOF
chmod 0755 "$unsafe_input_method_cli"
unsafe_input_method_json="$test_root/unsafe-input-method-certification.json"
if "$project_dir/scripts/certify-compositor.sh" --format json \
    --compositor kde --version 6.6.2 --backend input-method-v2 \
    --layout us,de,fr,altgr,multi-layout-switching --target-apps "$target_apps" \
    --results "$results" --cli "$unsafe_input_method_cli" --output "$unsafe_input_method_json" >/dev/null
then
    printf '%s\n' 'certification accepted an input-method route without full Unicode support' >&2
    exit 1
fi
jq -e '.certified == false and .backend_probe_valid == false' "$unsafe_input_method_json" >/dev/null

stale_daemon_cli="$test_root/stale-daemon-cli"
cat >"$stale_daemon_cli" <<'EOF'
#!/bin/sh
case "${1-} ${2-}" in
    "doctor --json") printf '%s\n' '{"healthy":true,"desktop":"KDE Plasma","wayexpand_commit":"test-commit"}' ;;
    "status --json") printf '%s\n' '{"response":"running","status_schema":0,"source":"evdev","backend":"libei"}' ;;
esac
EOF
chmod 0755 "$stale_daemon_cli"
stale_daemon_json="$test_root/stale-daemon-certification.json"
if "$project_dir/scripts/certify-compositor.sh" --format json \
    --compositor kde --version 6.6.2 --backend evdev+libei \
    --layout us,de,fr,altgr,multi-layout-switching --target-apps "$target_apps" \
    --results "$results" --cli "$stale_daemon_cli" --output "$stale_daemon_json" >/dev/null; then
    printf '%s\n' 'certification accepted an incompatible daemon status schema' >&2
    exit 1
fi
jq -e '.certified == false and .status_probe_valid == false' "$stale_daemon_json" >/dev/null

wrong_desktop_cli="$test_root/wrong-desktop-cli"
cat >"$wrong_desktop_cli" <<'EOF'
#!/bin/sh
case "${1-} ${2-}" in
    "doctor --json") printf '%s\n' '{"healthy":true,"desktop":"GNOME","wayexpand_commit":"test-commit"}' ;;
    "status --json") printf '%s\n' '{"response":"running","status_schema":6,"daemon_commit":"test-commit","source":"evdev","backend":"libei"}' ;;
esac
EOF
chmod 0755 "$wrong_desktop_cli"
wrong_desktop_json="$test_root/wrong-desktop-certification.json"
if "$project_dir/scripts/certify-compositor.sh" --format json \
    --compositor kde --version 6.6.2 --backend evdev+libei \
    --layout us,de,fr,altgr,multi-layout-switching --target-apps "$target_apps" \
    --results "$results" --cli "$wrong_desktop_cli" --output "$wrong_desktop_json" >/dev/null; then
    printf '%s\n' 'certification accepted a mismatched live desktop' >&2
    exit 1
fi
jq -e '.certified == false and .expected_desktop == "KDE Plasma" and
    .detected_desktop == "GNOME" and .desktop_probe_valid == false and .doctor_probe_valid == false' \
    "$wrong_desktop_json" >/dev/null

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
    --compositor kde --version 6.6.2 --backend ibus --layout us,de,fr,altgr,multi-layout-switching \
    --target-apps "$target_apps" --results "$results" \
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
    "doctor --json") printf '%s\n' '{"healthy":false,"desktop":"KDE Plasma"}' ;;
    "status --json") printf '%s\n' '{"response":"running","status_schema":6,"daemon_commit":"test-commit"}' ;;
esac
EOF
chmod 0755 "$unhealthy_probe"
unhealthy_json="$test_root/unhealthy.json"
if "$project_dir/scripts/certify-compositor.sh" --format json \
    --compositor kde --version 6.6.2 --backend ibus --layout us,de,fr,altgr,multi-layout-switching \
    --target-apps "$target_apps" --results "$results" \
    --cli "$unhealthy_probe" --output "$unhealthy_json"; then
    printf '%s\n' 'certification accepted an unhealthy doctor probe' >&2
    exit 1
fi
jq -e '.certified == false and .doctor_probe_valid == false' "$unhealthy_json" >/dev/null

if "$project_dir/scripts/certify-compositor.sh" --format json \
    --compositor kde --version 6.6.2 --backend ibus --layout us,de,fr,altgr,multi-layout-switching \
    --target-apps gtk4-demo,qt6-demo --results "$results" \
    --cli "$project_dir/target/debug/wayexpand" \
    --output "$test_root/missing-password-client.json"; then
    printf '%s\n' 'certification accepted missing password-field coverage' >&2
    exit 1
fi

# Client markers are exact identifiers. A name such as "notgtk" must not
# satisfy the required "gtk" marker by substring accident.
if "$project_dir/scripts/certify-compositor.sh" --format json \
    --compositor kde --version 6.6.2 --backend ibus --layout us,de,fr,altgr,multi-layout-switching \
    --target-apps notgtk,qt6-demo,browser-firefox,terminal-konsole,password-field,electron-vscode,text-editor-gedit \
    --results "$results" --cli "$certification_cli" \
    --output "$test_root/ambiguous-client-marker.json" >/dev/null 2>&1; then
    printf '%s\n' 'certification accepted an ambiguous client marker' >&2
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
jq -e --argjson cases "$expected_cases" '.schema == 2 and .certified == false and .status == "incomplete" and ([.out_of_scope_capabilities[] | select(.id == "active-ime-preedit" and .status == "unsupported-by-design")] | length == 1) and ([.scenarios[] | select(.result == "UNVERIFIED")] | length == 1) and (.scenarios | length == $cases)' "$missing_json" >/dev/null

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

unverified="$test_root/unverified.txt"
sed '1s/=pass$/=UNVERIFIED/' "$results" >"$unverified"
if run_certification --format json --results "$unverified" --output "$test_root/unverified.json"; then
    printf '%s\n' 'certification accepted an unverified matrix cell' >&2
    exit 1
fi
jq -e '.certified == false and .status == "incomplete" and ([.scenarios[] | select(.result == "UNVERIFIED")] | length == 1)' "$test_root/unverified.json" >/dev/null

unknown="$test_root/unknown.txt"
cp "$results" "$unknown"
printf '%s\n' 'not-a-scenario=pass' >>"$unknown"
if run_certification --results "$unknown" --output "$test_root/unknown.md"; then
    printf '%s\n' 'certification accepted an unknown scenario' >&2
    exit 1
fi

duplicate="$test_root/duplicate.txt"
cp "$results" "$duplicate"
head -n 1 "$results" >>"$duplicate"
if run_certification --results "$duplicate" --output "$test_root/duplicate.md"; then
    printf '%s\n' 'certification accepted a duplicate scenario' >&2
    exit 1
fi

malformed="$test_root/malformed.txt"
cp "$results" "$malformed"
sed -n '1s/=pass$/=maybe/p' "$results" >>"$malformed"
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
    --layout us,de,fr,altgr,multi-layout-switching --target-apps gtk >/dev/null 2>&1; then
    printf '%s\n' 'certification accepted an incompatible compositor/backend path' >&2
    exit 1
fi

printf '%s\n' 'certification contract test passed'
