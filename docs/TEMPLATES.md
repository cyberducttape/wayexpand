# Template variables

A replacement can contain `{{variable}}` markers that are filled in when the
snippet expands. Unknown variables are configuration errors, so a typo never
reaches an application. Rendering never runs a shell.

Variables have different trust levels. Built-ins are always available; the
others must be enabled explicitly and can be disabled by organization policy.

## Built-in (always available)

| Variable | Value |
| --- | --- |
| `{{date}}`, `{{time}}`, `{{datetime}}` | Current UTC date (`YYYY-MM-DD`), time (`HH:MM:SS`), or both (ISO 8601, ending in `Z`) |
| `{{local_date}}`, `{{local_time}}`, `{{local_datetime}}` | The same in the system time zone; `local_datetime` ends with the UTC offset, for example `2026-10-11T09:30:00-06:00` |
| `{{date+3d}}`, `{{time-90m}}`, `{{local_datetime+1w}}` | Any of the above with an offset in `m`, `h`, `d`, or `w` |
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
clipboard_mode = "fresh" # prefetch (default), fresh, or disabled

[[expansion]]
trigger = ":quote"
replacement = "> {{clipboard}}"
```

The daemon reads the clipboard with `wl-paste` (from wl-clipboard) with a
150 ms limit and never logs the contents. `prefetch` starts the read as soon
as the typed text can only become such a trigger; the value is used once and
erased after two seconds if the trigger is not completed. `fresh` skips that
early read and reads at render time. `disabled` blocks the variable even when
`allow_clipboard` is enabled. Nothing watches the clipboard in the
background. If the clipboard cannot be read, the snippet does not expand.
The IBus engine does not support the clipboard variable.

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
| `{{choice:name=A\|B\|C}}` | The same, labelled *name*; repeating it reuses the value. |

Tab moves between fields, Enter inserts, and Escape cancels, leaving the typed
trigger in place. While the form is open, expansion is paused so typing in the
form cannot trigger other snippets. After you press Enter, the trigger is erased
and the filled text typed only if focus has returned to the exact original
toplevel window and not into a password field. A backend that cannot provide a
bounded, exact window identity refuses to open the form; it will not guess based
on app ID or title, since multiple windows can share both.

Returning to the right window does not prove the caret is still after the
trigger: a click inside that window can move it, and no capture backend can see
mouse input. Forms therefore open, and their results are applied, only on output
routes whose replacement guarantee is `verified` or `atomic` (they refuse to
erase unless the trigger is at the cursor). On `best-effort` routes such as
evdev with libei or wlroots virtual-keyboard output, form snippets are refused
and the trigger is left in place. Input-method-v2 routes are `atomic`; the
`minimum_replacement_guarantee` organization policy uses the same levels.

The form is the `wayexpand-gui --form` window; under systemd the daemon starts
it with `systemd-run --user` so it runs outside the daemon's sandbox. Form
snippets cannot run commands and cannot be included in other snippets. The IBus
route drops its surrounding-text knowledge when focus moves to the form, so
form snippets do not expand through IBus.

## Literal braces

Write `{{{{` for a literal `{{` and `}}}}` for a literal `}}`. This lets a
snippet contain Jinja (Ansible), Go templates (Helm, Compose) and GitHub
Actions expressions alongside its own fields:

```toml
[[expansion]]
trigger = ";;loop"
replacement = """- name: Install {{field:what=packages}}
  ansible.builtin.package:
    name: "{{{{ item }}}}"
  loop: "{{{{ {{field:what=packages}} }}}}"
"""
```

renders `name: "{{ item }}"` and `loop: "{{ packages }}"`. A single `}}` in
plain text is left as it is.

## File templates (`wayexpand new`)

Long, whitespace-sensitive files such as Dockerfiles, playbooks and
Kubernetes manifests are better written than typed: editors re-indent typed
YAML, and typing hundreds of characters is slow. `wayexpand new` fills in a
snippet's fields and writes the result to a file. It needs no running daemon
and works on every desktop and over SSH.

```sh
wayexpand new                       # list file templates
wayexpand new playbook              # ask for each field, write site.yml
wayexpand new playbook --set package=nginx --set state=latest
wayexpand new dockerfile-python -o api/Dockerfile
wayexpand new compose --stdout      # print instead of writing
wayexpand new k8s-deployment --gui  # ask in the form window
wayexpand new --show playbook       # print it as a snippet to copy
```

A snippet becomes a file template by naming the file it writes:

```toml
[[expansion]]
trigger = ";;dockerfile-go"
output_file = "Dockerfile"
description = "Go service"
replacement = """FROM golang:{{field:go=1.24}} AS build
WORKDIR /src
COPY . .
RUN go build -o /out/{{field:binary}} .

FROM gcr.io/distroless/static
COPY --from=build /out/{{field:binary}} /{{field:binary}}
ENTRYPOINT ["/{{field:binary}}"]
"""
```

`wayexpand new dockerfile-go` finds it by its trigger without the
punctuation (`;;dockerfile-go`), by an exact trigger or alias, or by the file
name it writes. Typing the trigger still inserts the text as usual.

- `output_file` is relative to the current directory and may not be absolute
  or contain `..`. Missing directories are created. `-o FILE` writes
  elsewhere; `-o -` and `--stdout` print instead.
- An existing file is never replaced unless you pass `--force`. Then the new
  file replaces it in one step, keeping its permissions.
- `--set NAME=VALUE` fills a field by its label; a choice must be one of its
  options. In a terminal you are asked for the rest (Enter keeps the default).
  Without a terminal, defaults are used, and a field without one is an error.
- Fields, environment variables and includes work as they do when typing.
  `{{clipboard}}` is not available, and command snippets cannot be written to
  a file.

WayExpand ships starter templates: `dockerfile`, `dockerfile-python`,
`dockerfile-node`, `dockerfile-rust`, `compose`, `playbook`, `ansible-role`,
`inventory`, `systemd-service`, `github-actions`, `nginx-site` and
`k8s-deployment`. A snippet of yours with the same name replaces the built-in
one; `wayexpand new --show NAME` prints a built-in template to start from.

## Organization policy

In safe mode, `disable_template_env = true` and `disable_clipboard = true`
switch these variables off regardless of user settings; snippets that use them
then do not expand. `wayexpand explain <trigger>` shows which of these a
snippet uses and why it was blocked.
