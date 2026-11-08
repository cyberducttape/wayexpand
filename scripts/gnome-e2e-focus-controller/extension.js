// Test-only helper copied into the private GNOME session by test-gnome-e2e.sh.
// Activating a window through Mutter gives the test a compositor-generated
// focus change without trying to make an untrusted Wayland client steal focus.

import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Clutter from 'gi://Clutter';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

const OBJECT_PATH = '/org/wayexpand/GnomeE2eFocusController';
const INTERFACE = `
<node>
  <interface name="org.wayexpand.GnomeE2eFocusController">
    <method name="ActivateWindow">
      <arg type="s" direction="in" name="app_id"/>
    </method>
    <method name="TypeText">
      <arg type="s" direction="in" name="app_id"/>
      <arg type="s" direction="in" name="text"/>
    </method>
  </interface>
</node>`;

export default class GnomeE2eFocusController extends Extension {
    enable() {
        this._exported = Gio.DBusExportedObject.wrapJSObject(INTERFACE, this);
        this._exported.export(Gio.DBus.session, OBJECT_PATH);
    }

    disable() {
        this._exported?.unexport();
        this._exported = null;
        this._virtualKeyboard = null;
    }

    TypeTextAsync([appId, text], invocation) {
        // XTEST can report success while headless Mutter fails to deliver its
        // XWayland key events to GTK. Inject through Mutter's virtual keyboard
        // instead so the acceptance test exercises compositor keyboard
        // delivery without depending on a host keyboard or clipboard.
        const codepoints = Array.from(text, character => character.codePointAt(0));
        if (codepoints.length === 0 || codepoints.length > 128 ||
            codepoints.some(codepoint => codepoint < 0x20 || codepoint === 0x7f)) {
            invocation.return_dbus_error(
                'org.wayexpand.GnomeE2eFocusController.Error.InvalidText',
                'test input must contain 1–128 printable Unicode characters',
            );
            return;
        }

        const focusedWindow = global.display.get_focus_window();
        const targetIsFocused = focusedWindow && (
            focusedWindow.get_gtk_application_id() === appId ||
            focusedWindow.get_wm_class() === appId ||
            focusedWindow.get_title()?.includes(appId)
        );
        if (!targetIsFocused) {
            invocation.return_dbus_error(
                'org.wayexpand.GnomeE2eFocusController.Error.WrongTarget',
                `refusing to type because ${appId} is not the focused window`,
            );
            return;
        }

        try {
            if (!this._virtualKeyboard) {
                // GNOME Shell 51 removed Clutter.get_default_backend(). Keep
                // the old accessor only as a compatibility fallback for
                // older Shell versions.
                const backend = typeof global.stage?.context?.get_backend === 'function'
                    ? global.stage.context.get_backend()
                    : Clutter.get_default_backend();
                const seat = backend.get_default_seat();
                this._virtualKeyboard = seat.create_virtual_device(
                    Clutter.InputDeviceType.KEYBOARD_DEVICE,
                );
            }
            const delayUs = 12_000;
            let timestamp = GLib.get_monotonic_time();
            for (const codepoint of codepoints) {
                // X11's Unicode keysym encoding is also used for codepoints
                // outside ASCII. notify_keyval lets Mutter route these through
                // the active input source and focused client.
                const keyval = codepoint < 0x100
                    ? codepoint
                    : 0x01000000 + codepoint;
                this._virtualKeyboard.notify_keyval(
                    timestamp, keyval, Clutter.KeyState.PRESSED,
                );
                timestamp += delayUs;
                this._virtualKeyboard.notify_keyval(
                    timestamp, keyval, Clutter.KeyState.RELEASED,
                );
                timestamp += delayUs;
            }
        } catch (error) {
            invocation.return_dbus_error(
                'org.wayexpand.GnomeE2eFocusController.Error.KeyboardUnavailable',
                `Mutter virtual keyboard injection failed: ${error.message}`,
            );
            return;
        }

        // Let Mutter dispatch the queued events before the test client reads
        // its text field. The client still verifies the exact resulting text.
        GLib.timeout_add(GLib.PRIORITY_DEFAULT, codepoints.length * 24 + 50, () => {
            invocation.return_value(new GLib.Variant('()', []));
            return GLib.SOURCE_REMOVE;
        });
    }

    ActivateWindowAsync([appId], invocation) {
        const window = global.get_window_actors()
            .map(actor => actor.meta_window)
            .find(candidate => candidate && (
                candidate.get_gtk_application_id() === appId ||
                candidate.get_wm_class() === appId ||
                candidate.get_title()?.includes(appId)
            ));
        if (!window) {
            invocation.return_dbus_error(
                'org.wayexpand.GnomeE2eFocusController.Error.NotFound',
                `no mapped test window for ${appId}`,
            );
            return;
        }
        // Mutter may accept activate() before the focus transition has
        // completed. Do not acknowledge the request until the compositor's
        // focused-window state confirms that the test target owns focus.
        const startedAt = GLib.get_monotonic_time();
        let attempts = 0;
        const confirmFocus = () => {
            if (global.display.get_focus_window() === window) {
                invocation.return_value(new GLib.Variant('()', []));
                return GLib.SOURCE_REMOVE;
            }

            attempts += 1;
            if (attempts >= 100 ||
                GLib.get_monotonic_time() - startedAt >= 5_000_000) {
                invocation.return_dbus_error(
                    'org.wayexpand.GnomeE2eFocusController.Error.FocusTimeout',
                    `Mutter did not focus ${appId} within five seconds`,
                );
                return GLib.SOURCE_REMOVE;
            }

            window.activate(global.get_current_time());
            return GLib.SOURCE_CONTINUE;
        };

        window.activate(global.get_current_time());
        GLib.timeout_add(GLib.PRIORITY_DEFAULT, 50, confirmFocus);
    }
}
