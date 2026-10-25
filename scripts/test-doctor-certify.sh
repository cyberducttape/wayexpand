#!/bin/sh
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
binary=${1:-"$project_dir/target/debug/wayexpand"}
report=$(mktemp "${TMPDIR:-/tmp}/wayexpand-doctor-certify.XXXXXX")
trap 'rm -f "$report" "$report.invalid"' EXIT INT TERM

command -v jq >/dev/null 2>&1 || {
    printf '%s\n' 'error: jq is required' >&2
    exit 2
}
[ -x "$binary" ] || {
    printf '%s\n' "error: WayExpand binary is not executable: $binary" >&2
    exit 2
}

if "$binary" doctor --certify unexpected >"$report.invalid" 2>&1; then
    printf '%s\n' 'doctor --certify accepted an unexpected positional argument' >&2
    exit 1
fi
grep -F 'usage: wayexpand doctor --certify [--json]' "$report.invalid" >/dev/null

"$binary" doctor --certify --json >"$report"
jq -e '
    .schema == 1 and
    (.generated_at_unix | type == "number" and . > 0) and
    (.wayexpand.version | type == "string" and length > 0) and
    (.wayexpand.commit | type == "string" and length > 0) and
    .overall == "partial" and
    .certified == false and
    .test_session.desktop == "Private headless KWin / Wayland" and
    .test_session.compositor == "KWin / Wayland" and
    .test_session.isolation == "private headless compositor session" and
    .test_session.capture_route == "stdin simulation" and
    .test_session.capture_source == "stdin" and
    (.test_session.injection_backend | type == "string" and length > 0) and
    (.test_session.negotiated_insertion_mode | type == "string" and length > 0) and
    (.test_session.backend_mode_detail | type == "string" and length > 0) and
    (.test_session.sensitive_field_detection | type == "boolean") and
    ([.checks[] | select(.name == "simple-trigger-insertion") | .status] == ["passed"]) and
    ([.checks[] | select(.name == "word-boundary-delimiter") | .status] == ["passed"]) and
    ([.checks[] | select(.name == "cursor-placement") | .status] == ["passed"]) and
    ([.checks[] | select(.name == "multiline-replacement") | .status] == ["passed"]) and
    ([.checks[] | select(.name == "focus-switching") | .status] == ["passed"]) and
    ([.checks[] | select(.name == "focus-change-during-expansion") | .status] == ["passed"]) and
    ([.checks[] | select(.name == "target-closes-during-expansion") | .status] == ["passed"]) and
    ([.checks[] | select(.name == "command-completion") | .status] == ["passed"]) and
    ([.checks[] | select(.name == "configuration-reload") | .status] == ["passed"]) and
    ([.checks[] | select(.name == "daemon-restart") | .status] == ["passed"]) and
    ([.checks[] | select(.name == "unavailable-unicode-refusal") | .status] == ["passed"]) and
    ([.checks[] | select(.name == "unicode-insertion") | .status] | length == 1 and
        (.[0] == "passed" or .[0] == "not-tested")) and
    ([.checks[] | select(.name == "suspend-resume") | .status] == ["not-tested"]) and
    ([.checks[] | select(.status == "not-run")] | length == 0) and
    (tostring | contains("café 東京") | not)
' "$report" >/dev/null

printf '%s\n' 'isolated doctor certification contract passed'
