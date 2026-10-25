#!/usr/bin/env bash
# Stop processes that belong to one isolated test HOME without touching the
# caller's desktop services or other concurrent test sessions.
reap_private_home_processes() {
    local private_home="$1" pid process_home
    for pid in $(ps -u "$(id -u)" -o pid=); do
        process_home=$({ tr '\0' '\n' <"/proc/$pid/environ"; } 2>/dev/null \
            | sed -n 's/^HOME=//p') || continue
        if [ "$process_home" = "$private_home" ]; then
            kill "$pid" 2>/dev/null || true
        fi
    done
}
