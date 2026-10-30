#!/usr/bin/env bash
set -euo pipefail

allow_dependabot=false
if [ "${1:-}" = '--allow-dependabot' ]; then
    allow_dependabot=true
    shift
fi

if [ "$#" -gt 1 ]; then
    printf '%s\n' 'usage: check-commit-identity.sh [--allow-dependabot] [<git-revision-range>]' >&2
    exit 2
fi

ref=${1:-HEAD}
expected_name='Stephan Loesevitz'
expected_email='stephan.loesevitz@gmail.com'

violations=$(git log "$ref" --format='%H%x09%an%x09%ae%x09%cn%x09%ce' \
    | awk -F '\t' \
        -v name="$expected_name" \
        -v email="$expected_email" \
        -v allow_dependabot="$allow_dependabot" '
        function approved_dependabot() {
            return $2 == "dependabot[bot]" \
                && $3 == "49699333+dependabot[bot]@users.noreply.github.com" \
                && $4 == "GitHub" \
                && $5 == "noreply@github.com"
        }
        ($2 != name || $3 != email || $4 != name || $5 != email) \
            && !(allow_dependabot == "true" && approved_dependabot()) { print }
    ')

if [ -n "$violations" ]; then
    printf '%s\n' 'commit identity policy violation(s):' >&2
    printf '%s\n' "$violations" >&2
    exit 1
fi

printf 'commit identity policy passed for %s\n' "$ref"
