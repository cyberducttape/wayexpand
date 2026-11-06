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
                candidate.get_title()?.endsWith(appId)
            ));
        if (!window) {
            invocation.return_dbus_error(
                'org.wayexpand.GnomeE2eFocusController.Error.NotFound',
                `no mapped test window for ${appId}`,
            );
            return;
        }
        window.activate(global.get_current_time());
        invocation.return_value(new GLib.Variant('()', []));
    }
}
