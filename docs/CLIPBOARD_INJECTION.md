# Clipboard-Based Text Injection

WayExpand includes an experimental clipboard-based text injection backend for handling expansions with newlines in GUI text editors.

## Problem

When using the `ei_keyboard` fallback (for compositors without ei_text support), newlines in expansions don't work in GUI text editors:

- **CLI terminals:** ✅ Newlines work correctly
- **GUI text editors:** ❌ Newlines missing (no line breaks)

This is because `ei_keyboard` simulates individual key presses, and GUI text editors don't recognize synthetic Return key presses as line breaks.

## Solution: Clipboard Injection

The clipboard backend solves this by:

1. Copying the expansion text to the system clipboard
2. Simulating Ctrl+V paste
3. Restoring the previous clipboard contents

This preserves newlines because actual text (with newline characters) is pasted, not simulated key presses.

## Requirements

The clipboard backend needs:
- `xclip` or `xsel` — for clipboard operations
- `xdotool` — for simulating Ctrl+V paste

**Install (Ubuntu/Debian):**
```bash
sudo apt install xclip xsel xdotool
```

**Install (Arch):**
```bash
sudo pacman -S xclip xsel xdotool
```

## Usage

This is not a selectable `--backend=` value -- the daemon only accepts
`none`, `wlroots`, or `libei` there. `ClipboardInjector` is used
automatically and only as a per-expansion fallback: `text_contains_newlines`
in `crates/daemon/src/main.rs` checks each expansion's rendered replacement,
and routes just that one injection through the clipboard when it contains
`\n`/`\r` and the primary backend is `libei`/`wlroots`, silently falling back
to the primary backend if `ClipboardInjector::new()` fails (e.g. xclip/xsel
missing). Nothing to configure: run the daemon normally with
`--backend=wlroots` or `--backend=libei`, and any expansion with a newline
in it uses this path automatically.

## Trade-offs

✅ **Advantages:**
- Preserves newlines in all GUI text editors
- Works with any Wayland compositor
- Doesn't require special compositor support

❌ **Disadvantages:**
- Requires Ctrl+V instead of direct injection (one extra keystroke mentally)
- Uses system clipboard (flashes clipboard contents, privacy concern)
- Slightly slower than direct injection
- Requires xclip/xsel/xdotool to be installed

## Implementation Details

See `crates/backend-clipboard/src/lib.rs` for the implementation of the `ClipboardInjector` struct, which implements the `TextInjector` trait.

### How It Works

1. **Copy to clipboard:** Text is piped to `xclip -selection clipboard -i` or `xsel --clipboard --input`
2. **Trigger paste:** `xdotool key ctrl+v` simulates the Ctrl+V key combination
3. **Backspace erase:** Trigger is erased by simulating individual backspace presses

### Fallback chain

1. `ei_text` when the EIS server offers it (native newline support).
2. `ei_keyboard` keysym synthesis when it doesn't -- correct for simple
   text, but this is what motivated this backend, since a synthetic Return
   keypress isn't recognized as a line break by most GUI text editors.
3. `ClipboardInjector`, automatically, only for the specific expansion whose
   rendered replacement contains a newline (see Usage above).

## Future Improvements

- Support Shift+Insert as alternative to Ctrl+V
- Add option to use wayland-clipboard library instead of xclip
- Restore non-text clipboard content too (an image or other binary data the
  user had copied is currently left overwritten, since only UTF-8 text can
  be read back and restored)
