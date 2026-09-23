#!/bin/sh
# Enforce the security properties relied on by the user services. The
# systemd-analyze security score is useful context, but this explicit contract
# is what makes hardening regressions fail CI.
set -eu

units='systemd/wayexpand.service systemd/wayexpand-input-method.service systemd/wayexpand-evdev.service'
required='NoNewPrivileges=yes UMask=0077 PrivateTmp=yes ProtectSystem=strict ProtectHome=read-only ProtectHostname=yes ProtectProc=invisible ProcSubset=pid SystemCallArchitectures=native ProtectKernelTunables=yes ProtectControlGroups=yes RestrictSUIDSGID=yes RestrictNamespaces=yes RestrictRealtime=yes LockPersonality=yes MemoryDenyWriteExecute=yes RestrictAddressFamilies=AF_UNIX MemoryMax=256M TasksMax=32 LimitNOFILE=64 LimitCORE=0'

for unit in $units; do
    [ -f "$unit" ] || { printf '%s\n' "missing unit: $unit" >&2; exit 1; }
    for expected in $required; do
        if ! grep -Fx "$expected" "$unit" >/dev/null; then
            printf '%s\n' "$unit: required hardening directive missing or changed: $expected" >&2
            exit 1
        fi
    done
done

printf '%s\n' "systemd hardening contract passed for: $units"
