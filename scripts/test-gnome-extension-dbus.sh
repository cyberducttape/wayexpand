#!/bin/sh
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
work=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-gnome-dbus-test.XXXXXX")
trap 'rm -rf -- "$work"' EXIT INT TERM
mkdir "$work/bin"
cat >"$work/bin/gdbus" <<'EOF'
#!/bin/sh
object_path=
method=
while [ "$#" -gt 0 ]; do
    case "$1" in
        --object-path) shift; object_path=$1 ;;
        --method) shift; method=$1 ;;
    esac
    shift
done
printf '%s\n' "$object_path" >>"$GNOME_TEST_CALL_LOG"
[ "$object_path" = "$GNOME_TEST_OBJECT_PATH" ] || exit 1
case "$method" in
    org.gnome.Shell.Extensions.GetExtensionInfo)
        printf '%s\n' "({'state': <uint32 1>},)"
        ;;
    org.gnome.Shell.Extensions.DisableExtension)
        printf '%s\n' '()'
        ;;
    *) exit 2 ;;
esac
EOF
chmod 0755 "$work/bin/gdbus"
PATH="$work/bin:$PATH"
export PATH
# shellcheck disable=SC1091
. "$project_dir/scripts/gnome-extension-dbus.sh"

for supported_path in /org/gnome/Shell/Extensions /org/gnome/Shell; do
    : >"$work/calls"
    GNOME_TEST_OBJECT_PATH=$supported_path
    GNOME_TEST_CALL_LOG="$work/calls"
    export GNOME_TEST_OBJECT_PATH GNOME_TEST_CALL_LOG
    result=$(gnome_shell_extensions_call GetExtensionInfo wayexpand-test)
    case "$result" in *"'state': <uint32 1>"*) ;; *)
        printf '%s\n' "extension state response was not returned: $result" >&2
        exit 1
        ;;
    esac
    gnome_shell_extensions_call DisableExtension wayexpand-test >/dev/null
    if ! tail -n 1 "$work/calls" | grep -Fxq "$supported_path"; then
        printf '%s\n' "extension D-Bus helper did not reach $supported_path" >&2
        exit 1
    fi
done

printf '%s\n' 'GNOME extension D-Bus path compatibility passed'
