# Wayland integration testing

The repository CI validates the platform-independent engine, protocol-safe
backend logic, systemd units, and daemon lifecycle. A real compositor is still
required for end-to-end keyboard behavior.

## Common preparation

Build the workspace and run the compositor-independent checks:

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace
bash scripts/smoke-daemon.sh
bash scripts/test-e2e.sh
wayexpand doctor
```

`scripts/test-e2e.sh` starts a real daemon subprocess with a temporary Unix
control socket and configuration. It covers startup/status, rejected and
accepted reloads, pause/resume, app-filter matching across two window
contexts, and command-backed expansion execution. It uses `stdin` plus the
`none` injector so it is deterministic and does not require a compositor; the
smoke test and the real compositor procedures below remain necessary for
backend behavior.

## Matcher performance baseline

The matcher benchmark exercises 100, 1,000, and 10,000 configured snippets
while feeding a trigger one character at a time. It also measures engine
construction for 1,000, 5,000, and 10,000 snippets using an amplified config
with aliases, case propagation, and decomposable Unicode triggers. Config
parsing and cloning are outside the construction timing; this is startup CPU
cost, not a peak-memory/RSS measurement:

```sh
cargo bench --locked -p wayexpand-core --bench matcher
```

Criterion stores detailed results under `target/criterion`. The benchmark
shape is kept in the repository so future runs can compare the same workload;
record machine-specific numbers with the toolchain and CPU when investigating
a regression rather than treating one host's latency as a universal limit. The
captured reference run is in [`docs/BENCHMARKS.md`](BENCHMARKS.md).

Record the compositor, desktop session, keyboard layout, and output of
`wayexpand doctor` for every integration run.

## Long-duration lifecycle soak

The daemon uses bounded blocking polls and real worker threads rather than an
async runtime. Before calling a backend production-ready, run a representative
24-hour soak; repeat for 72 hours or 7 days before a major release. During the
run, exercise configuration edits and reloads, compositor restarts,
lock/unlock, suspend/resume, keyboard disconnect/reconnect (including two
simultaneous keyboards), failing command snippets, portal-session revocation,
and hundreds of window changes.

`scripts/soak-daemon.sh` is a compositor-independent stdin workload. It
periodically records daemon RSS, thread count, open descriptors, and CPU
percentage, and reports p50/p95 CLI round-trip latency for status and explain
requests. It does not simulate compositor, portal, keyboard, suspend, or
systemd-restart events; those must be exercised by a real-session operator or
the compositor-specific driver. Set `SOAK_REPORT_DIR` to a new directory to
preserve the CSV samples, raw latency samples, daemon log, and summary. The
script refuses to reuse an existing report directory.
Synthetic input is rate-limited to one pair of lines per second by default;
`SOAK_FEED_INTERVAL_SECONDS` can adjust that workload. Daemon logging is
limited to warnings during the soak so a 24-hour run does not accumulate
per-character info logs.

Record at regular intervals and after each lifecycle event:

- RSS, thread count, open file descriptors, and CPU usage;
- control-socket request latency and daemon restart time;
- expansion latency and counts of rejected, retried, or abandoned expansions;
- systemd restart count, journal warnings, and portal reconnect behavior.

The daemon logs `detached_injector_drop_started` when an injector destructor is
isolated from shutdown and `detached_injector_drop_finished` if it returns. A
missing completion event is evidence that backend or portal teardown blocked;
it must be investigated before certification. Do not include typed secrets or
replacement text in soak logs.

## Certification evidence

Compositor-independent CI cannot certify real keyboard behavior. Release
certification must include separate runs on real, supported Wayland sessions:

| Release certification target | Required session |
| --- | --- |
| KDE path | KDE Plasma / KWin |
| GNOME path | GNOME Shell / Mutter |
| wlroots path | Sway |
| wlroots path | Hyprland |

These runs should use dedicated physical or virtual-machine test clients and
record the compositor version, Wayland protocol exposure, keyboard layout,
selected backend, and target applications. They should be performed on
self-hosted Wayland runners or by an operator before publishing a release;
Ubuntu-hosted CI is not a substitute for them. A release is not compositor
certified merely because the unit, smoke, doctor, or protocol tests pass.

Use the evidence collector on a real compositor session:

```sh
scripts/certify-compositor.sh --compositor kde --version 6.6.2 \
  --backend ibus --layout us,de,fr,altgr,multi-layout-switching \
  --target-apps gtk4-demo,qt6-demo,browser-firefox,terminal-konsole,password-field,electron-vscode,text-editor-gedit \
  --output kde-run.md
```

Use `--format json` when a machine-readable evidence record is required:

```sh
scripts/certify-compositor.sh --format json --compositor kde \
  --version 6.6.2 --backend ibus \
  --layout us,de,fr,altgr,multi-layout-switching \
  --target-apps gtk4-demo,qt6-demo,browser-firefox,terminal-konsole,password-field,electron-vscode,text-editor-gedit \
  --results kde-results.txt --output kde-run.json
```

Pass `--cli /path/to/wayexpand` (or set `WAYEXPAND_CLI`) when certifying a
source build, so the doctor and status probes are taken from the exact binary
under test rather than whichever installation happens to be in `PATH`.

The JSON record includes the exact session metadata, live doctor/status
snapshots, and one result object for every scenario × layout × client cell. It reports
`certified: false` for missing or failed evidence; `status` distinguishes
`incomplete` from `failed`. It does not replace the CLI preflight report or
turn protocol availability into a certification.

For daemon-backed paths, preflight validates the live route together with the
doctor's Wayland, config, policy, and control-socket checks. A conservative
automatic-selection recommendation alone does not invalidate an explicitly
selected, healthy route; IBus still requires a healthy doctor report and an
installed IBus integration.

It captures the live doctor/status probes and writes every required
scenario × layout × client cell as `UNVERIFIED`; it never treats a probe as
certification. A compositor-specific
operator or self-hosted driver can provide a results file, for example:

```text
printable-press-release|us|gtk4-demo=pass
held-keys-repeat|de|qt6-demo=pass
modifier-navigation|fr|browser-firefox=pass
password-field|altgr|terminal-konsole=pass
expansion-after-committed-composition|multi-layout-switching|password-field=pass
```

Passing the script with `--results results.txt` requires an explicit `pass`
for every in-scope scenario × layout × client cell. Active IME/preedit is
recorded separately as `unsupported-by-design` in the evidence artifact and
does not count as tested or supported. The wrapper invokes the driver
separately for each cell and sets `WAYEXPAND_CERTIFICATION_LAYOUT` and
`WAYEXPAND_CERTIFICATION_TARGET_APP` to the selected profile and client. The
driver must actually select that keymap and exercise that client; echoing the
environment values is not evidence. Any `fail`, `unsupported-by-design`, or
`UNVERIFIED` cell keeps the report uncertified. Required clients include GTK,
Qt, browser, terminal, and password/PIN-field roles; required profiles are
`us`, `de`, `fr`, `altgr`, and `multi-layout-switching`; clients include GTK,
Qt, browser, terminal, password/PIN-field, Electron, and text-editor roles.

For repeatable automation, use `scripts/run-certification-driver.sh` with a
compositor-specific driver. The driver receives the scenario name as its first
argument and the exact session metadata plus selected layout and client through
`WAYEXPAND_CERTIFICATION_*` environment variables. Exit `0` for pass, `1` for an observed failure, and
`2` when the scenario cannot be verified. The wrapper runs every matrix
scenario, preserves each driver's stdout/stderr log beside the results file,
and produces the results file consumed by the evidence collector. Those logs
are reviewable evidence and must not contain typed secrets or replacement text.

## Input-method source

In a session that advertises `zwp_input_method_manager_v2`:

```sh
wayexpand-daemon --source=input-method /path/to/expansions.toml
```

Verify each item in a normal text editor, terminal, browser, and a native
toolkit application where available:

1. ASCII expansion replaces the complete trigger.
2. UTF-8 replacement works for accented text, CJK, emoji, and punctuation.
3. Backspace removes one Unicode scalar and selections are handled correctly.
4. Return and Tab clear the pending trigger buffer.
5. Password and hidden-text fields do not capture or expand.
6. Unsupported non-text keys are not interpreted as text, clear the pending
   trigger, and do not restart the daemon. When a libei pass-through injector
   is available, WayExpand forwards supported non-text presses with modifier
   state and preserves held-key/repeat/release lifecycles. Pass-through remains
   experimental: compositor-specific fidelity, reconnect behavior, and some
   unsupported keys are not certified, and a failed pass-through operation is
   surfaced rather than silently discarded.
7. Compositor restart causes input-method recovery with bounded backoff; a
   non-retryable protocol failure still stops visibly and systemd applies its
   bounded restart policy.

Capture `systemctl --user status wayexpand-input-method.service` and relevant
user-journal output after each failure. Do not include typed secrets or
replacement contents in reports.

Do not substitute a GNOME or KDE session for this prerequisite. The protocol
is experimental and current compositor support is not uniform; a failed probe
is a capability result, not a configuration error. For desktops that do not
expose this protocol, test the explicit libei/RemoteDesktop backend instead.

## wlroots output

In a compositor exposing `zwp_virtual_keyboard_v1`, first test the isolated
output path:

```sh
cargo run -p wayexpand-backend-wlroots --bin wlroots-type -- 'Hello 🙂'
```

Then exercise the stdin harness with `--backend=wlroots`. This validates output
only; it is not evidence of global input capture.

## libei output

For a direct EIS endpoint:

```sh
LIBEI_SOCKET=wayland-0-eis \
  wayexpand-daemon --backend=libei /path/to/expansions.toml
```

For the portal path, omit `LIBEI_SOCKET` and explicitly select `--backend=libei`.
Record consent, device capability negotiation, UTF-8 insertion, Backspace, and
behavior after portal revocation or compositor restart.

## Evidence boundary

Passing the repository smoke test proves daemon lifecycle behavior only. It does
not prove compositor protocol availability, input-method activation, keyboard
pass-through, preedit support, or desktop-wide compatibility.
