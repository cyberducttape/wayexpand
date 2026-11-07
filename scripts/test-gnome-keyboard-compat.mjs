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
assert.equal(reloads, 1);
assert.equal(activateSource(keyboard, 'xkb', 'missing').error, 'source-unavailable');

const unsupported = {getInputSourceManager: () => ({currentSource: active})};
assert.equal(activateSource(unsupported, 'xkb', 'de').error, 'unsupported-manager');
console.log('GNOME keyboard compatibility checks passed');
