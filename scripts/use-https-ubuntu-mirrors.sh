#!/bin/sh
# Rewrite Ubuntu archive URLs in an APT configuration root to HTTPS. CI
# containers use outbound HTTPS reliably; some hosted runners block mirror
# traffic over plain HTTP. An optional root argument makes this testable
# without modifying the host's APT configuration.
set -eu

apt_root=${1:-/etc/apt}
if [ ! -d "$apt_root" ]; then
    printf 'error: APT configuration directory does not exist: %s\n' "$apt_root" >&2
    exit 2
fi

for source in \
    "$apt_root/sources.list" \
    "$apt_root"/sources.list.d/*.list \
    "$apt_root"/sources.list.d/*.sources; do
    [ -f "$source" ] || continue
    sed -E -i \
        's#http://([[:alnum:].-]+\.ubuntu\.com)(/|$)#https://\1\2#g' \
        "$source"
done

mkdir -p "$apt_root/apt.conf.d"
cat > "$apt_root/apt.conf.d/99wayexpand-ci-network" <<'EOF'
Acquire::Retries "3";
Acquire::http::Timeout "20";
Acquire::https::Timeout "20";
EOF

if [ -n "${WAYEXPAND_APT_CA_FILE:-}" ]; then
    if [ ! -r "$WAYEXPAND_APT_CA_FILE" ]; then
        printf 'error: configured APT CA bundle is not readable: %s\n' \
            "$WAYEXPAND_APT_CA_FILE" >&2
        exit 2
    fi
    printf 'Acquire::https::CAInfo "%s";\n' "$WAYEXPAND_APT_CA_FILE" \
        >> "$apt_root/apt.conf.d/99wayexpand-ci-network"
fi
