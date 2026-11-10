// Bundled into the wayexpand-backend-kwin-window binary and written to a
// per-process temp file at runtime with __WAYEXPAND_BUS_NAME__ substituted
// for this daemon's unique D-Bus name (see lib.rs). KWin exposes no
// Wayland protocol for reading the focused window -- this scripting
// interface, reached over the session D-Bus, is the same mechanism tools
// like kdotool use.
var lastReportKey = null;

function report(window) {
    var appId = "";
    if (window) {
        appId = window.resourceClass || "";
        // Native Wayland windows may not expose an X11 resource class. KWin
        // provides the owning desktop-file identity separately; use it only
        // when resourceClass is absent so existing X11 IDs remain stable.
        if (!appId) {
            appId = window.desktopFileName || "";
            var slash = appId.lastIndexOf("/");
            if (slash >= 0) {
                appId = appId.substring(slash + 1);
            }
            if (appId.toLowerCase().slice(-8) === ".desktop") {
                appId = appId.slice(0, -8);
            }
        }
        // Some XWayland clients expose an empty resourceClass and no desktop
        // file association, while still providing the other half of WM_CLASS.
        // Use that stable window-manager identity as a last resort; never use
        // the editable title as an application identity.
        if (!appId) {
            appId = window.resourceName || "";
        }
    }
    var title = window ? window.caption : "";
    // internalId is a compositor-issued identity for this exact KWin window.
    // It is intentionally kept separate from appId/title, which are not unique.
    var instanceId = window && window.internalId ? String(window.internalId) : "";
    // KWin may deliver the same transition through both workspace activation
    // and a window's activeChanged signal. Avoid duplicate D-Bus traffic.
    var reportKey = appId + "\n" + title + "\n" + instanceId;
    if (instanceId && reportKey === lastReportKey) {
        return;
    }
    lastReportKey = reportKey;
    callDBus(
        "__WAYEXPAND_BUS_NAME__",
        "/WindowTracker",
        "org.wayexpand.WindowTracker1",
        "WindowChanged",
        appId,
        title,
        instanceId
    );
}

// Subscribe to each KWin window's own activation state as a second focus
// signal path. Some KWin/client combinations do not reliably deliver the
// workspace-level activation signal to scripts after startup. The active
// property is compositor-owned and does not infer focus from window titles
// or keyboard activity.
var watchedWindows = {};
function watchWindow(window) {
    if (!window || !window.internalId || !window.activeChanged) {
        return;
    }
    var id = String(window.internalId);
    if (watchedWindows[id]) {
        return;
    }
    watchedWindows[id] = true;
    window.activeChanged.connect(function () {
        if (watchedWindows[id] && window.active) {
            report(window);
        }
    });
    // A window can be added after this script starts already active. In that
    // case KWin need not emit a later activeChanged signal, so reporting only
    // from the signal handler would leave the tracker without a focus snapshot.
    // The active property is compositor-owned; never infer focus from stacking
    // order or window metadata.
    if (window.active === true) {
        report(window);
    }
}

function forgetWindow(window) {
    if (window && window.internalId) {
        delete watchedWindows[String(window.internalId)];
    }
}

function currentActiveWindow() {
    if (workspace.activeWindow) {
        return workspace.activeWindow;
    }

    // During KWin startup, activeWindow can briefly be null even though the
    // stacking snapshot already contains the active client. Recover only if
    // exactly one window explicitly reports active; never guess from order,
    // title, or the most recently created window.
    var activeWindow = null;
    var windows = workspace.stackingOrder || [];
    for (var index = 0; index < windows.length; index++) {
        var window = windows[index];
        if (window && window.active === true) {
            if (activeWindow) {
                return null;
            }
            activeWindow = window;
        }
    }
    return activeWindow;
}

if (workspace.stackingOrder) {
    for (var index = 0; index < workspace.stackingOrder.length; index++) {
        watchWindow(workspace.stackingOrder[index]);
    }
}
if (workspace.windowAdded) {
    workspace.windowAdded.connect(watchWindow);
}
if (workspace.windowRemoved) {
    workspace.windowRemoved.connect(forgetWindow);
}
workspace.windowActivated.connect(function (window) {
    if (window) {
        watchWindow(window);
    }
    // If activation's argument is temporarily null while KWin updates its
    // activeWindow property, use the current compositor snapshot. A true
    // no-active-window state is still published as empty context.
    report(window || currentActiveWindow());
});
report(currentActiveWindow());
