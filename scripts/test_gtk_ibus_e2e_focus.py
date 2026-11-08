#!/usr/bin/env python3
"""Regression tests for GTK IBus acceptance focus readiness."""

import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest import mock


SCRIPT = Path(__file__).parent / "certification-clients" / "gtk_ibus_e2e.py"
GNOME_ACCEPTANCE_SCRIPT = Path(__file__).parent / "test-gnome-e2e.sh"
FOCUS_CONTROLLER_SCRIPT = (
    Path(__file__).parent
    / "gnome-e2e-focus-controller"
    / "extension.js"
)
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
    def test_focus_controller_uses_shell_activation_with_workspace_handling(self):
        source = FOCUS_CONTROLLER_SCRIPT.read_text(encoding="utf-8")
        self.assertIn(
            "import * as Main from 'resource:///org/gnome/shell/ui/main.js';",
            source,
        )
        self.assertEqual(source.count("Main.activateWindow(window, global.get_current_time());"), 2)

    def test_remote_keyboard_is_created_after_shell_assigns_window_focus(self):
        source = REMOTE_INPUT_SCRIPT.read_text(encoding="utf-8")
        activation = source.index("activate_test_window(bus, args.app_id)")
        keyboard_setup = source.index("prime_keyboard_device(bus, session_path)")
        marker = source.index("publish_focus_confirmation(args.focus_marker)")
        self.assertLess(activation, keyboard_setup)
        self.assertLess(keyboard_setup, marker)

    def test_x11_clients_have_a_window_activation_helper(self):
        source = GNOME_ACCEPTANCE_SCRIPT.read_text(encoding="utf-8")
        self.assertRegex(source, r"(?m)^activate_window\(\) \{")
        self.assertIn(
            "org.wayexpand.GnomeE2eFocusController.ActivateWindow",
            source,
        )

    def test_x11_unicode_input_is_validated_and_sent_under_utf8_locale(self):
        source = GNOME_ACCEPTANCE_SCRIPT.read_text(encoding="utf-8")
        self.assertIn(
            'utf8_charmap=$(LC_ALL=C.UTF-8 locale charmap 2>/dev/null || true)',
            source,
        )
        self.assertIn('printf \'%s\' "$input_text" | iconv -f UTF-8 -t UTF-8', source)
        self.assertIn(
            'LC_ALL=C.UTF-8 DISPLAY="$x11_display" XAUTHORITY="$xwayland_auth" \\\n'
            '        xdotool type --clearmodifiers --delay 40 -- "$input_text"',
            source,
        )
        self.assertIn(
            "run_qt_xim_case unicode-passthrough 'Grüße' 'Grüße' false",
            source,
        )


class FocusReadinessTests(unittest.TestCase):
    def test_readiness_marker_is_removed_when_focus_is_lost(self):
        with tempfile.TemporaryDirectory() as directory:
            marker = Path(directory) / "ready"
            GTK_E2E.update_readiness_marker(marker, True)
            self.assertTrue(marker.is_file())
            GTK_E2E.update_readiness_marker(marker, False)
            self.assertFalse(marker.exists())

    def test_readiness_marker_can_be_republished_after_focus_returns(self):
        with tempfile.TemporaryDirectory() as directory:
            marker = Path(directory) / "ready"
            GTK_E2E.update_readiness_marker(marker, True)
            GTK_E2E.update_readiness_marker(marker, False)
            GTK_E2E.update_readiness_marker(marker, True)
            self.assertEqual(marker.read_bytes(), b"1")

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

    def test_window_activation_retries_only_while_window_is_unmapped(self):
        class FakeDbusError(Exception):
            pass

        class ActivationBus:
            def __init__(self):
                self.attempts = 0

            def call_sync(self, *_args):
                self.attempts += 1
                if self.attempts < 3:
                    raise FakeDbusError(
                        "GDBus.Error:" + REMOTE_INPUT.FOCUS_CONTROLLER_NOT_FOUND
                    )

        bus = ActivationBus()
        with mock.patch.object(REMOTE_INPUT.GLib, "Error", FakeDbusError):
            with mock.patch.object(REMOTE_INPUT.time, "monotonic", return_value=1.0):
                with mock.patch.object(REMOTE_INPUT.time, "sleep") as sleep:
                    REMOTE_INPUT.activate_test_window(bus, "org.example.Test")

        self.assertEqual(bus.attempts, 3)
        self.assertEqual(sleep.call_count, 2)

    def test_window_activation_does_not_retry_focus_or_authorization_failures(self):
        class FakeDbusError(Exception):
            pass

        class ActivationBus:
            def __init__(self, message):
                self.message = message
                self.attempts = 0

            def call_sync(self, *_args):
                self.attempts += 1
                raise FakeDbusError(self.message)

        for message in (
            "org.wayexpand.GnomeE2eFocusController.Error.FocusTimeout",
            "org.freedesktop.DBus.Error.AccessDenied",
        ):
            with self.subTest(message=message):
                bus = ActivationBus(message)
                with mock.patch.object(REMOTE_INPUT.GLib, "Error", FakeDbusError):
                    with mock.patch.object(REMOTE_INPUT.time, "monotonic", return_value=1.0):
                        with mock.patch.object(REMOTE_INPUT.time, "sleep") as sleep:
                            with self.assertRaisesRegex(FakeDbusError, message):
                                REMOTE_INPUT.activate_test_window(bus, "org.example.Test")
                self.assertEqual(bus.attempts, 1)
                sleep.assert_not_called()

    def test_window_activation_stops_retrying_after_the_map_deadline(self):
        class FakeDbusError(Exception):
            pass

        class ActivationBus:
            attempts = 0

            def call_sync(self, *_args):
                self.attempts += 1
                raise FakeDbusError(
                    "GDBus.Error:" + REMOTE_INPUT.FOCUS_CONTROLLER_NOT_FOUND
                )

        bus = ActivationBus()
        with mock.patch.object(REMOTE_INPUT.GLib, "Error", FakeDbusError):
            with mock.patch.object(
                REMOTE_INPUT.time, "monotonic", side_effect=[10.0, 15.0]
            ):
                with mock.patch.object(REMOTE_INPUT.time, "sleep") as sleep:
                    with self.assertRaisesRegex(
                        FakeDbusError, REMOTE_INPUT.FOCUS_CONTROLLER_NOT_FOUND
                    ):
                        REMOTE_INPUT.activate_test_window(bus, "org.example.Test")

        self.assertEqual(bus.attempts, 1)
        sleep.assert_not_called()

    def test_fixed_text_uses_layout_independent_keysyms_and_releases_them(self):
        bus = self.FakeBus()
        with tempfile.TemporaryDirectory() as directory:
            marker = Path(directory) / "ready"
            marker.write_text("1", encoding="ascii")
            with mock.patch.object(REMOTE_INPUT.time, "sleep"):
                REMOTE_INPUT.send_text(bus, "/session", ";e2e", 0.06, marker)

        expected = [
            ("NotifyKeyboardKeysym", (ord(character), pressed))
            for character in ";e2e"
            for pressed in (True, False)
        ]
        self.assertEqual(bus.calls, expected)
        self.assertTrue(all(method != "NotifyKeyboardKeycode" for method, _ in bus.calls))

    def test_keyboard_priming_uses_only_a_modifier_pair(self):
        bus = self.FakeBus()
        REMOTE_INPUT.prime_keyboard_device(bus, "/session")

        self.assertEqual(
            bus.calls,
            [
                ("NotifyKeyboardKeysym", (REMOTE_INPUT.SHIFT_L_KEYSYM, True)),
                ("NotifyKeyboardKeysym", (REMOTE_INPUT.SHIFT_L_KEYSYM, False)),
            ],
        )

    def test_focus_loss_stops_remaining_text_before_another_key_is_pressed(self):
        bus = self.FakeBus()
        with tempfile.TemporaryDirectory() as directory:
            marker = Path(directory) / "ready"
            marker.write_text("1", encoding="ascii")

            def lose_focus(_delay):
                marker.unlink(missing_ok=True)

            with mock.patch.object(
                REMOTE_INPUT.time, "sleep", side_effect=lose_focus
            ):
                with self.assertRaisesRegex(RuntimeError, "focus was lost"):
                    REMOTE_INPUT.send_text(bus, "/session", "ab", 0.06, marker)

        self.assertEqual(
            bus.calls,
            [
                ("NotifyKeyboardKeysym", (ord("a"), True)),
                ("NotifyKeyboardKeysym", (ord("a"), False)),
            ],
        )

    def test_input_is_not_started_without_a_current_focus_lease(self):
        bus = self.FakeBus()
        with tempfile.TemporaryDirectory() as directory:
            marker = Path(directory) / "ready"
            with self.assertRaisesRegex(RuntimeError, "focus was lost"):
                REMOTE_INPUT.send_text(bus, "/session", "a", 0.06, marker)
        self.assertEqual(bus.calls, [])

    def test_failed_press_reply_still_attempts_key_release(self):
        bus = self.FakeBus(fail_first=True)
        with tempfile.TemporaryDirectory() as directory:
            marker = Path(directory) / "ready"
            marker.write_text("1", encoding="ascii")
            with mock.patch.object(REMOTE_INPUT.time, "sleep"):
                with self.assertRaisesRegex(RuntimeError, "simulated lost method reply"):
                    REMOTE_INPUT.send_text(bus, "/session", "a", 0.06, marker)

        self.assertEqual(
            bus.calls,
            [
                ("NotifyKeyboardKeysym", (ord("a"), True)),
                ("NotifyKeyboardKeysym", (ord("a"), False)),
            ],
        )


if __name__ == "__main__":
    unittest.main()
