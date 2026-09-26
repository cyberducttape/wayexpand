#!/bin/sh
set -eu

ref=${1:-HEAD}
expected_name='Stephan Loesevitz'
expected_email='stephan.loesevitz@gmail.com'

violations=$(git log "$ref" --format='%H%x09%an%x09%ae%x09%cn%x09%ce' \
    | awk -F '\t' -v name="$expected_name" -v email="$expected_email" '
        $2 != name || $3 != email || $4 != name || $5 != email { print }
    ')

if [ -n "$violations" ]; then
    printf '%s\n' 'commit identity policy violation(s):' >&2
    printf '%s\n' "$violations" >&2
    exit 1
fi

printf 'commit identity policy passed for %s\n' "$ref"
