// WayExpand window tracker for runtime-gated GNOME Shell releases.
//
// Mutter offers no Wayland protocol or public D-Bus API that tells another
// process which window has focus (org.gnome.Shell.Introspect is limited to
// allowlisted callers, and Eval is disabled). This extension runs inside
// GNOME Shell and reports the focused window to WayExpand, which limits
// snippets to certain applications (app_filter) with it.
//
// Privacy boundary: the session bus is shared by processes running as this
// user, so caller checks are defense in depth, not a boundary against malicious
// same-user software. Every method checks the caller UID and executable.
// Subscribe() returns app IDs and opaque window IDs without titles. The
// trusted WayExpand client may explicitly request titles for configured
// title_contains filters. Focus data is not persisted.
// Input-source methods expose only the active source and delegate selection
// changes to GNOME Shell's own manager.

import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Shell from 'gi://Shell';
import * as Keyboard from 'resource:///org/gnome/shell/ui/status/keyboard.js';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import {
    activeSource,
    activateSource,
    inputSourceManager,
    keyboardLayoutSettings,
} from './keyboard-compat.js';
import {
    CancellableSources,
    AuthorizationAttempt,
    clearTitleWatch,
    isAuthorizedTrackerClient,
    LatestOnlyDelivery,
    setTitleAccess,
    SubscriptionReservations,
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
    <method name="GetActiveKeyboardLayout">
      <arg type="s" direction="out" name="kind"/>
      <arg type="s" direction="out" name="id"/>
      <arg type="s" direction="out" name="xkb_id"/>
      <arg type="s" direction="out" name="model"/>
      <arg type="as" direction="out" name="options"/>
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
    <property name="KeyboardLayoutVersion" type="u" access="read"/>
  </interface>
</node>`;
// The callback every subscriber serves (the same one WayExpand serves for
// its KWin bridge).
const CLIENT_PATH = '/WindowTracker';
const CLIENT_IFACE = 'org.wayexpand.WindowTracker1';
const PROTOCOL_VERSION = 2;
const MAX_SUBSCRIBERS = 8;
const MAX_PENDING_AUTHORIZATIONS = 8;
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
    // Avoid even reading a potentially sensitive document title for the
    // default app-ID-only subscription.
    return subscriberSnapshot(
        appId,
        () => window.get_title() || '',
        String(window.get_id()),
        includeTitle,
    );
}

export default class WayExpandWindowTracker extends Extension {
    enable() {
        this._subscribers = new Map();
        this._pendingSubscriptions = new Map();
        this._subscriptionReservations = new SubscriptionReservations(MAX_SUBSCRIBERS);
        this._pendingAuthorizedCalls = new Map();
        this._authorizationAttempts = new Map();
        this._authorizationReservations = new SubscriptionReservations(MAX_PENDING_AUTHORIZATIONS);
        this._focused = null;
        this._notifySource = 0;
        this._latestNotification = null;
        this._titleWindow = null;
        this._titleChangedId = 0;
        this._activationSources = new CancellableSources(sourceId => GLib.source_remove(sourceId));
        this._activationCalls = new Set();
        this._exported = Gio.DBusExportedObject.wrapJSObject(TRACKER_IFACE, this);
        this._exported.export(Gio.DBus.session, TRACKER_PATH);
        this._focusChangedId = global.display.connect('notify::focus-window',
            () => this._focusChanged());
        this._focusChanged();
    }

    disable() {
        this._activationSources.cancelAll();
        for (const invocation of this._activationCalls) {
            invocation.return_dbus_error(
                'org.wayexpand.ShellWindowTracker1.Error.Unavailable',
                'the GNOME window tracker was disabled during input-source activation',
            );
        }
        this._activationCalls.clear();
        this._clearTitleWatch();
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
        for (const invocation of this._pendingSubscriptions.values()) {
            this._cancelAuthorization(invocation);
            invocation.return_dbus_error(
                'org.wayexpand.ShellWindowTracker1.Error.Unavailable',
                'the GNOME window tracker was disabled during subscription',
            );
        }
        this._pendingSubscriptions.clear();
        this._subscriptionReservations.clear();
        for (const invocation of this._pendingAuthorizedCalls.keys()) {
            this._cancelAuthorization(invocation);
            invocation.return_dbus_error(
                'org.wayexpand.ShellWindowTracker1.Error.Unavailable',
                'the GNOME window tracker was disabled during caller authorization',
            );
        }
        this._pendingAuthorizedCalls.clear();
        for (const attempt of this._authorizationAttempts.values())
            attempt.cancel();
        this._authorizationAttempts.clear();
        this._authorizationReservations.clear();
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

    get KeyboardLayoutVersion() {
        return 2;
    }

    GetActiveInputSourceAsync(_parameters, invocation) {
        this._withAuthorizedClient(invocation, () => this._getActiveInputSource(invocation));
    }

    _getActiveInputSource(invocation) {
        const source = activeSource(Keyboard);
        if (!source) {
            invocation.return_dbus_error(
                'org.wayexpand.ShellWindowTracker1.Error.NoActiveInputSource',
                'GNOME Shell has no active input source',
            );
            return;
        }
        invocation.return_value(new GLib.Variant('(ss)', [source.type, source.id]));
    }

    GetActiveKeyboardLayoutAsync(_parameters, invocation) {
        this._withAuthorizedClient(invocation, () => this._getActiveKeyboardLayout(invocation));
    }

    _getActiveKeyboardLayout(invocation) {
        const manager = inputSourceManager(Keyboard);
        const source = manager?.currentSource;
        if (!source) {
            invocation.return_dbus_error(
                'org.wayexpand.ShellWindowTracker1.Error.NoActiveInputSource',
                'GNOME Shell has no active input source',
            );
            return;
        }
        // GNOME applies currentSource.xkbId to Mutter for both XKB and IBus
        // sources. Return that compositor-selected map, not a saved source
        // index or the IBus engine's display name.
        // Use the exact settings object chosen by GNOME's InputSourceManager.
        // It may be the per-user GSettings source or systemd-localed, depending
        // on the desktop configuration. Reading gsettings directly here would
        // pair the live source with options/model that Mutter may not apply.
        const xkbSettings = keyboardLayoutSettings(manager);
        if (typeof source.xkbId !== 'string' || source.xkbId.length === 0) {
            invocation.return_dbus_error(
                'org.wayexpand.ShellWindowTracker1.Error.InputSourceUnavailable',
                'GNOME Shell did not expose the active source XKB map',
            );
            return;
        }
        if (!xkbSettings) {
            invocation.return_dbus_error(
                'org.wayexpand.ShellWindowTracker1.Error.InputSourceUnavailable',
                'GNOME Shell did not expose its active XKB model and options',
            );
            return;
        }
        invocation.return_value(new GLib.Variant('(ssssas)', [source.type, source.id, source.xkbId, xkbSettings.model, xkbSettings.options]));
    }

    ActivateInputSourceAsync([kind, id], invocation) {
        this._withAuthorizedClient(invocation, () => this._activateInputSource(kind, id, invocation));
    }

    _activateInputSource(kind, id, invocation) {
        if (!['ibus', 'xkb'].includes(kind) || !id || id.length > 256) {
            invocation.return_dbus_error(
                'org.wayexpand.ShellWindowTracker1.Error.InvalidInputSource',
                'invalid GNOME input source',
            );
            return;
        }

        // Use Shell's live source manager, never the deprecated gsettings
        // `current` value. Reload can update the XKB sources synchronously but
        // receive IBus source registrations asynchronously, so wait briefly
        // for the requested source rather than reporting a false unavailable.
        let attempts = 0;
        const sources = this._activationSources;
        const generation = sources.generation;
        this._activationCalls.add(invocation);
        const finish = result => {
            if (!this._activationCalls.delete(invocation))
                return;
            if (!result.error) {
                const active = result.source;
                invocation.return_value(new GLib.Variant('(ss)', [active.type, active.id]));
                return;
            }
            const sourceDetails = result.error === 'source-pending'
                ? `; available sources: ${(result.availableSources ?? []).join(', ') || 'none'}`
                : '';
            const engineDetails = result.error === 'ibus-not-ready'
                ? '; GNOME has no ready IBus connection'
                : result.error === 'engine-pending'
                    ? '; GNOME has not confirmed the requested IBus engine'
                    : result.error === 'ibus-disabled-by-content-purpose'
                        ? '; GNOME disabled IBus for the focused sensitive field'
                        : '';
            invocation.return_dbus_error(
                ['source-pending', 'ibus-not-ready', 'engine-pending'].includes(result.error)
                    ? 'org.wayexpand.ShellWindowTracker1.Error.InputSourceUnavailable'
                    : 'org.wayexpand.ShellWindowTracker1.Error.ActivationFailed',
                `GNOME Shell input-source activation failed (${result.error})${sourceDetails}${engineDetails}`,
            );
        };
        const activate = () => {
            if (!sources.isCurrent(generation))
                return;
            const refreshIfMissing = attempts % 5 === 0;
            const result = activateSource(Keyboard, kind, id, refreshIfMissing);
            const pending = result.error === 'source-pending'
                || result.error === 'activation-unverified'
                || result.error === 'ibus-not-ready'
                || result.error === 'engine-pending';
            if (!pending || attempts >= 80) {
                finish(result);
                return;
            }
            attempts++;
            let sourceId = 0;
            sourceId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 100, () => {
                sources.forget(sourceId);
                if (!sources.isCurrent(generation))
                    return GLib.SOURCE_REMOVE;
                activate();
                return GLib.SOURCE_REMOVE;
            });
            sources.add(sourceId);
        };
        activate();
    }

    SubscribeAsync(_parameters, invocation) {
        this._subscribe(invocation, false);
    }

    SubscribeWithTitlesAsync([includeTitles], invocation) {
        this._subscribe(invocation, includeTitles === true);
    }

    _subscribe(invocation, includeTitles) {
        const subscriber = invocation.get_sender();
        if (!this._subscribers) {
            invocation.return_dbus_error(
                'org.wayexpand.ShellWindowTracker1.Error.Unavailable',
                'the GNOME window tracker is not enabled',
            );
            return;
        }
        const active = this._subscribers.get(subscriber);
        if (active) {
            // Treat every subscription call as an explicit privacy mode
            // update. In particular, Subscribe() revokes future and queued
            // title delivery from a connection that previously opted in.
            setTitleAccess(
                active,
                includeTitles,
                new GLib.Variant('(sss)', describe(this._focused, false)),
            );
            this._watchTitle(this._focused);
            invocation.return_value(new GLib.Variant('(sss)', describe(this._focused, active.includeTitles)));
            return;
        }
        if (this._pendingSubscriptions.has(subscriber)
            || !this._subscriptionReservations.reserve(subscriber, this._subscribers.size)) {
            invocation.return_error_literal(Gio.DBusError, Gio.DBusError.LIMITS_EXCEEDED,
                'too many pending or active WayExpand window-tracker subscribers');
            return;
        }
        this._pendingSubscriptions.set(subscriber, invocation);
        const authorization = this._isAuthorizedClient(subscriber, authorized => {
            this._authorizationAttempts.delete(invocation);
            if (this._pendingSubscriptions.get(subscriber) !== invocation)
                return;
            this._pendingSubscriptions.delete(subscriber);
            this._subscriptionReservations.release(subscriber);
            if (!this._subscribers) {
                invocation.return_dbus_error(
                    'org.wayexpand.ShellWindowTracker1.Error.Unavailable',
                    'the GNOME window tracker was disabled during subscription',
                );
                return;
            }
            if (!authorized) {
                invocation.return_error_literal(Gio.DBusError, Gio.DBusError.ACCESS_DENIED,
                    'window tracker subscriptions are limited to WayExpand executables');
                return;
            }
            this._addSubscriber(subscriber, includeTitles);
            const state = this._subscribers.get(subscriber);
            this._watchTitle(this._focused);
            invocation.return_value(new GLib.Variant('(sss)', describe(this._focused, state.includeTitles)));
        });
        if (authorization.active)
            this._authorizationAttempts.set(invocation, authorization);
    }

    _addSubscriber(subscriber, includeTitles) {
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
    }

    UnsubscribeAsync(_parameters, invocation) {
        const subscriber = invocation.get_sender();
        const pending = this._pendingSubscriptions.get(subscriber);
        if (pending) {
            this._pendingSubscriptions.delete(subscriber);
            this._cancelAuthorization(pending);
            this._subscriptionReservations.release(subscriber);
            pending.return_dbus_error(
                'org.wayexpand.ShellWindowTracker1.Error.Cancelled',
                'the WayExpand client cancelled its pending tracker subscription',
            );
        }
        for (const [pendingInvocation, pendingSender] of this._pendingAuthorizedCalls) {
            if (pendingSender !== subscriber)
                continue;
            this._pendingAuthorizedCalls.delete(pendingInvocation);
            this._cancelAuthorization(pendingInvocation);
            this._authorizationReservations.release(subscriber);
            pendingInvocation.return_dbus_error(
                'org.wayexpand.ShellWindowTracker1.Error.Cancelled',
                'the WayExpand client cancelled its pending tracker request',
            );
        }
        this._drop(subscriber);
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

    _isAuthorizedClient(subscriber, complete) {
        const cancellable = new Gio.Cancellable();
        const attempt = new AuthorizationAttempt(complete, () => cancellable.cancel());
        const finish = authorized => attempt.finish(authorized);
        try {
            Gio.DBus.session.call(DBUS_NAME, DBUS_PATH, DBUS_NAME,
                'GetConnectionUnixUser', new GLib.Variant('(s)', [subscriber]),
                new GLib.VariantType('(u)'), Gio.DBusCallFlags.NONE, 500, cancellable,
                (connection, userResult) => {
                    let uid;
                    try {
                        [uid] = connection.call_finish(userResult).deep_unpack();
                    } catch {
                        // Unknown peer identity must not gain access to focus metadata.
                        this._logAuthorizationDiagnostic('could not resolve peer UID');
                        finish(false);
                        return;
                    }
                    try {
                        Gio.DBus.session.call(DBUS_NAME, DBUS_PATH, DBUS_NAME,
                            'GetConnectionUnixProcessID', new GLib.Variant('(s)', [subscriber]),
                            new GLib.VariantType('(u)'), Gio.DBusCallFlags.NONE, 500, cancellable,
                            (pidConnection, pidResult) => {
                                try {
                                    const [pid] = pidConnection.call_finish(pidResult).deep_unpack();
                                    const executable = GLib.file_read_link(`/proc/${pid}/exe`);
                                    const ownName = Gio.DBus.session.get_unique_name();
                                    Gio.DBus.session.call(DBUS_NAME, DBUS_PATH, DBUS_NAME,
                                        'GetConnectionUnixUser', new GLib.Variant('(s)', [ownName]),
                                        new GLib.VariantType('(u)'), Gio.DBusCallFlags.NONE, 500, cancellable,
                                        (ownConnection, ownResult) => {
                                            try {
                                                const [ownUid] = ownConnection.call_finish(ownResult).deep_unpack();
                                                const authorized = isAuthorizedTrackerClient(
                                                    uid, ownUid, executable, GLib.get_home_dir(),
                                                    GLib.getenv('WAYEXPAND_GNOME_TEST_CLIENTS'),
                                                );
                                                this._logAuthorizationDiagnostic(
                                                    `peer uid=${uid} shell uid=${ownUid} executable=${executable.split('/').pop()} authorized=${authorized}`,
                                                );
                                                finish(authorized);
                                            } catch {
                                                this._logAuthorizationDiagnostic('could not resolve Shell UID');
                                                finish(false);
                                            }
                                        });
                                } catch (error) {
                                    const detail = String(error?.message || error)
                                        .replace(/\/proc\/[0-9]+\/exe/g, '/proc/<pid>/exe');
                                    this._logAuthorizationDiagnostic(
                                        `could not resolve peer executable (${detail})`,
                                    );
                                    finish(false);
                                }
                            });
                    } catch {
                        finish(false);
                    }
                });
        } catch {
            finish(false);
        }
        return attempt;
    }

    _cancelAuthorization(invocation) {
        const attempt = this._authorizationAttempts.get(invocation);
        if (attempt)
            attempt.cancel();
        this._authorizationAttempts.delete(invocation);
    }

    _logAuthorizationDiagnostic(message) {
        // Opt-in only for the isolated CI session. Never log request text,
        // snippet contents, or full executable paths.
        if (GLib.getenv('WAYEXPAND_GNOME_AUTH_DIAGNOSTICS') === '1')
            log(`WayExpand authorization diagnostic: ${message}`);
    }

    _withAuthorizedClient(invocation, operation) {
        const sender = invocation.get_sender();
        if (!this._authorizationReservations.reserve(sender, 0)) {
            invocation.return_error_literal(Gio.DBusError, Gio.DBusError.LIMITS_EXCEEDED,
                'too many pending GNOME tracker authorizations');
            return;
        }
        this._pendingAuthorizedCalls.set(invocation, sender);
        const authorization = this._isAuthorizedClient(sender, authorized => {
            this._authorizationAttempts.delete(invocation);
            if (!this._pendingAuthorizedCalls.has(invocation))
                return;
            this._pendingAuthorizedCalls.delete(invocation);
            this._authorizationReservations.release(sender);
            if (!this._subscribers) {
                invocation.return_dbus_error(
                    'org.wayexpand.ShellWindowTracker1.Error.Unavailable',
                    'the GNOME window tracker was disabled during caller authorization',
                );
                return;
            }
            if (!authorized) {
                invocation.return_error_literal(Gio.DBusError, Gio.DBusError.ACCESS_DENIED,
                    'this method is limited to WayExpand executables');
                return;
            }
            operation();
        });
        if (authorization.active)
            this._authorizationAttempts.set(invocation, authorization);
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
        this._clearTitleWatch();
        this._titleWindow = target;
        this._titleChangedId = target
            ? target.connect('notify::title', () => this._notify())
            : 0;
    }

    _clearTitleWatch() {
        const cleared = clearTitleWatch(this._titleWindow, this._titleChangedId);
        this._titleWindow = cleared.window;
        this._titleChangedId = cleared.signalId;
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
