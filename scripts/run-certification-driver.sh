#!/bin/sh
# Run a compositor-specific certification driver against every scenario in the
# checked-in matrix. The driver owns the real GTK/Qt/client interaction; this
# wrapper owns scenario coverage and result normalization.
set -eu
set -f

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
matrix="$project_dir/tests/certification/compositor-matrix.json"
driver=
compositor=
compositor_version=
backend=
keyboard_layout=
target_apps=
output=
log_dir=

command -v jq >/dev/null 2>&1 || {
    printf '%s\n' 'error: jq is required to validate the certification matrix' >&2
    exit 2
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --driver) driver=${2:?missing value for --driver}; shift 2 ;;
        --compositor) compositor=${2:?missing value for --compositor}; shift 2 ;;
        --version) compositor_version=${2:?missing value for --version}; shift 2 ;;
        --backend) backend=${2:?missing value for --backend}; shift 2 ;;
        --layout) keyboard_layout=${2:?missing value for --layout}; shift 2 ;;
        --target-apps) target_apps=${2:?missing value for --target-apps}; shift 2 ;;
        --output) output=${2:?missing value for --output}; shift 2 ;;
        --log-dir) log_dir=${2:?missing value for --log-dir}; shift 2 ;;
        --help|-h)
            printf '%s\n' "usage: $0 --driver PATH --compositor NAME --version VERSION --backend BACKEND --layout LAYOUT --target-apps APPS --output RESULTS [--log-dir DIR]"
            printf '%s\n' 'driver contract: argv[1] is the scenario; WAYEXPAND_CERTIFICATION_LAYOUT and WAYEXPAND_CERTIFICATION_TARGET_APP identify the required matrix cell. Exit 0=pass, 1=fail, 2=unverified, 3=unsupported-by-design.'
            exit 0
            ;;
        *) printf '%s\n' "error: unknown option $1" >&2; exit 2 ;;
    esac
done

[ -n "$driver" ] || { printf '%s\n' 'error: --driver is required' >&2; exit 2; }
[ -x "$driver" ] || { printf '%s\n' "error: driver is not executable: $driver" >&2; exit 2; }
[ -n "$compositor" ] || { printf '%s\n' 'error: --compositor is required' >&2; exit 2; }
[ -n "$compositor_version" ] || { printf '%s\n' 'error: --version is required' >&2; exit 2; }
[ -n "$backend" ] || { printf '%s\n' 'error: --backend is required' >&2; exit 2; }
[ -n "$keyboard_layout" ] || { printf '%s\n' 'error: --layout is required' >&2; exit 2; }
[ -n "$target_apps" ] || { printf '%s\n' 'error: --target-apps is required' >&2; exit 2; }
jq -en --arg apps "$target_apps" '
    ($apps | split(",")) as $items |
    ([ $items[] | select(test("^[a-zA-Z0-9._+-]+$")) ] | length) == ($items | length) and
    ($items | unique | length) == ($items | length)
' >/dev/null || {
    printf '%s\n' 'error: --target-apps must be a unique comma-separated list of simple client identifiers' >&2
    exit 2
}
[ -n "$output" ] || { printf '%s\n' 'error: --output is required' >&2; exit 2; }
required_client_markers=$(jq -r --arg compositor "$compositor" \
    '.targets[] | select(.id == $compositor) | .required_client_markers[]' "$matrix")
while IFS= read -r marker; do
    [ -n "$marker" ] || continue
    if ! jq -en --arg apps "$target_apps" --arg marker "$marker" '
        any(($apps | split(","))[];
            (ascii_downcase | test("(^|[._+-])" + ($marker | ascii_downcase) + "([0-9._+-]|$)")))
    ' >/dev/null; then
        printf '%s\n' "error: --target-apps must include a client matching '$marker'" >&2
        exit 2
    fi
done <<EOF
$required_client_markers
EOF

layout_profiles_lower=$(printf '%s' "$keyboard_layout" | tr '[:upper:]' '[:lower:]')
required_layout_profiles=$(jq -r '.required_layout_profiles[]' "$matrix")
while IFS= read -r profile; do
    [ -n "$profile" ] || continue
    case ",$layout_profiles_lower," in
        *,"$profile",*) ;;
        *) printf '%s\n' "error: --layout must include required profile '$profile'" >&2; exit 2 ;;
    esac
done <<EOF
$required_layout_profiles
EOF

jq -e --arg compositor "$compositor" --arg backend "$backend" \
    'any(.targets[]; .id == $compositor and (.input_paths | index($backend) != null))' \
    "$matrix" >/dev/null || {
    printf '%s\n' "error: backend '$backend' is not a declared certification path for $compositor" >&2
    exit 2
}

scenarios=$(jq -r '.required_scenarios[]' "$matrix")
mkdir -p "$(dirname -- "$output")"
if [ -z "$log_dir" ]; then
    log_dir="$(dirname -- "$output")/driver-logs"
fi
mkdir -p "$log_dir"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-certification-driver.XXXXXX")
trap 'rm -rf "$tmp"' EXIT INT TERM

: >"$output"
driver_status=0
while IFS= read -r scenario; do
    [ -n "$scenario" ] || continue
    old_ifs=$IFS
    IFS=,
    # shellcheck disable=SC2086 # Split the validated CSV on its explicit comma IFS; globbing is disabled.
    set -- $keyboard_layout
    IFS=$old_ifs
    for layout_profile do
        layout_profile=$(printf '%s' "$layout_profile" | tr '[:upper:]' '[:lower:]')
        old_ifs=$IFS
        IFS=,
        # shellcheck disable=SC2086 # Split the validated CSV on its explicit comma IFS; globbing is disabled.
        set -- $target_apps
        IFS=$old_ifs
        for target_app do
            cell_key="$scenario|$layout_profile|$target_app"
            safe_log=$(printf '%s' "$cell_key" | tr '|' '_')
            log="$log_dir/$safe_log.log"
            if WAYEXPAND_CERTIFICATION_COMPOSITOR="$compositor" \
                WAYEXPAND_CERTIFICATION_VERSION="$compositor_version" \
                WAYEXPAND_CERTIFICATION_BACKEND="$backend" \
                WAYEXPAND_CERTIFICATION_LAYOUT="$layout_profile" \
                WAYEXPAND_CERTIFICATION_LAYOUT_PROFILES="$keyboard_layout" \
                WAYEXPAND_CERTIFICATION_TARGET_APP="$target_app" \
                WAYEXPAND_CERTIFICATION_TARGET_APPS="$target_apps" \
                WAYEXPAND_CERTIFICATION_SCENARIO="$scenario" \
                "$driver" "$scenario" >"$log" 2>&1; then
                result=pass
            else
                exit_code=$?
                case "$exit_code" in
                    1) result=fail; driver_status=1 ;;
                    2) result=UNVERIFIED; driver_status=1 ;;
                    3) result=unsupported-by-design; driver_status=1 ;;
                    *)
                        printf '%s\n' "error: driver failed unexpectedly for $cell_key (exit $exit_code)" >&2
                        exit 2
                        ;;
                esac
            fi
            printf '%s=%s\n' "$cell_key" "$result" >>"$output"
        done
    done
done <<EOF
$scenarios
EOF

printf '%s\n' "wrote $output"
exit "$driver_status"
