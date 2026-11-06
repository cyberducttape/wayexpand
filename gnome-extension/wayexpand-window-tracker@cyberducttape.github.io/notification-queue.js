// Bounded latest-only delivery for window tracker notifications. Kept free of
// GNOME imports so the queue behavior can be stress-tested with Node.js.
const WAYEXPAND_CLIENTS = new Set([
    'wayexpand',
    'wayexpand-daemon',
    // This binary is built only for the isolated headless GNOME E2E test.
    'wx-e2e-gnome-tracker',
]);

/** Session-bus peer check; same-UID processes are still one trust domain. */
export function isAuthorizedTrackerClient(peerUid, currentUid, executablePath) {
    if (peerUid !== currentUid || typeof executablePath !== 'string')
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

/** Shape a per-subscriber snapshot without leaking titles by default. */
export function subscriberSnapshot(appId, title, instanceId, includeTitles = false) {
    return [appId, includeTitles ? title : '', instanceId];
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
