# Performance baselines

The checked-in matcher benchmark measures the cost of feeding a trigger one
character at a time through an initialized `ExpansionEngine` containing 100,
1,000, or 10,000 snippets. Configuration parsing and engine construction are
excluded from the timed section.

Run it with:

```sh
cargo bench --locked -p wayexpand-core --bench matcher
```

## Baseline: 2026-09-30

Captured on an AMD Ryzen 7 7735U (16 logical CPUs), Linux x86_64, with
`rustc 1.93.1` and Criterion 0.8.2. This is the benchmark recording
toolchain; the project MSRV remains Rust 1.87 (see `Cargo.toml`):

| Snippets | Estimate | 95% interval |
| ---: | ---: | ---: |
| 100 | 2.9129 µs | 2.9107–2.9153 µs |
| 1,000 | 2.9298 µs | 2.9276–2.9321 µs |
| 10,000 | 2.8840 µs | 2.8815–2.8867 µs |

These are machine-specific reference values, not a release performance
guarantee. Compare like-for-like hardware and toolchain when investigating a
regression; Criterion's full reports are written to `target/criterion`.
