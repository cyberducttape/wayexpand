#!/usr/bin/env python3
"""Add or remove the temporary live-certification snippets.

    live-certification-config.py add|restore CONFIG OUTPUT_DIR BLOCK

`add` keeps an exact copy of CONFIG in OUTPUT_DIR and appends BLOCK (marked
with BEGIN/END comments), leaving the rest of the file byte-for-byte intact.
`restore` puts the exact original back when the file is otherwise unchanged;
if something else edited it during the session, it removes only the marked
block. Both write atomically with owner-only permissions.
"""
import hashlib
import os
import sys
action, config, out, block = sys.argv[1:5]
original_copy = os.path.join(out, "original-configuration.toml")
marker_begin = "# BEGIN wayexpand-live-certification"
marker_end = "# END wayexpand-live-certification"

def atomic_write(data):
    tmp = config + ".live-certification.tmp"
    fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "wb") as handle:
        handle.write(data)
        handle.flush()
        os.fsync(handle.fileno())
    os.replace(tmp, config)

def digest(data):
    return hashlib.sha256(data).hexdigest()

if action == "add":
    data = open(config, "rb").read()
    if marker_begin.encode() in data:
        sys.exit("the configuration already holds live-certification snippets; remove that block first")
    fd = os.open(original_copy, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "wb") as handle:
        handle.write(data)
    modified = data + (b"" if data.endswith(b"\n") or not data else b"\n") + b"\n" + block.encode() + b"\n"
    atomic_write(modified)
    open(os.path.join(out, "modified-configuration.sha256"), "w").write(digest(modified))
else:
    if not os.path.exists(original_copy):
        sys.exit(0)
    current = open(config, "rb").read()
    expected = open(os.path.join(out, "modified-configuration.sha256")).read().strip()
    if digest(current) == expected:
        atomic_write(open(original_copy, "rb").read())
        print("restored your configuration exactly")
    else:
        text = current.decode()
        start, end = text.find(marker_begin), text.find(marker_end)
        if start >= 0 and end > start:
            end = text.index("\n", end) + 1 if "\n" in text[end:] else len(text)
            atomic_write((text[:start].rstrip("\n") + "\n" + text[end:]).encode())
            print("the configuration changed during the session; removed only the test snippets")
        else:
            print("warning: the test snippet block was edited; review " + config, file=sys.stderr)
    os.remove(original_copy)
