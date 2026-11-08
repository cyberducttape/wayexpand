#!/usr/bin/env python3
"""Verify or refresh the content-bound GNOME extension build identity."""

import argparse
import hashlib
import json
from pathlib import Path
import re
import sys


ROOT = Path(__file__).resolve().parent.parent
EXTENSION = ROOT / "gnome-extension/wayexpand-window-tracker@cyberducttape.github.io"
MODULES = ("extension.js", "notification-queue.js", "keyboard-compat.js")


def expected_identity():
    digest = hashlib.sha256()
    for name in MODULES:
        digest.update(name.encode())
        digest.update(b"\0")
        digest.update((EXTENSION / name).read_bytes())
        digest.update(b"\xff")
    return f"sha256:{digest.hexdigest()}"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--write",
        action="store_true",
        help="update metadata.json to match the bundled extension modules",
    )
    args = parser.parse_args()

    metadata_path = EXTENSION / "metadata.json"
    metadata_text = metadata_path.read_text(encoding="utf-8")
    metadata = json.loads(metadata_text)
    current = metadata.get("x-wayexpand-build-id")
    expected = expected_identity()
    if current == expected:
        print(f"GNOME extension build identity verified: {expected}")
        return 0
    if not args.write:
        print(
            f"GNOME extension build identity is stale: metadata={current!r}, "
            f"expected={expected}; run python3 scripts/gnome-extension-build-id.py --write",
            file=sys.stderr,
        )
        return 1

    old_field = re.compile(r'("x-wayexpand-build-id"\s*:\s*)"[^"]*"')
    updated, count = old_field.subn(
        lambda match: f'{match.group(1)}"{expected}"', metadata_text, count=1
    )
    if count != 1:
        raise SystemExit("metadata.json has no unique x-wayexpand-build-id field")
    metadata_path.write_text(updated, encoding="utf-8")
    print(f"Updated GNOME extension build identity: {expected}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
