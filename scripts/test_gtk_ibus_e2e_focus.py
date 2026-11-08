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

    def test_shell_reactivates_window_after_keyboard_priming(self):
        source = REMOTE_INPUT_SCRIPT.read_text(encoding="utf-8")
        activations = [
            index
            for index in range(len(source))
            if source.startswith("activate_test_window(bus, args.app_id)", index)
        ]
        keyboard_setup = source.index("prime_keyboard_device(bus, session_path)")
        marker = source.index("publish_focus_confirmation(args.focus_marker)")
        self.assertEqual(len(activations), 2)
        self.assertLess(activations[0], keyboard_setup)
        self.assertLess(keyboard_setup, activations[1])
        self.assertLess(activations[1], marker)

    def test_focus_controller_reports_live_pointer_coordinates(self):
        source = FOCUS_CONTROLLER_SCRIPT.read_text(encoding="utf-8")
        self.assertIn('GetPointerPositionAsync(_args, invocation)', source)
        self.assertIn('const [x, y] = global.get_pointer();', source)

    def test_pointer_click_converts_absolute_target_to_relative_delta(self):
        source = REMOTE_INPUT_SCRIPT.read_text(encoding="utf-8")
        self.assertIn('def get_pointer_position(bus):', source)
        self.assertIn('start = (pointer_x, pointer_y)', source)
        self.assertIn('actual_x, actual_y = get_pointer_position(bus)', source)
        self.assertIn('Mutter pointer {axis}-axis motion was not applied', source)
        self.assertIn('delta if axis == "x" else 0.0', source)
        self.assertIn('delta if axis == "y" else 0.0', source)
        self.assertNotIn('pointer_motion(bus, path, -2000.0, -2000.0)', source)

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

    def test_native_gtk_baseline_uses_evdev_keycode_probe(self):
        source = GNOME_ACCEPTANCE_SCRIPT.read_text(encoding="utf-8")
        self.assertIn('if [ "$case_name" = wayland-baseline ]; then', source)
        self.assertIn('keycode_probe=true', source)
        self.assertIn('--keycode-probe', source)

    def test_native_gtk_fixture_uses_undecorated_window_with_known_input_size(self):
        source = SCRIPT.read_text(encoding="utf-8")
        self.assertIn("window.set_default_size(500, 80)", source)
        self.assertIn("window.set_decorated(False)", source)
        self.assertIn("focus_widget.set_hexpand(True)", source)
        self.assertIn("focus_widget.set_vexpand(True)", source)

    def test_gtk_fixture_reports_pointer_delivery_without_retaining_input_data(self):
        source = SCRIPT.read_text(encoding="utf-8")
        self.assertIn("gtk_pointer_events += 1", source)
        self.assertIn("gtk_gdk_pointer_events += 1", source)
        self.assertIn("GTK pointer press events observed at target widget", source)
        self.assertIn("GDK pointer press events received by GTK process", source)


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

    def test_focus_requests_retry_until_window_and_widget_are_active(self):
        due, next_request = GTK_E2E.focus_request_due(False, False, 1_000_000, None)
        self.assertTrue(due)
        self.assertEqual(next_request, 1_000_000 + GTK_E2E.FOCUS_RETRY_INTERVAL_US)

        due, unchanged = GTK_E2E.focus_request_due(
            False, False, next_request - 1, next_request
        )
        self.assertFalse(due)
        self.assertEqual(unchanged, next_request)

        due, next_request = GTK_E2E.focus_request_due(
            False, False, next_request, next_request
        )
        self.assertTrue(due)
        self.assertEqual(next_request, 1_000_000 + 2 * GTK_E2E.FOCUS_RETRY_INTERVAL_US)

    def test_focus_requests_stop_when_window_and_widget_are_active(self):
        due, next_request = GTK_E2E.focus_request_due(True, True, 2_000_000, None)
        self.assertFalse(due)
        self.assertIsNone(next_request)

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
            self.pointer = [100, 50]

        def call_sync(self, _bus, _path, _interface, method, parameters, *_args):
            values = parameters.unpack() if parameters is not None else ()
            self.calls.append((method, values))
            if self.fail_first:
                self.fail_first = False
                raise RuntimeError("simulated lost method reply")
            if method == "GetPointerPosition":
                class Result:
                    def __init__(self, position):
                        self.position = position

                    def unpack(self):
                        return self.position

                return Result(tuple(self.pointer))
            if method == "NotifyPointerMotionRelative":
                self.pointer[0] += values[0]
                self.pointer[1] += values[1]

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

    def test_xkb_baseline_can_probe_evdev_keycode_translation(self):
        bus = self.FakeBus()
        with tempfile.TemporaryDirectory() as directory:
            marker = Path(directory) / "ready"
            marker.write_text("1", encoding="ascii")
            with mock.patch.object(REMOTE_INPUT.time, "sleep"):
                REMOTE_INPUT.send_text(
                    bus, "/session", "a", 0.06, marker, keycode_probe=True
                )

        self.assertEqual(
            bus.calls,
            [
                ("NotifyKeyboardKeycode", (30, True)),
                ("NotifyKeyboardKeycode", (30, False)),
            ],
        )

    def test_click_test_field_uses_mutter_pointer_before_keyboard_input(self):
        bus = self.FakeBus()
        with mock.patch.object(REMOTE_INPUT.time, "sleep"):
            REMOTE_INPUT.click_test_field(bus, "/session", (390, 360, 500, 80))

        self.assertEqual(
            bus.calls,
            [
                ("GetPointerPosition", ()),
                ("NotifyPointerMotionRelative", (540.0, 0.0)),
                ("GetPointerPosition", ()),
                ("NotifyPointerMotionRelative", (0.0, 350.0)),
                ("GetPointerPosition", ()),
                ("GetPointerPosition", ()),
                ("NotifyPointerButton", (REMOTE_INPUT.BTN_LEFT, True)),
                ("NotifyPointerButton", (REMOTE_INPUT.BTN_LEFT, False)),
            ],
        )

    def test_click_test_field_rejects_axis_motion_that_did_not_reach_target(self):
        class DroppedHorizontalMotionBus(self.FakeBus):
            def call_sync(self, bus_name, path, interface, method, parameters, *_args):
                values = parameters.unpack() if parameters is not None else ()
                if method == "NotifyPointerMotionRelative" and values[0] != 0.0:
                    self.calls.append((method, values))
                    return
                return super().call_sync(
                    bus_name, path, interface, method, parameters, *_args
                )

        bus = DroppedHorizontalMotionBus()
        with mock.patch.object(REMOTE_INPUT.time, "sleep"):
            with self.assertRaisesRegex(
                RuntimeError, r"Mutter pointer x-axis motion was not applied"
            ):
                REMOTE_INPUT.click_test_field(bus, "/session", (390, 360, 500, 80))

        self.assertNotIn(
            ("NotifyPointerButton", (REMOTE_INPUT.BTN_LEFT, True)), bus.calls
        )

    def test_test_window_rect_comes_from_shell_for_the_requested_app(self):
        class RectBus:
            def __init__(self):
                self.calls = []

            def call_sync(self, bus_name, path, interface, method, parameters, *_args):
                self.calls.append(
                    (bus_name, path, interface, method, parameters.unpack())
                )

                class Result:
                    @staticmethod
                    def unpack():
                        return (390, 360, 500, 80)

                return Result()

        bus = RectBus()
        rect = REMOTE_INPUT.get_test_window_rect(bus, "org.example.Test")

        self.assertEqual(rect, (390, 360, 500, 80))
        self.assertEqual(
            bus.calls,
            [
                (
                    "org.gnome.Shell",
                    REMOTE_INPUT.FOCUS_CONTROLLER_PATH,
                    REMOTE_INPUT.FOCUS_CONTROLLER_INTERFACE,
                    "GetWindowRect",
                    ("org.example.Test",),
                )
            ],
        )

    def test_gtk_focus_retries_start_before_pointer_click(self):
        source = REMOTE_INPUT_SCRIPT.read_text(encoding="utf-8")
        click = source.index("click_test_field(bus, session_path, window_rect)")
        publish = source.index("publish_focus_confirmation(args.focus_marker)")
        wait = source.index("if not wait_for_gtk_input_ready(")
        self.assertLess(publish, click)
        self.assertLess(click, wait)

    def test_gtk_readiness_reissues_shell_activation_until_stable(self):
        times = [0.0, 0.0, 0.0, 0.26, 0.26]
        with (
            mock.patch.object(REMOTE_INPUT.time, "monotonic", side_effect=times),
            mock.patch.object(REMOTE_INPUT.time, "sleep"),
            mock.patch.object(REMOTE_INPUT.os.path, "isfile", return_value=True),
            mock.patch.object(REMOTE_INPUT, "activate_test_window") as activate,
        ):
            self.assertTrue(
                REMOTE_INPUT.wait_for_gtk_input_ready(
                    object(), "org.example.Test", "/tmp/ready", 13.0
                )
            )
        self.assertEqual(activate.call_count, 2)

    def test_shell_focus_controller_reports_mapped_window_geometry(self):
        source = FOCUS_CONTROLLER_SCRIPT.read_text(encoding="utf-8")
        self.assertIn('<method name="GetWindowRect">', source)
        self.assertIn("GetWindowRectAsync([appId], invocation)", source)
        self.assertIn("window.get_frame_rect()", source)

    def test_evdev_keycode_probe_rejects_text_beyond_its_fixed_baseline(self):
        bus = self.FakeBus()
        with tempfile.TemporaryDirectory() as directory:
            marker = Path(directory) / "ready"
            marker.write_text("1", encoding="ascii")
            with self.assertRaisesRegex(ValueError, "limited to the fixed ASCII"):
                REMOTE_INPUT.send_text(
                    bus, "/session", ";e2e", 0.06, marker, keycode_probe=True
                )

        self.assertEqual(bus.calls, [])

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
