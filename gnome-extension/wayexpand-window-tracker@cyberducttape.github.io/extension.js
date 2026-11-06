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
// Unsubscribe() or leave the bus. No window information is stored. The
// separate input-source methods expose only GNOME's configured active source
// and delegate selection changes to GNOME Shell's own manager.

import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Shell from 'gi://Shell';
import * as Keyboard from 'resource:///org/gnome/shell/ui/status/keyboard.js';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import {LatestOnlyDelivery} from './notification-queue.js';

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
    <method name="Ping"/>
    <method name="GetActiveInputSource">
      <arg type="s" direction="out" name="kind"/>
      <arg type="s" direction="out" name="id"/>
    </method>
    <method name="ActivateInputSource">
      <arg type="s" direction="in" name="kind"/>
      <arg type="s" direction="in" name="id"/>
      <arg type="s" direction="out" name="active_kind"/>
      <arg type="s" direction="out" name="active_id"/>
    </method>
    <property name="Version" type="u" access="read"/>
    <property name="BuildIdentity" type="s" access="read"/>
    <property name="InputSourceControlVersion" type="u" access="read"/>
  </interface>
</node>`;
// The callback every subscriber serves (the same one WayExpand serves for
// its KWin bridge).
const CLIENT_PATH = '/WindowTracker';
const CLIENT_IFACE = 'org.wayexpand.WindowTracker1';
const PROTOCOL_VERSION = 2;
const MAX_SUBSCRIBERS = 8;
const CALL_TIMEOUT_MS = 2000;
const NOTIFY_COALESCE_MS = 30;

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
        this._notifySource = 0;
        this._latestNotification = null;
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
        if (this._notifySource) {
            GLib.source_remove(this._notifySource);
            this._notifySource = 0;
        }
        for (const subscriber of this._subscribers.values())
            Gio.bus_unwatch_name(subscriber.watchId);
        this._subscribers = null;
        this._exported.unexport();
        this._exported = null;
        this._focused = null;
    }

    get Version() {
        return PROTOCOL_VERSION;
    }

    get BuildIdentity() {
        return this.metadata['x-wayexpand-build-id'] || '';
    }

    get InputSourceControlVersion() {
        return 1;
    }

    GetActiveInputSourceAsync(_parameters, invocation) {
        const source = Keyboard.getInputSourceManager().currentSource;
        if (!source) {
            invocation.return_dbus_error(
                'org.wayexpand.ShellWindowTracker1.Error.NoActiveInputSource',
                'GNOME Shell has no active input source',
            );
            return;
        }
        invocation.return_value(new GLib.Variant('(ss)', [source.type, source.id]));
    }

    ActivateInputSourceAsync([kind, id], invocation) {
        if (!['ibus', 'xkb'].includes(kind) || !id || id.length > 256) {
            invocation.return_dbus_error(
                'org.wayexpand.ShellWindowTracker1.Error.InvalidInputSource',
                'invalid GNOME input source',
            );
            return;
        }

        const manager = Keyboard.getInputSourceManager();
        // GNOME Shell may have loaded the source list before a newly
        // registered IBus engine appeared. Reload the configured list before
        // looking up the source so activation uses Shell's live objects.
        manager.reload();
        const source = Object.values(manager.inputSources).find(candidate =>
            candidate.type === kind && candidate.id === id);
        if (!source) {
            invocation.return_dbus_error(
                'org.wayexpand.ShellWindowTracker1.Error.InputSourceUnavailable',
                `GNOME input source ${kind}:${id} is unavailable`,
            );
            return;
        }

        // Use GNOME Shell's own source activation path. In particular this
        // updates the live InputSourceManager and its MRU persistence; writing
        // the deprecated gsettings "current" key does neither reliably.
        source.activate(true);

        const active = manager.currentSource;
        if (!active || active.type !== kind || active.id !== id) {
            invocation.return_dbus_error(
                'org.wayexpand.ShellWindowTracker1.Error.ActivationFailed',
                'GNOME Shell did not activate the requested input source',
            );
            return;
        }
        invocation.return_value(new GLib.Variant('(ss)', [active.type, active.id]));
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
            const state = {watchId, delivery: null};
            state.delivery = new LatestOnlyDelivery((parameters, complete) => {
                Gio.DBus.session.call(subscriber, CLIENT_PATH, CLIENT_IFACE, 'WindowChanged',
                    parameters, null, Gio.DBusCallFlags.NO_AUTO_START, CALL_TIMEOUT_MS, null,
                    (connection, result) => {
                        try {
                            connection.call_finish(result);
                            complete(null);
                        } catch (error) {
                            complete(error);
                        }
                    });
            }, () => this._drop(subscriber));
            this._subscribers.set(subscriber, state);
        }
        invocation.return_value(new GLib.Variant('(sss)', describe(this._focused)));
    }

    UnsubscribeAsync(_parameters, invocation) {
        this._drop(invocation.get_sender());
        invocation.return_value(null);
    }

    PingAsync(_parameters, invocation) {
        if (!this._subscribers.has(invocation.get_sender())) {
            invocation.return_error_literal(Gio.DBusError, Gio.DBusError.ACCESS_DENIED,
                'window tracker subscription is not active');
            return;
        }
        invocation.return_value(null);
    }

    _drop(subscriber) {
        const state = this._subscribers?.get(subscriber);
        if (!state)
            return;
        state.delivery.close();
        Gio.bus_unwatch_name(state.watchId);
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
        this._latestNotification = new GLib.Variant('(sss)', describe(this._focused));
        if (this._notifySource)
            return;
        this._notifySource = GLib.timeout_add(GLib.PRIORITY_DEFAULT,
            NOTIFY_COALESCE_MS, () => {
                this._notifySource = 0;
                this._flushNotifications();
                return GLib.SOURCE_REMOVE;
            });
    }

    _flushNotifications() {
        if (!this._subscribers || !this._latestNotification)
            return;
        const parameters = this._latestNotification;
        this._latestNotification = null;
        for (const state of this._subscribers.values()) {
            // A slow subscriber gets at most one outstanding method call and
            // one replaceable pending snapshot. Bursts never accumulate a
            // queue of stale window titles in GNOME Shell.
            state.delivery.enqueue(parameters);
        }
    }
}
