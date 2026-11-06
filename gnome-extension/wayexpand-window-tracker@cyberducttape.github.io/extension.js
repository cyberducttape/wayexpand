// WayExpand window tracker for GNOME Shell (GNOME 45+).
//
// Mutter offers no Wayland protocol or public D-Bus API that tells another
// process which window has focus (org.gnome.Shell.Introspect is limited to
// allowlisted callers, and Eval is disabled). This extension runs inside
// GNOME Shell and reports the focused window to WayExpand, which limits
// snippets to certain applications (app_filter) with it.
//
// Privacy: nothing is broadcast. A process calls Subscribe() and receives
// the current window as the reply; later changes are sent only to
// subscribers, as method calls to their private bus names, until they call
// Unsubscribe() or leave the bus. Flatpak-sandboxed apps cannot reach
// org.gnome.Shell unless their sandbox grants it. Nothing is stored.

import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Shell from 'gi://Shell';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

const TRACKER_PATH = '/org/wayexpand/ShellWindowTracker';
const TRACKER_IFACE = `
<node>
  <interface name="org.wayexpand.ShellWindowTracker1">
    <method name="Subscribe">
      <arg type="s" direction="out" name="app_id"/>
      <arg type="s" direction="out" name="title"/>
      <arg type="s" direction="out" name="instance_id"/>
    </method>
    <method name="Unsubscribe"/>
    <property name="Version" type="u" access="read"/>
  </interface>
</node>`;
// The callback every subscriber serves (the same one WayExpand serves for
// its KWin bridge).
const CLIENT_PATH = '/WindowTracker';
const CLIENT_IFACE = 'org.wayexpand.WindowTracker1';
const PROTOCOL_VERSION = 1;
const MAX_SUBSCRIBERS = 8;
const CALL_TIMEOUT_MS = 2000;

/** [app_id, title, instance_id] for `window`, or empty strings for none. */
function describe(window) {
    if (!window)
        return ['', '', ''];
    // Prefer the desktop-file ID (stable, matches .desktop and Flatpak IDs).
    // Shell invents "window:N" IDs for windows it cannot match to an app.
    let appId = '';
    const app = Shell.WindowTracker.get_default().get_window_app(window);
    const desktopId = app ? app.get_id() : null;
    if (desktopId && !desktopId.startsWith('window:'))
        appId = desktopId.replace(/\.desktop$/, '');
    if (!appId)
        appId = window.get_gtk_application_id() || window.get_wm_class() || '';
    return [appId, window.get_title() || '', String(window.get_id())];
}

export default class WayExpandWindowTracker extends Extension {
    enable() {
        this._subscribers = new Map();
        this._focused = null;
        this._titleChangedId = 0;
        this._exported = Gio.DBusExportedObject.wrapJSObject(TRACKER_IFACE, this);
        this._exported.export(Gio.DBus.session, TRACKER_PATH);
        this._focusChangedId = global.display.connect('notify::focus-window',
            () => this._focusChanged());
        this._focusChanged();
    }

    disable() {
        global.display.disconnect(this._focusChangedId);
        this._focusChangedId = 0;
        this._watchTitle(null);
        for (const watchId of this._subscribers.values())
            Gio.bus_unwatch_name(watchId);
        this._subscribers = null;
        this._exported.unexport();
        this._exported = null;
        this._focused = null;
    }

    get Version() {
        return PROTOCOL_VERSION;
    }

    SubscribeAsync(_parameters, invocation) {
        const subscriber = invocation.get_sender();
        if (!this._subscribers.has(subscriber)) {
            if (this._subscribers.size >= MAX_SUBSCRIBERS) {
                invocation.return_error_literal(Gio.DBusError, Gio.DBusError.LIMITS_EXCEEDED,
                    'too many WayExpand window-tracker subscribers');
                return;
            }
            const watchId = Gio.bus_watch_name_on_connection(Gio.DBus.session, subscriber,
                Gio.BusNameWatcherFlags.NONE, null, () => this._drop(subscriber));
            this._subscribers.set(subscriber, watchId);
        }
        invocation.return_value(new GLib.Variant('(sss)', describe(this._focused)));
    }

    UnsubscribeAsync(_parameters, invocation) {
        this._drop(invocation.get_sender());
        invocation.return_value(null);
    }

    _drop(subscriber) {
        const watchId = this._subscribers?.get(subscriber);
        if (watchId === undefined)
            return;
        Gio.bus_unwatch_name(watchId);
        this._subscribers.delete(subscriber);
    }

    _focusChanged() {
        this._focused = global.display.get_focus_window();
        this._watchTitle(this._focused);
        this._notify();
    }

    _watchTitle(window) {
        if (this._titleWindow && this._titleChangedId) {
            try {
                this._titleWindow.disconnect(this._titleChangedId);
            } catch {
                // The window was already destroyed with its handlers.
            }
        }
        this._titleWindow = window;
        this._titleChangedId = window
            ? window.connect('notify::title', () => this._notify())
            : 0;
    }

    _notify() {
        if (!this._subscribers || this._subscribers.size === 0)
            return;
        const parameters = new GLib.Variant('(sss)', describe(this._focused));
        for (const subscriber of this._subscribers.keys()) {
            Gio.DBus.session.call(subscriber, CLIENT_PATH, CLIENT_IFACE, 'WindowChanged',
                parameters, null, Gio.DBusCallFlags.NO_AUTO_START, CALL_TIMEOUT_MS, null,
                (connection, result) => {
                    try {
                        connection.call_finish(result);
                    } catch {
                        // A subscriber that cannot take the update is gone
                        // or broken; it subscribes again when it reconnects.
                        this._drop(subscriber);
                    }
                });
        }
    }
}
