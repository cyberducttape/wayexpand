#!/bin/sh

# gdbus renders the extension State enum as an integer variant. Depending on
# GLib/GJS versions it may include the uint32 type name, so accept both forms.
gnome_extension_state_active() {
    printf '%s\n' "$1" | grep -Eq "'state': <(uint32 )?1>"
}
