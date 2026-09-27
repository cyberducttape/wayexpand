#!/usr/bin/env python3
"""Check repository-local Markdown links without network access."""

from __future__ import annotations

import re
import sys
from pathlib import Path
from urllib.parse import unquote

ROOT = Path(__file__).resolve().parents[1]
LINK = re.compile(r"(?<!!)\[[^\]]*\]\(([^)\s]+)(?:\s+\"[^\"]*\")?\)")
FENCE = re.compile(r"```.*?```", re.DOTALL)


def markdown_files() -> list[Path]:
    files = [path for path in ROOT.glob("*.md") if path.is_file()]
    files.extend(path for path in (ROOT / "docs").rglob("*.md") if path.is_file())
    return sorted(files)


def main() -> int:
    errors: list[str] = []
    links_checked = 0
    root = ROOT.resolve()

    for source in markdown_files():
        text = FENCE.sub("", source.read_text(encoding="utf-8"))
        for raw_target in LINK.findall(text):
            target = raw_target.strip("<>")
            if not target or target.startswith(("#", "http://", "https://", "mailto:", "tel:")):
                continue
            target = unquote(target.split("#", 1)[0])
            if not target:
                continue
            links_checked += 1
            resolved = (source.parent / target).resolve()
            try:
                resolved.relative_to(root)
            except ValueError:
                errors.append(f"{source.relative_to(root)}: link escapes repository: {raw_target}")
                continue
            if not resolved.exists():
                errors.append(f"{source.relative_to(root)}: missing target: {raw_target}")

    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        return 1
    print(f"checked {len(markdown_files())} Markdown files and {links_checked} local links")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
