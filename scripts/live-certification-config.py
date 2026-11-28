#!/usr/bin/env python3
"""Add or remove the temporary live-certification snippets.

    live-certification-config.py add|restore CONFIG OUTPUT_DIR BLOCK

`add` keeps an exact copy of CONFIG in OUTPUT_DIR and appends BLOCK (marked
with BEGIN/END comments), leaving the rest of the file byte-for-byte intact.
`restore` puts the exact original back when the file is otherwise unchanged;
if something else edited it during the session, it removes only the marked
block. The live file is owner-only while the temporary block is present, and
the original permission bits are restored with the original bytes.
"""
import hashlib
import os
import stat
import sys
action, config, out, block = sys.argv[1:5]
original_copy = os.path.join(out, "original-configuration.toml")
original_mode = os.path.join(out, "original-configuration.mode")
modified_digest = os.path.join(out, "modified-configuration.sha256")
marker_begin = "# BEGIN wayexpand-live-certification"
marker_end = "# END wayexpand-live-certification"

def atomic_write(data, mode=0o600):
    tmp = config + ".live-certification.tmp"
    created = False
    try:
        fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        created = True
        with os.fdopen(fd, "wb") as handle:
            os.fchmod(handle.fileno(), mode)
            handle.write(data)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(tmp, config)
        directory = os.path.dirname(os.path.abspath(config))
        directory_fd = os.open(directory, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory_fd)
        finally:
            os.close(directory_fd)
    finally:
        if created:
            try:
                os.unlink(tmp)
            except FileNotFoundError:
                pass

def write_exclusive(path, data):
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "wb") as handle:
        handle.write(data)
        handle.flush()
        os.fsync(handle.fileno())

def write_digest(data):
    tmp = modified_digest + ".tmp"
    created = False
    try:
        fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        created = True
        with os.fdopen(fd, "wb") as handle:
            handle.write(digest(data).encode() + b"\n")
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(tmp, modified_digest)
        directory = os.path.dirname(os.path.abspath(modified_digest))
        directory_fd = os.open(directory, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory_fd)
        finally:
            os.close(directory_fd)
    finally:
        if created:
            try:
                os.unlink(tmp)
            except FileNotFoundError:
                pass

def digest(data):
    return hashlib.sha256(data).hexdigest()

def read_config():
    fd = os.open(config, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
    with os.fdopen(fd, "rb") as handle:
        metadata = os.fstat(handle.fileno())
        if not stat.S_ISREG(metadata.st_mode):
            sys.exit("the active configuration is not a regular file")
        return handle.read(), metadata

if action == "add":
    data, metadata = read_config()
    if marker_begin.encode() in data:
        sys.exit("the configuration already holds live-certification snippets; remove that block first")
    write_exclusive(original_copy, data)
    write_exclusive(original_mode, f"{stat.S_IMODE(metadata.st_mode):04o}\n".encode())
    modified = data + (b"" if data.endswith(b"\n") or not data else b"\n") + b"\n" + block.encode() + b"\n"
    # Make the recovery metadata durable before replacing the active config.
    # If interrupted before the replacement, restore recognizes the original
    # bytes and safely does nothing.
    write_digest(modified)
    atomic_write(modified)
else:
    if not os.path.exists(original_copy):
        sys.exit(0)
    current, metadata = read_config()
    original = open(original_copy, "rb").read()
    if current == original:
        print("configuration was not changed; nothing to restore")
    elif not os.path.exists(modified_digest) or not os.path.exists(original_mode):
        sys.exit("recovery metadata is incomplete; preserve and inspect " + original_copy)
    elif digest(current) == open(modified_digest).read().strip():
        mode = int(open(original_mode).read().strip(), 8)
        atomic_write(original, mode)
        print("restored your configuration exactly")
    else:
        start = current.find(marker_begin.encode())
        end = current.find(marker_end.encode(), start)
        if start >= 0 and end > start:
            newline = current.find(b"\n", end)
            end = newline + 1 if newline >= 0 else len(current)
            mode = stat.S_IMODE(metadata.st_mode)
            atomic_write(current[:start].rstrip(b"\n") + b"\n" + current[end:], mode)
            print("the configuration changed during the session; removed only the test snippets")
        else:
            sys.exit("the test snippet block was edited or removed; preserve and inspect " + original_copy)
    os.remove(original_copy)
    for path in (original_mode, modified_digest):
        try:
            os.remove(path)
        except FileNotFoundError:
            pass
