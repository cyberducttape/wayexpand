# Configuration Limits

These limits are enforced by `wayexpand-core` during configuration loading
and validation. They are resource and abuse boundaries, not performance
targets. Changes to them are compatibility-relevant and should update this
page and the associated tests together.

| Resource | Limit |
|---|---:|
| Configuration file | 16 MiB |
| Expansion entries | 10,000 |
| Hotkey entries | 1,024 |
| Aggregate enabled trigger data | 256 KiB |
| Trigger length | 128 Unicode scalar values |
| Replacement size | 1 MiB |
| Description length | 512 Unicode scalar values |
| Tags per expansion | 32 |
| Tag length | 64 Unicode scalar values |
| App filters per expansion | 32 |
| App-filter length | 256 Unicode scalar values |
| Category length | 64 Unicode scalar values |
| Command arguments | 32 |
| Command program length | 256 Unicode scalar values |
| Command argument length | 1,024 Unicode scalar values each |
| Aggregate command argument data | 16 KiB |
| Command environment allowlist | 32 names |
| Environment variable name length | 256 Unicode scalar values |
| Command timeout | 1–5,000 ms |
| Command output | 1 MiB |
| Command cache duration | 0–60,000 ms |

Invalid values are rejected before a configuration becomes active. Runtime
command execution is additionally bounded by the configured timeout and runs
in a process group so timeout cleanup reaches ordinary descendants. This is
best-effort resource cleanup, not sandbox containment: a trusted command can
deliberately fork, call `setsid()`, and escape the original process group.
