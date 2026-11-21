#!/usr/bin/env python3
"""Regression tests for the GNOME acceptance evidence gate."""

import runpy
import tempfile
import unittest
from pathlib import Path


CHECKER = runpy.run_path("scripts/check-gnome-acceptance-matrix.py")
REPORT_GENERATOR = runpy.run_path("scripts/new-gnome-acceptance-report.py")
PLAN = CHECKER["load"](CHECKER["MATRIX"])


def complete_report():
    platform = PLAN["platforms"][0]
    return {
        "platform_id": platform["id"],
        "wayexpand_commit": "a" * 40,
        "package_identity": "test-package-1",
        "distribution_release": platform["release"],
        "gnome_shell_version": "51.1",
        "session_kind": "physical-wayland-session",
        "physical_keyboard": "reviewer-recorded keyboard model",
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
                "keyboard_profile": profile,
                "result": "pass",
                "evidence_ref": f"artifacts/{application}-{scenario}-{profile}.json",
                **(
                    {
                        "trigger_captured": True,
                        "original_trigger_removed": True,
                        "replacement_occurrences": 1,
                        "intended_field_retained_focus": True,
                    }
                    if scenario == "real-trigger-replacement"
                    else {}
                ),
            }
            for application in PLAN["application_classes"]
            for scenario in PLAN["required_application_scenarios"]
            for profile in PLAN["required_keyboard_profiles"]
        ],
        "lifecycle_results": [
            {"scenario": scenario, "result": "pass", "evidence_ref": f"artifacts/{scenario}.json"}
            for scenario in PLAN["required_lifecycle_scenarios"]
        ],
    }


class AcceptanceMatrixTests(unittest.TestCase):
    def test_report_generator_requires_gnome_desktop_identity(self):
        is_gnome_session = REPORT_GENERATOR["is_gnome_session"]
        self.assertTrue(is_gnome_session("ubuntu:GNOME", "ubuntu"))
        self.assertTrue(is_gnome_session("", "gnome-classic"))
        self.assertFalse(is_gnome_session("KDE", "gnome"))

    def test_report_generator_reads_distribution_identity(self):
        with tempfile.TemporaryDirectory() as directory:
            os_release = Path(directory) / "os-release"
            os_release.write_text(
                'ID="ubuntu"\nVERSION_ID="24.04"\n', encoding="utf-8"
            )
            identity = REPORT_GENERATOR["os_release_identity"](os_release)
        self.assertEqual(identity, ("ubuntu", "24.04"))
        ubuntu = next(item for item in PLAN["platforms"] if item["id"] == "ubuntu-24.04-gnome")
        fedora = next(item for item in PLAN["platforms"] if item["id"] == "fedora-45-gnome51")
        matches = REPORT_GENERATOR["platform_matches_distribution"]
        self.assertTrue(matches(ubuntu, identity))
        self.assertFalse(matches(fedora, identity))

    def test_report_generator_validates_exact_pinned_shell_major(self):
        fedora = next(item for item in PLAN["platforms"] if item["id"] == "fedora-45-gnome51")
        matches = REPORT_GENERATOR["platform_matches_shell_version"]
        self.assertTrue(matches(fedora, "GNOME Shell 51.1"))
        self.assertTrue(matches(fedora, "51.1"))
        self.assertFalse(matches(fedora, "GNOME Shell 50.9"))
        self.assertFalse(matches(fedora, "GNOME Shell 51.x"))

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

    def test_headless_reports_cannot_be_passed_as_physical_session_evidence(self):
        report = complete_report()
        report["session_kind"] = "headless"
        report["status"] = "incomplete"
        errors = CHECKER["validate_report"](PLAN, report)
        self.assertTrue(any("physical keyboard" in error for error in errors))

    def test_application_cells_must_link_reviewable_evidence(self):
        report = complete_report()
        report["application_results"][0].pop("evidence_ref")
        report["status"] = "incomplete"
        errors = CHECKER["validate_report"](PLAN, report)
        self.assertTrue(any("evidence_ref" in error for error in errors))


if __name__ == "__main__":
    unittest.main()
