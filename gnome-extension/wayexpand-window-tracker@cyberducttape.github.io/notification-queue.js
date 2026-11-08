// Bounded latest-only delivery for window tracker notifications. Kept free of
// GNOME imports so the queue behavior can be stress-tested with Node.js.
const WAYEXPAND_CLIENTS = new Set([
    'wayexpand',
    'wayexpand-daemon',
    // IBus owns a separate engine process that needs authenticated GNOME
    // input-source and keyboard-layout state for its focused input context.
    'wayexpand-ibus',
    // This binary is built only for the isolated headless GNOME E2E test.
    'wx-e2e-gnome-tracker',
]);

/** Session-bus peer check; same-UID processes are still one trust domain. */
export function isAuthorizedTrackerClient(peerUid, currentUid, executablePath) {
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
    const executable = executablePath.slice(executablePath.lastIndexOf('/') + 1)
        .replace(/ \(deleted\)$/, '');
    return WAYEXPAND_CLIENTS.has(executable);
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
