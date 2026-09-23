#!/bin/sh
# Record an explicit compositor certification run. This script is deliberately
# evidence-oriented: it never turns a doctor probe or a unit test into a
# certification claim. A human or compositor-specific driver must mark every
# scenario as pass/fail.
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
matrix="$project_dir/tests/certification/compositor-matrix.json"
command -v jq >/dev/null 2>&1 || {
    printf '%s\n' 'error: jq is required to validate the certification matrix' >&2
    exit 2
}

compositor=
compositor_version=
backend=
keyboard_layout=
target_apps=
output="certification-$(date -u +%Y%m%dT%H%M%SZ).md"
results_file=
while [ "$#" -gt 0 ]; do
    case "$1" in
        --compositor) compositor=${2:?missing value for --compositor}; shift 2 ;;
        --version) compositor_version=${2:?missing value for --version}; shift 2 ;;
        --backend) backend=${2:?missing value for --backend}; shift 2 ;;
        --layout) keyboard_layout=${2:?missing value for --layout}; shift 2 ;;
        --target-apps) target_apps=${2:?missing value for --target-apps}; shift 2 ;;
        --output) output=${2:?missing value for --output}; shift 2 ;;
        --results) results_file=${2:?missing value for --results}; shift 2 ;;
        --help|-h)
            printf '%s\n' "usage: $0 --compositor NAME --version VERSION --backend BACKEND --layout LAYOUT --target-apps APPS [--output FILE] [--results FILE]"
            printf '%s\n' "results format: one SCENARIO=pass|fail entry per line"
            exit 0
            ;;
        *) printf '%s\n' "error: unknown option $1" >&2; exit 2 ;;
    esac
done
[ -n "$compositor" ] || { printf '%s\n' "error: --compositor is required" >&2; exit 2; }

[ -n "$backend" ] || {
    printf '%s\n' "error: --backend is required" >&2
    exit 2
}

if ! jq -e --arg compositor "$compositor" --arg backend "$backend" \
    'any(.targets[]; .id == $compositor and (.input_paths | index($backend) != null))' \
    "$matrix" >/dev/null; then
    printf '%s\n' "error: backend '$backend' is not a declared certification path for $compositor" >&2
    exit 2
fi

[ -n "$compositor_version" ] || {
    printf '%s\n' "error: --version is required for reproducible evidence" >&2
    exit 2
}
[ -n "$keyboard_layout" ] || {
    printf '%s\n' "error: --layout is required for reproducible evidence" >&2
    exit 2
}
[ -n "$target_apps" ] || {
    printf '%s\n' "error: --target-apps is required for reproducible evidence" >&2
    exit 2
}

scenarios=$(jq -r '.required_scenarios[]' "$matrix" | tr '\n' ' ')
[ -n "$scenarios" ] || {
    printf '%s\n' 'error: certification matrix declares no required scenarios' >&2
    exit 2
}

# Results are evidence, not free-form annotations. Reject malformed, unknown,
# or duplicate entries before collecting probes so an accidental typo cannot
# leave one required scenario looking covered.
if [ -n "$results_file" ]; then
    [ -f "$results_file" ] || {
        printf '%s\n' "error: results file does not exist: $results_file" >&2
        exit 2
    }
    awk -F= -v allowed="$scenarios" '
        BEGIN {
            count = split(allowed, names, " ")
            for (i = 1; i <= count; i++) valid[names[i]] = 1
        }
        NF == 0 { next }
        NF != 2 || $2 !~ /^(pass|fail)$/ {
            printf "error: malformed certification result: %s\n", $0 > "/dev/stderr"
            invalid = 1
            next
        }
        !($1 in valid) {
            printf "error: unknown certification scenario: %s\n", $1 > "/dev/stderr"
            invalid = 1
            next
        }
        ++seen[$1] > 1 {
            printf "error: certification scenario appears more than once: %s\n", $1 > "/dev/stderr"
            invalid = 1
        }
        END { exit invalid }
    ' "$results_file" || exit 2
fi

tmp=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-certify.XXXXXX")
trap 'rm -rf "$tmp"' EXIT INT TERM
doctor_status=0
wayexpand doctor --json >"$tmp/doctor.json" 2>"$tmp/doctor.stderr" || doctor_status=$?
status_json='unavailable'
if wayexpand status --json >"$tmp/status.json" 2>/dev/null; then
    status_json=$(cat "$tmp/status.json")
fi

date_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)
{
    printf '%s\n\n' "# WayExpand compositor certification evidence"
    printf '%s\n' "- compositor_version: $compositor_version"
    printf '%s\n' "- backend: $backend"
    printf '%s\n' "- keyboard_layout: $keyboard_layout"
    printf '%s\n' "- target_apps: $target_apps"
    printf '%s\n' '- compositor: `'"$compositor"'`'
    printf '%s\n' '- recorded_at_utc: `'"$date_utc"'`'
    printf '%s\n' '- desktop: `'"${XDG_CURRENT_DESKTOP:-unknown}"'`'
    printf '%s\n' '- session: `'"${XDG_SESSION_TYPE:-unknown}"'`'
    printf '%s\n' '- doctor_exit: `'"$doctor_status"'`'
    printf '%s\n\n' "- certification rule: every scenario below must be explicitly marked pass or fail"
    printf '%s\n' '## Probes'
    printf '%s\n\n' '```json'
    cat "$tmp/doctor.json"
    printf '%s\n' '```'
    printf '%s\n\n' 'Daemon status: `'"$status_json"'`'
    printf '%s\n' '## Required scenarios'
    for scenario in $scenarios; do
        result=UNVERIFIED
        if [ -n "$results_file" ] && [ -f "$results_file" ]; then
            result=$(awk -F= -v key="$scenario" '$1 == key {print $2; found=1} END {if (!found) print "UNVERIFIED"}' "$results_file")
        fi
        case "$result" in pass|fail|UNVERIFIED) ;; *) result=INVALID ;; esac
        printf '%s\n' "- $scenario: **$result**"
    done
    printf '\n%s\n' 'A PASS result is valid only when the operator records the exact compositor version, backend, layout, target application, and observed behavior. **UNVERIFIED is not certified.**'
} >"$output"
printf '%s\n' "wrote $output"

complete=1
failed=0
if [ -z "$results_file" ]; then
    complete=0
else
    for scenario in $scenarios; do
        if ! grep -Eq "^$scenario=pass$" "$results_file" 2>/dev/null; then
            complete=0
            grep -Eq "^$scenario=fail$" "$results_file" 2>/dev/null && failed=1
        fi
    done
fi
if [ "$complete" -eq 0 ]; then
    if [ "$failed" -eq 1 ]; then
        printf '%s\n' "certification failed: one or more scenarios were explicitly marked fail" >&2
    else
        printf '%s\n' "certification remains incomplete: provide pass results for every scenario" >&2
    fi
    exit 1
fi
