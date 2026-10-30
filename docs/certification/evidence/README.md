# Certification evidence

## Private KWin smoke run — 2026-10-30

Raw machine-readable result: [`kwin-6.6.6-private-30e4557c.json`](kwin-6.6.6-private-30e4557c.json).

This is compatibility smoke evidence, **not production certification**. It
tested commit `30e4557c1a96` in an isolated private headless KWin 6.6.6 session
on Linux `7.0.0-38-generic` x86_64. Input came from the harness's stdin
simulation. The custom `wx-e2e-sink` client verified received text; no GTK,
Qt, browser, terminal, or password-field client was covered.

The negotiated injection mode was libei keysym fallback with 12 ms key
pacing. Twelve checks passed, one backend-latency check was measured, and four
checks were not tested: physical keyboard capture, a second trigger during a
non-atomic replacement, Unicode insertion, and suspend/resume. The report
therefore correctly records `certified: false` and `overall: partial`.

Backend injection latency excludes input capture and client paint latency:

| Replacement size | Samples | p50 | p95 | p99 |
| --- | ---: | ---: | ---: | ---: |
| Small | 20 | 12.581 ms | 12.714 ms | 12.718 ms |
| Medium | 20 | 778.925 ms | 779.772 ms | 780.216 ms |
| Large | 20 | 3,124.733 ms | 3,138.028 ms | 3,192.974 ms |

The CLI, daemon, and test-client binary SHA-256 values for this run were:

| Artifact | SHA-256 |
| --- | --- |
| `wayexpand` | `05ad50d2e4b0a7196c0aa482a0ea2b1fe108d999a37bd22022001e1755eb1a27` |
| `wayexpand-daemon` | `d055a8bd53aafd2faef8be2a3189e836e1c38fe7d8c0b250bd97788cfb2f3268` |
| `wx-e2e-sink` | `de4b34298c9f9d9adb7143abbd8f796da1f90c9acd7ce73a95c96f58275ae76a` |

The report itself has SHA-256
`32dfe322923ed2c2eb3ab9fba674e9791eb93b413abfd3be65203399687d6da0`.

This run does not establish the active desktop's behavior, physical capture
correctness, password-field protection, or any production SLO. See the
[production acceptance checklist](../../PRODUCTION_ACCEPTANCE_CHECKLIST.md)
and [support matrix](../../SUPPORT_MATRIX.md) for the remaining release gates.
