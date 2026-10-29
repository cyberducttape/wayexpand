#!/usr/bin/env bash
set -euo pipefail

# Exercise a reproducible, seeded schedule with concurrent snapshot readers.
# Seeds vary across invocations and can be supplied to reproduce one locally.
run_test() {
    local package=$1 test_name=$2
    shift 2
    local output
    output=$(mktemp "${TMPDIR:-/tmp}/wayexpand-targeted-test.XXXXXX")
    if ! env "$@" cargo test --locked -p "$package" "$test_name" -- --exact 2>&1 | tee "$output"; then
        rm -f "$output"
        return 1
    fi
    if ! grep -F "test $test_name ... ok" "$output" >/dev/null; then
        printf 'error: expected targeted test did not pass: %s::%s\n' \
            "$package" "$test_name" >&2
        rm -f "$output"
        return 1
    fi
    rm -f "$output"
}

for seed in 1 104729 209458; do
    echo "ConfigStore snapshot stress seed: ${seed}"
    run_test wayexpand-core \
        store::tests::seeded_snapshot_stress_keeps_configuration_and_generation_coherent \
        WAYEXPAND_STRESS_SEED="$seed"
done

# Keep representative fault-injection boundaries visible and fast to rerun.
run_test wayexpand-core \
    config::storage::durability_tests::directory_sync_failure_reports_that_replacement_already_happened
run_test wayexpand-core \
    config::storage::fallback_tests::config_lock_rejects_hard_linked_lock_files
run_test wayexpand-daemon \
    usage::tests::worker_failures_are_counted_and_shutdown_remains_bounded
run_test wayexpand-daemon \
    pending_output::tests::focus_moving_during_non_atomic_output_stops_the_stream_and_drops_the_batch
run_test action-broker \
    child_process::tests::continuous_stderr_output_cannot_starve_the_action_deadline
run_test action-broker \
    audit::tests::failed_rollback_poisons_writer_and_prevents_a_false_durable_success
