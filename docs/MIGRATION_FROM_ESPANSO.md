# Migrating from Espanso to WayExpand

**Navigation:** [Home](../README.md) > [Getting Started](GETTING_STARTED.md) > **Migrating from Espanso**

Espanso is a mature cross-platform text expander with forms, scripts,
packages, a search interface, and application-specific configuration on
platforms where its window filters are available. Its current documentation
also calls out that application-specific configuration is not available on
Wayland. WayExpand is aimed at the gap around Wayland input correctness:
explicit routes, diagnostics, policy controls, and evidence about what the
current desktop can actually guarantee.

Neither project should be treated as universally equivalent on every desktop.
WayExpand's KWin application-tracking path is implemented but awaiting broader
certification; filtered snippets fail closed elsewhere. Active IME/preedit
composition is currently unsupported, and evdev is an explicit compatibility
fallback with no sensitive-field signal. Check the [support matrix](SUPPORT_MATRIX.md)
before switching a production workflow.

## Import first, decide second

Point the importer at an Espanso match file:

```bash
wayexpand import espanso ~/.config/espanso/match/base.yml > imported.toml
```

Use `--strict` to discard matches whose Espanso behavior cannot be preserved;
unmapped top-level options reject the import. Use `--report-json` to emit the
machine-readable report on stderr for review or fleet tooling. Without
`--strict`, the importer keeps supported matches and records semantic warnings.

The command writes converted TOML to stdout and prints a migration report to
stderr. The report separates fully migrated entries, entries migrated with
warnings or unmapped options, and unsupported dynamic matches.

The source file is never modified. In the GUI, Import Espanso previews the
same report and merges by default: new triggers are added, identical
duplicates are ignored, and trigger conflicts keep the existing WayExpand
entry. Replacing the whole library is a separate explicit action.

For a large migration, keep the report with the converted file:

```bash
wayexpand import espanso ~/.config/espanso/match/base.yml \
  > ~/wayexpand-import.toml 2> ~/wayexpand-import-report.txt
wayexpand validate ~/wayexpand-import.toml
```

For a strict migration with a structured report:

```bash
wayexpand import espanso ~/.config/espanso/match/base.yml --strict --report-json \
  > ~/wayexpand-import.toml 2> ~/wayexpand-import-report.json
```

This gives you a concrete answer such as “fully migrated / warnings /
unsupported,” rather than assuming that a syntactically valid conversion is
semantics-preserving.

## Format conversion

Espanso:

```yaml
matches:
  - trigger: ";hello"
    replace: "Hello, world!"
  - trigger: ";sig"
    replace: |
      Best regards,
      Alex
```

WayExpand:

```toml
[[expansion]]
trigger = ";hello"
replacement = "Hello, world!"

[[expansion]]
trigger = ";sig"
replacement = """Best regards,
Alex"""
```

An Espanso match with a `triggers:` list imports as one snippet: the first
trigger becomes `trigger` and the rest become `aliases`.

Categories, tags, descriptions, enabled state, and case propagation have
direct WayExpand representations. Dynamic matches, external filters, forms,
extensions, and options with no direct equivalent are reported for manual
review rather than silently approximated.

## Capability comparison

| Capability | Espanso | WayExpand today |
| --- | --- | --- |
| Static and multiline replacements | Supported | Supported |
| Forms and interactive variables | Supported | Not yet supported as a general form system |
| Scripts and command integrations | Supported | Structured command expansions with bounds and timeouts |
| Community package workflow | Espanso Hub and package commands | Manual/local packs; signed bundles are future work |
| Search interface | Supported | GUI library search and filtering |
| Application-specific configuration | Supported on supported platforms; Espanso documents this as unavailable on Wayland | `app_filter` through the KWin tracker path; currently uncertified and fail-closed elsewhere |
| Linux Wayland input route | Wayland distribution path with documented limitations | Native input/injection routes with explicit capability and lifecycle diagnostics |
| Sensitive-field awareness | Depends on integration | Available only when the selected route/compositor supplies reliable content-purpose information; not universally certified |
| Administrative policy | Project/package configuration | Organization policy, allowed backends/packs, command restrictions, and fleet-oriented config |
| Evidence about the current route | Diagnose through project tooling | `wayexpand doctor`, runtime status, and compositor certification artifacts |

The useful WayExpand distinction is not “Espanso has no filtering” or
“WayExpand has every Espanso feature.” It is that WayExpand makes the Wayland
route and its limitations inspectable. A green certification result must come
from a real evidence artifact, not from protocol availability alone.

## Common conversions

### Static replacement

```toml
[[expansion]]
trigger = ";;addr"
replacement = "123 Main Street, Anytown"
```

### Command-backed replacement

WayExpand uses an explicit command object rather than a shell string:

```toml
[[expansion]]
trigger = ";date"
replacement = ""
description = "Insert today's date"

[expansion.command]
program = "date"
args = ["+%Y-%m-%d"]
timeout_ms = 5000
```

Review command snippets as executable code. WayExpand bounds output and
execution time, but a command still runs with the configured user identity and
should not be treated as a sandbox or a credential boundary.

### App-aware snippets

```toml
[[expansion]]
trigger = ";;ticket"
replacement = "https://tickets.example.test/"
app_filter = ["org.example.TicketApp"]
```

This is useful only when the active route provides trustworthy window
tracking. The current shipped tracker is KWin-specific. On other routes an
app-filtered snippet fails closed instead of expanding globally.

### Categories and tags

```toml
[[expansion]]
trigger = ";;email"
replacement = "alex@example.test"
category = "Contact"
tags = ["personal", "email"]
```

The GUI indexes searchable fields and virtualizes large libraries, so imported
collections can remain practical without requiring a package service.

## A safe migration workflow

1. Export or copy the Espanso match files; keep the originals unchanged.
2. Run the importer and save both its TOML output and migration report.
3. Run `wayexpand validate` on the converted file.
4. Review every warning and unsupported entry, especially forms, scripts,
   external filters, and dynamic matches.
5. Start with the recommended safe route shown by setup/diagnostics. Do not
   enable evdev merely because it offers broader raw keyboard coverage.
6. Test ordinary text fields, password fields, modifiers, Unicode, terminals,
   GTK/Qt/Electron applications, focus changes, and your keyboard layouts.
7. Keep Espanso available until the Wayland route and imported library are
   proven for your workflow.

Use `wayexpand doctor` when an imported trigger does not expand. It reports
the daemon state, selected route, backend capabilities, policy restrictions,
and whether application context is available.

## What WayExpand is trying to make different

WayExpand is not currently a replacement for every Espanso feature. Its
intended differentiators are:

- a Compatibility Center built from real compositor evidence;
- one-click diagnosis and conservative route selection;
- local-only diagnostics that explain why a snippet did or did not expand;
- administrative policy and fleet-friendly configuration;
- safe, reviewable imports with compatibility reports;
- eventually, signed snippet bundles, forms, richer template variables, and
  rollback/history workflows.

The product promise is deliberately narrower and more testable: a Wayland
text expander that tells you what its current route can provide.

## Troubleshooting

If import reports an unsupported feature, do not delete the source entry. Keep
it in the report and adapt it manually. Common cases include:

- Espanso forms or interactive variables: no general WayExpand equivalent yet;
- `external_filter` or shell variables: consider a bounded structured command,
  after reviewing its execution and data exposure;
- application filters: verify that the current session is a certified KWin
  route before relying on them;
- IME/preedit workflows: active composition is currently outside WayExpand's
  supported scope;
- evdev fallback: it has no password-field signal and is not the recommended
  production route.

Validate and inspect status with:

```bash
wayexpand validate ~/.config/wayexpand/expansions.toml
wayexpand doctor ~/.config/wayexpand/expansions.toml
journalctl --user -u wayexpand-input-method.service -n 50
```

See [Getting Started](GETTING_STARTED.md), [Troubleshooting](TROUBLESHOOTING.md),
and [Support Matrix](SUPPORT_MATRIX.md) for current route-specific guidance.
