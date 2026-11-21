#!/usr/bin/env python3
"""Fail when the newest finished Launchpad PPA build of a series failed.

Launchpad builds the daily recipe without telling anyone when a build fails,
so a broken PPA can go unnoticed for days. This reads the PPA's public build
records (no credentials needed) and, for every series and architecture, looks
at the newest build that has finished. Any such build that did not succeed
fails the check, and the build log's failure lines are printed.

Usage:
    check-launchpad-builds.py [--archive OWNER/NAME] [--records FILE]

--records reads a saved getBuildRecords response instead of calling Launchpad
(used by the tests).
"""

import argparse
from datetime import datetime, timedelta
import gzip
import json
import re
import sys
import urllib.request

API = "https://api.launchpad.net/devel"
DEFAULT_ARCHIVE = "cyberducttape/ppa"
SUCCESS = "Successfully built"
# Builds in these states have not finished, or were never meant to finish, so
# they say nothing about whether the package builds. The newest build before
# them decides.
NOT_FINISHED = {
    "Needs building",
    "Currently building",
    "Gathering build output",
    "Uploading build",
    "Cancelling build",
    "Cancelled build",
    "Build for superseded Source",
}
LOG_FAILURE = re.compile(
    r"(FAILED|panicked at|^error(\[E\d+\])?:|bundled fonts lack|"
    r"make(\[\d+\])?: \*\*\*|dpkg-buildpackage: error)"
)
MAX_LOG_LINES = 25
# A series with no build this close to the archive's newest build is no longer
# targeted by the recipe (jammy and noble were dropped for the Rust MSRV), so
# its last result is history, not the PPA's current health.
RETIRED_AFTER = timedelta(days=14)
USER_AGENT = "wayexpand-launchpad-build-check"


def fetch(url: str) -> bytes:
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    with urllib.request.urlopen(request, timeout=60) as response:
        return response.read()


def fetch_records(archive: str, pages: int = 3) -> list[dict]:
    owner, name = archive.split("/", 1)
    url = (
        f"{API}/~{owner}/+archive/ubuntu/{name}"
        "?ws.op=getBuildRecords&ws.size=50"
    )
    entries: list[dict] = []
    for _ in range(pages):
        page = json.loads(fetch(url))
        entries.extend(page.get("entries", []))
        url = page.get("next_collection_link")
        if not url:
            break
    return entries


def series_key(build: dict) -> str:
    series = (build.get("distro_series_link") or "").rstrip("/").rsplit("/", 1)[-1]
    return f"{series or 'unknown'}/{build.get('arch_tag') or 'unknown'}"


def newest_finished(records: list[dict]) -> dict[str, dict]:
    """Return the newest finished build for every series/architecture."""
    newest: dict[str, dict] = {}
    for build in records:
        if build.get("buildstate") in NOT_FINISHED:
            continue
        key = series_key(build)
        current = newest.get(key)
        if current is None or build.get("datecreated", "") > current.get(
            "datecreated", ""
        ):
            newest[key] = build
    return newest


def created(build: dict) -> datetime:
    return datetime.fromisoformat(build.get("datecreated") or "1970-01-01T00:00:00+00:00")


def active(newest: dict[str, dict]) -> tuple[dict[str, dict], list[str]]:
    """Split the newest builds into series still built and retired series."""
    if not newest:
        return {}, []
    latest = max(created(build) for build in newest.values())
    current = {
        key: build
        for key, build in newest.items()
        if latest - created(build) <= RETIRED_AFTER
    }
    return current, sorted(set(newest) - set(current))


def failures(records: list[dict]) -> list[dict]:
    current, _ = active(newest_finished(records))
    return [
        build
        for _, build in sorted(current.items())
        if build.get("buildstate") != SUCCESS
    ]


def log_excerpt(url: str) -> list[str]:
    try:
        data = fetch(url)
        if data[:2] == b"\x1f\x8b":
            data = gzip.decompress(data)
    except Exception as error:  # the excerpt is a convenience only
        return [f"(build log unavailable: {error})"]
    lines = data.decode("utf-8", "replace").splitlines()
    return [line for line in lines if LOG_FAILURE.search(line)][:MAX_LOG_LINES]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--archive", default=DEFAULT_ARCHIVE)
    parser.add_argument("--records")
    parser.add_argument("--no-logs", action="store_true")
    args = parser.parse_args()

    if args.records:
        with open(args.records, encoding="utf-8") as handle:
            loaded = json.load(handle)
        records = loaded.get("entries", loaded) if isinstance(loaded, dict) else loaded
    else:
        records = fetch_records(args.archive)

    current, retired = active(newest_finished(records))
    if not current:
        print(f"error: no finished builds found in ppa:{args.archive}", file=sys.stderr)
        return 1
    for key, build in sorted(current.items()):
        print(f"{key}: {build.get('buildstate')} ({build.get('title')})")
    if retired:
        print(f"not checked (no recent builds): {', '.join(retired)}")

    failed = failures(records)
    for build in failed:
        print(f"\n::error::{build.get('title')}: {build.get('buildstate')}")
        print(f"  {build.get('web_link')}")
        log_url = build.get("build_log_url")
        if log_url and not args.no_logs:
            for line in log_excerpt(log_url):
                print(f"  | {line}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
