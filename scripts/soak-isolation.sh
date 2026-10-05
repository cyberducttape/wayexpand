#!/bin/sh
# Run a synthetic soak child with only its private runtime and test config.
run_isolated() {
    private_runtime=$1
    private_config=$2
    shift 2
    env -i \
        PATH="$PATH" \
        HOME="$private_runtime/home" \
        TMPDIR="$private_runtime" \
        XDG_RUNTIME_DIR="$private_runtime" \
        WAYEXPAND_CONFIG="$private_config" \
        RUST_LOG=warn \
        "$@"
}
