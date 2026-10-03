#!/bin/sh
# Signed packs: sign with an SSH key, verify against allowed_signers, and
# reject tampering, untrusted signers, undeclared capabilities, and packs
# that need a newer WayExpand.
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
root=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-pack-signing.XXXXXX")
trap 'rm -rf "$root"' EXIT INT TERM
cli="$project_dir/target/debug/wayexpand"
cargo build --locked -q -p wayexpand
command -v ssh-keygen >/dev/null || { echo "ssh-keygen is required" >&2; exit 2; }

ssh-keygen -q -t ed25519 -N '' -C support-team -f "$root/team" </dev/null
ssh-keygen -q -t ed25519 -N '' -C stranger -f "$root/stranger" </dev/null
printf 'support@example.com namespaces="wayexpand-pack" %s\n' "$(cat "$root/team.pub")" >"$root/allowed_signers"

pack="$root/support-team"
mkdir -p "$pack/snippets"
cat >"$pack/wayexpand-pack.toml" <<'TOML'
format_version = 1
id = "com.example.support"
version = "1.0.0"
name = "Support team"
publisher = "Example Support"
capabilities = []
TOML
cat >"$pack/snippets/replies.toml" <<'TOML'
[[expansion]]
trigger = ";;thanks"
replacement = "Thanks for contacting support."
TOML

"$cli" pack inspect "$pack" | grep -F 'signature: unsigned' >/dev/null
"$cli" pack sign "$pack" --key "$root/team" >/dev/null
"$cli" pack verify "$pack" --signers "$root/allowed_signers" | grep -F 'signed by support@example.com' >/dev/null
"$cli" pack import "$pack" --signers "$root/allowed_signers" 2>&1 >/dev/null | grep -F 'signed by support@example.com' >/dev/null

# Tampering with any signed file breaks the signature.
printf '\n# edited\n' >>"$pack/snippets/replies.toml"
if "$cli" pack verify "$pack" --signers "$root/allowed_signers" >/dev/null 2>&1; then
    echo "a tampered pack verified" >&2; exit 1
fi
"$cli" pack sign "$pack" --key "$root/team" >/dev/null

# An untrusted signer is rejected.
"$cli" pack sign "$pack" --key "$root/stranger" >/dev/null
if "$cli" pack verify "$pack" --signers "$root/allowed_signers" >/dev/null 2>&1; then
    echo "an untrusted signer was accepted" >&2; exit 1
fi

# Capabilities used must be declared.
cat >>"$pack/snippets/replies.toml" <<'TOML'

[[expansion]]
trigger = ";;quote"
replacement = "> {{clipboard}}"
TOML
if "$cli" pack inspect "$pack" >"$root/out" 2>&1; then
    echo "an undeclared capability was accepted" >&2; exit 1
fi
grep -F 'clipboard' "$root/out" >/dev/null

# A pack for a newer WayExpand is refused.
sed -i 's/^capabilities = \[\]/capabilities = ["clipboard"]\nmin_wayexpand_version = "999.0.0"/' "$pack/wayexpand-pack.toml"
if "$cli" pack inspect "$pack" >"$root/out" 2>&1; then
    echo "a pack for a newer version was accepted" >&2; exit 1
fi
grep -F '999.0.0' "$root/out" >/dev/null
echo "pack signing test passed"
