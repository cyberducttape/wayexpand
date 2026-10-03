#!/bin/sh
# Exercise `wayexpand sync` between two configuration directories ("machines")
# and a bare remote: changes travel both ways, private files stay local, an
# invalid library is never committed, and a remote change that breaks the
# library is rolled back to the local version.
set -eu

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
test_parent="$project_dir/.test-sync-tmp"
mkdir -m 700 "$test_parent"
root=$(mktemp -d "$test_parent/wayexpand-sync.XXXXXX")
trap 'rm -rf "$root" "$test_parent"' EXIT INT TERM
cli="$project_dir/target/debug/wayexpand"
cargo build --locked -q -p wayexpand

export GIT_AUTHOR_NAME=WayExpand GIT_AUTHOR_EMAIL=sync@example.invalid
export GIT_COMMITTER_NAME=WayExpand GIT_COMMITTER_EMAIL=sync@example.invalid
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
# No daemon: the reload request after a sync simply fails.
export XDG_RUNTIME_DIR="$root/run"
mkdir -m 700 "$root/run" "$root/a" "$root/b"
git init --quiet --bare "$root/remote.git"

library() {
    printf '[[expansion]]\nid = "%s"\ntrigger = "%s"\nreplacement = "%s"\n' "$2" "$3" "$4" >>"$1/expansions.toml"
    chmod 600 "$1/expansions.toml"
}
library "$root/a" 11111111-1111-4111-8111-111111111111 ":sig" "Best regards"
printf 'secret-token' >"$root/a/libei-portal-token"
chmod 600 "$root/a/libei-portal-token"

"$cli" sync init --remote "$root/remote.git" "$root/a/expansions.toml" >/dev/null
"$cli" sync "$root/a/expansions.toml" | grep -F 'pushed to origin' >/dev/null
# Private files never leave the machine.
if git -C "$root/a" ls-files | grep -F libei-portal-token >/dev/null; then
    echo "portal token was tracked" >&2; exit 1
fi

# Machine B starts from the remote.
git clone --quiet "$root/remote.git" "$root/b/repo"
mv "$root/b/repo/.git" "$root/b/repo/.gitignore" "$root/b/repo/expansions.toml" "$root/b/"
rmdir "$root/b/repo"
chmod 600 "$root/b/expansions.toml"
library "$root/b" 22222222-2222-4222-8222-222222222222 ":addr" "1 Main St"
"$cli" sync "$root/b/expansions.toml" >/dev/null

# A pulls B's snippet and stays valid.
"$cli" sync "$root/a/expansions.toml" | grep -F 'pulled remote changes' >/dev/null
"$cli" list "$root/a/expansions.toml" | grep -F ':addr' >/dev/null

# An invalid local library is never committed.
cp "$root/a/expansions.toml" "$root/a/good.toml"
printf '[[expansion]]\ntrigger = \n' >>"$root/a/expansions.toml"
if "$cli" sync "$root/a/expansions.toml" >/dev/null 2>&1; then
    echo "an invalid library was synchronized" >&2; exit 1
fi
cp "$root/a/good.toml" "$root/a/expansions.toml"
rm "$root/a/good.toml"

# A remote change that breaks the library is rolled back locally.
library "$root/b" 33333333-3333-4333-8333-333333333333 ":sig" "duplicate trigger"
git -C "$root/b" commit --quiet -am "break the library"
git -C "$root/b" push --quiet origin HEAD
if "$cli" sync "$root/a/expansions.toml" >"$root/out" 2>&1; then
    echo "a broken remote library was accepted" >&2; exit 1
fi
grep -F 'kept the local library' "$root/out" >/dev/null
"$cli" validate "$root/a/expansions.toml" >/dev/null
# A real conflict (both machines change the same snippet) leaves the local
# library unchanged and loadable.
git -C "$root/b" reset --quiet --hard HEAD~1
git -C "$root/b" push --quiet --force origin HEAD
"$cli" sync "$root/a/expansions.toml" >/dev/null 2>&1 || true
sed -i 's/Best regards/Kind regards/' "$root/a/expansions.toml"
"$cli" sync "$root/a/expansions.toml" >/dev/null
git -C "$root/b" pull --quiet --rebase origin HEAD 2>/dev/null || git -C "$root/b" fetch --quiet origin
sed -i 's/Best regards/Warm regards/; s/Kind regards/Warm regards/' "$root/b/expansions.toml"
git -C "$root/b" commit --quiet -am "edit signature on B" || true
git -C "$root/b" push --quiet --force origin HEAD
sed -i 's/Kind regards/Cheers/' "$root/a/expansions.toml"
if "$cli" sync "$root/a/expansions.toml" >"$root/out" 2>&1; then
    echo "a conflicting sync was reported as clean" >&2; exit 1
fi
grep -F 'conflicts with local changes' "$root/out" >/dev/null
grep -F 'Cheers' "$root/a/expansions.toml" >/dev/null
"$cli" validate "$root/a/expansions.toml" >/dev/null
echo "sync test passed"
