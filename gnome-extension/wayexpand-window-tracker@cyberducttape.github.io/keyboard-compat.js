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

export function activateSource(keyboard, kind, id, refreshIfMissing = true) {
    const manager = inputSourceManager(keyboard);
    if (!manager || typeof manager.reload !== 'function' ||
        !manager.inputSources || typeof manager.inputSources !== 'object')
        return {error: 'unsupported-manager'};
    try {
        // Shell updates currentSource optimistically. For IBus sources that
        // does not prove the IBus daemon accepted the engine switch: the
        // manager can report WayExpand as current while its IBus connection
        // is still unavailable. Keep these version-sensitive fields inside
        // this compatibility boundary and fail closed when their state is
        // unknown.
        const ibusManager = kind === 'ibus' ? manager._ibusManager : null;
        if (kind === 'ibus') {
            if (manager._disableIBus)
                return {error: 'ibus-disabled-by-content-purpose'};
            if (manager._ibusReady !== true || !ibusManager ||
                ibusManager._ready !== true)
                return {error: 'ibus-not-ready'};
        }

        let source = Object.values(manager.inputSources).find(candidate =>
            candidate && candidate.type === kind && candidate.id === id);
        // Reloading an already complete manager can cancel GNOME's pending
        // IBus engine switch, especially immediately after returning from an
        // XKB layout. Preserve the live source objects and refresh only when
        // the requested source is absent (for example after registration).
        if (!source) {
            if (refreshIfMissing) {
                manager.reload();
                source = Object.values(manager.inputSources).find(candidate =>
                    candidate && candidate.type === kind && candidate.id === id);
            }
            if (!source) {
                const availableSources = Object.values(manager.inputSources)
                    .filter(candidate => candidate && typeof candidate.type === 'string' &&
                        typeof candidate.id === 'string')
                    .map(candidate => `${candidate.type}:${candidate.id}`)
                    .sort();
                return {error: 'source-pending', availableSources};
            }
        }
        if (!source || typeof source.activate !== 'function')
            return {error: 'source-unavailable'};
        const current = manager.currentSource;
        if (!current || current.type !== kind || current.id !== id ||
            (kind === 'ibus' && ibusManager._currentEngineName !== id))
            source.activate(true);
        const active = manager.currentSource;
        if (!active || active.type !== kind || active.id !== id)
            return {error: 'activation-unverified'};
        if (kind === 'ibus' && ibusManager._currentEngineName !== id)
            return {error: 'engine-pending'};
        return {source: active};
    } catch (_error) {
        return {error: 'activation-failed'};
    }
}
