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
while feeding a trigger one character at a time:

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
  --target-apps gtk4-demo,qt6-demo,terminal,browser,password-field \
  --output kde-run.md
```

Use `--format json` when a machine-readable evidence record is required:

```sh
scripts/certify-compositor.sh --format json --compositor kde \
  --version 6.6.2 --backend ibus \
  --layout us,de,fr,altgr,multi-layout-switching \
  --target-apps gtk4-demo,qt6-demo,terminal,browser,password-field \
  --results kde-results.txt --output kde-run.json
```

Pass `--cli /path/to/wayexpand` (or set `WAYEXPAND_CLI`) when certifying a
source build, so the doctor and status probes are taken from the exact binary
under test rather than whichever installation happens to be in `PATH`.

The JSON record includes the exact session metadata, live doctor/status
snapshots, and one result object for every matrix scenario. It reports
`certified: false` for missing or failed evidence; `status` distinguishes
`incomplete` from `failed`. It does not replace the CLI preflight report or
turn protocol availability into a certification.

It captures the live doctor/status probes and writes every required scenario as
`UNVERIFIED`; it never treats a probe as certification. A compositor-specific
operator or self-hosted driver can provide a results file, for example:

```text
printable-press-release=pass
held-keys-repeat=pass
modifier-navigation=pass
unicode-combining=pass
multiline-rapid=pass
password-field=pass
focus-cross-window=pass
config-reload=pass
daemon-restart=pass
compositor-restart=pass
failed-insertion=pass
ime-preedit=fail
```

Passing the script with `--results results.txt` requires an explicit `pass`
result for every scenario. Any `fail` or `UNVERIFIED` result keeps the report
uncertified. `--layout` and `--target-apps` are required so the report records
the exact keyboard-layout profile set and client set used by the run.
Certification drivers must include `us`, `de`, `fr`, `altgr`, and
`multi-layout-switching`; list every tested client as a comma-separated value.
The collector requires at least one GTK
client, one Qt client, and one password/PIN-field client because those are
mandatory coverage dimensions in the certification matrix.

For repeatable automation, use `scripts/run-certification-driver.sh` with a
compositor-specific driver. The driver receives the scenario name as its first
argument and the exact session metadata through `WAYEXPAND_CERTIFICATION_*`
environment variables. Exit `0` for pass, `1` for an observed failure, and
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
   trigger, and do not restart the daemon. Because input-method-v2 gives the
   backend an exclusive grab, the individual unsupported event may be lost;
   this is a known limitation until compositor-specific pass-through is
   implemented.
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
