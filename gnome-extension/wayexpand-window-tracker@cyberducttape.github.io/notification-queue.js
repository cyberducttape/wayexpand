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
