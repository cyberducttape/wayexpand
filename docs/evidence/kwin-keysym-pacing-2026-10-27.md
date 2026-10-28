# KWin keysym-pacing exploration

**Status:** Exploratory performance evidence; not a reliability certification  
**Compositor:** Private headless KWin 6.6.6 / Wayland  
**Base source:** WayExpand commit `7208ea1bd48f`  
**Output route:** libei `ei_keyboard` keysym fallback  
**Capture:** stdin simulation; no physical keyboard capture  
**Date:** 2026-10-27

## Method

The isolated `scripts/test-kwin-e2e.sh output-latency` scenario was run twice
at each pacing interval. Each run measured 20 replacements in each bucket:
1 character, 64 characters, and 257 characters. Every sample checked the
sink's exact final text and cursor. All six runs passed.

The 8 ms and 10 ms variants changed the compile-time key interval and the
matching diagnostic label in a temporary worktree. The production source and
default remain at 12 ms. One earlier exploratory run at each alternative
interval also passed, but is excluded from the table because that temporary
build still emitted a hard-coded “12 ms” mode label.

The table reports the arithmetic mean of the two runs' p50, p95, and p99
statistics. These are descriptive summaries of two short runs, not pooled
percentiles or an SLO result.

| Key interval | Replacement size | p50 (ms) | p95 (ms) | p99 (ms) |
| --- | --- | ---: | ---: | ---: |
| 8 ms | 1 char | 8.466 | 8.625 | 8.828 |
| 8 ms | 64 chars | 522.428 | 524.740 | 526.131 |
| 8 ms | 257 chars | 2095.963 | 2131.375 | 2159.705 |
| 10 ms | 1 char | 10.453 | 10.589 | 10.594 |
| 10 ms | 64 chars | 651.149 | 654.449 | 655.979 |
| 10 ms | 257 chars | 2615.969 | 2619.780 | 2621.201 |
| 12 ms | 1 char | 12.442 | 12.567 | 12.574 |
| 12 ms | 64 chars | 779.777 | 783.759 | 784.110 |
| 12 ms | 257 chars | 3126.800 | 3160.353 | 3215.224 |

## Limits

This establishes that the tested KWin sink received exact output in these
samples. It does not establish that a faster interval is reliable during
physical typing, under CPU or compositor load, across other toolkits or
compositors, or after suspend/resume. It does not test Unicode insertion on a
keysym-only route, password fields, or physical capture.

Do not reduce the production default based on this exploration. A pacing
change requires repeated real-session tests of exact output and typing
corruption on each supported compositor/client combination.
