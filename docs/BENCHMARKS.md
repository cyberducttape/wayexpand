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
the effective-trigger cap. This run used a 2 s warmup and 5 s measurement
with 20 samples:

| Snippets | Estimate | 95% interval |
| ---: | ---: | ---: |
| 1,000 | 38.706 ms | 38.199–39.192 ms |
| 5,000 | 57.176 ms | 56.914–57.472 ms |
| 10,000 | 122.88 ms | 122.36–123.47 ms |

The timed operation is engine construction; TOML parsing and config cloning
are outside the measurement. The full benchmark process peaked at 125,304 KiB
RSS under GNU `time -v`. This includes the benchmark harness and prepared
configuration, so it is process-level evidence rather than isolated matcher
memory. Reproduce the RSS observation by building the benchmark, then running
its executable directly with Criterion's `--bench` flag under `/usr/bin/time
-v`; do not time `cargo bench` if you want to exclude compiler memory.

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
