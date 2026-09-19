# Troubleshooting Checklist

**Navigation:** [Home](../README.md) > [Getting Started](GETTING_STARTED.md) > [Troubleshooting](TROUBLESHOOTING.md) > **Checklist**

Use this page when WayExpand is installed but a snippet does not expand. The
flow is intentionally output-driven: collect the diagnostic line, apply the
matching fix, and then retest with a harmless snippet.

## 1. Capture a diagnostic snapshot

Run these commands as the same user who runs the daemon:

```sh
wayexpand --version
wayexpand doctor
wayexpand status --json
systemctl --user status wayexpand-input-method.service --no-pager
journalctl --user -u wayexpand-input-method.service -n 80 --no-pager
```

Do not paste snippet contents, passwords, portal tokens, or full environment
dumps into an issue. `wayexpand doctor --json` is useful for automation, but
the human-readable output explains the remediation more clearly.

## 2. Read the result from top to bottom

### `Config validation: OK`

Continue to the control socket and backend lines. If it says `FAILED`, or the
file is `not found`/`unreadable`, fix the configuration before investigating
the compositor.

```sh
wayexpand validate ~/.config/wayexpand/expansions.toml
stat -c '%a %U:%G %n' ~/.config/wayexpand/expansions.toml
chmod 600 ~/.config/wayexpand/expansions.toml
```

The file must be a regular file owned by you (or root), and its parent
directories must not be writable by another user. Fix a reported parent with
the exact path printed by `doctor`, for example:

```sh
chmod go-w ~/.config ~/.config/wayexpand
```

Do not use `chmod 777`; it disables the security property the check is
protecting. After editing, run `wayexpand validate` again, then:

```sh
wayexpand reload
wayexpand status --json
```

### `Control socket: disabled` or `control socket unavailable`

The daemon and CLI cannot coordinate without a private runtime directory.

```sh
printf 'XDG_RUNTIME_DIR=%s\n' "$XDG_RUNTIME_DIR"
stat -c '%a %U:%G %n' "$XDG_RUNTIME_DIR"
systemctl --user restart wayexpand-input-method.service
```

If `XDG_RUNTIME_DIR` is empty, start the command from the graphical user
session rather than a root shell, cron job, or unrelated SSH environment. If
you intentionally use `WAYEXPAND_SOCKET`, its parent must exist, be owned by
you or root, and not be group/world-writable.

### Backend line says `Implemented` but no usable combination is printed

`Implemented` means code exists; it does not mean that the current compositor
has advertised the protocol. The probe lines are the decision:

| Doctor output | Meaning | Next action |
| --- | --- | --- |
| `input-method-v2 probe: manager and seat connection succeeded` | Experimental direct Wayland path is available | Use only with explicit opt-in if accepting possible loss of Escape, arrows, or function keys |
| `wlroots probe: virtual keyboard globals available` | A wlroots output path was detected | Use `--source=evdev --backend=wlroots` only when doctor lists it as usable |
| `RequiresPermission` / `permission=Required` | The device or portal needs explicit access | Apply the evdev or portal remediation below; raw evdev is never automatic |
| `Unavailable` / `NotImplemented` | The path cannot be used in this session | Choose another path or compositor |
| `Capture readiness: NOT READY` | No complete source + output pair was found | Resolve the first unavailable probe; do not keep restarting the daemon |

Inspect the conservative selection explanation before choosing a backend:

```sh
wayexpand backend select --explain
```

For an explicit input-method-v2 setup (experimental; unsupported non-text keys
may be lost):

```sh
wayexpand-daemon --source=input-method ~/.config/wayexpand/expansions.toml
wayexpand doctor
```

For an explicit evdev setup (acknowledges global keyboard visibility and has no
password-field signal):

```sh
sudo usermod -aG input "$USER"
# Log out and back in, then:
systemctl --user enable --now wayexpand-evdev.service
wayexpand doctor
```

Evdev has broader keyboard visibility and cannot report password-field focus.
Use it only when that tradeoff is acceptable. For libei, the first connection
may ask for portal consent; if a stored consent token is stale:

```sh
wayexpand portal status
wayexpand portal reset
systemctl --user restart wayexpand-evdev.service
```

## 3. Separate service, backend, and snippet failures

### Service is inactive or repeatedly exits

```sh
systemctl --user status wayexpand-input-method.service --no-pager
journalctl --user -u wayexpand-input-method.service -n 100 --no-pager
```

Interpret the first failure, not the final systemd summary:

- `configuration invalid` → repair the file and run `wayexpand validate`.
- `XDG_RUNTIME_DIR ... required` → launch from the user session or set a
  private runtime environment.
- `input-method ... unavailable` → choose a supported source/backend shown by
  `doctor`; this is not fixed by restarting the same unit.
- `reconnecting` → the compositor or portal disconnected; wait for
  `state=connected`, then try a fresh trigger.

### Service is active and `state=connected`, but a trigger does not match

First prove the config and matcher independently of the compositor:

```sh
wayexpand list
wayexpand search 'hello'
wayexpand test ';;hello'
```

If `wayexpand test` says `no expansion matched`, check the exact trigger,
whether it is disabled, and whether `match_mode = "word-boundary"` is waiting
for a boundary. If the one-shot test matches but typing does not, the remaining
problem is input capture/output or the focused application.

### An app-filtered snippet never matches

Test the unfiltered snippet first. Then check the current support boundary:

- KDE/KWin: run `wayexpand doctor`; window tracking is implemented but still
  best-effort and awaiting broader independent certification.
- Sway, Hyprland, and river: the window-tracking path is a scaffold; filters
  fail closed and therefore do not match.
- GNOME/Mutter: no supported window tracker is available; use a global
  expansion or hotkey.

An unavailable tracker should produce no expansion, not an expansion in the
wrong application. See [SUPPORT_MATRIX.md](SUPPORT_MATRIX.md) before relying
on filtering for sensitive workflows.

## 4. Final retest

After remediation, verify in this order:

```sh
wayexpand doctor
wayexpand status --json
wayexpand test ';;hello'
```

Then type a simple, non-sensitive trigger in a plain text editor. If it still
fails, report the version, compositor and version, the relevant `doctor`
lines, `status --json`, and a short journal excerpt.

For the longer issue catalog, see [TROUBLESHOOTING.md](TROUBLESHOOTING.md).
