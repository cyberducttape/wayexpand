#!/bin/sh
# Run a compositor-specific certification driver against every scenario in the
# checked-in matrix. The driver owns the real GTK/Qt/client interaction; this
# wrapper owns scenario coverage and result normalization.
set -eu

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
            printf '%s\n' 'driver contract: argv[1] is the scenario; exit 0=pass, 1=fail, 2=unverified'
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
[ -n "$output" ] || { printf '%s\n' 'error: --output is required' >&2; exit 2; }
target_apps_lower=$(printf '%s' "$target_apps" | tr '[:upper:]' '[:lower:]')
required_client_markers=$(jq -r --arg compositor "$compositor" \
    '.targets[] | select(.id == $compositor) | .required_client_markers[]' "$matrix")
while IFS= read -r marker; do
    [ -n "$marker" ] || continue
    case "$target_apps_lower" in
        *"$marker"*) ;;
        *) printf '%s\n' "error: --target-apps must include a client matching '$marker'" >&2; exit 2 ;;
    esac
done <<EOF
$required_client_markers
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
    log="$log_dir/$scenario.log"
    if WAYEXPAND_CERTIFICATION_COMPOSITOR="$compositor" \
        WAYEXPAND_CERTIFICATION_VERSION="$compositor_version" \
        WAYEXPAND_CERTIFICATION_BACKEND="$backend" \
        WAYEXPAND_CERTIFICATION_LAYOUT="$keyboard_layout" \
        WAYEXPAND_CERTIFICATION_TARGET_APPS="$target_apps" \
        WAYEXPAND_CERTIFICATION_SCENARIO="$scenario" \
        "$driver" "$scenario" >"$log" 2>&1; then
        result=pass
    else
        exit_code=$?
        case "$exit_code" in
            1) result=fail; driver_status=1 ;;
            2) result=UNVERIFIED; driver_status=1 ;;
            *)
                printf '%s\n' "error: driver failed unexpectedly for $scenario (exit $exit_code)" >&2
                exit 2
                ;;
        esac
    fi
    printf '%s=%s\n' "$scenario" "$result" >>"$output"
done <<EOF
$scenarios
EOF

printf '%s\n' "wrote $output"
exit "$driver_status"
