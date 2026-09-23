#!/bin/sh
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
binary=${1:-"$project_dir/target/debug/wayexpand"}
matrix="$project_dir/tests/certification/compositor-matrix.json"
report=$(mktemp "${TMPDIR:-/tmp}/wayexpand-certification-report.XXXXXX")
trap 'rm -f "$report"' EXIT INT TERM

command -v jq >/dev/null 2>&1 || {
    printf '%s\n' 'error: jq is required' >&2
    exit 2
}
[ -x "$binary" ] || {
    printf '%s\n' "error: certification binary is not executable: $binary" >&2
    exit 2
}

"$binary" certify --json >"$report"
expected=$(jq -c '.required_scenarios | sort' "$matrix")
jq -e --argjson expected "$expected" '
    .schema == 1 and
    (.certified | type == "boolean") and
    (.required_scenarios | sort == $expected) and
    ([.checks[] | select(.status == "not-run") | .name] | sort == $expected) and
    (.certified == false)
' "$report" >/dev/null

printf '%s\n' 'certification report contract passed'
