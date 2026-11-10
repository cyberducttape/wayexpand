// Bundled into the wayexpand-backend-kwin-window binary and written to a
// per-process temp file at runtime with __WAYEXPAND_BUS_NAME__ substituted
// for this daemon's unique D-Bus name (see lib.rs). KWin exposes no
// Wayland protocol for reading the focused window -- this scripting
// interface, reached over the session D-Bus, is the same mechanism tools
// like kdotool use.
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
workspace.windowActivated.connect(report);
report(workspace.activeWindow);
