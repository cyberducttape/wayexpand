#!/bin/sh
# Check a snippet repository in CI or from a pre-commit hook.
#
#   check-snippet-repo.sh [--deny-risks] [--signers FILE] [PATH...]
#
# PATHs are files or directories under the current directory (default: the
# current directory). A directory with a wayexpand-pack.toml is checked as a
# pack: its format with `pack inspect`, or its signature with `pack verify`
# when --signers is given, and its merged snippets with `validate`. Any other
# TOML file that defines [[expansion]] or [[hotkey]] entries is validated on
# its own; other TOML files (Cargo.toml, pyproject.toml, ...) are skipped.
#
# Trigger risks (immediate-mode words, very short triggers, triggers that are
# the start of another) are reported as warnings, or as failures with
# --deny-risks. Under GitHub Actions every finding is also a file annotation.
#
# wayexpand refuses configuration files that others can read, which every
# checkout is, so the checks run on a private copy of the current directory.
# WAYEXPAND_CLI selects the CLI (default: wayexpand on PATH). Exit status: 0
# when everything passed, 1 when something failed, 2 for usage errors.
set -eu

cli=${WAYEXPAND_CLI:-wayexpand}
deny_risks=
signers=
while [ "$#" -gt 0 ]; do
    case "$1" in
        --deny-risks) deny_risks=--deny-risks; shift ;;
        --signers) signers=${2:?missing value for --signers}; shift 2 ;;
        --help|-h)
            sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        --) shift; break ;;
        -*) printf 'error: unknown option %s\n' "$1" >&2; exit 2 ;;
        *) break ;;
    esac
done
[ "$#" -gt 0 ] || set -- .
command -v jq >/dev/null 2>&1 || { printf '%s\n' 'error: jq is required' >&2; exit 2; }
"$cli" --version >/dev/null 2>&1 || {
    printf 'error: cannot run the WayExpand CLI (%s); install it or set WAYEXPAND_CLI\n' "$cli" >&2
    exit 2
}
if [ -n "$signers" ]; then
    signers=$(CDPATH='' cd -- "$(dirname -- "$signers")" && pwd)/$(basename -- "$signers")
fi

root=$(pwd)
copy=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-check.XXXXXX")
trap 'rm -rf "$copy"' EXIT INT TERM
# Only TOML files are needed: snippet files, their includes, and pack
# manifests. Build output and dependency trees can be gigabytes.
find . \( -name .git -o -name 'target*' -o -name node_modules \) -prune -o \
    -name '*.toml' -type f -print | tar -cf - -T - | tar -xf - -C "$copy"
chmod -R go-rwx "$copy"

failures=0
annotate() {
    # annotate LEVEL FILE MESSAGE
    if [ "${GITHUB_ACTIONS:-}" = true ]; then
        printf '::%s file=%s::%s\n' "$1" "$2" "$3"
    fi
}
fail() {
    printf 'FAIL %s: %s\n' "$1" "$2"
    annotate error "$1" "$2"
    failures=$((failures + 1))
}

# relative PATH: PATH relative to the repository root, or fail.
relative() {
    absolute=$(CDPATH='' cd -- "$(dirname -- "$1")" 2>/dev/null && pwd)/$(basename -- "$1") ||
        return 1
    case "$absolute" in
        "$root") printf '.' ;;
        "$root"/*) printf '%s' "${absolute#"$root"/}" ;;
        *) return 1 ;;
    esac
}

is_snippet_file() {
    grep -Eq '^[[:space:]]*\[\[(expansion|hotkey)\]\]' "$1" 2>/dev/null
}

# validate_file NAME COPY: validate one configuration and report its risks.
validate_file() {
    status=0
    output=$("$cli" validate --json $deny_risks "$2" 2>&1) || status=$?
    report=$(printf '%s\n' "$output" | sed -n '/^{/p' | tail -n 1)
    if [ -n "$report" ]; then
        printf '%s\n' "$report" | jq -r '.trigger_risks[] | "\(.trigger)\t\(.reason)"' |
            while IFS="$(printf '\t')" read -r trigger reason; do
                printf 'WARN %s: trigger %s %s\n' "$1" "$trigger" "$reason"
                annotate warning "$1" "trigger $trigger $reason"
            done
    fi
    if [ "$status" -ne 0 ]; then
        fail "$1" "$(printf '%s\n' "$output" | grep -v '^{' | sed 's/^Error: //' | tail -n 1)"
        return 0
    fi
    printf 'ok   %s (%s expansions, %s commands)\n' "$1" \
        "$(printf '%s\n' "$report" | jq -r .expansion_count)" \
        "$(printf '%s\n' "$report" | jq -r .command_count)"
}

check_pack() {
    # POSIX sh has no locals; keep clear of check_path's variables.
    pack_name=${1#./}
    pack_dir="$copy/$pack_name"
    if [ -n "$signers" ]; then
        pack_output=$("$cli" pack verify "$pack_dir" --signers "$signers" 2>&1) || {
            fail "$pack_name" "pack signature: $(printf '%s\n' "$pack_output" | sed 's/^Error: //' | tail -n 1)"
            return 0
        }
    fi
    pack_output=$("$cli" pack inspect "$pack_dir" 2>&1) || {
        fail "$pack_name" "pack format: $(printf '%s\n' "$pack_output" | sed 's/^Error: //' | tail -n 1)"
        return 0
    }
    merged="$copy/.wayexpand-check-$(printf '%s' "$pack_name" | tr '/' '_').toml"
    (umask 077 && "$cli" pack import "$pack_dir" >"$merged" 2>/dev/null) || {
        fail "$pack_name" 'pack snippets could not be merged'
        return 0
    }
    validate_file "$pack_name" "$merged"
}

check_path() {
    name=$(relative "$1") || {
        fail "$1" 'not inside the current directory'
        return 0
    }
    if [ -d "$1" ]; then
        if [ -f "$1/wayexpand-pack.toml" ]; then
            check_pack "$name"
            return 0
        fi
        # The copy holds TOML files only; a directory without any is empty.
        [ -d "$copy/$name" ] || return 0
        # Packs first; their snippet files are checked as part of the pack.
        packs=$(cd "$copy" && find "$name" -name wayexpand-pack.toml -type f | sort)
        for manifest in $packs; do
            check_pack "$(dirname -- "$manifest")"
        done
        files=$(cd "$copy" && find "$name" -name '*.toml' -type f ! -name wayexpand-pack.toml | sort)
        for file in $files; do
            in_pack=false
            for manifest in $packs; do
                case "$file" in "$(dirname -- "$manifest")"/*) in_pack=true ;; esac
            done
            [ "$in_pack" = false ] || continue
            if is_snippet_file "$copy/$file"; then
                validate_file "${file#./}" "$copy/$file"
            fi
        done
        return 0
    fi
    case "$name" in
        */wayexpand-pack.toml | wayexpand-pack.toml) check_pack "$(dirname -- "$name")" ;;
        *)
            if is_snippet_file "$copy/$name"; then
                validate_file "$name" "$copy/$name"
            fi
            ;;
    esac
}

# Found paths are split on whitespace, so names containing it are refused
# rather than checked under the wrong name.
for path do
    case "$path" in
        *[[:space:]]*) fail "$path" 'file names with whitespace are not supported'; continue ;;
    esac
    if [ -d "$path" ]; then
        spaced=$(find "$path" \( -name .git -o -name 'target*' -o -name node_modules \) -prune -o \
            -name '*[[:space:]]*' -name '*.toml' -print)
        if [ -n "$spaced" ]; then
            fail "$path" 'contains TOML files whose names have whitespace; rename them'
            continue
        fi
    fi
    [ -e "$path" ] || { fail "$path" 'does not exist'; continue; }
    check_path "$path"
done

if [ "$failures" -gt 0 ]; then
    printf '%s check(s) failed\n' "$failures"
    exit 1
fi
printf '%s\n' 'all snippet checks passed'
