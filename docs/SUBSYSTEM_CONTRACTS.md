# Subsystem contracts

These contracts are the seams a backend or feature change must preserve. They
describe guarantees the current interfaces can enforce, and call out where
the operating system or target application remains outside WayExpand's
guarantee. They are review boundaries, not claims that every possible
interleaving has been tested.

| Subsystem | Contract | Main interface or enforcement point | Limit |
| --- | --- | --- | --- |
| Capture | Declare negotiated privacy, exclusivity, key-state, composition, layout, and pass-through capabilities. New sources default conservatively. Deliver normalized committed text and state changes without inferring unsupported guarantees. | `InputSource`, `InputSourceCapabilities` in `crates/core/src/backend.rs` | A backend cannot claim password awareness or active-IME awareness unless its source actually reports it. Physical event arrival time is not currently exposed as an end-to-end latency guarantee. |
| Matcher | For a given configuration, buffered input, and context, make deterministic match decisions. Bound retained input and do not erase source text itself. | `ExpansionEngine` and `process_text_*` in `crates/core/src/engine/` | Matcher success does not authorize output or establish that output reached the intended client. |
| Focus tracker | Publish exact window identity only when the backend has one; advance a generation on every change so A→B→A invalidates stale work. Missing or stale identity fails closed for operations that require exact targeting. | `FocusState` in `crates/daemon/src/focus.rs`, focus stamps and output checkpoints | Focus is sampled evidence. It cannot guarantee that the compositor keeps the same target throughout an external delivery operation. |
| Output planner | Recommend only a path admitted by the route capabilities, size limits, and compatibility policy. Unsupported or unknown routes fall back to manual paste. | `InsertionRouteCapabilities`, `plan_replacement` in `crates/core/src/insertion_plan.rs`; runtime admission in `backend.rs` | A plan is not a delivery result. Non-atomic replacement remains non-atomic even with interruption polling. |
| Injector | Describe replacement guarantees, distinguish pre-submit refusal from uncertain post-submit errors, report acknowledged progress, and stop future output when a checkpoint is invalidated. | `InjectorCapabilities`, `TextInjector`, `OutputProgress`, `OutputCheckpoint` in `crates/core/src/backend.rs` | A successful backend return confirms only that backend's contract; it does not prove target rendering. Uncertain delivery is never automatically replayed. |
| Action Broker | Authorize each action at execution time, run only configured executables and argument/environment policy, bound time and concurrency, and fail closed when a configured per-action sandbox cannot be applied. | `ActionExecutor`, `ActionConfig`, `ActionSandbox` in `crates/action-broker/` | Polkit and process supervision are not isolation from the desktop UID. The packaged service denies direct IP networking but Unix-socket proxies remain a possible indirect channel. |
| GUI | Present the actual operation state, distinguish confirmed insertion, known no-insertion, and uncertain delivery, and avoid offering an ordinary retry for uncertain outcomes. | Quick Picker delivery state in `crates/gui/src/picker.rs` | GUI labels must follow the backend outcome; they cannot infer target-visible completion without application feedback. |

## Change review checklist

When changing one of these seams:

1. Keep the guarantee in the owning interface or state transition rather than
   inferring it from a backend name elsewhere.
2. Add a regression for the guarantee at the nearest shared enforcement
   point. Add a composition test when a change joins lifecycles that were
   previously independent.
3. Report measured performance per route, compositor, and target application;
   keep matcher, backend acknowledgment, and client-visible completion as
   separate measurements.
4. State explicitly which guarantees depend on desktop protocols or behavior
   outside the process.

This document is intentionally small and linked to implementation. It does
not prescribe a wholesale crate or file reorganization. Extract modules when
doing so gives a boundary one clear owner and makes its state transitions
easier to test.
