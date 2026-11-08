import assert from 'node:assert/strict';
import {
    activateSource,
    activeSource,
    inputSourceManager,
    keyboardLayoutSettings,
} from '../gnome-extension/wayexpand-window-tracker@cyberducttape.github.io/keyboard-compat.js';

assert.equal(inputSourceManager({}), null);
assert.equal(activeSource({}), null);
assert.equal(keyboardLayoutSettings(null), null);
assert.equal(keyboardLayoutSettings({_settings: {keyboardOptions: 'invalid'}}), null);

let active = {type: 'xkb', id: 'us'};
let reloads = 0;
const manager = {
    get currentSource() { return active; },
    _settings: {keyboardModel: 'pc105', keyboardOptions: ['grp:alt_shift_toggle']},
    reload() { reloads++; },
    inputSources: {
        de: {type: 'xkb', id: 'de', activate() { active = this; }},
    },
};
const keyboard = {getInputSourceManager: () => manager};
assert.equal(activeSource(keyboard).id, 'us');
assert.deepEqual(keyboardLayoutSettings(manager), {
    model: 'pc105', options: ['grp:alt_shift_toggle'],
});
assert.deepEqual(activateSource(keyboard, 'xkb', 'de'), {
    source: manager.inputSources.de,
});
assert.equal(reloads, 0, 'keep a live manager when the target source is already registered');

const refreshableSources = {};
const refreshable = {
    currentSource: active,
    _ibusReady: true,
    _ibusManager: {_ready: true, _currentEngineName: 'wayexpand'},
    reload() { reloads++; },
    inputSources: refreshableSources,
};
const refreshableKeyboard = {getInputSourceManager: () => refreshable};
assert.equal(activateSource(refreshableKeyboard, 'ibus', 'wayexpand').error, 'source-pending');
assert.equal(reloads, 1, 'refresh only once while GNOME asynchronously registers the IBus source');
refreshableSources.ibus = {
    type: 'ibus', id: 'wayexpand', activate() { refreshable.currentSource = this; },
};
assert.deepEqual(activateSource(refreshableKeyboard, 'ibus', 'wayexpand', false), {
    source: refreshableSources.ibus,
});
assert.equal(reloads, 1, 'reload only to discover a source missing from the live manager');
const intermittentSources = {};
let intermittentReloads = 0;
const intermittent = {
    currentSource: active,
    _ibusReady: true,
    _ibusManager: {_ready: true, _currentEngineName: 'wayexpand'},
    reload() {
        intermittentReloads++;
        if (intermittentReloads === 2) {
            intermittentSources.ibus = {
                type: 'ibus', id: 'wayexpand', activate() { intermittent.currentSource = this; },
            };
        }
    },
    inputSources: intermittentSources,
};
const intermittentKeyboard = {getInputSourceManager: () => intermittent};
assert.equal(activateSource(intermittentKeyboard, 'ibus', 'wayexpand', true).error, 'source-pending');
assert.deepEqual(activateSource(intermittentKeyboard, 'ibus', 'wayexpand', true), {
    source: intermittentSources.ibus,
});
assert.equal(intermittentReloads, 2, 'refresh can discover a source registered after a layout change');
assert.equal(activateSource(keyboard, 'xkb', 'missing').error, 'source-pending');

let ibusSource = {type: 'ibus', id: 'wayexpand', activate() { active = this; }};
const ibusManager = {_ready: true, _currentEngineName: 'xkb:us::eng'};
const liveIbusManager = {
    get currentSource() { return active; },
    _ibusReady: true,
    _ibusManager: ibusManager,
    reload() {},
    inputSources: {wayexpand: ibusSource},
};
const liveIbusKeyboard = {getInputSourceManager: () => liveIbusManager};
assert.equal(
    activateSource(liveIbusKeyboard, 'ibus', 'wayexpand', false).error,
    'engine-pending',
    'an optimistic currentSource is not proof that IBus accepted activation',
);
ibusManager._currentEngineName = 'wayexpand';
assert.deepEqual(activateSource(liveIbusKeyboard, 'ibus', 'wayexpand', false), {
    source: ibusSource,
});
ibusManager._ready = false;
assert.equal(
    activateSource(liveIbusKeyboard, 'ibus', 'wayexpand', false).error,
    'ibus-not-ready',
    'a disconnected IBus manager cannot report an active route',
);
ibusManager._ready = true;
liveIbusManager._disableIBus = true;
assert.equal(
    activateSource(liveIbusKeyboard, 'ibus', 'wayexpand', false).error,
    'ibus-disabled-by-content-purpose',
    'GNOME password-field privacy mode must not be mistaken for route failure or success',
);

const unsupported = {getInputSourceManager: () => ({currentSource: active})};
assert.equal(activateSource(unsupported, 'xkb', 'de').error, 'unsupported-manager');
console.log('GNOME keyboard compatibility checks passed');
