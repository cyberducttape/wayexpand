# WayExpand settings UI

WayExpand ships two settings frontends: `wayexpand-ui` is dependency-light and
terminal-native, while `wayexpand-gui` is a graphical Wayland-capable editor.
Both use the same core model and atomic-save path.

Start it with:

```sh
wayexpand-ui
wayexpand-ui /path/to/expansions.toml
wayexpand-gui /path/to/expansions.toml
```

If the requested configuration does not exist, the graphical frontend creates
an empty private configuration and opens the editor. Existing malformed,
insecure, or unreadable files are never replaced automatically; they remain a
visible startup error for safe recovery.

Controls:

- `/` starts snippet search
- `j`/`k` or the arrow keys select a snippet
- `n` creates a snippet, prompting for trigger and replacement
- `e` edits the selected replacement
- `D` edits the selected description
- `t` edits comma-separated tags
- `Ctrl-U` clears the active prompt before entering replacement text
- `m` toggles immediate and Unicode-aware word-boundary matching
- `E` opens the selected replacement in `$VISUAL` or `$EDITOR` for multiline editing
- `d` then `d` again deletes the selected snippet (Esc cancels)
- `u` undoes the last saved change
- `Space` toggles the selected snippet and saves atomically
- `r` reloads the configuration from disk
- `p` pauses or resumes the running daemon
- `q`, `Esc`, or `Ctrl-C` exits

The UI uses the core `Config` model and `Config::save_atomic`; it does not
perform text-based edits or maintain a second configuration format. Preview
rendering therefore has the same validation and template behavior as the
daemon and CLI.

Each snippet row shows its matching mode, tags, and whether it is command-backed.
Word-boundary snippets wait until a trailing boundary is observed, preventing
accidental expansion inside larger words. Command-backed snippets run their
configured direct program when previewed or matched; see the command security
limits in the operations guide.

If the daemon is not running, editing and preview still work. Pause/resume is
reported in the status area as unavailable until the user session socket is
reachable.

The graphical frontend provides the same snippet workflow in a native window:
search, select, edit replacement/description/tags, choose matching mode,
toggle enablement, preview, create, delete, undo, reload, and pause/resume.
It marks drafts with unsaved changes and asks whether to save, discard, or
cancel before switching to another snippet.
Triggers are editable in the graphical editor, so a newly created snippet can
be renamed without touching TOML. Empty, duplicate, oversized, or otherwise
invalid triggers are rejected by the same core validation used by the daemon.
`Duplicate` copies the selected snippet, including tags, match mode, templates,
and command settings, then assigns a collision-free trigger for quick editing.
It can also create and edit bounded direct-program expansions: enter the
program, one argument per line, timeout, and optional successful-output cache
duration. Shell syntax is never interpreted by this editor. Invalid command
settings or unsafe limits are rejected before the file is changed.
The `Template variables` palette inserts supported built-ins such as date,
time, hostname, username, newline, and tab directly into the replacement;
unknown template syntax is still rejected by core validation.
The preview input is editable, allowing a trigger to be tested inside larger
text and making word-boundary behavior visible before saving.

The `Settings` window exposes the bounded matcher buffer limit (1–4096
characters). Changes use the same validation, atomic save, undo history, and
daemon reload path as snippet edits.

The `Diagnostics` window reports daemon control-socket connectivity and the
currently discoverable input/output backends, including permission and
Wayland-session hints. When a Wayland session is active, its refresh action
also performs non-mutating input-method-v2 and wlroots virtual-keyboard
protocol probes. It never opens a libei portal consent prompt from passive
diagnostics.

`Import Espanso` accepts an existing Espanso YAML file, validates and previews
the converted expansion count, and reports unsupported entries before offering
an explicit “Replace current library” action. The current TOML is preserved in
the bounded undo history, and the daemon is asked to reload only after the new
file is durably saved.

It requires a Wayland-capable desktop session for window creation, but does
not require the global input-method backend to edit configuration.
