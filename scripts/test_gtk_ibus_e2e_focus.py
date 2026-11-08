#!/usr/bin/env python3
"""Regression tests for GTK IBus acceptance focus readiness."""

import importlib.util
from pathlib import Path
import unittest
from unittest import mock


SCRIPT = Path(__file__).parent / "certification-clients" / "gtk_ibus_e2e.py"
GNOME_ACCEPTANCE_SCRIPT = Path(__file__).parent / "test-gnome-e2e.sh"
SPEC = importlib.util.spec_from_file_location("gtk_ibus_e2e", SCRIPT)
GTK_E2E = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(GTK_E2E)

REMOTE_INPUT_SCRIPT = Path(__file__).parent / "certification-clients" / "gnome_remote_input.py"
REMOTE_INPUT_SPEC = importlib.util.spec_from_file_location(
    "gnome_remote_input", REMOTE_INPUT_SCRIPT
)
REMOTE_INPUT = importlib.util.module_from_spec(REMOTE_INPUT_SPEC)
REMOTE_INPUT_SPEC.loader.exec_module(REMOTE_INPUT)


class GnomeAcceptanceShellContractTests(unittest.TestCase):
    def test_x11_clients_have_a_window_activation_helper(self):
        source = GNOME_ACCEPTANCE_SCRIPT.read_text(encoding="utf-8")
        self.assertRegex(source, r"(?m)^activate_window\(\) \{")
        self.assertIn(
            "org.wayexpand.GnomeE2eFocusController.ActivateWindow",
            source,
        )


class FocusReadinessTests(unittest.TestCase):
    def test_unfocused_widget_never_becomes_ready_after_mutter_activation(self):
        since, ready = GTK_E2E.update_focus_stability(
            True, True, False, None, 1_000_000
        )
        self.assertIsNone(since)
        self.assertFalse(ready)

    def test_inactive_toplevel_never_becomes_ready(self):
        since, ready = GTK_E2E.update_focus_stability(
            True, False, True, None, 1_000_000
        )
        self.assertIsNone(since)
        self.assertFalse(ready)

    def test_unconfirmed_mutter_focus_never_becomes_ready(self):
        since, ready = GTK_E2E.update_focus_stability(
            False, True, True, None, 1_000_000
        )
        self.assertIsNone(since)
        self.assertFalse(ready)

    def test_focus_must_remain_inside_widget_for_full_interval(self):
        since, ready = GTK_E2E.update_focus_stability(
            True, True, True, None, 1_000_000
        )
        self.assertEqual(since, 1_000_000)
        self.assertFalse(ready)

        since, ready = GTK_E2E.update_focus_stability(
            True, True, True, since, 1_249_999
        )
        self.assertEqual(since, 1_000_000)
        self.assertFalse(ready)

        since, ready = GTK_E2E.update_focus_stability(
            True, True, True, since, 1_250_000
        )
        self.assertEqual(since, 1_000_000)
        self.assertTrue(ready)

    def test_focus_loss_resets_the_stability_interval(self):
        since, ready = GTK_E2E.update_focus_stability(
            True, True, True, None, 1_000_000
        )
        self.assertFalse(ready)
        since, ready = GTK_E2E.update_focus_stability(
            True, True, False, since, 1_100_000
        )
        self.assertIsNone(since)
        self.assertFalse(ready)

        since, ready = GTK_E2E.update_focus_stability(
            True, True, True, since, 1_200_000
        )
        self.assertEqual(since, 1_200_000)
        self.assertFalse(ready)


class InputModeEnvironmentTests(unittest.TestCase):
    def test_simple_xkb_baseline_requires_builtin_simple_context(self):
        self.assertEqual(
            GTK_E2E.input_mode_environment_error("simple", {}),
            "GTK_IM_MODULE must be set to gtk-im-context-simple",
        )
        self.assertIsNone(
            GTK_E2E.input_mode_environment_error(
                "simple", {"GTK_IM_MODULE": "gtk-im-context-simple"}
            )
        )

    def test_simple_xkb_baseline_rejects_ibus_module_overrides(self):
        self.assertEqual(
            GTK_E2E.input_mode_environment_error(
                "simple", {"GTK_IM_MODULE": "ibus"}
            ),
            "GTK_IM_MODULE must be set to gtk-im-context-simple",
        )
        self.assertEqual(
            GTK_E2E.input_mode_environment_error(
                "simple",
                {
                    "GTK_IM_MODULE": "gtk-im-context-simple",
                    "XMODIFIERS": "@im=ibus",
                },
            ),
            "XMODIFIERS must be unset for the GTK simple input path",
        )
        self.assertEqual(
            GTK_E2E.input_mode_environment_error(
                "simple",
                {
                    "GTK_IM_MODULE": "gtk-im-context-simple",
                    "IBUS_ADDRESS": "unix:path=/tmp/ibus",
                },
            ),
            "IBUS_ADDRESS must be unset for the GTK simple input path",
        )

    def test_ibus_test_requires_module_and_private_address(self):
        self.assertEqual(
            GTK_E2E.input_mode_environment_error("ibus", {}),
            "GTK_IM_MODULE must be set to ibus",
        )
        self.assertEqual(
            GTK_E2E.input_mode_environment_error(
                "ibus", {"GTK_IM_MODULE": "ibus"}
            ),
            "IBUS_ADDRESS must identify the private IBus daemon",
        )
        self.assertIsNone(
            GTK_E2E.input_mode_environment_error(
                "ibus", {"GTK_IM_MODULE": "ibus", "IBUS_ADDRESS": "unix:path=/tmp/ibus"}
            )
        )


class DisplayBackendEnvironmentTests(unittest.TestCase):
    def test_native_wayland_requires_its_socket_and_rejects_x11_fallback(self):
        self.assertEqual(
            GTK_E2E.display_backend_environment_error({"GDK_BACKEND": "wayland"}),
            "WAYLAND_DISPLAY must identify GNOME's private Wayland compositor",
        )
        self.assertEqual(
            GTK_E2E.display_backend_environment_error(
                {
                    "GDK_BACKEND": "wayland",
                    "WAYLAND_DISPLAY": "wayexpand-gnome-e2e",
                    "DISPLAY": ":99",
                }
            ),
            "X11 display variables must be unset for the native Wayland test",
        )
        self.assertIsNone(
            GTK_E2E.display_backend_environment_error(
                {
                    "GDK_BACKEND": "wayland",
                    "WAYLAND_DISPLAY": "wayexpand-gnome-e2e",
                }
            )
        )

    def test_x11_fixture_requires_private_display(self):
        self.assertEqual(
            GTK_E2E.display_backend_environment_error({"GDK_BACKEND": "x11"}),
            "DISPLAY must identify GNOME's private XWayland server",
        )
        self.assertIsNone(
            GTK_E2E.display_backend_environment_error(
                {"GDK_BACKEND": "x11", "DISPLAY": ":99"}
            )
        )

    def test_display_backend_must_be_explicit(self):
        self.assertEqual(
            GTK_E2E.display_backend_environment_error({}),
            "GDK_BACKEND must explicitly select wayland or x11",
        )


class WidgetInputModuleTests(unittest.TestCase):
    class FakeWidget:
        def __init__(self):
            self.properties = {"im-module": None}

        def set_property(self, name, value):
            self.properties[name] = value

        def get_property(self, name):
            return self.properties[name]

    def test_simple_gtk3_control_pins_the_widget_module(self):
        widget = self.FakeWidget()
        selected = GTK_E2E.configure_widget_im_module(widget, "3", "simple")
        self.assertEqual(selected, "gtk-im-context-simple")
        self.assertEqual(widget.get_property("im-module"), "gtk-im-context-simple")

    def test_ibus_context_keeps_the_desktop_default(self):
        widget = self.FakeWidget()
        selected = GTK_E2E.configure_widget_im_module(widget, "3", "ibus")
        self.assertIsNone(selected)
        self.assertIsNone(widget.get_property("im-module"))

    def test_gtk4_does_not_read_gtk3_only_widget_property(self):
        widget = object()
        self.assertIsNone(GTK_E2E.configure_widget_im_module(widget, "4", "simple"))


class MutterRemoteInputTests(unittest.TestCase):
    class FakeBus:
        def __init__(self, fail_first=False):
            self.calls = []
            self.fail_first = fail_first

        def call_sync(self, _bus, _path, _interface, method, parameters, *_args):
            values = parameters.unpack()
            self.calls.append((method, values))
            if self.fail_first:
                self.fail_first = False
                raise RuntimeError("simulated lost method reply")

    def test_fixed_text_uses_layout_independent_keysyms_and_releases_them(self):
        bus = self.FakeBus()
        with mock.patch.object(REMOTE_INPUT.time, "sleep"):
            REMOTE_INPUT.send_text(bus, "/session", ";e2e", 0.06)

        expected = [
            ("NotifyKeyboardKeysym", (ord(character), pressed))
            for character in ";e2e"
            for pressed in (True, False)
        ]
        self.assertEqual(bus.calls, expected)
        self.assertTrue(all(method != "NotifyKeyboardKeycode" for method, _ in bus.calls))

    def test_keyboard_is_created_before_window_activation_without_text_input(self):
        bus = self.FakeBus()
        REMOTE_INPUT.prime_keyboard_device(bus, "/session")

        self.assertEqual(
            bus.calls,
            [
                ("NotifyKeyboardKeysym", (REMOTE_INPUT.SHIFT_L_KEYSYM, True)),
                ("NotifyKeyboardKeysym", (REMOTE_INPUT.SHIFT_L_KEYSYM, False)),
            ],
        )

    def test_failed_press_reply_still_attempts_key_release(self):
        bus = self.FakeBus(fail_first=True)
        with mock.patch.object(REMOTE_INPUT.time, "sleep"):
            with self.assertRaisesRegex(RuntimeError, "simulated lost method reply"):
                REMOTE_INPUT.send_text(bus, "/session", "a", 0.06)

        self.assertEqual(
            bus.calls,
            [
                ("NotifyKeyboardKeysym", (ord("a"), True)),
                ("NotifyKeyboardKeysym", (ord("a"), False)),
            ],
        )


if __name__ == "__main__":
    unittest.main()
