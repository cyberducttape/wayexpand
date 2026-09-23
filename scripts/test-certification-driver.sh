#!/bin/sh
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
test_root=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-certification-driver-test.XXXXXX")
trap 'rm -rf "$test_root"' EXIT INT TERM
driver="$test_root/driver"
cat >"$driver" <<'EOF'
#!/bin/sh
case "$1" in
    failed-insertion) exit 1 ;;
    ime-preedit) exit 2 ;;
    *) [ "$WAYEXPAND_CERTIFICATION_SCENARIO" = "$1" ] ;;
esac
EOF
chmod 0755 "$driver"

results="$test_root/results.txt"
if "$project_dir/scripts/run-certification-driver.sh" \
    --driver "$driver" --compositor kde --version 6.6.2 \
    --backend ibus --layout us --target-apps gtk4-demo,qt6-demo,password-field \
    --output "$results"; then
    printf '%s\n' 'driver accepted failed scenarios' >&2
    exit 1
fi

sorted_results="$test_root/results.sorted"
sort "$results" >"$sorted_results"
jq -R -e -s '
    (split("\n") | map(select(length > 0))) as $results |
    ($results | length == 12) and
    ($results | map(select(. == "printable-press-release=pass")) | length == 1) and
    ($results | map(select(. == "failed-insertion=fail")) | length == 1) and
    ($results | map(select(. == "ime-preedit=UNVERIFIED")) | length == 1)
' "$sorted_results" >/dev/null

printf '%s\n' 'certification driver contract test passed'
