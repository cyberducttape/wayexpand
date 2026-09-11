# Contributing to WayExpand

Development policy and the complete local verification workflow are documented
in [`docs/wiki/Contributing.md`](docs/wiki/Contributing.md).

Every change should preserve the project's operational guarantees:

- invalid configuration never replaces a working configuration;
- user text and snippet contents never enter logs;
- input, output, queues, and child processes remain bounded;
- compositor-specific behavior stays behind backend boundaries;
- known support limitations are documented with reproducible evidence.

Run the same checks used by CI before opening a pull request:

```sh
cargo fmt --all -- --check
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
shellcheck scripts/*.sh
git diff --check
```
