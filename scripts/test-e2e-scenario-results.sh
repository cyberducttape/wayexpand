#!/usr/bin/env bash
set -euo pipefail

project_dir=$(cd -- "$(dirname -- "$0")/.." && pwd)
work=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-e2e-results.XXXXXX")
trap 'rm -rf "$work"' EXIT INT TERM

export E2E_BIN=/bin/true
export E2E_WORK=$work
export E2E_RESULTS=$work/results.txt
export XDG_RUNTIME_DIR=$work
export WAYEXPAND_CONFIG=$work/config.toml
source "$project_dir/scripts/e2e-lib.sh"

scenario_expected_failure() {
    printf '%s\n' 'assertion detail'
    return 1
}

scenario_expected_unverified() {
    printf '%s\n' 'dependency detail'
    return 2
}

scenario_after_nonzero() {
    printf '%s\n' 'continued after nonzero result'
}

run_scenarios 'expected-failure expected-unverified after-nonzero'

grep -Fx 'fail expected-failure assertion detail' "$E2E_RESULTS" >/dev/null
grep -Fx 'unverified expected-unverified dependency detail' "$E2E_RESULTS" >/dev/null
grep -Fx 'pass after-nonzero continued after nonzero result' "$E2E_RESULTS" >/dev/null
printf '%s\n' 'e2e scenario results survive expected nonzero statuses'
