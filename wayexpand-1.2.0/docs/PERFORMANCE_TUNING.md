# Performance Tuning

WayExpand keeps matching bounded so typing latency does not grow with the
amount of text in the focused application. The matcher uses a trie and a
rolling input buffer; it does not scan the whole document or run every
snippet's command on each keystroke.

## What is bounded

The important limits are:

| Resource | Default | Allowed maximum | Why it matters |
| --- | ---: | ---: | --- |
| Rolling matcher buffer | 128 characters | 4096 characters | Bounds per-keystroke state and memory |
| Expansions | — | 10,000 | Prevents unbounded config/matcher growth |
| Replacement text | — | 1 MiB | Bounds injection and command output handling |
| Config file | — | 16 MiB | Prevents oversized reloads |

The matcher limit is measured in characters, not kilobytes. A 128-character
default is usually enough for short triggers; it is not a 1 KB replacement
limit. Raise it only when a trigger genuinely needs more preceding context.

## Choosing `max_buffer_chars`

Keep the default for ordinary snippets:

```toml
[settings]
max_buffer_chars = 128
```

Increase it when the longest trigger or the context required by a
`word-boundary` match is longer than the current window:

```toml
[settings]
max_buffer_chars = 512
```

The tradeoff is small but real: a larger window retains more recent input and
does more bounded work per event. If the buffer is too small, a
word-boundary match fails closed when the preceding context has been evicted;
it does not guess and expand unsafely.

## Large snippet libraries

The supported ceiling is 10,000 expansions. For a large library:

1. Keep triggers distinctive. Very short common prefixes create more trie
   branches and more opportunities to inspect candidate matches.
2. Disable unused snippets with `enabled = false` rather than deleting them;
   disabled entries remain editable and are not added to the active matcher.
3. Use `wayexpand search` and tags to find stale entries, then remove or
   disable them in the TUI/GUI.
4. Split organizational content into fleet layers when appropriate so a
   machine loads only the snippets it needs.
5. Keep command-backed snippets for deliberate dynamic values. Commands are
   bounded, but process startup and output handling cost more than a literal
   replacement.

Measure the actual config instead of guessing:

```sh
wayexpand validate --json
wayexpand list --json | jq '{count, hotkey_count}'
```

`validate --json` reports `expansion_count` and `max_buffer_chars`. The
configuration is rejected above the documented limits, so a successful
validation is also a useful capacity check.

## If matching feels slow

First separate matching latency from backend latency:

```sh
time wayexpand test ';;hello'
wayexpand validate --json
wayexpand status --json
journalctl --user -u wayexpand-input-method.service -n 100 --no-pager
```

If the one-shot test is fast but live expansion is slow, inspect compositor,
portal, or injection reconnect messages. If validation is slow, check config
size and expansion count. Do not increase `max_buffer_chars` as a general
performance fix; it increases retained input and is only useful for longer
context requirements.

For reproducible engine measurements, contributors can use the matcher
benchmark in `crates/core/benches/matcher.rs`:

```sh
cargo bench --locked -p wayexpand-core --bench matcher
```

The benchmark measures matching itself; it does not certify a compositor or
input backend.
