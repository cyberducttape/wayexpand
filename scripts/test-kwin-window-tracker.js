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
const workspace = {
  activeWindow: null,
  windowActivated: {
    connect(callback) {
      activated = callback;
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

console.log("KWin window tracker identity fallback checks passed");
