#!/bin/sh

# Return success only when a production-required route has a reviewed,
# explicitly physical input record from the expected Wayland session.
wayexpand_physical_session_attestation_valid() {
    [ "$#" -eq 6 ] || return 1
    [ "$1" = true ] \
        && [ "$2" = 1 ] \
        && [ "$3" = true ] \
        && wayexpand_attestation_value_is_nonblank "$4" \
        && wayexpand_attestation_value_is_nonblank "$5" \
        && wayexpand_attestation_value_is_nonblank "$6"
}

wayexpand_attestation_value_is_nonblank() {
    case "$1" in
        *[![:space:]]*) return 0 ;;
        *) return 1 ;;
    esac
}
