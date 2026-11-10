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

console.log("KWin window tracker identity and activation checks passed");
