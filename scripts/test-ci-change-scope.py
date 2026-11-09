#!/usr/bin/env python3

import importlib.util
from pathlib import Path
import unittest


MODULE_PATH = Path(__file__).with_name("classify-ci-changes.py")
SPEC = importlib.util.spec_from_file_location("classify_ci_changes", MODULE_PATH)
classifier = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(classifier)


class ChangeScopeTests(unittest.TestCase):
    def test_docs_only_keeps_expensive_integrations_out_of_pr(self):
        result = classifier.classify(["docs/GETTING_STARTED.md"])
        self.assertEqual(result, {
            "integration": False,
            "kwin": False,
            "sway": False,
            "ibus": False,
            "gnome_extension": False,
        })

    def test_shared_core_change_runs_all_desktop_regressions(self):
        result = classifier.classify(["crates/core/src/engine/matching.rs"])
        self.assertTrue(result["integration"])
        self.assertTrue(result["kwin"])
        self.assertTrue(result["sway"])
        self.assertTrue(result["ibus"])
        self.assertFalse(result["gnome_extension"])

    def test_backend_changes_run_affected_desktop_suites(self):
        ibus = classifier.classify(["crates/backend-ibus/src/lib.rs"])
        self.assertTrue(ibus["integration"])
        self.assertTrue(ibus["ibus"])
        self.assertFalse(ibus["kwin"])
        self.assertFalse(ibus["sway"])

        kwin = classifier.classify(["crates/backend-kwin-window/src/lib.rs"])
        self.assertTrue(kwin["kwin"])
        self.assertFalse(kwin["ibus"])

        extension = classifier.classify(["gnome-extension/example/extension.js"])
        self.assertTrue(extension["gnome_extension"])
        self.assertTrue(extension["ibus"])
        self.assertFalse(extension["kwin"])

    def test_workflow_changes_force_full_validation(self):
        result = classifier.classify([".github/workflows/ci.yml"])
        self.assertTrue(all(result.values()))

    def test_distribution_and_integration_assets_keep_install_checks(self):
        for path in (
            "debian/rules",
            "desktop/wayexpand-ibus.xml",
            "gnome-extension/example/extension.js",
            "polkit/org.wayexpand.action.policy",
            "systemd/wayexpand.service",
        ):
            with self.subTest(path=path):
                self.assertTrue(classifier.classify([path])["integration"])

    def test_non_pr_events_keep_full_matrix(self):
        result = classifier.classify(["docs/GETTING_STARTED.md"], full=True)
        self.assertTrue(all(result.values()))


if __name__ == "__main__":
    unittest.main()
