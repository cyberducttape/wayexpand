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

run_certification() {
    PATH="$project_dir/target/debug:$PATH" \
        "$project_dir/scripts/certify-compositor.sh" \
        --compositor kde --version 6.6.2 --backend ibus --layout us \
        --target-apps gtk4-demo,qt6-demo "$@"
}

run_certification --results "$results" --output "$output"
grep -F -- '- keyboard_layout: us' "$output" >/dev/null
grep -F -- '- target_apps: gtk4-demo,qt6-demo' "$output" >/dev/null

missing="$test_root/missing.txt"
sed '$d' "$results" >"$missing"
if run_certification --results "$missing" --output "$test_root/missing.md"; then
    printf '%s\n' 'certification accepted an incomplete results file' >&2
    exit 1
fi

failed="$test_root/failed.txt"
sed '1s/=pass$/=fail/' "$results" >"$failed"
if run_certification --results "$failed" --output "$test_root/failed.md"; then
    printf '%s\n' 'certification accepted an explicit failed scenario' >&2
    exit 1
fi

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
