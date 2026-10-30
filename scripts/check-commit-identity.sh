#!/usr/bin/env bash
set -euo pipefail

usage='usage: check-commit-identity.sh [--allow-dependabot] [--allow-web-merges] [--web-flow-key FILE --web-flow-fingerprint FINGERPRINT] [<git-revision-range>]'
project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)

allow_dependabot=false
allow_web_merges=false
# GitHub's web-flow commit-signing key (https://github.com/web-flow.gpg). It
# is committed and pinned so CI never trusts a key fetched at check time. The
# overrides exist for tests that sign fixtures with a throwaway key.
web_flow_key="$project_dir/.github/keys/github-web-flow.asc"
web_flow_fingerprint=968479A1AFF927E37D1A566BB5690EEEBB952194

while [ "$#" -gt 0 ]; do
    case $1 in
        --allow-dependabot) allow_dependabot=true; shift ;;
        --allow-web-merges) allow_web_merges=true; shift ;;
        --web-flow-key|--web-flow-fingerprint)
            if [ "$#" -lt 2 ]; then
                printf '%s\n' "$usage" >&2
                exit 2
            fi
            if [ "$1" = --web-flow-key ]; then
                web_flow_key=$2
            else
                web_flow_fingerprint=$2
            fi
            shift 2
            ;;
        -*)
            printf '%s\n' "$usage" >&2
            exit 2
            ;;
        *) break ;;
    esac
done

if [ "$#" -gt 1 ]; then
    printf '%s\n' "$usage" >&2
    exit 2
fi

ref=${1:-HEAD}
expected_name='Stephan Loesevitz'
expected_email='stephan.loesevitz@gmail.com'
# The address GitHub records for the owner's web-UI merges.
owner_noreply_email='129720061+cyberducttape@users.noreply.github.com'

format='%H%x09%an%x09%ae%x09%cn%x09%ce%x09%P'
if [ "$allow_dependabot" = true ] || [ "$allow_web_merges" = true ]; then
    # Exceptions are accepted only with a good signature from the pinned
    # key, verified in a private keyring holding nothing else.
    GNUPGHOME=$(mktemp -d)
    export GNUPGHOME
    trap 'rm -rf "$GNUPGHOME"' EXIT
    chmod 0700 "$GNUPGHOME"
    gpg --batch --quiet --import "$web_flow_key" 2>/dev/null
    imported=$(gpg --batch --with-colons --list-keys \
        | awk -F: '$1 == "pub" { primary = 1; next } primary && $1 == "fpr" { print $10; primary = 0 }')
    if [ "$imported" != "$web_flow_fingerprint" ]; then
        printf '%s\n' "error: $web_flow_key does not contain exactly the pinned web-flow key" >&2
        exit 1
    fi
    format="$format%x09%G?%x09%GF%x09%GP"
fi

violations=$(git log "$ref" --format="$format" \
    | awk -F '\t' \
        -v name="$expected_name" \
        -v email="$expected_email" \
        -v owner_noreply="$owner_noreply_email" \
        -v fingerprint="$web_flow_fingerprint" \
        -v allow_dependabot="$allow_dependabot" \
        -v allow_web_merges="$allow_web_merges" '
        function owner() {
            return $2 == name && $3 == email && $4 == name && $5 == email
        }
        # "U" is a good signature from a key without ownertrust; the
        # fingerprint pin is what establishes trust here.
        function web_flow_signed() {
            return ($7 == "G" || $7 == "U") \
                && ($9 != "" ? $9 : $8) == fingerprint
        }
        function approved_dependabot() {
            return allow_dependabot == "true" \
                && $2 == "dependabot[bot]" \
                && $3 == "49699333+dependabot[bot]@users.noreply.github.com" \
                && $4 == "GitHub" \
                && $5 == "noreply@github.com" \
                && web_flow_signed()
        }
        # Only true merges: squash merges and web edits carry content that
        # must still be authored under the owner identity.
        function approved_web_merge() {
            return allow_web_merges == "true" \
                && $2 == name \
                && $3 == owner_noreply \
                && $4 == "GitHub" \
                && $5 == "noreply@github.com" \
                && split($6, parents, " ") >= 2 \
                && web_flow_signed()
        }
        !(owner() || approved_dependabot() || approved_web_merge()) {
            print $1 "\t" $2 "\t" $3 "\t" $4 "\t" $5
        }
    ')

if [ -n "$violations" ]; then
    printf '%s\n' 'commit identity policy violation(s):' >&2
    printf '%s\n' "$violations" >&2
    exit 1
fi

printf 'commit identity policy passed for %s\n' "$ref"
