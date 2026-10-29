#!/bin/sh
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
test_root=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-production-gate.XXXXXX")
trap 'rm -rf -- "$test_root"' EXIT INT TERM
script="$project_dir/scripts/check-production-certification.sh"
matrix="$project_dir/tests/certification/compositor-matrix.json"

blocked_summary="$test_root/blocked.md"
if GITHUB_STEP_SUMMARY="$blocked_summary" "$script" true "$matrix" \
    >"$test_root/blocked.out" 2>"$test_root/blocked.err"; then
    printf '%s\n' 'production gate accepted a matrix with no eligible route' >&2
    exit 1
fi
grep -F 'No declared compositor route currently meets' "$test_root/blocked.err" >/dev/null
grep -F 'Compatibility jobs cannot satisfy this gate' "$blocked_summary" >/dev/null

compatibility_summary="$test_root/compatibility.md"
GITHUB_STEP_SUMMARY="$compatibility_summary" "$script" false "$matrix" \
    >"$test_root/compatibility.out"
grep -F 'This run gathers compatibility evidence only' "$compatibility_summary" >/dev/null
grep -F '::warning::' "$test_root/compatibility.out" >/dev/null

eligible_matrix="$test_root/eligible.json"
jq '.targets[0].production_certification_required = true' "$matrix" >"$eligible_matrix"
eligible_summary="$test_root/eligible.md"
GITHUB_STEP_SUMMARY="$eligible_summary" "$script" true "$eligible_matrix" \
    >"$test_root/eligible.out"
grep -F 'Production-required compositor routes: 1' "$test_root/eligible.out" >/dev/null

invalid_matrix="$test_root/invalid.json"
printf '%s\n' '{"targets":[{"id":"kde","production_certification_required":"true"}]}' \
    >"$invalid_matrix"
if "$script" false "$invalid_matrix" >"$test_root/invalid.out" 2>"$test_root/invalid.err"; then
    printf '%s\n' 'production gate accepted a non-boolean target declaration' >&2
    exit 1
fi
grep -F 'invalid certification target declarations' "$test_root/invalid.err" >/dev/null

printf '%s\n' 'production certification release gate tests passed'
