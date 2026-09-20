#!/bin/sh
# Record an explicit compositor certification run. This script is deliberately
# evidence-oriented: it never turns a doctor probe or a unit test into a
# certification claim. A human or compositor-specific driver must mark every
# scenario as pass/fail.
set -eu

compositor=
output="certification-$(date -u +%Y%m%dT%H%M%SZ).md"
results_file=
while [ "$#" -gt 0 ]; do
    case "$1" in
        --compositor) compositor=${2:?missing value for --compositor}; shift 2 ;;
        --output) output=${2:?missing value for --output}; shift 2 ;;
        --results) results_file=${2:?missing value for --results}; shift 2 ;;
        --help|-h)
            printf '%s\n' "usage: $0 --compositor NAME [--output FILE] [--results FILE]"
            printf '%s\n' "results format: one SCENARIO=pass|fail entry per line"
            exit 0
            ;;
        *) printf '%s\n' "error: unknown option $1" >&2; exit 2 ;;
    esac
done
[ -n "$compositor" ] || { printf '%s\n' "error: --compositor is required" >&2; exit 2; }

case "$compositor" in
    sway|hyprland|river|kde|gnome) ;;
    *) printf '%s\n' "error: unsupported compositor name $compositor" >&2; exit 2 ;;
esac

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
    for scenario in capture-replacement unicode navigation password-field focus-change fast-typing hotplug compositor-restart; do
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
if [ -z "$results_file" ]; then
    complete=0
else
    for scenario in capture-replacement unicode navigation password-field focus-change fast-typing hotplug compositor-restart; do
        grep -Eq "^${scenario}=(pass|fail)$" "$results_file" 2>/dev/null || complete=0
    done
fi
if [ "$complete" -eq 0 ]; then
    printf '%s\n' "certification remains incomplete: provide one valid result for every scenario" >&2
    exit 1
fi
