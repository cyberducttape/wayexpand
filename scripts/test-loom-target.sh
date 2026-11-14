#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
    echo "usage: $0 PACKAGE TEST_FILTER" >&2
    exit 2
fi

package=$1
test_filter=$2
test_list=$(cargo test --locked -p "$package" --features loom-tests "$test_filter" -- --list)
if ! grep -Eq ': test$' <<<"$test_list"; then
    echo "error: Loom test filter matched no tests: $package::$test_filter" >&2
    exit 1
fi

cargo test --locked -p "$package" --features loom-tests "$test_filter"
