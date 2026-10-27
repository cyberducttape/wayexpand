#!/bin/sh

kwin_e2e_capture_label() {
    case "$1" in
        stdin) printf '%s' 'stdin-simulated' ;;
        input-method) printf '%s' 'input-method-v2-requested' ;;
        *) return 2 ;;
    esac
}
