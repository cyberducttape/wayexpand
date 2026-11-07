import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import test from 'node:test';
import {
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

test('subscription is limited to the current user and WayExpand executables', () => {
    assert.equal(isAuthorizedTrackerClient(1000, 1000, '/usr/bin/wayexpand-daemon'), true);
    // GJS exposes get_user_id() as a string; D-Bus uint32 unpacks as number.
    assert.equal(isAuthorizedTrackerClient(1000, '1000', '/usr/bin/wayexpand-daemon'), true);
    assert.equal(isAuthorizedTrackerClient('1000', 1000, '/usr/bin/wayexpand-daemon'), true);
    assert.equal(isAuthorizedTrackerClient(1000, 1000, '/opt/wayexpand/wayexpand'), true);
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

test('GNOME Shell peer authorization uses asynchronous D-Bus calls', () => {
    assert.match(extensionSource, /GetConnectionUnixUser/);
    assert.match(extensionSource, /GetConnectionUnixProcessID/);
    assert.doesNotMatch(extensionSource, /\.call_sync\s*\(/);
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
