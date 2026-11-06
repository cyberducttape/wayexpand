#!/bin/sh

# gdbus renders the extension State enum using the numeric type selected by
# GNOME Shell's GJS/D-Bus bridge. Accept its integer and floating-point forms,
# with or without the uint32 annotation, but match the complete enum value.
gnome_extension_state_active() {
    printf '%s\n' "$1" | grep -Eq "'state': <(uint32 )?1([.]0+)?>"
}

gnome_extension_state_inactive() {
    printf '%s\n' "$1" | grep -Eq "'state': <(uint32 )?2([.]0+)?>"
}
