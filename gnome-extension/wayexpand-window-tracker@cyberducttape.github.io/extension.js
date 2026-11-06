// WayExpand window tracker for GNOME Shell (GNOME 45+).
//
// Mutter offers no Wayland protocol or public D-Bus API that tells another
// process which window has focus (org.gnome.Shell.Introspect is limited to
// allowlisted callers, and Eval is disabled). This extension runs inside
// GNOME Shell and reports the focused window to WayExpand, which limits
// snippets to certain applications (app_filter) with it.
//
// Privacy boundary: the session bus is shared by processes running as this
// user, so subscriber checks are defense in depth, not a boundary against
// malicious same-user software. Subscribe() returns app IDs and opaque window
// IDs without titles. The trusted WayExpand client may explicitly request
// titles for configured title_contains filters. Focus data is not persisted.
// Input-source methods expose only the active source and delegate selection
// changes to GNOME Shell's own manager.

import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Shell from 'gi://Shell';
import * as Keyboard from 'resource:///org/gnome/shell/ui/status/keyboard.js';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import {
    isAuthorizedTrackerClient,
    LatestOnlyDelivery,
    setTitleAccess,
    subscriberSnapshot,
} from './notification-queue.js';

const TRACKER_PATH = '/org/wayexpand/ShellWindowTracker';
const TRACKER_IFACE = `
<node>
  <interface name="org.wayexpand.ShellWindowTracker1">
    <method name="Subscribe">
      <arg type="s" direction="out" name="app_id"/>
      <arg type="s" direction="out" name="title"/>
      <arg type="s" direction="out" name="instance_id"/>
    </method>
    <method name="SubscribeWithTitles">
      <arg type="b" direction="in" name="include_titles"/>
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
const DBUS_NAME = 'org.freedesktop.DBus';
const DBUS_PATH = '/org/freedesktop/DBus';

/** [app_id, empty title, instance_id] for `window`, or empty strings for none. */
function describe(window, includeTitle = false) {
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
    const title = includeTitle ? (window.get_title() || '') : '';
    return subscriberSnapshot(appId, title, String(window.get_id()), includeTitle);
}

export default class WayExpandWindowTracker extends Extension {
    enable() {
        this._subscribers = new Map();
        this._focused = null;
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
        if (this._notifySource) {
            GLib.source_remove(this._notifySource);
            this._notifySource = 0;
        }
        for (const subscriber of this._subscribers.values()) {
            subscriber.delivery.close();
            Gio.bus_unwatch_name(subscriber.watchId);
        }
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
        this._subscribe(invocation, false);
    }

    SubscribeWithTitlesAsync([includeTitles], invocation) {
        this._subscribe(invocation, includeTitles === true);
    }

    _subscribe(invocation, includeTitles) {
        const subscriber = invocation.get_sender();
        if (!this._isAuthorizedSubscriber(subscriber)) {
            invocation.return_error_literal(Gio.DBusError, Gio.DBusError.ACCESS_DENIED,
                'window tracker subscriptions are limited to WayExpand executables');
            return;
        }
        if (!this._subscribers.has(subscriber)) {
            if (this._subscribers.size >= MAX_SUBSCRIBERS) {
                invocation.return_error_literal(Gio.DBusError, Gio.DBusError.LIMITS_EXCEEDED,
                    'too many WayExpand window-tracker subscribers');
                return;
            }
            const watchId = Gio.bus_watch_name_on_connection(Gio.DBus.session, subscriber,
                Gio.BusNameWatcherFlags.NONE, null, () => this._drop(subscriber));
            const state = {watchId, delivery: null, includeTitles};
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
        } else {
            // Treat every subscription call as an explicit privacy mode
            // update. In particular, Subscribe() must revoke title delivery
            // from a connection that previously opted in.
            setTitleAccess(this._subscribers.get(subscriber), includeTitles);
        }
        const state = this._subscribers.get(subscriber);
        this._watchTitle(this._focused);
        invocation.return_value(new GLib.Variant('(sss)', describe(this._focused, state.includeTitles)));
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
        this._watchTitle(this._focused);
    }

    _isAuthorizedSubscriber(subscriber) {
        try {
            const userReply = Gio.DBus.session.call_sync(DBUS_NAME, DBUS_PATH, DBUS_NAME,
                'GetConnectionUnixUser', new GLib.Variant('(s)', [subscriber]),
                new GLib.VariantType('(u)'), Gio.DBusCallFlags.NONE, 500, null);
            const [uid] = userReply.deep_unpack();
            const pidReply = Gio.DBus.session.call_sync(DBUS_NAME, DBUS_PATH, DBUS_NAME,
                'GetConnectionUnixProcessID', new GLib.Variant('(s)', [subscriber]),
                new GLib.VariantType('(u)'), Gio.DBusCallFlags.NONE, 500, null);
            const [pid] = pidReply.deep_unpack();
            const executable = GLib.file_read_link(`/proc/${pid}/exe`);
            return isAuthorizedTrackerClient(uid, GLib.get_user_id(), executable);
        } catch {
            // Unknown peer identity must not gain access to focus metadata.
            return false;
        }
    }

    _focusChanged() {
        this._focused = global.display.get_focus_window();
        this._watchTitle(this._focused);
        this._notify();
    }

    _watchTitle(window) {
        const shouldWatch = [...this._subscribers.values()].some(subscriber => subscriber.includeTitles);
        const target = shouldWatch ? window : null;
        if (this._titleWindow === target && (!target || this._titleChangedId))
            return;
        if (this._titleWindow && this._titleChangedId) {
            try {
                this._titleWindow.disconnect(this._titleChangedId);
            } catch {
                // The window was already destroyed with its handlers.
            }
        }
        this._titleWindow = target;
        this._titleChangedId = target
            ? target.connect('notify::title', () => this._notify())
            : 0;
    }

    _notify() {
        if (!this._subscribers || this._subscribers.size === 0)
            return;
        this._latestNotification = true;
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
        this._latestNotification = null;
        for (const state of this._subscribers.values()) {
            // A slow subscriber gets at most one outstanding method call and
            // one replaceable pending snapshot. Bursts never accumulate a
            // queue of stale window titles in GNOME Shell.
            state.delivery.enqueue(new GLib.Variant('(sss)',
                describe(this._focused, state.includeTitles)));
        }
    }
}
