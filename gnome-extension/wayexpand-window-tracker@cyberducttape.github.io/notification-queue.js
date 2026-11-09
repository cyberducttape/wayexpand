// Bounded latest-only delivery for window tracker notifications. Kept free of
// GNOME imports so the queue behavior can be stress-tested with Node.js.
const SYSTEM_WAYEXPAND_CLIENTS = new Set([
    '/usr/bin/wayexpand', '/usr/bin/wayexpand-daemon', '/usr/bin/wayexpand-ibus',
    '/usr/local/bin/wayexpand', '/usr/local/bin/wayexpand-daemon',
    '/usr/local/bin/wayexpand-ibus',
    '/usr/libexec/wayexpand/wayexpand-ibus',
    '/usr/local/libexec/wayexpand/wayexpand-ibus',
]);

/**
 * Session-bus peer check; same-UID processes remain one trust domain. Match
 * the resolved executable path, not its basename: a program named `wayexpand`
 * in /tmp is not an installed WayExpand client. User installs are accepted
 * only from the installer's versioned payload directory. This narrows the
 * accepted identity but does not isolate malicious processes of the same UID.
 */
export function isAuthorizedTrackerClient(peerUid, currentUid, executablePath, homeDirectory = null,
    isolatedTestClientPaths = null) {
    // GetConnectionUnixUser() supplies both peer and Shell UIDs. Compare
    // canonical numeric identities; never fall back to username, process
    // arguments, or executable basename without the UID check.
    const normalizeUid = uid => {
        if (typeof uid === 'number' && Number.isSafeInteger(uid) && uid >= 0)
            return String(uid);
        if (typeof uid === 'string' && /^(0|[1-9][0-9]*)$/.test(uid))
            return uid;
        return null;
    };
    const peer = normalizeUid(peerUid);
    if (peer === null || peer !== normalizeUid(currentUid) || typeof executablePath !== 'string')
        return false;
    const executable = executablePath.replace(/ \(deleted\)$/, '');
    if (SYSTEM_WAYEXPAND_CLIENTS.has(executable))
        return true;
    if (typeof homeDirectory === 'string' && homeDirectory.startsWith('/')
        && executable.startsWith(`${homeDirectory}/.local/lib/wayexpand/`)) {
        const relative = executable.slice(`${homeDirectory}/.local/lib/wayexpand/`.length);
        return /^[^/]+\/bin\/(wayexpand|wayexpand-daemon|wayexpand-ibus)$/.test(relative);
    }
    // Only the isolated CI session sets this exact-path exception. No test
    // executable name or basename is trusted by normal desktop sessions.
    return typeof isolatedTestClientPaths === 'string'
        && isolatedTestClientPaths.split(':').some(path =>
            path.startsWith('/') && executable === path);
}

/** Keep title access scoped to the caller's current request and clear any
 * queued title-bearing update when an active subscription opts out. */
export function setTitleAccess(state, requested, titleFreeSnapshot = null) {
    const wasEnabled = state.includeTitles;
    state.includeTitles = requested === true;
    if (wasEnabled && !state.includeTitles && titleFreeSnapshot !== null)
        state.delivery.enqueue(titleFreeSnapshot);
    return state.includeTitles;
}

/** Shape a per-subscriber snapshot without reading titles unless requested. */
export function subscriberSnapshot(appId, readTitle, instanceId, includeTitles = false) {
    const title = includeTitles && typeof readTitle === 'function' ? readTitle() : '';
    return [appId, title, instanceId];
}

/** Return the XKB model/options GNOME's InputSourceManager applies. GNOME 45
 * has no session keyboard-model setting; its Mutter keymap uses pc105. */
export function activeXkbSettings(settings) {
    if (!settings)
        return null;
    let model = settings.keyboardModel;
    if (typeof model !== 'string' || model.length === 0)
        model = 'pc105';
    const options = settings.keyboardOptions;
    if (!Array.isArray(options) || options.some(option => typeof option !== 'string'))
        return null;
    return {model, options};
}

/** Bound authorization requests while allowing active subscribers to reconfigure. */
export class SubscriptionReservations {
    constructor(limit) {
        this._limit = limit;
        this._pending = new Set();
    }

    reserve(sender, activeCount) {
        if (this._pending.has(sender) || activeCount + this._pending.size >= this._limit)
            return false;
        this._pending.add(sender);
        return true;
    }

    release(sender) {
        return this._pending.delete(sender);
    }

    clear() {
        this._pending.clear();
    }

    get size() {
        return this._pending.size;
    }
}

/** Own GLib source IDs for work that must not outlive an extension instance. */
export class CancellableSources {
    constructor(removeSource) {
        this._removeSource = removeSource;
        this._sources = new Set();
        this._generation = 0;
    }

    get generation() {
        return this._generation;
    }

    isCurrent(generation) {
        return generation === this._generation;
    }

    add(sourceId) {
        if (Number.isInteger(sourceId) && sourceId > 0)
            this._sources.add(sourceId);
    }

    forget(sourceId) {
        this._sources.delete(sourceId);
    }

    cancelAll() {
        this._generation++;
        for (const sourceId of this._sources) {
            try {
                this._removeSource(sourceId);
            } catch {
                // The source may already have expired.
            }
        }
        this._sources.clear();
    }
}

/** Disconnect a focused-window title signal and forget its owner. */
export function clearTitleWatch(window, signalId) {
    if (window && signalId) {
        try {
            window.disconnect(signalId);
        } catch {
            // The window may already have been destroyed with its handlers.
        }
    }
    return {window: null, signalId: 0};
}

export class LatestOnlyDelivery {
    constructor(send, onFailure) {
        this._send = send;
        this._onFailure = onFailure;
        this._inFlight = false;
        this._pending = null;
        this._closed = false;
    }

    enqueue(value) {
        if (this._closed)
            return;
        this._pending = value;
        this._flush();
    }

    close() {
        this._closed = true;
        this._pending = null;
    }

    _flush() {
        if (this._closed || this._inFlight || this._pending === null)
            return;
        const value = this._pending;
        this._pending = null;
        this._inFlight = true;
        let completed = false;
        const complete = error => {
            if (completed || this._closed)
                return;
            completed = true;
            if (error) {
                this.close();
                this._onFailure(error);
                return;
            }
            this._inFlight = false;
            this._flush();
        };
        try {
            this._send(value, complete);
        } catch (error) {
            complete(error);
        }
    }
}
