/**
 * Narrow compatibility boundary for GNOME Shell's input-source manager.
 * GNOME does not provide a public extension API for reading or activating
 * the session's active keyboard source. Every internal shape is feature
 * detected here and unsupported shapes fail closed.
 */
export function inputSourceManager(keyboard) {
    if (!keyboard || typeof keyboard.getInputSourceManager !== 'function')
        return null;
    try {
        const manager = keyboard.getInputSourceManager();
        if (!manager || typeof manager !== 'object')
            return null;
        if (!manager.currentSource || typeof manager.currentSource !== 'object')
            return null;
        return manager;
    } catch (_error) {
        return null;
    }
}

export function activeSource(keyboard) {
    const manager = inputSourceManager(keyboard);
    return manager ? manager.currentSource : null;
}

export function keyboardLayoutSettings(manager) {
    // `_settings` is private in GNOME Shell. Keep this access isolated and
    // require the exact fields Mutter's current manager exposes; never guess
    // layout options when the runtime changes its shape.
    const settings = manager?._settings;
    if (!settings || typeof settings !== 'object')
        return null;
    let model = settings.keyboardModel;
    if (typeof model !== 'string' || model.length === 0)
        model = 'pc105';
    const options = settings.keyboardOptions;
    if (!Array.isArray(options) || options.some(option => typeof option !== 'string'))
        return null;
    return {model, options};
}

export function activateSource(keyboard, kind, id) {
    const manager = inputSourceManager(keyboard);
    if (!manager || typeof manager.reload !== 'function' ||
        !manager.inputSources || typeof manager.inputSources !== 'object')
        return {error: 'unsupported-manager'};
    try {
        let source = Object.values(manager.inputSources).find(candidate =>
            candidate && candidate.type === kind && candidate.id === id);
        // Reloading an already complete manager can cancel GNOME's pending
        // IBus engine switch, especially immediately after returning from an
        // XKB layout. Preserve the live source objects and refresh only when
        // the requested source is absent (for example after registration).
        if (!source) {
            manager.reload();
            source = Object.values(manager.inputSources).find(candidate =>
                candidate && candidate.type === kind && candidate.id === id);
        }
        if (!source || typeof source.activate !== 'function')
            return {error: 'source-unavailable'};
        source.activate(true);
        const active = manager.currentSource;
        if (!active || active.type !== kind || active.id !== id)
            return {error: 'activation-unverified'};
        return {source: active};
    } catch (_error) {
        return {error: 'activation-failed'};
    }
}
