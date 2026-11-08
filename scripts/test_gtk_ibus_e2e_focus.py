#!/usr/bin/env python3
"""Regression tests for GTK IBus acceptance focus readiness."""

import importlib.util
from pathlib import Path
import unittest


SCRIPT = Path(__file__).parent / "certification-clients" / "gtk_ibus_e2e.py"
SPEC = importlib.util.spec_from_file_location("gtk_ibus_e2e", SCRIPT)
GTK_E2E = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(GTK_E2E)


class FocusReadinessTests(unittest.TestCase):
    def test_unfocused_widget_never_becomes_ready(self):
        since, ready = GTK_E2E.update_focus_stability(True, False, None, 1_000_000)
        self.assertIsNone(since)
        self.assertFalse(ready)

    def test_inactive_toplevel_never_becomes_ready(self):
        since, ready = GTK_E2E.update_focus_stability(False, True, None, 1_000_000)
        self.assertIsNone(since)
        self.assertFalse(ready)

    def test_focus_must_remain_inside_widget_for_full_interval(self):
        since, ready = GTK_E2E.update_focus_stability(True, True, None, 1_000_000)
        self.assertEqual(since, 1_000_000)
        self.assertFalse(ready)

        since, ready = GTK_E2E.update_focus_stability(True, True, since, 1_249_999)
        self.assertEqual(since, 1_000_000)
        self.assertFalse(ready)

        since, ready = GTK_E2E.update_focus_stability(True, True, since, 1_250_000)
        self.assertEqual(since, 1_000_000)
        self.assertTrue(ready)

    def test_focus_loss_resets_the_stability_interval(self):
        since, ready = GTK_E2E.update_focus_stability(True, True, None, 1_000_000)
        self.assertFalse(ready)
        since, ready = GTK_E2E.update_focus_stability(True, False, since, 1_100_000)
        self.assertIsNone(since)
        self.assertFalse(ready)

        since, ready = GTK_E2E.update_focus_stability(True, True, since, 1_200_000)
        self.assertEqual(since, 1_200_000)
        self.assertFalse(ready)


if __name__ == "__main__":
    unittest.main()
