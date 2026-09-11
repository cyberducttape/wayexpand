# Troubleshooting

Start with a non-invasive snapshot:

```sh
wayexpand --version
wayexpand doctor
wayexpand status --json
systemctl --user status wayexpand-input-method.service --no-pager
journalctl --user -u wayexpand-input-method.service -n 80 --no-pager
```

## `configuration invalid`

Run `wayexpand doctor`. Common causes are malformed TOML, duplicate triggers,
a file writable by group or other users, an untrusted parent directory, or a
non-regular path. Fix the reported mode/owner and validate again. Never bypass
the check with `chmod 777`.

## Service is active but snippets do not expand

Check `wayexpand status`, `wayexpand backend`, and `echo "$WAYLAND_DISPLAY"`.
The service may be using the stdin harness, may be reconnecting, or may be
running on a compositor without input-method-v2 support. Confirm that the
input-method unit—not the harness unit—is enabled and that `state=connected`.

## `control socket unavailable`

Ensure `XDG_RUNTIME_DIR` is set and points to a user-owned runtime directory.
If using `WAYEXPAND_SOCKET`, its parent must exist, be a directory, and not be
group/world-writable. Remove only a stale socket owned by the current user; the
daemon deliberately refuses to remove arbitrary files.

## Reload was rejected

The daemon keeps the previous working configuration by design. Fix the file and
request another reload:

```sh
wayexpand validate
wayexpand reload
wayexpand status
```

If the editor is saving repeatedly, wait for its atomic save to finish and
retry. An unstable or half-written file is not activated.

## GUI will not start

Use the terminal UI to separate configuration problems from graphics/session
problems:

```sh
wayexpand-ui
wayexpand doctor
```

Confirm `WAYLAND_DISPLAY` is present and try launching from the same graphical
session. On a headless machine, use the CLI or terminal UI.

## Output backend reconnects

Transport failures are retried with bounded backoff. The failed replacement is
not replayed because the compositor may have accepted part of it. Once
`state=connected` returns, type a fresh trigger. Permanent protocol or
validation errors require operator action and are not retried indefinitely.
