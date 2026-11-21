#!/usr/bin/env python3

import importlib.util
from pathlib import Path
import unittest


MODULE_PATH = Path(__file__).with_name("check-launchpad-builds.py")
SPEC = importlib.util.spec_from_file_location("check_launchpad_builds", MODULE_PATH)
checker = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(checker)

SERIES = "https://api.launchpad.net/devel/ubuntu/{}"


def build(series: str, state: str, created: str) -> dict:
    return {
        "title": f"amd64 build of wayexpand in ubuntu {series}",
        "buildstate": state,
        "datecreated": created,
        "distro_series_link": SERIES.format(series),
        "arch_tag": "amd64",
    }


class LaunchpadBuildTests(unittest.TestCase):
    def test_newest_success_after_failures_passes(self):
        records = [
            build("resolute", "Failed to build", "2026-11-20T21:00:00+00:00"),
            build("resolute", "Successfully built", "2026-11-21T21:00:00+00:00"),
        ]
        self.assertEqual(checker.failures(records), [])

    def test_newest_failure_fails_even_after_older_success(self):
        records = [
            build("resolute", "Successfully built", "2026-11-19T21:00:00+00:00"),
            build("resolute", "Failed to build", "2026-11-20T21:00:00+00:00"),
        ]
        self.assertEqual(
            [b["buildstate"] for b in checker.failures(records)],
            ["Failed to build"],
        )

    def test_each_series_is_judged_separately(self):
        records = [
            build("resolute", "Successfully built", "2026-11-20T21:00:00+00:00"),
            build("stonking", "Dependency wait", "2026-11-20T21:00:00+00:00"),
        ]
        self.assertEqual(
            [checker.series_key(b) for b in checker.failures(records)],
            ["stonking/amd64"],
        )

    def test_unfinished_builds_defer_to_the_previous_result(self):
        records = [
            build("resolute", "Failed to build", "2026-11-20T21:00:00+00:00"),
            build("resolute", "Currently building", "2026-11-21T21:00:00+00:00"),
            build("stonking", "Successfully built", "2026-11-20T21:00:00+00:00"),
            build("stonking", "Build for superseded Source", "2026-11-21T21:00:00+00:00"),
        ]
        self.assertEqual(
            [checker.series_key(b) for b in checker.failures(records)],
            ["resolute/amd64"],
        )

    def test_series_no_longer_built_are_not_judged(self):
        records = [
            build("jammy", "Failed to build", "2026-09-16T14:00:00+00:00"),
            build("resolute", "Successfully built", "2026-11-20T21:00:00+00:00"),
        ]
        self.assertEqual(checker.failures(records), [])
        _, retired = checker.active(checker.newest_finished(records))
        self.assertEqual(retired, ["jammy/amd64"])

    def test_log_filter_keeps_failure_lines(self):
        lines = [
            "test result: ok. 4 passed",
            "test fonts::tests::x ... FAILED",
            "thread 'x' panicked at crates/gui/src/fonts.rs:195:9:",
            "make[1]: *** [debian/rules:67: override_dh_auto_test] Error 101",
            "   Compiling wayexpand-gui v1.3.6",
        ]
        kept = [line for line in lines if checker.LOG_FAILURE.search(line)]
        self.assertEqual(kept, lines[1:4])


if __name__ == "__main__":
    unittest.main()
