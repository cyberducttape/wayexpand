# Performance baselines

The checked-in matcher benchmark measures the cost of feeding a trigger one
character at a time through an initialized `ExpansionEngine` containing 100,
1,000, or 10,000 snippets. Configuration parsing and engine construction are
excluded from the timed section.

Run it with:

```sh
cargo bench --locked -p wayexpand-core --bench matcher
```

## Baseline: 2026-10-01

Captured on an AMD Ryzen 7 7735U (16 logical CPUs), Linux x86_64, with
`rustc 1.96.0` and Criterion 0.8.2. This is the benchmark recording
toolchain; the project MSRV is Rust 1.95 (see `Cargo.toml`):

| Snippets | Estimate | 95% interval |
| ---: | ---: | ---: |
| 100 | 2.8081 µs | 2.8062–2.8105 µs |
| 1,000 | 2.8166 µs | 2.8151–2.8180 µs |
| 10,000 | 2.8191 µs | 2.8183–2.8200 µs |

These are machine-specific reference values, not a release performance
guarantee. Compare like-for-like hardware and toolchain when investigating a
regression; Criterion's full reports are written to `target/criterion`.

### Amplified construction baseline: 2026-10-04

Same machine and toolchain. The workload builds an `ExpansionEngine` from a
pre-parsed, pre-cloned configuration containing aliases, case propagation,
and decomposable Unicode triggers. At 1,000 snippets it uses eight aliases
per snippet; at 5,000 and 10,000 it uses one alias per snippet to stay below
the effective-trigger cap. The short run used 1 s warmup and 2 s measurement
with 10 samples, so treat these as a rough startup-CPU baseline only:

| Snippets | Estimate | 95% interval |
| ---: | ---: | ---: |
| 1,000 | 38.295 ms | 37.802–38.659 ms |
| 5,000 | 57.517 ms | 57.315–57.715 ms |
| 10,000 | 165.18 ms | 142.78–196.60 ms |

The 10,000-snippet sample had one high outlier. The timed operation is engine
construction; TOML parsing and config cloning are outside the measurement.
It does not measure peak RSS. A separate memory measurement is still needed
before making a maximum-configuration memory claim.

## Configuration editing

The GUI edits a copy of the whole configuration and the save path validates
that copy before writing, so one edit costs a full clone plus a validation.
The `config_edit` benchmark measures both at library sizes up to the
documented 10,000-snippet maximum:

```sh
cargo bench --locked -p wayexpand-core --bench config_edit
```

### Baseline: 2026-10-03

Same machine and toolchain as the matcher baseline above. Each snippet has a
trigger, a one-sentence replacement, a description, and two tags.

| Snippets | Clone | Validate |
| ---: | ---: | ---: |
| 100 | 18.2 µs | 25.9 µs |
| 1,000 | 208.7 µs | 306.1 µs |
| 10,000 | 2.14 ms | 3.32 ms |

At the maximum library size an edit costs about 5.5 ms of copying and
validation, well under one 60 Hz frame and only on an explicit edit, never
per frame. Revisit the copy-then-validate model (for example with
per-snippet diffs) only if this grows past a frame budget.
