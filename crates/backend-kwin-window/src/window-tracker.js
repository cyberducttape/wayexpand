// Bundled into the wayexpand-backend-kwin-window binary and written to a
// per-process runtime file at runtime with __WAYEXPAND_BUS_NAME__ substituted
// for this daemon's unique D-Bus name (see lib.rs). KWin exposes no
// Wayland protocol for reading the focused window -- this scripting
// interface, reached over the session D-Bus, is the same mechanism tools
// like kdotool use.
var lastReportKey = null;
var pendingReportKey = null;
var pendingReport = null;
var pendingReportGeneration = 0;
var pendingReportAttempts = 0;
var reportRetryTimer = null;
var MAX_REPORT_ATTEMPTS = 8;
var REPORT_RETRY_INTERVAL = 250;
var reconciliationTimer = null;

function stopReportRetry() {
    if (reportRetryTimer) {
        reportRetryTimer.stop();
        reportRetryTimer = null;
    }
}

function scheduleReportRetry(generation) {
    if (reportRetryTimer || typeof QTimer === "undefined") {
        return;
    }
    try {
        reportRetryTimer = new QTimer();
        reportRetryTimer.interval = REPORT_RETRY_INTERVAL;
        reportRetryTimer.timeout.connect(function () {
            reportRetryTimer.stop();
            reportRetryTimer = null;
            if (generation !== pendingReportGeneration || pendingReportKey === null) {
                return;
            }
            if (pendingReportAttempts >= MAX_REPORT_ATTEMPTS) {
                pendingReportKey = null;
                pendingReport = null;
                return;
            }
            sendPendingReport(generation);
        });
        reportRetryTimer.start();
    } catch (_error) {
        reportRetryTimer = null;
    }
}

function sendPendingReport(generation) {
    if (generation !== pendingReportGeneration || pendingReportKey === null) {
        return;
    }
    var reportKey = pendingReportKey;
    var report = pendingReport;
    if (!report) {
        return;
    }
    pendingReportAttempts += 1;
    scheduleReportRetry(generation);
    try {
        callDBus(
            "__WAYEXPAND_BUS_NAME__",
            "/WindowTracker",
            "org.wayexpand.WindowTracker1",
            "WindowChanged",
            report.appId,
            report.title,
            report.instanceId,
            report.identitySource,
            report.processId,
            function (accepted) {
                if (generation !== pendingReportGeneration || pendingReportKey !== reportKey) {
                    return;
                }
                if (accepted !== true) {
                    return;
                }
                stopReportRetry();
                lastReportKey = reportKey;
                pendingReportKey = null;
                pendingReport = null;
            }
        );
    } catch (_error) {
        // The bounded retry timer resends this content-free focus snapshot.
    }
}

// KWin can load a script before its active-window snapshot is available, and
// it can miss an activation callback while this D-Bus client reconnects. Keep
// a low-rate in-compositor reconciliation running for the script lifetime.
// This is an in-process property read, not a D-Bus query, and report()
// deduplicates unchanged snapshots. It also retries reports after the bounded
// per-call delivery retry has expired.
function startReconciliation() {
    if (reconciliationTimer || typeof QTimer === "undefined") {
        return;
    }
    try {
        reconciliationTimer = new QTimer();
        reconciliationTimer.interval = 1000;
        reconciliationTimer.timeout.connect(function () {
            report(currentActiveWindow());
        });
        reconciliationTimer.start();
    } catch (_error) {
        // A timer is an optional recovery aid. It must never prevent the
        // ordinary focus callback path from starting.
        reconciliationTimer = null;
    }
}

function report(window) {
    var appId = "";
    var identitySource = "unavailable";
    if (window) {
        var resourceClass = window.resourceClass || "";
        var normalizedClass = resourceClass.toLowerCase();
        // Some KWin-owned Wayland windows expose the compositor executable as
        // resourceClass instead of the application owning the text surface.
        // It is not a usable application identity for matching terminal or
        // browser allowlists. Prefer KWin's documented desktop-file identity
        // for those windows; if it is absent, leave the app identity unknown
        // rather than attributing the compositor to the focused client.
        var classIsCompositor =
            normalizedClass === "kwin_wayland" || normalizedClass === "kwin_x11";
        var desktopFileName = window.desktopFileName || "";
        var desktopIdentity = desktopFileName;
        var desktopSlash = desktopIdentity.lastIndexOf("/");
        if (desktopSlash >= 0) {
            desktopIdentity = desktopIdentity.substring(desktopSlash + 1);
        }
        if (desktopIdentity.toLowerCase().slice(-8) === ".desktop") {
            desktopIdentity = desktopIdentity.slice(0, -8);
        }
        var desktopIsCompositor =
            desktopIdentity.toLowerCase() === "kwin_wayland" ||
            desktopIdentity.toLowerCase() === "kwin_x11";
        var identityIsCompositor = classIsCompositor || desktopIsCompositor;
        if (!classIsCompositor) {
            appId = resourceClass;
            if (appId) {
                identitySource = "resource-class";
            }
        }
        // Native Wayland windows may not expose an X11 resource class, and a
        // compositor-owned class is not the app. KWin's desktopFileName is
        // the owning application's desktop identity in either case. If that
        // value names KWin itself, no client identity is available; do not
        // fall through to resourceName or attribute the compositor to a
        // focused terminal/browser.
        if (!appId) {
            if (!desktopIsCompositor) {
                appId = desktopIdentity;
                if (appId) {
                    identitySource = "desktop-file-name";
                }
            }
        }
        // Some Wayland windows expose a compositor-owned resourceClass while
        // resourceName still identifies the client. Accept that separate
        // KWin window property only when it is itself not a compositor
        // identity. Never use the editable title as an application identity.
        var resourceName = window.resourceName || "";
        var normalizedResourceName = resourceName.toLowerCase();
        var resourceNameIsCompositor =
            normalizedResourceName === "kwin_wayland" ||
            normalizedResourceName === "kwin_x11";
        if (!appId && resourceName && !resourceNameIsCompositor) {
            appId = resourceName;
            identitySource = "resource-name";
        }
        if (!appId && identityIsCompositor) {
            identitySource = "compositor-identity-only";
        }
    }
    var title = window ? window.caption : "";
    var processId = window && Number.isInteger(Number(window.pid)) && Number(window.pid) > 0
        ? Number(window.pid)
        : 0;
    // internalId is a compositor-issued identity for this exact KWin window.
    // It is intentionally kept separate from appId/title, which are not unique.
    var instanceId = window && window.internalId ? String(window.internalId) : "";
    // KWin may deliver the same transition through both workspace activation
    // and a window's activeChanged signal. Avoid duplicate D-Bus traffic.
    var reportKey = JSON.stringify([appId, title, instanceId, processId]);
    if (reportKey === lastReportKey || reportKey === pendingReportKey) {
        return;
    }
    stopReportRetry();
    pendingReportGeneration += 1;
    pendingReportKey = reportKey;
    pendingReport = {
        appId: appId,
        title: title,
        instanceId: instanceId,
        identitySource: identitySource,
        processId: processId,
    };
    pendingReportAttempts = 0;
    sendPendingReport(pendingReportGeneration);
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
startReconciliation();
report(currentActiveWindow());

// Reconciliation remains active for this script's lifetime, including when
// KWin temporarily reports no focused window. It never infers focus from
// stacking order, titles, or keyboard activity.
