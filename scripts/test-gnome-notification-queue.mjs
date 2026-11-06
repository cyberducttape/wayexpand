import assert from 'node:assert/strict';
import test from 'node:test';
import {
    isAuthorizedTrackerClient,
    LatestOnlyDelivery,
    setTitleAccess,
    subscriberSnapshot,
} from '../gnome-extension/wayexpand-window-tracker@cyberducttape.github.io/notification-queue.js';

test('subscription is limited to the current user and WayExpand executables', () => {
    assert.equal(isAuthorizedTrackerClient(1000, 1000, '/usr/bin/wayexpand-daemon'), true);
    assert.equal(isAuthorizedTrackerClient(1000, 1000, '/opt/wayexpand/wayexpand'), true);
    assert.equal(isAuthorizedTrackerClient(1001, 1000, '/usr/bin/wayexpand'), false);
    assert.equal(isAuthorizedTrackerClient(1000, 1000, '/usr/bin/terminal'), false);
    assert.equal(isAuthorizedTrackerClient(1000, 1000, ''), false);
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
    assert.deepEqual(
        subscriberSnapshot('org.example.Editor', 'secret document title', 'window-7'),
        ['org.example.Editor', '', 'window-7'],
    );
    assert.deepEqual(
        subscriberSnapshot('org.example.Editor', 'configured title match', 'window-7', true),
        ['org.example.Editor', 'configured title match', 'window-7'],
    );
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
