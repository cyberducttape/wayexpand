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

### Previous amplified construction baseline: 2026-10-04 (one alias)

Same machine and toolchain. The workload builds an `ExpansionEngine` from a
pre-parsed, pre-cloned configuration containing aliases, case propagation,
and decomposable Unicode triggers. At 1,000 snippets it used eight aliases
per snippet; at 5,000 and 10,000 it used one alias per snippet. This is a
historical, less-amplified workload; the current benchmark uses two aliases
at 5,000 and 10,000 snippets. This run used a 2 s warmup and 5 s measurement
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

### Current amplified construction baseline: 2026-10-05 (two aliases)

Same machine and toolchain. The current 10,000-snippet workload has two aliases
per snippet, case propagation enabled, and a decomposable Unicode trigger.
The measured operation excludes TOML parsing and configuration cloning. With
a 1 s warmup and 2 s measurement over 10 samples, matcher construction was
160.66 ms (95% interval 159.24–162.11 ms); `/usr/bin/time -v` reported
114,644 KiB maximum process RSS. This is not directly comparable to the
one-alias 2026-10-04 measurement because the workload is more amplified.

Reproduce the current 10,000-snippet measurement and process RSS with:

```sh
cargo bench --locked -p wayexpand-core --bench matcher --no-run
bench_binary=$(find target/release/deps -maxdepth 1 -type f -perm -111 \
  -name 'matcher-*' -printf '%T@ %p\n' | sort -nr | head -n1 | cut -d' ' -f2-)
/usr/bin/time -v "$bench_binary" --bench matcher_construction_amplified/10000 \
  --sample-size 10 --warm-up-time 1 --measurement-time 2
```

### Recheck: 2026-10-05, commit `1bd5ea9`

On the same host and toolchain, rerunning the 10,000-snippet case produced a
162.72 ms estimate (95% interval 160.64–165.09 ms), with a direct-process
maximum RSS of 135,336 KiB. A controlled run of the same benchmark from
`fe65d4b` in a separate worktree and target directory measured 163.31 ms
(160.92–166.14 ms) and 131,048 KiB RSS. Timing is statistically similar; the
current peak RSS is about 3.3% higher. Earlier rechecks on the current binary
reported 132,592 and 133,636 KiB, while the previous single baseline reported
114,644 KiB. This spread indicates the older RSS figure was not representative;
the controlled comparison does not show a large memory regression. Treat these
as process-level observations, not a memory guarantee. The runs used Linux
7.0.0-34-generic and Rust 1.96.0 on the same host.

### Arena matcher: 2026-10-05

The matcher's two tries changed from a `HashMap<char, Node>` per node to
flat arenas with character-sorted edge runs. Same host, Linux
7.0.0-38-generic, Rust 1.96.0. Criterion compared the change against a
baseline saved from the previous commit in the same session (`cargo bench
... -- --save-baseline before`, then `-- --baseline before`); absolute
timings on this host vary between sessions, so only the relative change is
meaningful:

| Benchmark | Before | After | Change |
| --- | ---: | ---: | ---: |
| `matcher_latency/10000` | 8.94 µs | 8.12 µs | −9.1% |
| `matcher_construction_amplified/1000` | 56.5 ms | 39.4 ms | −30.3% |
| `matcher_construction_amplified/5000` | 114.9 ms | 69.7 ms | −39.3% |
| `matcher_construction_amplified/10000` | 224.9 ms | 152.6 ms | −32.2% |

Isolated matcher heap, counted by a global allocator around `Matcher::new`
only (10,000 snippets; "amplified" adds two aliases, case propagation, and a
decomposable trigger; "unicode" uses CJK, Cyrillic, and accented Latin
triggers with the same amplification):

| Workload | Effective triggers | Heap before | Heap after | Build peak after |
| --- | ---: | ---: | ---: | ---: |
| plain | 10,000 | 28.4 MiB | 1.6 MiB | 8.9 MiB |
| amplified | 80,000 | 77.8 MiB | 3.8 MiB | 26.3 MiB |
| unicode | 90,000 | 101.7 MiB | 5.2 MiB | 37.1 MiB |

The build peak is the temporary insertion structure, released before
`Matcher::new` returns. Reproduce the heap figures with:

```sh
cargo run --locked --release -p wayexpand-core --example matcher_memory
```

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
