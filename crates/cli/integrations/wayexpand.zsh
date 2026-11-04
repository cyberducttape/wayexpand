# Source this file from ~/.zshrc to report explicit shell context to WayExpand.
autoload -Uz add-zsh-hook
if [[ ! ${WAYEXPAND_CONTEXT_SESSION_ID:-} =~ '^[[:xdigit:]]{32}$' ]] && (( $+commands[wayexpand] )); then
    WAYEXPAND_CONTEXT_SESSION_ID=$(command wayexpand context session-id 2>/dev/null) || WAYEXPAND_CONTEXT_SESSION_ID=
    export WAYEXPAND_CONTEXT_SESSION_ID
fi

_wayexpand_context_report() {
    [[ ${WAYEXPAND_CONTEXT_SESSION_ID:-} =~ '^[[:xdigit:]]{32}$' ]] || return 0
    (( $+commands[wayexpand] )) || return 0

    local project_root='' kube_context='' remote_host=''
    if [[ ${1:-} != --local ]]; then
        remote_host=${WAYEXPAND_CONTEXT_REMOTE_HOST:-}
    fi
    if [[ -z $remote_host ]]; then
        project_root=$(command git rev-parse --show-toplevel 2>/dev/null) || project_root=''
        if (( $+commands[kubectl] )); then
            kube_context=$(command kubectl config current-context 2>/dev/null) || kube_context=''
        fi
    fi

    local -a arguments
    arguments=(context report --quiet --session "$WAYEXPAND_CONTEXT_SESSION_ID" --shell zsh --ttl 90)
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
        (( elapsed += 1 ))
        if (( elapsed >= interval )); then
            WAYEXPAND_CONTEXT_REMOTE_HOST="$target" _wayexpand_context_report
            elapsed=0
        fi
    done
}

add-zsh-hook precmd _wayexpand_context_report

# Explicit SSH wrapper. A target is mandatory and never inferred by parsing
# arbitrary command text or inspecting terminal output.
wayexpand-ssh() {
    if (( $# < 1 )); then
        print -u2 -- 'usage: wayexpand-ssh TARGET [SSH_OPTIONS...] [-- REMOTE_COMMAND...]'
        return 2
    fi
    local target=$1
    shift
    if [[ $target == -* ]]; then
        print -u2 -- 'SSH target cannot begin with -'
        return 2
    fi
    local -a options remote_command
    options=()
    remote_command=()
    while (( $# )); do
        if [[ $1 == -- ]]; then
            shift
            remote_command=("$@")
            break
        fi
        options+=("$1")
        shift
    done
    if [[ ! ${WAYEXPAND_CONTEXT_SESSION_ID:-} =~ '^[[:xdigit:]]{32}$' ]]; then
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
