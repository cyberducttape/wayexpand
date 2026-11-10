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
  },
});

assert.equal(typeof activated, "function", "tracker subscribes to window activation");

function lastIdentity() {
  return Array.from(reports.at(-1).slice(-3));
}

activated({
  resourceClass: "org.example.Class",
  desktopFileName: "org.example.Desktop",
  resourceName: "example-resource",
  caption: "Private document title",
  internalId: "window-1",
});
assert.deepEqual(lastIdentity(), [
  "org.example.Class",
  "Private document title",
  "window-1",
]);

activated({
  resourceClass: "",
  desktopFileName: "/opt/example/share/org.example.App.desktop",
  resourceName: "example-resource",
  caption: "Private document title",
  internalId: "window-2",
});
assert.deepEqual(lastIdentity(), [
  "org.example.App",
  "Private document title",
  "window-2",
]);

activated({
  resourceClass: "",
  desktopFileName: "",
  resourceName: "example-resource",
  caption: "Private document title",
  internalId: "window-3",
});
assert.deepEqual(lastIdentity(), [
  "example-resource",
  "Private document title",
  "window-3",
]);

activated({
  resourceClass: "",
  desktopFileName: "",
  resourceName: "",
  caption: "Private document title",
  internalId: "window-4",
});
assert.deepEqual(lastIdentity(), [
  "",
  "Private document title",
  "window-4",
]);

assert.equal(typeof windowAdded, "function", "tracker watches new windows");
assert.equal(typeof windowRemoved, "function", "tracker forgets removed windows");

const changedWindow = {
  resourceClass: "org.example.FocusFallback",
  desktopFileName: "org.example.FocusFallback",
  resourceName: "focus-fallback",
  caption: "Focus fallback test",
  internalId: "window-5",
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
  },
});
assert.deepEqual(
  Array.from(startupReports.at(-1).slice(-3)),
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
  },
});
assert.deepEqual(
  Array.from(ambiguousReports.at(-1).slice(-3)),
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
  },
});
const delayedStartupTimer = reconcileTimers.at(-1);
assert.equal(delayedStartupTimer.interval, 100, "startup reconciliation is paced");
assert.equal(delayedStartupTimer.stopped, false, "startup reconciliation starts");
delayedStartupWorkspace.activeWindow = delayedActiveWindow;
delayedStartupTimer.callback();
assert.deepEqual(
  Array.from(delayedStartupReports.at(-1).slice(-3)),
  ["org.example.DelayedStartup", "Delayed startup window", "window-delayed-startup"],
  "delayed compositor active-window state is reconciled",
);
assert.equal(delayedStartupTimer.stopped, true, "timer stops after finding focus");

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
  },
});
const noFocusTimer = noFocusTimers.at(-1);
for (let attempt = 0; attempt < 30; attempt += 1) {
  noFocusTimer.callback();
}
assert.equal(noFocusTimer.stopped, true, "empty focus reconciliation is bounded");
assert.equal(noFocusReports.length, 1, "no focus is not guessed after timeout");

console.log("KWin window tracker identity and activation checks passed");
