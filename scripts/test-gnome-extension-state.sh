#!/bin/sh
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
. "$project_dir/scripts/gnome-extension-state.sh"

gnome_extension_state_active "({'state': <1>, 'uuid': <'wayexpand'>},)"
gnome_extension_state_active "({'state': <uint32 1>, 'uuid': <'wayexpand'>},)"

if gnome_extension_state_active "({'state': <0>, 'uuid': <'wayexpand'>},)"; then
    printf '%s\n' 'disabled extension was reported active' >&2
    exit 1
fi
if gnome_extension_state_active "({'state': <11>, 'uuid': <'wayexpand'>},)"; then
    printf '%s\n' 'unrecognized extension state was reported active' >&2
    exit 1
fi
if gnome_extension_state_active "({'error': <'extension not found'>},)"; then
    printf '%s\n' 'missing extension was reported active' >&2
    exit 1
fi

printf '%s\n' 'GNOME extension state parsing passed'
