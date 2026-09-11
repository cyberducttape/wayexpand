## Summary

<!-- What changed and why? -->

## Verification

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo test --locked --workspace`
- [ ] `cargo clippy --locked --workspace --all-targets -- -D warnings`
- [ ] Relevant smoke or integration test run
- [ ] Documentation and changelog updated

## Safety review

- [ ] No typed text, trigger, replacement, or secret is logged
- [ ] New inputs, queues, outputs, and subprocesses are bounded
- [ ] Retryable and permanent failures are classified
- [ ] Configuration changes preserve last-known-good behavior
- [ ] Backend limitations and compositor coverage are documented
