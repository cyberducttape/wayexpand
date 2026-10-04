// Bundled into the wayexpand-backend-kwin-window binary and written to a
// per-process temp file at runtime with __WAYEXPAND_BUS_NAME__ substituted
// for this daemon's unique D-Bus name (see lib.rs). KWin exposes no
// Wayland protocol for reading the focused window -- this scripting
// interface, reached over the session D-Bus, is the same mechanism tools
// like kdotool use.
function report(window) {
    var appId = window ? window.resourceClass : "";
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
