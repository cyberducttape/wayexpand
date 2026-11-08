// Test-only helper copied into the private GNOME session by test-gnome-e2e.sh.
// Activating a window through Mutter gives the test a compositor-generated
// focus change without trying to make an untrusted Wayland client steal focus.

import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

const OBJECT_PATH = '/org/wayexpand/GnomeE2eFocusController';
const INTERFACE = `
<node>
  <interface name="org.wayexpand.GnomeE2eFocusController">
    <method name="ActivateWindow">
      <arg type="s" direction="in" name="app_id"/>
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
