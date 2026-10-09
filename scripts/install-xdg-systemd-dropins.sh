#!/bin/sh
set -eu

if [ "$#" -ne 6 ]; then
    printf '%s\n' "usage: $0 UNIT_DIR CONFIG_HOME CONFIG_DIR STATE_HOME STATE_DIR BIN_DIR" >&2
    exit 2
fi
unit_dir=$1
config_home=$2
config_dir=$3
state_home=$4
state_dir=$5
bin_dir=$6

# Quote systemd values and suppress specifier expansion in operator paths.
systemd_quote() {
    printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g; s/%/%%/g'
}
for path in "$config_home" "$config_dir" "$state_home" "$state_dir" "$bin_dir"; do
    case "$path" in
        /*) ;;
        *) printf '%s\n' "error: systemd paths must be absolute: $path" >&2; exit 1 ;;
    esac
    case "$path" in
        *'
'*) printf '%s\n' 'error: systemd paths may not contain newlines' >&2; exit 1 ;;
    esac
done

write_dropin() {
    destination=$1
    content=$2
    directory=${destination%/*}
    install -d -m 0755 "$directory"
    temporary="$destination.tmp.$$"
    (umask 022; printf '%s\n' "$content" >"$temporary")
    chmod 0644 "$temporary"
    mv -f -- "$temporary" "$destination"
}

config_home_q=$(systemd_quote "$config_home")
config_dir_q=$(systemd_quote "$config_dir")
state_home_q=$(systemd_quote "$state_home")
state_dir_q=$(systemd_quote "$state_dir")
bin_dir_q=$(systemd_quote "$bin_dir")
for service in wayexpand-input-method.service wayexpand-evdev.service; do
    write_dropin "$unit_dir/$service.d/10-xdg-paths.conf" "[Service]
Environment=\"XDG_CONFIG_HOME=$config_home_q\"
Environment=\"XDG_STATE_HOME=$state_home_q\"
Environment=\"WAYEXPAND_PORTAL_TOKEN_PATH=$config_dir_q/libei-portal-token\"
ReadWritePaths=
ReadWritePaths=%t \"$config_dir_q\" \"$state_dir_q\""
done

write_dropin "$unit_dir/wayexpand-action-broker.service.d/10-xdg-paths.conf" "[Service]
Environment=\"XDG_CONFIG_HOME=$config_home_q\"
Environment=\"XDG_STATE_HOME=$state_home_q\"
ExecStart=
ExecStart=\"$bin_dir_q/wayexpand-action-broker\" --config \"$config_dir_q/broker.toml\" --socket %t/wayexpand-broker.sock
ReadWritePaths=
ReadWritePaths=%t \"$config_dir_q\" \"$state_dir_q\""

# Remove the older installer-generated broker override on upgrade; the new
# drop-in replaces its writable-path list and incorporates the XDG state path.
legacy="$unit_dir/wayexpand-action-broker.service.d/10-state-directory.conf"
if [ -e "$legacy" ]; then
    rm -f -- "$legacy"
fi
