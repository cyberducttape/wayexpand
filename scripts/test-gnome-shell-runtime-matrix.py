#!/usr/bin/env python3
"""Regression tests for the GNOME Shell runtime compatibility gate."""

import importlib.util
import json
from pathlib import Path
from tempfile import TemporaryDirectory
import unittest

SCRIPT = Path(__file__).with_name("check-gnome-shell-runtime-matrix.py")
spec = importlib.util.spec_from_file_location("gnome_shell_runtime_matrix", SCRIPT)
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)


class RuntimeMatrixTests(unittest.TestCase):
    def setUp(self):
        self.metadata = json.loads(checker.METADATA.read_text(encoding="utf-8"))
        self.matrix = json.loads(checker.MATRIX.read_text(encoding="utf-8"))
        self.workflow = checker.WORKFLOW.read_text(encoding="utf-8")
        self.e2e = checker.E2E

    def test_advertised_majors_have_pinned_runtime_jobs(self):
        self.assertEqual(checker.validate(self.metadata, self.matrix, self.workflow, self.e2e), [])

    def test_uncovered_advertised_major_fails_the_gate(self):
        self.metadata["shell-version"].append("49")
        errors = checker.validate(self.metadata, self.matrix, self.workflow, self.e2e)
        self.assertTrue(any("exactly match runtime-gated majors" in error for error in errors))

    def test_removing_a_required_integration_checkpoint_fails_the_gate(self):
        original = self.e2e.read_text(encoding="utf-8")
        with TemporaryDirectory(prefix="wayexpand-gnome-matrix-") as directory:
            self.e2e = Path(directory) / "test-gnome-e2e.sh"
            self.e2e.write_text(original.replace("gnome-ibus-source-active ", ""), encoding="utf-8")
            errors = checker.validate(self.metadata, self.matrix, self.workflow, self.e2e)
            self.assertTrue(any("gnome-ibus-source-active" in error for error in errors))

    def test_headless_runtime_gate_never_claims_physical_certification(self):
        self.matrix["evidence_scope"] = "production-certified"
        errors = checker.validate(self.metadata, self.matrix, self.workflow, self.e2e)
        self.assertTrue(any("must not be labeled physical certification" in error for error in errors))


if __name__ == "__main__":
    unittest.main()
