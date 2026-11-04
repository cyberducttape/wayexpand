#!/usr/bin/env bash
set -euo pipefail

repo_root=$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
temporary_directory=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-context-lifecycle.XXXXXX")
trap 'rm -rf -- "$temporary_directory"' EXIT
mkdir -p "$temporary_directory/bin"

cat >"$temporary_directory/bin/wayexpand" <<'MOCK_WAYEXPAND'
#!/usr/bin/env bash
set -eu
case "${1:-} ${2:-}" in
    'context session-id')
        printf '%s\n' '0123456789abcdef0123456789abcdef'
        ;;
    'context clear')
        printf '%s\n' 'clear' >>"$WAYEXPAND_CONTEXT_TEST_LOG"
        ;;
    'context report')
        remote_host='local'
        while (($#)); do
            if [[ $1 == --remote-host && $# -ge 2 ]]; then
                remote_host=$2
                shift 2
            else
                shift
            fi
        done
        printf 'report:%s\n' "$remote_host" >>"$WAYEXPAND_CONTEXT_TEST_LOG"
        ;;
    *)
        printf 'unexpected wayexpand invocation: %s\n' "$*" >&2
        exit 2
        ;;
esac
MOCK_WAYEXPAND

cat >"$temporary_directory/bin/ssh" <<'MOCK_SSH'
#!/usr/bin/env bash
printf 'ssh:%s\n' "$*" >>"$WAYEXPAND_CONTEXT_TEST_LOG"
exit "${WAYEXPAND_CONTEXT_TEST_SSH_STATUS:-17}"
MOCK_SSH
chmod 700 "$temporary_directory/bin/wayexpand" "$temporary_directory/bin/ssh"

export PATH="$temporary_directory/bin:$PATH"
export WAYEXPAND_CONTEXT_TEST_LOG="$temporary_directory/events"
export WAYEXPAND_CONTEXT_SESSION_ID='0123456789abcdef0123456789abcdef'
export WAYEXPAND_CONTEXT_REMOTE_HOST='stale-host'
export WAYEXPAND_CONTEXT_TEST_SSH_STATUS=17

# shellcheck disable=SC1091 # The integration path is resolved from the checkout root.
source "$repo_root/crates/cli/integrations/wayexpand.bash"

set +e
wayexpand-ssh build-vm -p 2222 -- hostname
ssh_status=$?
set -e

[[ $ssh_status == 17 ]] || {
    printf 'SSH exit status changed: expected 17, got %s\n' "$ssh_status" >&2
    exit 1
}
[[ ! ${WAYEXPAND_CONTEXT_REMOTE_HOST+x} ]] || {
    printf '%s\n' 'SSH wrapper left the remote host exported in the local shell' >&2
    exit 1
}

mapfile -t events <"$WAYEXPAND_CONTEXT_TEST_LOG"
expected=(
    'report:build-vm'
    'ssh:-p 2222 build-vm hostname'
    'clear'
    'report:local'
)
[[ ${events[*]} == "${expected[*]}" ]] || {
    printf 'Unexpected SSH context lifecycle:\n%s\n' "${events[*]}" >&2
    exit 1
}

# Keep the other opt-in integrations aligned with the tested lifecycle.
for shell in zsh fish; do
    integration="$repo_root/crates/cli/integrations/wayexpand.$shell"
    grep -Fq 'command wayexpand context clear' "$integration"
    grep -Fq '_wayexpand_context_report --local' "$integration" ||
        grep -Fq '__wayexpand_context_report --local' "$integration"
done
grep -Fq 'unset WAYEXPAND_CONTEXT_REMOTE_HOST' \
    "$repo_root/crates/cli/integrations/wayexpand.zsh"
grep -Fq 'set -e WAYEXPAND_CONTEXT_REMOTE_HOST' \
    "$repo_root/crates/cli/integrations/wayexpand.fish"
if command -v zsh >/dev/null 2>&1; then
    zsh -n "$repo_root/crates/cli/integrations/wayexpand.zsh"
fi
if command -v fish >/dev/null 2>&1; then
    fish --no-execute "$repo_root/crates/cli/integrations/wayexpand.fish"
fi

printf '%s\n' 'SSH context lifecycle test passed'
