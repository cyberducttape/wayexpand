#!/bin/sh
# Find processes owned by this user that belong to a private certification
# run. Some desktop launchers delegate an application to systemd --user, which
# gives it the user's normal HOME even though its argv names a file under the
# private test tree.

certification_private_pids() {
    uid=$1
    private_root=$2
    proc_root=${3:-/proc}

    for process_dir in "$proc_root"/[0-9]*; do
        [ -d "$process_dir" ] || continue
        pid=${process_dir##*/}
        owner=$(stat -c '%u' "$process_dir" 2>/dev/null) || continue
        [ "$owner" = "$uid" ] || continue

        belongs=false
        if [ -r "$process_dir/environ" ]; then
            environment=$(cat "$process_dir/environ" 2>/dev/null | tr '\000' '\n') || environment=
            case "$environment" in
                *"HOME=$private_root/"*|*"XDG_CONFIG_HOME=$private_root/"*|*"XDG_DATA_HOME=$private_root/"*|*"XDG_CACHE_HOME=$private_root/"*)
                    belongs=true
                    ;;
            esac
        fi
        if [ "$belongs" != true ] && [ -r "$process_dir/cmdline" ]; then
            arguments=$(tr '\000' '\n' <"$process_dir/cmdline" 2>/dev/null) || arguments=
            case "$arguments" in
                *"$private_root/"*) belongs=true ;;
            esac
        fi
        [ "$belongs" = true ] && printf '%s\n' "$pid"
    done
    return 0
}
