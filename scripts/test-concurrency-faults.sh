#!/usr/bin/env bash
set -euo pipefail

# Exercise a reproducible, seeded schedule with concurrent snapshot readers.
# Seeds vary across invocations and can be supplied to reproduce one locally.
for seed in 1 104729 209458; do
    echo "ConfigStore snapshot stress seed: ${seed}"
    WAYEXPAND_STRESS_SEED="${seed}" cargo test --locked -p wayexpand-core \
        store::tests::seeded_snapshot_stress_keeps_configuration_and_generation_coherent
done

# Keep representative fault-injection boundaries visible and fast to rerun.
cargo test --locked -p wayexpand-core \
    config::storage::durability_tests::directory_sync_failure_reports_that_replacement_already_happened
cargo test --locked -p wayexpand-core \
    config::storage::fallback_tests::config_lock_rejects_hard_linked_lock_files
cargo test --locked -p wayexpand-daemon \
    usage::tests::worker_failures_are_counted_and_shutdown_remains_bounded
cargo test --locked -p wayexpand-daemon \
    pending_output::tests::focus_moving_while_a_transaction_types_refuses_the_queued_rest_of_the_batch
cargo test --locked -p action-broker \
    child_process::tests::continuous_stderr_output_cannot_starve_the_action_deadline
cargo test --locked -p action-broker \
    audit::tests::failed_rollback_poisons_writer_and_prevents_a_false_durable_success
