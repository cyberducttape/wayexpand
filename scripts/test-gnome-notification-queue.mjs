import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import test from 'node:test';
import {
    AuthorizationAttempt,
    CancellableSources,
    clearTitleWatch,
    activeXkbSettings,
    isAuthorizedTrackerClient,
    LatestOnlyDelivery,
    SubscriptionReservations,
    setTitleAccess,
    subscriberSnapshot,
} from '../gnome-extension/wayexpand-window-tracker@cyberducttape.github.io/notification-queue.js';

const extensionSource = readFileSync(
    new URL('../gnome-extension/wayexpand-window-tracker@cyberducttape.github.io/extension.js', import.meta.url),
    'utf8',
);
const authorizationSource = readFileSync(
    new URL('../gnome-extension/wayexpand-window-tracker@cyberducttape.github.io/notification-queue.js', import.meta.url),
    'utf8',
);

test('subscription requires the current user and an installed executable path', () => {
    assert.equal(isAuthorizedTrackerClient(1000, 1000, '/usr/bin/wayexpand-daemon'), true);
    assert.equal(isAuthorizedTrackerClient(1000, 1000, '/usr/libexec/wayexpand/wayexpand-ibus'), true);
    // GJS exposes get_user_id() as a string; D-Bus uint32 unpacks as number.
    assert.equal(isAuthorizedTrackerClient(1000, '1000', '/usr/bin/wayexpand-daemon'), true);
    assert.equal(isAuthorizedTrackerClient('1000', 1000, '/usr/bin/wayexpand-daemon'), true);
    assert.equal(isAuthorizedTrackerClient(1000, 1000,
        '/home/alice/.local/lib/wayexpand/1.2.3-abc123/bin/wayexpand', '/home/alice'), true);
    assert.equal(isAuthorizedTrackerClient(1000, 1000, '/tmp/wayexpand'), false);
    assert.equal(isAuthorizedTrackerClient(1000, 1000, '/tmp/wx-e2e-gnome-tracker'), false);
    assert.doesNotMatch(authorizationSource, /wx-e2e-gnome-tracker/);
    assert.equal(isAuthorizedTrackerClient(1000, 1000, '/opt/wayexpand/wayexpand'), false);
    assert.equal(isAuthorizedTrackerClient(1000, 1000,
        '/home/alice/.local/bin/wayexpand', '/home/alice'), false);
    assert.equal(isAuthorizedTrackerClient(1000, 1000,
        '/home/alice/.local/lib/wayexpand/../other/bin/wayexpand', '/home/alice'), false);
    const testClients = '/tmp/wayexpand:/tmp/wayexpand-ibus:/tmp/wx-e2e-gnome-tracker';
    assert.equal(isAuthorizedTrackerClient(1000, 1000, '/tmp/wx-e2e-gnome-tracker', null, testClients), true);
    assert.equal(isAuthorizedTrackerClient(1000, 1000, '/tmp/wayexpand-ibus', null, testClients), true);
    assert.equal(isAuthorizedTrackerClient(1000, 1000, '/tmp/other', null, testClients), false);
    assert.equal(isAuthorizedTrackerClient(1001, 1000, '/usr/bin/wayexpand'), false);
    assert.equal(isAuthorizedTrackerClient('01000', 1000, '/usr/bin/wayexpand-daemon'), false);
    assert.equal(isAuthorizedTrackerClient('unknown', 1000, '/usr/bin/wayexpand-daemon'), false);
    assert.equal(isAuthorizedTrackerClient(1000, 1000, '/usr/bin/terminal'), false);
    assert.equal(isAuthorizedTrackerClient(1000, 1000, ''), false);
});

test('XKB settings match GNOME source and Mutter defaults', () => {
    assert.deepEqual(activeXkbSettings({keyboardOptions: ['grp:alt_shift_toggle']}), {
        model: 'pc105',
        options: ['grp:alt_shift_toggle'],
    });
    assert.deepEqual(activeXkbSettings({
        keyboardModel: 'jp106',
        keyboardOptions: ['compose:ralt'],
    }), {
        model: 'jp106',
        options: ['compose:ralt'],
    });
    assert.equal(activeXkbSettings({keyboardModel: 'pc105', keyboardOptions: 'invalid'}), null);
});

test('pending authorization reservations are deduplicated, bounded, and released', () => {
    const reservations = new SubscriptionReservations(2);
    assert.equal(reservations.reserve(':1.1', 0), true);
    assert.equal(reservations.reserve(':1.1', 0), false);
    assert.equal(reservations.reserve(':1.2', 0), true);
    assert.equal(reservations.reserve(':1.3', 0), false);
    assert.equal(reservations.size, 2);
    assert.equal(reservations.release(':1.1'), true);
    assert.equal(reservations.reserve(':1.3', 0), true);
    assert.equal(reservations.reserve(':1.4', 1), false);
    assert.equal(reservations.size, 2);
    reservations.clear();
    assert.equal(reservations.size, 0);
});

test('slow D-Bus authorization replies remain pending and settle only once', () => {
    const results = [];
    const attempt = new AuthorizationAttempt(value => results.push(value));
    assert.equal(attempt.active, true);
    assert.deepEqual(results, [], 'the caller remains pending until the async reply arrives');
    assert.equal(attempt.finish(true), true);
    assert.equal(attempt.finish(false), false, 'duplicate/late replies are ignored');
    assert.deepEqual(results, [true]);
});

test('a disappearing D-Bus client fails authorization closed', () => {
    const results = [];
    const attempt = new AuthorizationAttempt(value => results.push(value));
    // GetConnectionUnixUser/GetConnectionUnixProcessID fail when a unique
    // name disappears during lookup; the extension resolves that path false.
    assert.equal(attempt.finish(false), true);
    assert.deepEqual(results, [false]);
});

test('Shell shutdown cancels authorization and ignores a late successful reply', () => {
    const results = [];
    let cancelled = 0;
    const attempt = new AuthorizationAttempt(value => results.push(value), () => cancelled++);
    assert.equal(attempt.cancel(), true);
    assert.equal(attempt.finish(true), false);
    assert.equal(cancelled, 1, 'pending D-Bus calls are cancelled with the authorization attempt');
    assert.deepEqual(results, []);
    assert.match(extensionSource, /for \(const attempt of this\._authorizationAttempts\.values\(\)\)[\s\S]*?attempt\.cancel\(\)/);
});

test('unsubscribing and resubscribing the same peer cannot revive the old reply', () => {
    const reservations = new SubscriptionReservations(2);
    const oldResults = [];
    const newResults = [];
    assert.equal(reservations.reserve(':1.8', 0), true);
    const oldAttempt = new AuthorizationAttempt(value => oldResults.push(value));
    oldAttempt.cancel(); // Unsubscribe releases the sender reservation.
    reservations.release(':1.8');
    assert.equal(reservations.reserve(':1.8', 0), true);
    const newAttempt = new AuthorizationAttempt(value => newResults.push(value));
    assert.equal(newAttempt.finish(true), true);
    assert.equal(oldAttempt.finish(true), false);
    assert.deepEqual(oldResults, []);
    assert.deepEqual(newResults, [true]);
    assert.match(extensionSource, /this\._cancelAuthorization\(pending\)/);
});

test('GNOME Shell peer authorization uses asynchronous D-Bus calls', () => {
    assert.match(extensionSource, /GetConnectionUnixUser/);
    assert.match(extensionSource, /GetConnectionUnixProcessID/);
    assert.doesNotMatch(extensionSource, /\.call_sync\s*\(/);
    assert.equal((extensionSource.match(/Gio\.DBusCallFlags\.NONE, 500, cancellable/g) || []).length, 3,
        'each identity lookup has a finite timeout and shares the cancellable lifecycle');
});

test('every metadata and input-source control method authenticates its caller', () => {
    for (const handler of [
        'GetActiveInputSourceAsync(_parameters, invocation)',
        'GetActiveKeyboardLayoutAsync(_parameters, invocation)',
        'ActivateInputSourceAsync([kind, id], invocation)',
    ]) {
        const start = extensionSource.indexOf(handler);
        assert.notEqual(start, -1, `${handler} must exist`);
        const body = extensionSource.slice(start, start + 240);
        assert.match(body, /this\._withAuthorizedClient\(invocation/,
            `${handler} must authorize before reading or changing input-source state`);
    }
    assert.match(extensionSource,
        /invocation\.return_error_literal\(Gio\.DBusError, Gio\.DBusError\.ACCESS_DENIED/);
    assert.match(extensionSource, /MAX_PENDING_AUTHORIZATIONS = 8/);
    assert.match(extensionSource, /this\._authorizationReservations\.reserve\(sender, 0\)/);
    assert.match(extensionSource, /this\._pendingAuthorizedCalls\.clear\(\)/);
    assert.match(extensionSource, /this\._authorizationAttempts\.set\(invocation, authorization\)/);
    assert.match(extensionSource, /_cancelAuthorization\(invocation\)/);
});

test('title access is opt-in per request and can be revoked on an active subscription', () => {
    const subscriber = {includeTitles: false};
    assert.equal(setTitleAccess(subscriber, true), true);
    assert.equal(subscriber.includeTitles, true);
    assert.equal(setTitleAccess(subscriber, false), false);
    assert.equal(subscriber.includeTitles, false);
    assert.equal(setTitleAccess(subscriber, 'true'), false);
});

test('revoking title access replaces a queued title-bearing notification', () => {
    let finish;
    const sent = [];
    const delivery = new LatestOnlyDelivery((value, done) => {
        sent.push(value);
        finish = done;
    }, () => assert.fail('healthy test subscriber was dropped'));
    const subscriber = {includeTitles: true, delivery};

    delivery.enqueue(['org.example.Editor', 'first title', 'window-1']);
    delivery.enqueue(['org.example.Editor', 'queued secret title', 'window-1']);
    setTitleAccess(subscriber, false, ['org.example.Editor', '', 'window-1']);
    finish(null);

    assert.deepEqual(sent, [
        ['org.example.Editor', 'first title', 'window-1'],
        ['org.example.Editor', '', 'window-1'],
    ]);
});

test('title-free snapshots exclude titles while preserving app and instance identity', () => {
    let titleReads = 0;
    const readTitle = () => {
        titleReads++;
        return 'secret document title';
    };
    assert.deepEqual(
        subscriberSnapshot('org.example.Editor', readTitle, 'window-7'),
        ['org.example.Editor', '', 'window-7'],
    );
    assert.equal(titleReads, 0, 'default subscriptions must not read the window title');
    assert.deepEqual(
        subscriberSnapshot('org.example.Editor', readTitle, 'window-7', true),
        ['org.example.Editor', 'secret document title', 'window-7'],
    );
    assert.equal(titleReads, 1, 'title filters read the title only when explicitly requested');
});

test('slow subscribers retain only the latest update and one in-flight call', () => {
    let complete;
    const sent = [];
    const delivery = new LatestOnlyDelivery((value, done) => {
        sent.push(value);
        complete = done;
    }, () => assert.fail('healthy test subscriber was dropped'));

    delivery.enqueue('initial');
    for (let i = 0; i < 100_000; i++)
        delivery.enqueue(`title-${i}`);

    assert.deepEqual(sent, ['initial']);
    complete(null);
    assert.deepEqual(sent, ['initial', 'title-99999']);
});

test('disabling a title-bearing subscription disconnects its window signal', () => {
    const handlers = new Map();
    let nextId = 1;
    let notifications = 0;
    const window = {
        connect(signal, callback) {
            assert.equal(signal, 'notify::title');
            const id = nextId++;
            handlers.set(id, callback);
            return id;
        },
        disconnect(id) {
            handlers.delete(id);
        },
    };
    const signalId = window.connect('notify::title', () => notifications++);
    assert.equal(handlers.size, 1, 'title opt-in established a live signal');

    const cleared = clearTitleWatch(window, signalId);
    assert.equal(cleared.window, null);
    assert.equal(cleared.signalId, 0);
    assert.equal(handlers.size, 0, 'disable disconnects the title signal');
    for (const callback of handlers.values())
        callback();
    assert.equal(notifications, 0);

    const disable = extensionSource.slice(
        extensionSource.indexOf('    disable() {'),
        extensionSource.indexOf('    get Version()'),
    );
    assert.match(disable, /this\._clearTitleWatch\(\)/,
        'extension disable must clear the subscribed title window');
    assert.match(extensionSource, /_clearTitleWatch\(\)\s*\{\s*const cleared = clearTitleWatch/);
});

test('disabling cancels pending input-source activation retries and invalidates callbacks', () => {
    const removed = [];
    const sources = new CancellableSources(id => removed.push(id));
    const generation = sources.generation;
    sources.add(12);
    sources.add(13);
    assert.equal(sources.isCurrent(generation), true);

    sources.cancelAll();
    assert.deepEqual(removed, [12, 13]);
    assert.equal(sources.isCurrent(generation), false);

    const disable = extensionSource.slice(
        extensionSource.indexOf('    disable() {'),
        extensionSource.indexOf('    get Version()'),
    );
    assert.match(disable, /this\._activationSources\.cancelAll\(\)/);
    assert.match(disable, /this\._activationCalls\.clear\(\)/);
    const activation = extensionSource.slice(
        extensionSource.indexOf('    _activateInputSource('),
        extensionSource.indexOf('    SubscribeAsync('),
    );
    assert.match(activation, /sources\.isCurrent\(generation\)/);
    assert.match(activation, /sources\.forget\(sourceId\)/);
    assert.match(activation, /sources\.add\(sourceId\)/);
});

test('a stalled subscriber cannot grow the outstanding queue', () => {
    let outstanding = 0;
    let maximumOutstanding = 0;
    const sent = [];
    const delivery = new LatestOnlyDelivery((value, _done) => {
        outstanding++;
        maximumOutstanding = Math.max(maximumOutstanding, outstanding);
        sent.push(value);
        // Deliberately never complete: represents a nonresponsive D-Bus peer.
    }, () => assert.fail('stalled peer should remain bounded until call timeout'));

    for (let i = 0; i < 100_000; i++)
        delivery.enqueue(i);

    assert.equal(maximumOutstanding, 1);
    assert.deepEqual(sent, [0]);
    delivery.close();
    delivery.enqueue('after-close');
    assert.deepEqual(sent, [0]);
});

test('failed calls close the subscriber and discard pending stale state', () => {
    let complete;
    let failures = 0;
    const sent = [];
    const delivery = new LatestOnlyDelivery((value, done) => {
        sent.push(value);
        complete = done;
    }, () => failures++);

    delivery.enqueue('first');
    delivery.enqueue('stale');
    complete(new Error('call timed out'));
    delivery.enqueue('later');

    assert.deepEqual(sent, ['first']);
    assert.equal(failures, 1);
});
