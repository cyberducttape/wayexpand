#!/usr/bin/env python3
"""Regression tests for the GNOME acceptance evidence gate."""

import runpy
import unittest


CHECKER = runpy.run_path("scripts/check-gnome-acceptance-matrix.py")
PLAN = CHECKER["load"](CHECKER["MATRIX"])


def complete_report():
    platform = PLAN["platforms"][0]
    return {
        "platform_id": platform["id"],
        "wayexpand_commit": "a" * 40,
        "package_identity": "test-package-1",
        "distribution_release": platform["release"],
        "gnome_shell_version": "51.1",
        "test_date": "2026-11-07",
        "reviewed_by": "test-reviewer",
        "status": "passed",
        "application_results": [
            {
                "application_class": application,
                "scenario": scenario,
                "application_id": application,
                "application_version": "1.0",
                "input_route": "ibus",
                "keyboard_profiles": ["us", "de"],
                "result": "pass",
            }
            for application in PLAN["application_classes"]
            for scenario in PLAN["required_application_scenarios"]
        ],
        "lifecycle_results": [
            {"scenario": scenario, "result": "pass"}
            for scenario in PLAN["required_lifecycle_scenarios"]
        ],
    }


class AcceptanceMatrixTests(unittest.TestCase):
    def test_plan_covers_every_supported_platform_and_client(self):
        self.assertEqual(CHECKER["validate_plan"](PLAN), [])

    def test_complete_reviewed_report_passes(self):
        self.assertEqual(CHECKER["validate_report"](PLAN, complete_report()), [])

    def test_missing_application_cell_is_rejected(self):
        report = complete_report()
        report["application_results"].pop()
        self.assertTrue(
            any("missing application cells" in error for error in CHECKER["validate_report"](PLAN, report))
        )

    def test_unsupported_application_cell_cannot_certify(self):
        report = complete_report()
        report["application_results"][0]["result"] = "unsupported"
        report["status"] = "incomplete"
        self.assertEqual(CHECKER["validate_report"](PLAN, report), [])

    def test_fedora_evidence_must_identify_gnome_51(self):
        report = complete_report()
        report["gnome_shell_version"] = "50.9"
        report["status"] = "incomplete"
        errors = CHECKER["validate_report"](PLAN, report)
        self.assertTrue(any("GNOME Shell 51.x" in error for error in errors))


if __name__ == "__main__":
    unittest.main()
