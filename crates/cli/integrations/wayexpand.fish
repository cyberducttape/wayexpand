# Source this file from ~/.config/fish/config.fish to report shell context.
if not set -q WAYEXPAND_CONTEXT_SESSION_ID; and command -q wayexpand
    set -gx WAYEXPAND_CONTEXT_SESSION_ID (command wayexpand context session-id 2>/dev/null)
end

function __wayexpand_context_report --on-event fish_prompt
    if not set -q WAYEXPAND_CONTEXT_SESSION_ID; or not command -q wayexpand
        return
    end

    set -l project_root ''
    set -l kube_context ''
    set -l remote_host ''
    set -l report_mode ''
    if test (count $argv) -gt 0
        set report_mode $argv[1]
    end
    if test "$report_mode" != '--local'; and set -q WAYEXPAND_CONTEXT_REMOTE_HOST
        set remote_host $WAYEXPAND_CONTEXT_REMOTE_HOST
    end
    if test -z "$remote_host"
        if command -q git
            set project_root (command git rev-parse --show-toplevel 2>/dev/null)
        end
        if command -q kubectl
            set kube_context (command kubectl config current-context 2>/dev/null)
        end
    end

    set -l arguments context report --quiet --session $WAYEXPAND_CONTEXT_SESSION_ID --shell fish --ttl 90
    if test -n "$project_root"
        set -a arguments --project-root "$project_root"
    end
    if test -n "$kube_context"
        set -a arguments --kubernetes-context "$kube_context"
    end
    if test -n "$remote_host"
        set -a arguments --remote-host "$remote_host"
    end
    if set -q WAYEXPAND_CONTEXT_ENVIRONMENT; and test -n "$WAYEXPAND_CONTEXT_ENVIRONMENT"
        set -a arguments --environment "$WAYEXPAND_CONTEXT_ENVIRONMENT"
    end
    if test -z "$remote_host"; and set -q WAYEXPAND_CONTEXT_CONTAINER; and test -n "$WAYEXPAND_CONTEXT_CONTAINER"
        set -a arguments --container "$WAYEXPAND_CONTEXT_CONTAINER"
    end
    command wayexpand $arguments >/dev/null 2>&1
end

function __wayexpand_context_remote_heartbeat --argument-names target stop_file interval
    if test -z "$interval"
        set interval 30
    end
    set -l elapsed 0
    while not test -e "$stop_file"
        sleep 1
        if test -e "$stop_file"
            break
        end
        set elapsed (math $elapsed + 1)
        if test $elapsed -ge $interval
            set -lx WAYEXPAND_CONTEXT_REMOTE_HOST $target
            __wayexpand_context_report
            set elapsed 0
        end
    end
end

# Explicit SSH wrapper. A target is mandatory; no shell command parsing or
# terminal scraping is used to determine the destination.
function wayexpand-ssh
    if test (count $argv) -lt 1
        echo 'usage: wayexpand-ssh TARGET [SSH_OPTIONS...] [-- REMOTE_COMMAND...]' >&2
        return 2
    end
    set -l target $argv[1]
    if string match -q -- '-*' $target
        echo 'SSH target cannot begin with -' >&2
        return 2
    end
    set -l options
    set -l remote_command
    set -l index 2
    while test $index -le (count $argv)
        if test "$argv[$index]" = '--'
            set remote_command $argv[(math $index + 1)..-1]
            break
        end
        set -a options $argv[$index]
        set index (math $index + 1)
    end
    if not set -q WAYEXPAND_CONTEXT_SESSION_ID
        command ssh $options $target $remote_command
        return $status
    end
    set -l temp_root /tmp
    if set -q TMPDIR; and test -n "$TMPDIR"
        set temp_root "$TMPDIR"
    end
    set -l stop_file (command mktemp "$temp_root/wayexpand-context-stop.XXXXXX")
    set -l mktemp_status $status
    if test $mktemp_status -ne 0; or test -z "$stop_file"
        command ssh $options $target $remote_command
        return $status
    end
    __wayexpand_context_remote_heartbeat $target $stop_file &
    set -l reporter_pid $last_pid
    set -lx WAYEXPAND_CONTEXT_REMOTE_HOST $target
    __wayexpand_context_report
    command ssh $options $target $remote_command
    set -l ssh_status $status
    printf 'stop\n' >"$stop_file"
    wait $reporter_pid 2>/dev/null
    command wayexpand context clear >/dev/null 2>&1
    command rm -f -- "$stop_file"
    set -e WAYEXPAND_CONTEXT_REMOTE_HOST
    __wayexpand_context_report --local
    return $ssh_status
end
