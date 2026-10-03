# Fuzzing

WayExpand parses several kinds of untrusted input: configuration files,
imported Espanso files and snippet packs, Action Broker IPC frames from local
clients, app-filter expressions, and IBus key events. Each has a
[cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) target in `fuzz/`, a
separate workspace so the product keeps building on the pinned stable
toolchain.

| Target | Input | Property checked |
| --- | --- | --- |
| `config_parse` | configuration TOML | no panic; anything accepted builds an engine |
| `matcher` | event streams | a match only erases text that was typed |
| `broker_frame` | broker request frames, read through varying buffer sizes | no panic, bounded memory |
| `espanso_import` | Espanso YAML | no panic; anything imported validates |
| `pack_import` | pack manifest and snippet file | no panic; imported packs never carry commands |
| `app_filter` | filter expression and focused app ID | no panic; filtered snippets fail closed without a window |
| `ibus_keys` | IBus keys, content types, surrounding text | never deletes more than is before the cursor |

Run one locally (nightly and `cargo install cargo-fuzz` required):

```sh
cd fuzz
cargo +nightly fuzz run matcher -- -max_total_time=60
```

CI runs every target for 60 seconds on changes to the parsing crates and for
20 minutes nightly (`.github/workflows/fuzz.yml`). A crash fails the job and
uploads the reproducing input; reproduce it with
`cargo +nightly fuzz run <target> fuzz/artifacts/<target>/<file>` and add a
regression unit test with the fix.
