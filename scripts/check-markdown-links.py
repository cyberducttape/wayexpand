#!/usr/bin/env python3
"""Check repository-local Markdown links without network access."""

from __future__ import annotations

import re
import sys
import unicodedata
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import unquote

ROOT = Path(__file__).resolve().parents[1]
LINK = re.compile(r"!?\[[^\]]*\]\((<[^>]+>|[^)\s]+)(?:\s+\"[^\"]*\")?\)")
REFERENCE = re.compile(r"(?m)^\s{0,3}\[[^\]]+\]:\s*(<[^>]+>|\S+)")
FENCE = re.compile(r"```.*?```", re.DOTALL)
HEADING = re.compile(r"^ {0,3}(#{1,6})\s+(.+?)\s*#*\s*$", re.MULTILINE)
HTML_TAG = re.compile(r"<[^>]+>")
MARKDOWN_LINK = re.compile(r"!?\[([^\]]*)\]\([^)]*\)")


class HtmlAnchors(HTMLParser):
    def __init__(self) -> None:
        super().__init__(convert_charrefs=True)
        self.anchors: set[str] = set()
        self.links: list[str] = []

    def handle_starttag(self, _tag: str, attrs: list[tuple[str, str | None]]) -> None:
        for name, value in attrs:
            if name in {"id", "name"} and value:
                self.anchors.add(value)
            elif name in {"href", "src"} and value:
                self.links.append(value)


def heading_slug(text: str) -> str:
    """Match GitHub's common heading-anchor rules for local Markdown links."""
    text = re.sub(r"`([^`]*)`", r"\1", text)
    text = MARKDOWN_LINK.sub(r"\1", text)
    text = HTML_TAG.sub("", text)
    text = unicodedata.normalize("NFC", text).lower()
    output: list[str] = []
    pending_dash = False
    for char in text:
        category = unicodedata.category(char)
        if char.isspace():
            pending_dash = bool(output)
        elif char == "-" or char == "_" or category[0] in {"L", "N"}:
            if pending_dash and output and output[-1] != "-":
                output.append("-")
            pending_dash = False
            output.append(char)
        # GitHub drops punctuation and symbols from generated heading IDs.
    return "".join(output).strip("-")


def markdown_anchors(text: str) -> set[str]:
    content = FENCE.sub("", text)
    parser = HtmlAnchors()
    parser.feed(content)
    anchors = set(parser.anchors)
    seen: dict[str, int] = {}
    for heading in HEADING.finditer(content):
        base = heading_slug(heading.group(2))
        suffix = seen.get(base, 0)
        seen[base] = suffix + 1
        anchors.add(base if suffix == 0 else f"{base}-{suffix}")
    return anchors


def markdown_files() -> list[Path]:
    return sorted(
        path
        for path in ROOT.rglob("*.md")
        if path.is_file() and not {".git", "target", "vendor"}.intersection(path.parts)
    )


def main() -> int:
    errors: list[str] = []
    links_checked = 0
    anchors_checked = 0
    anchor_cache: dict[Path, set[str]] = {}
    root = ROOT.resolve()

    for source in markdown_files():
        original_text = source.read_text(encoding="utf-8")
        text = FENCE.sub("", original_text)
        source_anchors = markdown_anchors(original_text)
        anchor_cache[source.resolve()] = source_anchors
        html_parser = HtmlAnchors()
        html_parser.feed(text)
        targets = [*LINK.findall(text), *REFERENCE.findall(text), *html_parser.links]
        for raw_target in targets:
            target = raw_target.strip("<>")
            if not target or target.startswith(("http://", "https://", "mailto:", "tel:", "data:")):
                continue
            target_path, separator, raw_anchor = target.partition("#")
            anchor = unquote(raw_anchor) if separator else ""
            target_path = unquote(target_path)
            links_checked += 1
            resolved = (source.parent / target_path).resolve() if target_path else source.resolve()
            try:
                resolved.relative_to(root)
            except ValueError:
                errors.append(f"{source.relative_to(root)}: link escapes repository: {raw_target}")
                continue
            if not resolved.exists():
                errors.append(f"{source.relative_to(root)}: missing target: {raw_target}")
                continue
            if not anchor:
                continue
            anchors_checked += 1
            if resolved.suffix.lower() == ".md":
                anchors = anchor_cache.get(resolved)
                if anchors is None:
                    anchors = markdown_anchors(resolved.read_text(encoding="utf-8"))
                    anchor_cache[resolved] = anchors
            elif resolved.suffix.lower() in {".html", ".htm"}:
                parser = HtmlAnchors()
                parser.feed(resolved.read_text(encoding="utf-8"))
                anchors = parser.anchors
            else:
                continue
            if anchor not in anchors:
                errors.append(
                    f"{source.relative_to(root)}: missing anchor #{anchor} in "
                    f"{resolved.relative_to(root)}"
                )

    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        return 1
    print(
        f"checked {len(markdown_files())} Markdown files, {links_checked} local links, "
        f"and {anchors_checked} local anchors"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
