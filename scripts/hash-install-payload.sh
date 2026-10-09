#!/bin/sh
set -eu

if [ "$#" -ne 1 ] || [ ! -d "$1" ]; then
    printf '%s\n' 'usage: hash-install-payload.sh STAGED_DIRECTORY' >&2
    exit 2
fi

payload_dir=$(CDPATH='' cd -- "$1" && pwd)
manifest=$(
    cd -- "$payload_dir"
    for path in bin/* systemd/* desktop/* ibus/*; do
        if [ ! -f "$path" ] || [ -L "$path" ]; then
            printf '%s\n' "error: staged payload entry is missing or not a regular file: $path" >&2
            exit 1
        fi
        file_hash=$(sha256sum "$path") || exit 1
        printf '%s  %s\n' "${file_hash%% *}" "$path"
    done
)
printf '%s\n' "$manifest" | sha256sum | awk '{print $1}'
