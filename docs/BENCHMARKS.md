# Performance baselines

The checked-in matcher benchmark measures the cost of feeding a trigger one
character at a time through an initialized `ExpansionEngine` containing 100,
1,000, or 10,000 snippets. Configuration parsing and engine construction are
excluded from the timed section.

Run it with:

```sh
cargo bench --locked -p wayexpand-core --bench matcher
```

## Baseline: 2026-09-19

Captured on an AMD Ryzen 7 7735U (16 logical CPUs), Linux x86_64, with
`rustc 1.93.1` and Criterion 0.5.1:

| Snippets | Median | 95% interval |
| ---: | ---: | ---: |
| 100 | 799.54 ns | 798.34–800.97 ns |
| 1,000 | 828.36 ns | 824.23–834.94 ns |
| 10,000 | 833.91 ns | 832.91–835.14 ns |

These are machine-specific reference values, not a release performance
guarantee. Compare like-for-like hardware and toolchain when investigating a
regression; Criterion's full reports are written to `target/criterion`.
