#!/bin/sh
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
binary=${1:-"$project_dir/target/debug/wayexpand"}
if [ ! -x "$binary" ]; then
    printf '%s\n' "error: WayExpand CLI is not executable: $binary" >&2
    exit 1
fi

test_dir=$(mktemp -d "$project_dir/wayexpand-support-test.XXXXXX")
trap 'rm -rf -- "$test_dir"' EXIT INT TERM
config_home="$test_dir/config"
mkdir -p "$config_home/wayexpand"
chmod 0700 "$config_home" "$config_home/wayexpand"
cat >"$config_home/wayexpand/expansions.toml" <<'EOF'
[[expansion]]
trigger = ":secret-support-trigger"
replacement = "SUPPORT_REPLACEMENT_SECRET"
EOF
chmod 0600 "$config_home/wayexpand/expansions.toml"

report="$test_dir/report.json"
XDG_CONFIG_HOME="$config_home" "$binary" support-bundle >"$report"
jq -e '
  .schema == 1 and
  .configuration.valid == true and
  (.application.version | type == "string") and
  .certification.required_scenario_count == 45 and
  (.certification.check_status_counts["not-run"] == 45)
' "$report" >/dev/null
for secret in "$config_home" ':secret-support-trigger' 'SUPPORT_REPLACEMENT_SECRET'; do
    if grep -F "$secret" "$report" >/dev/null; then
        printf '%s\n' "support report leaked sensitive input: $secret" >&2
        exit 1
    fi
done

# File output is private and non-overwriting, independent of the caller umask.
umask 000
XDG_CONFIG_HOME="$config_home" "$binary" support-bundle --output "$test_dir/private.json" >/dev/null
[ "$(stat -c '%a' "$test_dir/private.json")" = 600 ]
jq -e '.schema == 1' "$test_dir/private.json" >/dev/null
before=$(sha256sum "$test_dir/private.json")
if XDG_CONFIG_HOME="$config_home" "$binary" support-bundle --output "$test_dir/private.json" >/dev/null 2>&1; then
    printf '%s\n' 'support bundle overwrote an existing file' >&2
    exit 1
fi
[ "$(sha256sum "$test_dir/private.json")" = "$before" ]

printf '%s\n' 'support bundle test passed'
