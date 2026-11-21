#!/bin/sh

# Shared by the source and release installers. Keep the public route guidance
# and local GNOME version diagnosis in one place so both install paths agree.
wayexpand_print_desktop_compatibility() {
    printf '%s\n' "Desktop compatibility (likely route; verify in your own applications):"
    printf '%s\n' "  GNOME Shell 46, 48, 50, 51: IBus + WayExpand extension (headless-tested; no real-session certification)"
    printf '%s\n' "  GNOME Shell 49: extension support undeclared; German/French layout switching unverified in a real session"
    printf '%s\n' "  Other GNOME Shell majors: extension support undeclared; compatibility unknown"
    printf '%s\n' "  KDE Plasma / other Wayland desktops: setup checks available routes; application behavior is unverified"
    printf '%s\n' "  If no automatic route is verified, Quick Picker can copy snippets for manual paste."
    case ":${XDG_CURRENT_DESKTOP:-}:${XDG_SESSION_DESKTOP:-}:" in
        *:GNOME:*|*:gnome:*)
            if command -v gnome-shell >/dev/null 2>&1; then
                detected_gnome_shell_version=$(gnome-shell --version 2>/dev/null || true)
                if [ -n "$detected_gnome_shell_version" ]; then
                    detected_gnome_shell_major=$(printf '%s\n' "$detected_gnome_shell_version" | awk '{for (i = 1; i <= NF; i++) if ($i ~ /^[0-9]+([.][0-9]+)*$/) {split($i, parts, "."); print parts[1]; exit}}')
                    printf '%s\n' "Detected GNOME Shell: $detected_gnome_shell_version"
                    case "$detected_gnome_shell_major" in
                        46|48|50|51)
                            printf '%s\n' "  This GNOME major has headless integration coverage; real-session insertion is not certified."
                            ;;
                        49)
                            printf '%s\n' "  GNOME 49 extension support is undeclared; German/French layout switching is unverified in a real session."
                            ;;
                        '')
                            printf '%s\n' "  GNOME Shell version could not be parsed; compare it with the compatibility table above."
                            ;;
                        *)
                            printf '%s\n' "  GNOME Shell $detected_gnome_shell_major is not a declared extension target; compatibility is unknown."
                            ;;
                    esac
                else
                    printf '%s\n' "GNOME Shell is installed but its version could not be read; compare it with the compatibility table above."
                fi
            else
                printf '%s\n' "Check GNOME version with: gnome-shell --version"
            fi
            ;;
    esac
    printf '%s\n' ""
}
