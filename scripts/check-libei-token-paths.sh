#!/bin/sh
# Verify that all units capable of using libei have the necessary token storage
# configuration (Environment and ReadWritePaths) to securely persist portal
# restoration tokens. This prevents silent failures where libei setup succeeds
# but token storage is denied by systemd hardening. Both evdev (explicit --backend=libei)
# and input-method (can use libei via pass-through) sources must have this configured.
set -eu

# Units that can use libei: evdev always, input-method with pass-through support
libei_capable_units='systemd/wayexpand-evdev.service systemd/wayexpand-input-method.service'

failed=0

for unit in $libei_capable_units; do
    [ -f "$unit" ] || { printf '%s\n' "missing unit: $unit" >&2; exit 1; }

    # Check for Environment=WAYEXPAND_PORTAL_TOKEN_PATH
    if ! grep -q 'Environment=WAYEXPAND_PORTAL_TOKEN_PATH' "$unit"; then
        printf '%s\n' "$unit: missing WAYEXPAND_PORTAL_TOKEN_PATH environment override" >&2
        failed=1
    fi

    # Check for .config/wayexpand in ReadWritePaths
    if ! grep -q 'ReadWritePaths=.*%h/.config/wayexpand' "$unit"; then
        printf '%s\n' "$unit: missing %h/.config/wayexpand in ReadWritePaths (required for token persistence)" >&2
        failed=1
    fi
done

if [ "$failed" -eq 1 ]; then
    exit 1
fi

printf '%s\n' "libei token storage contract passed for: $libei_capable_units"
