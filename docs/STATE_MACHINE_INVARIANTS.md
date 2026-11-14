# State-machine invariants

WayExpand's safety properties cross crate and thread boundaries. Tests for one
function are not enough when a reload, focus transition, worker completion, or
shutdown can interleave with another operation. This document names the
invariants that changes to those paths must preserve and points to the current
models.

This inventory is also a complexity boundary. Each asynchronous subsystem must
have an explicit state owner and a documented handoff; adding another worker,
queue, or retry path without extending the relevant invariant and regression
model increases the number of cross-component schedules reviewers must reason
about. The models below cover specific admission and publication boundaries,
not every combination of the daemon's independent state machines. Changes that
connect two previously independent lifecycles must add a composition test for
that boundary before being treated as covered.

## Invariants

### Output transactions

- A transaction with an unknown or partial result is never replayed.
- Restoring or discarding a deferred result clears only that result's
  provisional undo state; a stale result without its reservation cannot clear
  undo state belonging to a newer expansion.
- Resolving an output-integrity gate is idempotent: duplicate completion or
  late input cannot change its first terminal decision.
- Physical input observed during non-atomic output quarantines the transaction,
  even if the injector reports that all output was submitted.
- Atomicity is established only when the backend's atomic flag and replacement
  guarantee agree; interruption monitoring and diagnostics use this same
  contract so an inconsistent capability declaration cannot skip safeguards.
- Cancellation can stop future output but cannot retract text already sent.
- Queued output and follow-up input are not applied to a different focus
  context.
- An in-flight output checkpoint remains bound to the focus stamp captured at
  transaction start. Publishing a newer focus stamp must never rebase that
  stream onto the newly focused window.
- An in-flight compatibility-mode replacement stops if its consent changes or
  a previously confirmed target loses tracker confirmation.
- A final admission refusal before the backend submits any output is
  `not_applied`, even if consent or focus changed after initial admission; it
  must not be reported as a partial write or increment uncertainty counters.
- `not_applied` permits retry only while the original target context remains
  valid. If physical input, focus, or another integrity signal invalidated
  that context while output was pending, discard the deferred trigger and any
  target-bound undo state even when the backend confirms it emitted nothing.
- Consent uses a generation as well as a mode value; revoking and restoring
  the same value cannot revive an output admitted under the prior generation.

Production transitions live in `crates/daemon/src/pending_output.rs` and
`crates/daemon/src/output_loop.rs`. The integrity gate has exhaustive event
ordering and generated-sequence tests (all sequences through six events,
including repeated input, focus changes, and completion events); Loom models
cancellation at the injector checkpoint/send boundary. These models do not
prove that an application applied emitted text.
`integrity_gate_resolution_is_immutable_and_idempotent` also checks that a
duplicate resolution and post-resolution input cannot rewrite the transaction
outcome.
The post-outcome choice to quarantine, advance a batch, or replay follow-up
input is centralized in `output_continuation`; both the reactor and the Loom
output-race model call that production function. Its truth-table regression
ensures an invalidated transaction can neither advance queued output nor replay
follow-up input. A known-not-applied command or insert may replay follow-up
input, while typed failures retain the existing quarantine behavior.
The core regression `restoring_or_discarding_a_deferred_result_clears_only_its_provisional_undo`
protects undo state at the reservation boundary, including stale duplicate
results.
The deterministic regression
`focus_change_after_insertion_but_before_ack_refuses_the_next_batch_item`
also covers the post-insertion, pre-acknowledgment interval: a focus update is
published while the first insert is blocked. The next queued replacement is
refused before it erases in the new focus, and a buffered trigger from the old
target is rejected by the live focus witness. This establishes admission
behavior for later work; it cannot verify the application result of the
already-submitted first item.
The output guard keeps a separate immutable transaction stamp while the
reactor's current stamp advances. Focus updates are applied to the engine
before that current stamp advances, so a checkpoint cannot observe a new
baseline while the old engine context is still active. Regression tests cover
focus changing during paced output and the ordering between engine update and
guard advancement. This closes a software-level reauthorization race; it
cannot make a focus check and compositor keystroke delivery atomic.
Loom also exhausts checkpoint-versus-invalidation schedules for a
focus-away-and-back transition and for consent revocation followed by restoring
the same mode. The shared `output_baseline_is_valid` predicate is used by both
the live output guard and the model. A backend event whose checkpoint already
passed may still be in flight when invalidation arrives; the model permits
that one event and verifies that a subsequent checkpoint cannot continue.
This bounds work after the daemon observes invalidation; it does not prove
that the compositor's focus and the daemon's tracker are synchronized at the
physical event boundary.
`consent_revoked_after_admission_is_reported_as_clean_refusal` exercises the
separate final-check edge: compatibility is revoked after initial admission,
and the backend's refusal crosses the transaction boundary as `NotApplied`
with zero erased or inserted units.
The pending-output completion path also passes context validity into outcome
handling. `invalidated_context_drops_known_not_applied_trigger_instead_of_restoring_it`
and `invalidated_context_discards_undo_after_known_not_applied_output` cover
the ordering where the target context is reset before a clean refusal arrives;
the ordinary known-not-applied retry regression remains in place for a valid
context. `focus_refusal_and_physical_input_compose_as_terminal_invalidation`
checks the cross-product of physical-input observation and focus versus
policy refusal, including that a late input event cannot rewrite a resolved
gate. `focus_aba_before_worker_start_discards_the_deferred_trigger` drives a
reserved match through the serialized worker and pending-output resolver after
the focus witness crosses A→B→A; it proves the production path emits nothing
and does not restore the old trigger into the matcher.
The deterministic sequence regression in `assurance.rs` exhausts every
ordering of checkpoint, send, focus away/back, and consent revoke/restore
through six events, then checks 512 reproducible sequences of 64 events. It
calls the same production admission predicate and asserts that
focus/consent invalidation remains terminal and at most one already-admitted
backend event can cross it. The pending-output generated-sequence regression
also composes physical input with applied, uncertain, focus-refused, and
policy-refused outcomes through six events using the production outcome
classifier. It preserves the first terminal outcome, so a duplicate or late
`Applied` notification cannot clear an earlier uncertain result; an explicit
regression checks that ordering. The Loom tests cover scheduler interleavings;
exhaustive and seeded sequences cover bounded and longer state histories.
The stdin/libei pipe-driven path uses a monotonically increasing, content-free
input generation. Output checkpoints compare the generation of the line being
processed with the latest line read by the background reader. A later line
invalidates the paced output stream; the reactor resets matcher context and
quarantines all generations already observed. A focused Loom model covers the
checkpoint ordering, and the KWin regression exercises the actual pipe-driven
reactor. That regression still demonstrates partial text corruption when
input overlaps a non-atomic replacement. The generation gate prevents the
stale follow-up trigger from expanding, but it cannot retract already-emitted
characters and does not satisfy the replacement-integrity release gate. The
libei worker shares its content-free per-unit progress with the transaction
trace, so the observed interrupted run reports `partial_write`, unknown target
result, and the number of acknowledged insertion units. This is diagnostic
evidence, not proof that every acknowledged unit appeared in the client.
For serialized output that is queued before the worker begins, the reactor
defers focus application while the job is pending. The worker captures the
last applied stamp, while the live witness has already advanced; therefore a
focus switch (including a switch away and back) invalidates the job before its
first output unit.

### Control-plane shutdown

- Control-request admission and shutdown closure share one mutex-protected
  state transition. A request that observes the open state is ordered before
  closure; requests observing the closed state are dropped before worker
  creation.
- The `stop` request closes admission before publishing the daemon stop flag.
  Server teardown also closes admission before waking and joining the listener.
- A request admitted before closure may still be running while shutdown starts;
  its handler remains bounded by the control I/O deadline. The model proves
  the admission ordering, not that every detached request worker has exited.

`ControlAdmissionState` is the transition used by the listener and the Loom
model `control_admission_loom::shutdown_closes_control_request_admission`.
The model checks both scheduler orders around admission and closure, then
asserts that every subsequent admission attempt is refused. Socket-level
shutdown and handler deadlines remain covered by the control integration
tests.

### Asynchronous completion and target identity

- A completion is applicable only to the input generation and focus identity
  against which its work was requested.
- IBus contexts share one bounded process-wide command pool and a separate
  bounded hotkey pool. Queue admission, shutdown state, input generation,
  completion channel, and cancellation remain owned by the originating context.
- A per-context admission slot is held until the matching completion is
  drained, not only until its worker starts. The slot limit equals that
  context's completion-channel capacity, so a stalled consumer cannot fill its
  channel and then park every process-wide worker on a blocked send.
- Runtime shutdown removes that owner's queued tasks before waiting for its
  active tasks to acknowledge cancellation. A completion can only be sent to
  the originating runtime's channel and cannot be applied to a replacement
  engine context.
- Shutdown closes the runtime's active-job admission gate before waiting for
  its active count to reach zero. A worker dequeued just before queue cleanup
  either registered before closure and is awaited, or is refused after closure.
- Input, focus, pause, sensitivity, and composition changes invalidate old
  command output.
- A running direct expansion subprocess is terminated when its input generation
  becomes stale; the supervisor kills and reaps its owned process group.
- The final direct-command generation/shutdown check and process spawn are
  serialized with input-generation changes and runtime shutdown. Start and
  invalidation therefore have a single order: a command that starts first may
  be canceled after invalidation, while one invalidated first cannot spawn.
- A completion published before its wakeup is either consumed or leaves a
  wakeup pending; shutdown must release a producer blocked by a full queue.

The shared generation predicate is in
`crates/core/src/engine/mod.rs`; completion admission is in
`crates/core/src/engine/async_runtime.rs`. Loom covers publication/wakeup,
shutdown under backpressure, and stale completion after focus changes.
The direct-command cancellation path also has a process-level regression test
that advances the generation while a child is running and verifies its later
side effect does not occur. This is best-effort containment: it cannot undo a
side effect that completed before cancellation was observed. Named broker
actions retain their separate explicit authorization lifecycle.
The command start gate is checked by a generated event-sequence test and a Loom
scheduler model covering target changes and shutdown racing with process
admission. Async completion delivery also models an A→B→A focus transition:
returning to the same target does not make work from the earlier visit current,
because the input generation has advanced. The gate applies to direct
expansion commands; broker actions use
their own authorization and shutdown linearization boundary. The gate is held
only across the final check and OS process creation; because spawn is a
synchronous system call, the input reactor can wait briefly while that
boundary is being established.

### Configuration reload

- Only a fully validated, still-current configuration snapshot may replace the
  active engine.
- A failed or stale build leaves the active engine intact.
- Runtime restrictions are carried into the replacement engine before it is
  published.
- The promoted policy is checked against the negotiated live capture and
  injection contract. A newly promoted requirement or a reconnected route that
  the other side cannot meet blocks automatic matching; relaxing the
  requirement or negotiating a compatible route clears that gate only after
  the corresponding validated transition.
- Completion or action state from a retired engine must not become applicable
  to the replacement engine.

Reload admission uses `reload_snapshot_is_current` in
`crates/daemon/src/reload.rs`; exhaustive predicate tests and filesystem-backed
reload tests cover freshness and activation. The real-thread regression
`reload_while_a_command_runs_drops_its_result_and_keeps_typing_working` in
`crates/daemon/src/reload/tests.rs` exercises an old command running across an
engine swap, verifies the new configuration remains responsive, and proves its
completion is not delivered into the new engine. A scheduler model in
`crates/daemon/src/reload.rs` combines reload promotion and completion draining.
It invokes the production reload-snapshot predicate and represents
the per-engine completion queues as separate epoch-owned mailboxes. It proves
the modeled reactor cannot drain an old mailbox after promoting the new engine;
it does not model the complete reactor, std channel implementation, or
compositor/application effects. Any change that shares completion queues or
runtimes across reloads must add an explicit runtime/configuration epoch and
extend this model before it is accepted.

The composed Loom scenario
`output_reload_focus_and_shutdown_interleaving_quarantines_stale_work` also
interleaves a pending old-epoch command completion and output worker with focus
and consent invalidation, reload promotion, shutdown, and a partial injector
failure. It uses the production output-admission predicate, reload-snapshot
predicate, output-result classifier, continuation decision, and integrity
gate. Its scheduler explores up to three preemptions to keep the added actor
bounded. It checks that neither retired completion is applied to the new
epoch, the partial result is quarantined, replay stays forbidden, and at most
one backend call already admitted before invalidation can remain in flight.
This is a bounded composition model, not a proof of application-level text
integrity or of the complete reactor implementation.
The filesystem-backed regression
`reload_enforcing_unavailable_route_capability_disables_expansion_until_policy_relaxes`
checks the composition between embedded organization policy, the negotiated
route contract, reload promotion, and matcher admission. It verifies that
unblocking begins with a fresh trigger buffer rather than reviving input from
the prior generation.

The IBus service publishes a validated generation only after it has acquired
the factory snapshot, instance registry, and all live adapter locks. A poisoned
lock aborts that handoff before any adapter or factory snapshot changes. If the
instance registry becomes poisoned while context-generation diagnostics are
being refreshed, the service preserves the last published generation and
reports `WX-IBUS-RELOAD-001`; it does not leave stale context counts appearing
healthy. `poisoned_context_registry_invalidates_generation_readiness` covers
this failure path.

### Configuration writes

- Saves from WayExpand writers serialize on a stable per-path lock, not the
  replaceable configuration inode.
- A GUI save whose source revision changed during temporary-file staging must
  preserve the external edit and report a conflict.

Conditional saves compare the loaded source under the stable lock and compare
again after staging and syncing, immediately before descriptor-relative rename
on Linux. A deterministic storage test injects an external edit in that
staging interval. A GUI regression also saves, applies an external edit after
that save completes, and attempts another GUI save; the disk edit is retained,
the GUI draft remains available, and the conflict is reported. Concurrent
changes to different fields of one snippet are intentionally treated as a
conflict. A non-cooperating writer can still race after the last comparison and
before the rename syscall; eliminating that interval requires the external
writer to use the same lock protocol or a filesystem primitive with
compare-and-swap semantics.

### Focus and window identity

- Text and pending output belong to one focus identity.
- An identity change invalidates buffered trigger state and asynchronous work.
- A retitle of the same window may refresh title filters, but must not invent a
  new focus target.
- Unknown identity fails closed for app-scoped behavior.

The production identity comparison is in
`crates/core/src/engine/expansion.rs`; split-trigger tests cover same-window
retitles, new instances, changed applications, and unavailable identity. The
generated focus-history model in that module checks 3,906 deterministic
sequences of exact-target updates, title-only refreshes, other windows,
unknown identities, and missing-window reports. It asserts that buffered
trigger text survives only updates proven to name the same exact target, and
that every uncertain identity transition advances the input generation.
Compositor behavior still requires desktop certification.

### Shutdown and authorization

- Shutdown rejects new work, cancels active blocking reads/work, and does not
  report teardown complete while a worker can still perform the protected
  operation.
- Authorization and target identity must still be valid at the irreversible
  operation boundary; a prior check alone is not sufficient if the target or
  policy can change before execution.
- No uncertain operation is automatically retried during cleanup.

Output worker retirement has a Loom model. Broker client-read cancellation has
a repeated real-Unix-socket race test, including registration before and after
shutdown starts. Action process startup and shutdown publication now share a
launch gate: startup either holds the gate and linearizes before shutdown, or
observes shutdown and does not spawn. A barrier-driven race test exercises both
orderings; executor tests also verify that shutdown during a delayed action
prevents its later side effect while canceling and reaping the process tree.
The successful PolicyKit decision is carried as an action-specific permit to
the spawn gate. A missing permit or one naming a different action is refused
before process creation. The broker Loom scheduler model now interleaves
authorization, shutdown, and launch, and calls the production admission
predicate. Loom models that ordering with a model mutex; the production
`std::sync::Mutex` remains covered by the barrier-driven race test, not directly
by Loom.
The live PolicyKit-authorized shutdown/audit integration remains separate.
Review broker authorization whenever request identity, effective policy, or
process execution ordering changes.

The output reconnect decision is centralized in
`output_reconnect_allowed`. The reactor passes the liveness of every retired
worker to this predicate, and the Loom model uses the same predicate before
allowing a replacement route. This keeps the key shutdown invariant attached
to production code: a route is not reconnected while any retired worker may
still own or use compositor-facing resources.

The shared IBus worker pool uses a process-wide bounded command queue and a
separate hotkey queue, while each engine context owns bounded completion
credits. Its active-job admission state (`try_enter`, `close`, `leave`) is
shared between the production mutex-backed implementation and a Loom model.
This verifies that shutdown waits for jobs admitted before closure and refuses
workers dequeued but not yet admitted. A queue saturation regression additionally
holds completion credits until the owning reactor drains results, then verifies
another context can complete work while that first result channel remains full.
These checks cover lifecycle admission and one cross-context saturation case,
but not service-wide throughput fairness under many simultaneously active,
long-running contexts.

## Required review for stateful changes

For each change to reload, focus, output, command completion, authorization, or
shutdown:

1. Identify the state owner and every concurrent actor that can mutate or
   observe that state.
2. Enumerate the adversarial event orderings, including failure, timeout,
   cancellation, and shutdown.
3. State the safety invariant independently of implementation details.
4. Exercise the production transition predicate in a deterministic or Loom
   model; use property-based generation where the state space is larger than a
   small exhaustive enumeration.
5. Keep integration tests for effects the model cannot establish, especially
   compositor delivery, application text integrity, and privileged execution.

Models are evidence about the modeled transition system only. They do not
replace physical-keyboard, real-application, or deployment certification.
