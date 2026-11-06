#!/bin/sh

# GNOME Shell has exposed the extension-management interface at both paths
# across supported releases. Try the original path first, then the current
# Shell object path.
gnome_shell_extensions_call() {
    method=$1
    uuid=$2
    response=
    for object_path in /org/gnome/Shell/Extensions /org/gnome/Shell; do
        if response=$(gdbus call --session --dest org.gnome.Shell --object-path "$object_path" \
            --method "org.gnome.Shell.Extensions.$method" "$uuid" 2>&1); then
            printf '%s\n' "$response"
            return 0
        fi
    done
    printf '%s\n' "$response"
    return 1
}
