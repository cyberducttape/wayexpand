#!/bin/sh
set -eu

usage() {
    printf '%s\n' 'usage: check-production-certification.sh true|false MATRIX.json' >&2
    exit 2
}

[ "$#" -eq 2 ] || usage
require_production=$1
matrix=$2
case "$require_production" in
    true|false) ;;
    *) usage ;;
esac
command -v jq >/dev/null 2>&1 || {
    printf '%s\n' 'error: jq is required to check the production certification gate' >&2
    exit 2
}

if ! jq -e '
    (.targets | type) == "array" and
    (.targets | length) > 0 and
    all(.targets[]; (.production_certification_required | type) == "boolean")
' "$matrix" >/dev/null; then
    printf '%s\n' "error: invalid certification target declarations in $matrix" >&2
    exit 2
fi

production_targets=$(jq '[.targets[] | select(.production_certification_required)] | length' "$matrix")
message='No declared compositor route currently meets WayExpand production certification requirements. The release gate remains blocked until a real-session route demonstrates sensitive-field handling, atomic replacement, and the required input/output contract.'
summary_file=${GITHUB_STEP_SUMMARY:-}

if [ "$production_targets" -eq 0 ]; then
    if [ "$require_production" = true ]; then
        printf '%s\n' "$message" >&2
        if [ -n "$summary_file" ]; then
            {
                printf '%s\n' '## Production compositor certification blocked'
                printf '%s\n' "$message"
                printf '%s\n' 'Compatibility jobs cannot satisfy this gate. Do not mark a route production-certified from protocol availability or headless tests.'
            } >>"$summary_file"
        fi
        exit 1
    fi
    if [ -n "$summary_file" ]; then
        {
            printf '%s\n' '## Compatibility certification run'
            printf '%s\n' "$message"
            printf '%s\n' 'This run gathers compatibility evidence only; it cannot satisfy the release gate.'
        } >>"$summary_file"
    fi
    printf '::warning::%s Compatibility jobs will still be selected when their real-session runners are online.\n' "$message"
    exit 0
fi

printf 'Production-required compositor routes: %s\n' "$production_targets"
if [ -n "$summary_file" ]; then
    printf 'Production-required compositor routes: %s\n' "$production_targets" >>"$summary_file"
fi
