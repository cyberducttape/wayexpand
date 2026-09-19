# GUI Performance Analysis & Optimization Roadmap

## Overview

This document analyzes known performance bottlenecks in the WayExpand GUI
and tracks fixes for them.

---

## Resolved

### "Use current app" Button Could Freeze the GUI Indefinitely

**Original symptom:** clicking "Use current app" in the Snippet Editor made
the GUI unresponsive, expected to last 1-2 seconds (KWin script
registration delay).

**Actual severity turned out to be worse than documented here:** the
button handler called `KwinWindowTracker::new()` synchronously on the UI
thread, and while the *window-wait* step inside it was bounded to 5
seconds, the D-Bus connection and KWin script registration steps before
that had **no bound at all**. A session bus or KWin left in a bad state
(observed in practice after a prior daemon instance was forcibly killed)
could hang that call indefinitely, freezing the entire window with no way
to recover short of killing the process.

**Fix:** the whole detection (connection, script load, window-wait) now
runs on a background thread; the UI polls a channel each frame, shows a
spinner while waiting, and offers a Cancel button that stops waiting on
the thread (without joining or killing it — it's simply abandoned if it
never answers). See `GuiApp::app_detection` in `crates/gui/src/main.rs`.

The daemon's `KwinWindowTracker::probe()` had the identical unbounded-hang
shape, called synchronously in `main()` before the event loop starts;
fixed the same way (bounded to 3 seconds on a detached thread) after it
caused a real production outage — a hung probe meant the daemon never
processed a single keystroke, with no log output explaining why.

---

## Other Known Issues

None currently tracked. If a new performance bottleneck is found, add it
here with a symptom, root cause, and severity before fixing it, so the fix
can be verified against a concrete description.
