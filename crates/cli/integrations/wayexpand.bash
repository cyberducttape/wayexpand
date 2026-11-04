# Source this file from ~/.bashrc to report explicit shell context to WayExpand.
if [[ ! ${WAYEXPAND_CONTEXT_SESSION_ID:-} =~ ^[[:xdigit:]]{32}$ ]] && command -v wayexpand >/dev/null 2>&1; then
    WAYEXPAND_CONTEXT_SESSION_ID=$(command wayexpand context session-id 2>/dev/null) || WAYEXPAND_CONTEXT_SESSION_ID=
    export WAYEXPAND_CONTEXT_SESSION_ID
fi

_wayexpand_context_report() {
    [[ ${WAYEXPAND_CONTEXT_SESSION_ID:-} =~ ^[[:xdigit:]]{32}$ ]] || return 0
    command -v wayexpand >/dev/null 2>&1 || return 0

    local project_root='' kube_context='' remote_host=''
    if [[ ${1:-} != --local ]]; then
        remote_host=${WAYEXPAND_CONTEXT_REMOTE_HOST:-}
    fi
    if [[ -z $remote_host ]]; then
        project_root=$(command git rev-parse --show-toplevel 2>/dev/null) || project_root=''
        if command -v kubectl >/dev/null 2>&1; then
            kube_context=$(command kubectl config current-context 2>/dev/null) || kube_context=''
        fi
    fi

    local -a arguments=(context report --quiet --session "$WAYEXPAND_CONTEXT_SESSION_ID" --shell bash --ttl 90)
    [[ -n $project_root ]] && arguments+=(--project-root "$project_root")
    [[ -n $kube_context ]] && arguments+=(--kubernetes-context "$kube_context")
    [[ -n $remote_host ]] && arguments+=(--remote-host "$remote_host")
    [[ -n ${WAYEXPAND_CONTEXT_ENVIRONMENT:-} ]] && arguments+=(--environment "$WAYEXPAND_CONTEXT_ENVIRONMENT")
    [[ -z $remote_host && -n ${WAYEXPAND_CONTEXT_CONTAINER:-} ]] && arguments+=(--container "$WAYEXPAND_CONTEXT_CONTAINER")
    command wayexpand "${arguments[@]}" >/dev/null 2>&1 || :
}

_wayexpand_context_remote_heartbeat() {
    local target=$1 stop_file=$2 interval=${3:-30} elapsed=0
    while [[ ! -e $stop_file ]]; do
        sleep 1
        [[ -e $stop_file ]] && break
        elapsed=$((elapsed + 1))
        if ((elapsed >= interval)); then
            WAYEXPAND_CONTEXT_REMOTE_HOST="$target" _wayexpand_context_report
            elapsed=0
        fi
    done
}

# Explicit SSH wrapper: the target is the first argument, ssh options follow,
# and `--` separates options from an optional remote command. It never guesses
# a host from arbitrary ssh syntax or terminal output.
wayexpand-ssh() {
    (($# >= 1)) || { printf '%s\n' 'usage: wayexpand-ssh TARGET [SSH_OPTIONS...] [-- REMOTE_COMMAND...]' >&2; return 2; }
    local target=$1
    shift
    [[ $target != -* ]] || { printf '%s\n' 'SSH target cannot begin with -' >&2; return 2; }
    local -a options=() remote_command=()
    while (($#)); do
        if [[ $1 == -- ]]; then
            shift
            remote_command=("$@")
            break
        fi
        options+=("$1")
        shift
    done
    if [[ ! ${WAYEXPAND_CONTEXT_SESSION_ID:-} =~ ^[[:xdigit:]]{32}$ ]]; then
        command ssh "${options[@]}" "$target" "${remote_command[@]}"
        return $?
    fi
    local stop_file
    stop_file=$(mktemp "${TMPDIR:-/tmp}/wayexpand-context-stop.XXXXXX") || {
        command ssh "${options[@]}" "$target" "${remote_command[@]}"
        return $?
    }
    _wayexpand_context_remote_heartbeat "$target" "$stop_file" &
    local reporter_pid=$!
    WAYEXPAND_CONTEXT_REMOTE_HOST="$target" _wayexpand_context_report
    command ssh "${options[@]}" "$target" "${remote_command[@]}"
    local ssh_status=$?
    printf 'stop\n' >"$stop_file"
    wait "$reporter_pid" 2>/dev/null || :
    command wayexpand context clear >/dev/null 2>&1 || :
    command rm -f -- "$stop_file"
    unset WAYEXPAND_CONTEXT_REMOTE_HOST
    _wayexpand_context_report --local
    return "$ssh_status"
}

if [[ $(declare -p PROMPT_COMMAND 2>/dev/null) == 'declare -a'* ]]; then
    if [[ " ${PROMPT_COMMAND[*]} " != *' _wayexpand_context_report '* ]]; then
        PROMPT_COMMAND=(_wayexpand_context_report "${PROMPT_COMMAND[@]}")
    fi
else
    case ";${PROMPT_COMMAND:-};" in
        *";_wayexpand_context_report;"*) ;;
        *) PROMPT_COMMAND="_wayexpand_context_report${PROMPT_COMMAND:+;${PROMPT_COMMAND}}" ;;
    esac
fi
