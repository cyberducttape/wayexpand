#!/usr/bin/env node
"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const vm = require("node:vm");

const source = fs.readFileSync(
  "crates/backend-kwin-window/src/window-tracker.js",
  "utf8",
);
const reports = [];
let activated;
let windowAdded;
let windowRemoved;
const workspace = {
  activeWindow: null,
  stackingOrder: [],
  windowActivated: {
    connect(callback) {
      activated = callback;
    },
  },
  windowAdded: {
    connect(callback) {
      windowAdded = callback;
    },
  },
  windowRemoved: {
    connect(callback) {
      windowRemoved = callback;
    },
  },
};

vm.runInNewContext(source, {
  workspace,
  callDBus(...args) {
    reports.push(args);
    if (typeof args.at(-1) === "function") args.at(-1)(true);
  },
});

assert.equal(typeof activated, "function", "tracker subscribes to window activation");

function lastIdentity() {
  return Array.from(reports.at(-1).slice(4, 7));
}

function lastIdentitySource() {
  return reports.at(-1)[7];
}

function lastProcessId() {
  return reports.at(-1)[8];
}

activated({
  resourceClass: "org.example.Class",
  desktopFileName: "org.example.Desktop",
  resourceName: "example-resource",
  caption: "Private document title",
  internalId: "window-1",
  pid: process.pid,
});
assert.deepEqual(lastIdentity(), [
  "org.example.Class",
  "Private document title",
  "window-1",
]);
assert.equal(lastIdentitySource(), "resource-class");
assert.equal(lastProcessId(), process.pid, "authenticated focus report includes KWin's window PID");

activated({
  resourceClass: "kwin_wayland",
  desktopFileName: "/usr/share/applications/org.kde.konsole.desktop",
  resourceName: "kwin_wayland",
  caption: "Terminal title",
  internalId: "window-kwin-class",
  pid: process.pid,
});
assert.deepEqual(
  lastIdentity(),
  ["org.kde.konsole", "Terminal title", "window-kwin-class"],
  "a compositor resource class must use KWin's owning desktop-file identity",
);
assert.equal(lastIdentitySource(), "desktop-file-name");

activated({
  resourceClass: "kwin_wayland",
  desktopFileName: "",
  resourceName: "kwin_wayland",
  caption: "Compositor-owned window",
  internalId: "window-kwin-unknown",
  pid: process.pid,
});
assert.deepEqual(
  lastIdentity(),
  ["", "Compositor-owned window", "window-kwin-unknown"],
  "a compositor identity without an owning desktop ID must fail closed",
);
assert.equal(lastIdentitySource(), "compositor-identity-only");

activated({
  resourceClass: "kwin_wayland",
  desktopFileName: "",
  resourceName: "org.kde.konsole",
  caption: "Terminal title",
  internalId: "window-kwin-resource-name",
  pid: process.pid,
});
assert.deepEqual(
  lastIdentity(), ["org.kde.konsole", "Terminal title", "window-kwin-resource-name"],
  "a client resource name can identify a window when KWin's class is compositor-owned",
);
assert.equal(lastIdentitySource(), "resource-name");

activated({
  resourceClass: "kwin_wayland",
  desktopFileName: "/usr/share/applications/kwin_wayland.desktop",
  resourceName: "kwin_wayland",
  caption: "Wayland window",
  internalId: "window-kwin-desktop",
  pid: process.pid,
});
assert.deepEqual(
  lastIdentity(),
  ["", "Wayland window", "window-kwin-desktop"],
  "a KWin desktop-file identity must not be reported as the focused application",
);
assert.equal(lastIdentitySource(), "compositor-identity-only");

activated({
  resourceClass: "",
  desktopFileName: "/opt/example/share/org.example.App.desktop",
  resourceName: "example-resource",
  caption: "Private document title",
  internalId: "window-2",
  pid: process.pid,
});
assert.deepEqual(lastIdentity(), [
  "org.example.App",
  "Private document title",
  "window-2",
]);
assert.equal(lastIdentitySource(), "desktop-file-name");

activated({
  resourceClass: "",
  desktopFileName: "",
  resourceName: "example-resource",
  caption: "Private document title",
  internalId: "window-3",
  pid: process.pid,
});
assert.deepEqual(lastIdentity(), [
  "example-resource",
  "Private document title",
  "window-3",
]);
assert.equal(lastIdentitySource(), "resource-name");

activated({
  resourceClass: "",
  desktopFileName: "",
  resourceName: "",
  caption: "Private document title",
  internalId: "window-4",
  pid: process.pid,
});
assert.deepEqual(lastIdentity(), [
  "",
  "Private document title",
  "window-4",
]);
assert.equal(lastIdentitySource(), "unavailable");

assert.equal(typeof windowAdded, "function", "tracker watches new windows");
assert.equal(typeof windowRemoved, "function", "tracker forgets removed windows");

const changedWindow = {
  resourceClass: "org.example.FocusFallback",
  desktopFileName: "org.example.FocusFallback",
  resourceName: "focus-fallback",
  caption: "Focus fallback test",
  internalId: "window-5",
  pid: process.pid,
  active: false,
  activeChanged: {
    connect(callback) {
      this.callback = callback;
    },
  },
};
windowAdded(changedWindow);
assert.equal(
  typeof changedWindow.activeChanged.callback,
  "function",
  "tracker watches each new window's activeChanged signal",
);
changedWindow.active = true;
changedWindow.activeChanged.callback();
assert.deepEqual(lastIdentity(), [
  "org.example.FocusFallback",
  "Focus fallback test",
  "window-5",
]);
assert.equal(lastIdentitySource(), "resource-class");

const afterWindowSignal = reports.length;
activated(changedWindow);
assert.equal(
  reports.length,
  afterWindowSignal,
  "workspace and window activation paths coalesce duplicate state",
);

windowRemoved(changedWindow);
changedWindow.activeChanged.callback();
assert.equal(
  reports.length,
  afterWindowSignal,
  "removed windows no longer report focus changes",
);

const alreadyActiveWindow = {
  resourceClass: "org.example.AlreadyActive",
  desktopFileName: "",
  resourceName: "already-active",
  caption: "Active when added",
  internalId: "window-already-active",
  pid: process.pid,
  active: true,
  activeChanged: {
    connect(callback) {
      this.callback = callback;
    },
  },
};
const beforeAlreadyActive = reports.length;
windowAdded(alreadyActiveWindow);
assert.deepEqual(
  lastIdentity(),
  ["org.example.AlreadyActive", "Active when added", "window-already-active"],
  "a window added already active publishes the initial focus snapshot",
);
assert.equal(
  reports.length,
  beforeAlreadyActive + 1,
  "an already-active window is reported once",
);

const startupActiveWindow = {
  resourceClass: "org.example.StartupFocus",
  desktopFileName: "",
  resourceName: "startup-focus",
  caption: "Startup focus test",
  internalId: "window-startup-active",
  pid: process.pid,
  active: true,
  activeChanged: { connect() {} },
};
let startupActivated;
const startupReports = [];
const startupWorkspace = {
  activeWindow: null,
  stackingOrder: [
    {
      resourceClass: "org.example.Background",
      caption: "Background window",
      internalId: "window-background",
      active: false,
      activeChanged: { connect() {} },
    },
    startupActiveWindow,
  ],
  windowActivated: {
    connect(callback) {
      startupActivated = callback;
    },
  },
  windowAdded: { connect() {} },
  windowRemoved: { connect() {} },
};
vm.runInNewContext(source, {
  workspace: startupWorkspace,
  callDBus(...args) {
    startupReports.push(args);
    if (typeof args.at(-1) === "function") args.at(-1)(true);
  },
});
assert.deepEqual(
  Array.from(startupReports.at(-1).slice(4, 7)),
  [
    "org.example.StartupFocus",
    "Startup focus test",
    "window-startup-active",
  ],
  "initial focus falls back to the unique active client in KWin's snapshot",
);
startupActivated(null);
assert.equal(
  startupReports.length,
  1,
  "a null activation signal coalesces with the recovered active client",
);

const ambiguousReports = [];
const ambiguousWorkspace = {
  ...startupWorkspace,
  stackingOrder: [
    startupActiveWindow,
    { ...startupActiveWindow, internalId: "window-also-active" },
  ],
};
vm.runInNewContext(source, {
  workspace: ambiguousWorkspace,
  callDBus(...args) {
    ambiguousReports.push(args);
    if (typeof args.at(-1) === "function") args.at(-1)(true);
  },
});
assert.deepEqual(
  Array.from(ambiguousReports.at(-1).slice(4, 7)),
  ["", "", ""],
  "ambiguous active clients fail closed instead of selecting one",
);

const reconcileTimers = [];
class FakeQTimer {
  constructor() {
    this.interval = 0;
    this.stopped = false;
    this.timeout = {
      connect: (callback) => {
        this.callback = callback;
      },
    };
    reconcileTimers.push(this);
  }

  start() {}

  stop() {
    this.stopped = true;
  }
}
const delayedActiveWindow = {
  resourceClass: "org.example.DelayedStartup",
  caption: "Delayed startup window",
  internalId: "window-delayed-startup",
  active: true,
  activeChanged: { connect() {} },
};
const delayedStartupReports = [];
const delayedStartupWorkspace = {
  activeWindow: null,
  stackingOrder: [],
  windowActivated: { connect() {} },
  windowAdded: { connect() {} },
  windowRemoved: { connect() {} },
};
vm.runInNewContext(source, {
  workspace: delayedStartupWorkspace,
  QTimer: FakeQTimer,
  callDBus(...args) {
    delayedStartupReports.push(args);
    if (typeof args.at(-1) === "function") args.at(-1)(true);
  },
});
const delayedStartupTimer = reconcileTimers.find((timer) => !timer.stopped);
assert.equal(delayedStartupTimer.interval, 1000, "startup reconciliation is low rate");
assert.equal(delayedStartupTimer.stopped, false, "startup reconciliation starts");
for (let attempt = 0; attempt < 30; attempt += 1) {
  delayedStartupTimer.callback();
}
assert.equal(
  delayedStartupTimer.stopped,
  false,
  "reconciliation remains active while KWin has not reported a focused app",
);
delayedStartupWorkspace.activeWindow = delayedActiveWindow;
delayedStartupTimer.callback();
assert.deepEqual(
  Array.from(delayedStartupReports.at(-1).slice(4, 7)),
  ["org.example.DelayedStartup", "Delayed startup window", "window-delayed-startup"],
  "delayed compositor active-window state is reconciled",
);
assert.equal(
  delayedStartupTimer.stopped,
  false,
  "low-rate reconciliation remains active after finding an app ID",
);

const noFocusTimers = [];
class NoFocusQTimer extends FakeQTimer {
  constructor() {
    super();
    noFocusTimers.push(this);
  }
}
const noFocusReports = [];
vm.runInNewContext(source, {
  workspace: {
    activeWindow: null,
    stackingOrder: [],
    windowActivated: { connect() {} },
    windowAdded: { connect() {} },
    windowRemoved: { connect() {} },
  },
  QTimer: NoFocusQTimer,
  callDBus(...args) {
    noFocusReports.push(args);
    if (typeof args.at(-1) === "function") args.at(-1)(true);
  },
});
const noFocusTimer = noFocusTimers.find((timer) => !timer.stopped);
for (let attempt = 0; attempt < 300; attempt += 1) {
  noFocusTimer.callback();
}
assert.equal(noFocusTimer.stopped, false, "unknown focus is reconciled until KWin reports an app ID");
assert.equal(noFocusReports.length, 1, "unchanged unknown focus is deduplicated");

const deliveryRetryTimers = [];
class DeliveryRetryTimer extends FakeQTimer {
  constructor() {
    super();
    deliveryRetryTimers.push(this);
  }
}
const retryWindow = {
  resourceClass: "org.example.Retry",
  caption: "Retry delivery test",
  internalId: "window-retry",
};
let deliveryAttempts = 0;
const retryWorkspace = {
  activeWindow: retryWindow,
  stackingOrder: [],
  windowActivated: { connect(callback) { this.callback = callback; } },
  windowAdded: { connect() {} },
  windowRemoved: { connect() {} },
};
vm.runInNewContext(source, {
  workspace: retryWorkspace,
  QTimer: DeliveryRetryTimer,
  callDBus(...args) {
    deliveryAttempts += 1;
    if (deliveryAttempts === 2 && typeof args.at(-1) === "function") {
      args.at(-1)(true);
    }
  },
});
assert.equal(deliveryAttempts, 1, "initial focus report is sent once");
const deliveryRetryTimer = deliveryRetryTimers.find((timer) => timer.interval === 250);
assert.equal(deliveryRetryTimer.interval, 250, "D-Bus delivery retry is bounded");
deliveryRetryTimer.callback();
assert.equal(deliveryAttempts, 2, "unacknowledged initial focus report is retried");
assert.equal(deliveryRetryTimer.stopped, true, "successful report acknowledgment stops retry");
retryWorkspace.windowActivated.callback(retryWindow);
assert.equal(deliveryAttempts, 2, "acknowledged focus report is deduplicated");

const persistentRetryTimers = [];
class PersistentRetryTimer extends FakeQTimer {
  constructor() {
    super();
    persistentRetryTimers.push(this);
  }
}
let persistentAttempts = 0;
const persistentWorkspace = {
  activeWindow: retryWindow,
  stackingOrder: [],
  windowActivated: { connect() {} },
  windowAdded: { connect() {} },
  windowRemoved: { connect() {} },
};
vm.runInNewContext(source, {
  workspace: persistentWorkspace,
  QTimer: PersistentRetryTimer,
  callDBus(...args) {
    persistentAttempts += 1;
    if (typeof args.at(-1) === "function") {
      args.at(-1)(persistentAttempts >= 9);
    }
  },
});
for (let attempt = 0; attempt < 8; attempt += 1) {
  const retryTimer = persistentRetryTimers.find(
    (timer) => timer.interval === 250 && !timer.stopped,
  );
  assert.ok(retryTimer, "a bounded delivery retry is scheduled");
  retryTimer.callback();
}
assert.equal(persistentAttempts, 8, "delivery retries stop at the configured limit");
const persistentReconciliationTimer = persistentRetryTimers.find(
  (timer) => timer.interval === 1000 && !timer.stopped,
);
assert.ok(persistentReconciliationTimer, "focus reconciliation remains active");
persistentReconciliationTimer.callback();
assert.equal(
  persistentAttempts,
  9,
  "periodic reconciliation retries a report after bounded delivery attempts expire",
);

console.log("KWin window tracker identity and activation checks passed");
