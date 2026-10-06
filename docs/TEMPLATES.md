# Template variables

A replacement can contain `{{variable}}` markers that are filled in when the
snippet expands. Unknown variables are configuration errors, so a typo never
reaches an application. Rendering never runs a shell.

Variables have different trust levels. Built-ins are always available; the
others must be enabled explicitly and can be disabled by organization policy.

## Built-in (always available)

| Variable | Value |
| --- | --- |
| `{{date}}`, `{{time}}`, `{{datetime}}` | Current UTC date (`YYYY-MM-DD`), time (`HH:MM:SS`), or both (ISO 8601) |
| `{{date+3d}}`, `{{time-90m}}`, `{{datetime+1w}}` | The same with an offset in `m`, `h`, `d`, or `w` |
| `{{unix_timestamp}}` | Seconds since 1970-01-01 UTC |
| `{{username}}`, `{{hostname}}` | The login name and the machine's host name |
| `{{newline}}`, `{{tab}}` | A line break or a tab |
| `{{cursor}}` | Where the cursor ends up after expansion (one per snippet) |

## Snippet includes

`{{snippet:TRIGGER}}` inserts another snippet's text, by its trigger or an
alias. Only enabled, static snippets can be included; command-backed snippets
cannot. Includes may nest up to 8 levels; a cycle is a configuration error, as
is an included snippet containing `{{cursor}}`.

```toml
[[expansion]]
trigger = ":sig"
replacement = "Best regards,{{newline}}Sam"

[[expansion]]
trigger = ":thanks"
replacement = "Thanks for the update.{{newline}}{{newline}}{{snippet::sig}}"
```

## Environment variables

`{{env:NAME}}` reads an environment variable of the expanding process, but only
names listed in `settings.template_env` can be read. Any other name is a
configuration error. An unset listed variable expands to nothing.

```toml
[settings]
template_env = ["TICKET_PREFIX"]

[[expansion]]
trigger = ":tk"
replacement = "{{env:TICKET_PREFIX}}-{{cursor}}"
```

## Clipboard

`{{clipboard}}` inserts the current clipboard text. It is off by default,
because the clipboard often holds passwords. Enable it explicitly:

```toml
[settings]
allow_clipboard = true

[[expansion]]
trigger = ":quote"
replacement = "> {{clipboard}}"
```

The daemon reads the clipboard with `wl-paste` (from wl-clipboard) only for a
snippet that uses the variable, with a 150 ms limit, and never logs the
contents. So the expansion does not wait for that read, it starts as soon as
the typed text can only become such a trigger (at least two characters, with
no other snippet still possible); the value is used for that one expansion
and erased after two seconds if the trigger is not completed. Nothing watches
the clipboard in the background. If the clipboard cannot be read, the snippet
does not expand. The IBus engine does not support the clipboard variable.

## Form fields (interactive snippets)

A snippet with fields opens a small form when it is triggered, so one snippet
can produce a filled-in ticket reply, incident note, or email:

```toml
[[expansion]]
trigger = ";;assign"
replacement = """Hello {{field:name}},

Your ticket {{field:ticket=OPS-}} has been assigned to {{field:agent}}.

Current status: {{choice:Open|Waiting on customer|Resolved}}

{{cursor}}"""
```

| Marker | Form input |
| --- | --- |
| `{{field:name}}` | A text box labelled *name*. Repeating the marker reuses the value. |
| `{{field:name=default}}` | The same, prefilled. `{{prompt:name}}` is a synonym. |
| `{{choice:A\|B\|C}}` | A drop-down with 2–32 options. |

Tab moves between fields, Enter inserts, and Escape cancels, leaving the typed
trigger in place. While the form is open, expansion is paused so typing in the
form cannot trigger other snippets. After you press Enter, the trigger is erased
and the filled text typed only if focus has returned to the exact original
toplevel window and not into a password field. A backend that cannot provide a
bounded, exact window identity refuses to open the form; it will not guess based
on app ID or title, since multiple windows can share both.

The form is the `wayexpand-gui --form` window; under systemd the daemon starts
it with `systemd-run --user` so it runs outside the daemon's sandbox. Form
snippets cannot run commands and cannot be included in other snippets. The IBus
route drops its surrounding-text knowledge when focus moves to the form, so
form snippets do not expand through IBus.

## Organization policy

In safe mode, `disable_template_env = true` and `disable_clipboard = true`
switch these variables off regardless of user settings; snippets that use them
then do not expand. `wayexpand explain <trigger>` shows which of these a
snippet uses and why it was blocked.
