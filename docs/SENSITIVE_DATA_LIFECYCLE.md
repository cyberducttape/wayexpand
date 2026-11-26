# Sensitive data lifecycle

WayExpand can reduce the lifetime of sensitive values in memory, but it cannot
promise complete memory erasure. Allocators, compiler-generated temporaries,
kernel buffers, desktop protocols, clipboard managers, and target applications
may retain copies outside the ownership controls listed here.

## Ownership inventory

| Data | Main owners and boundaries | Current controls and limits |
|---|---|---|
| Raw typed characters | Input backend, event queue, engine matcher | The engine wipes owned `InputEvent::Text` after processing and wipes its matcher buffer. `TextChar` avoids a heap copy on supported routes. Earlier backend/event copies are outside that final owner. |
| Command output | Child process pipes, worker result, command cache, template renderer | Output is moved into `SensitiveText`; cache expiry and drop wipe owned text. OS pipe buffers and child-process copies are external. |
| Clipboard content | Clipboard worker/prefetch slot, template renderer, replacement result | Clipboard reads move into protected template output where possible; prefetch and desktop clipboard machinery can have other copies. Clipboard history behavior depends on compositor/manager. |
| Replacement text | Engine result, output transaction, backend request, target app | Engine results and transaction text use `SensitiveText`; backend/protocol buffers and the receiving application may retain copies. Delivery confirmation describes the backend contract, not erasure. |
| Form values | GUI input, daemon IPC, `FormValues`, template renderer | `FormValues` redacts Debug and wipes value strings on drop. GUI widgets, serialized IPC frames, temporary deserialization buffers, and map keys are not all protected by the same type. |
| Secrets | Template context, command environment/arguments, subprocess | Access is policy-gated and command output is protected after capture. Environment/argv strings are ordinary OS process data and can be copied by the child or inspected by same-UID tools. Avoid putting secrets in argv. |
| Undo data | Engine undo state | Owned replacement text is protected and cleared on invalidation; copies already passed to an output backend or target remain outside the engine. |

`SensitiveText` is the common protected owner for transient rendered and
replacement text. It is suitable at Rust ownership boundaries where WayExpand
controls the allocation. Converting it to `String`, serializing it, or passing
it to an external API creates a new boundary that must be documented; the type
does not make those copies disappear.

## Action capability profiles

Broker actions run as the desktop user. Authentication and process supervision
do not isolate that UID from its repositories, credentials, or application
sockets. Optional per-action Landlock/seccomp controls therefore remain
important. Configured `capability_profile` values require an explicit action
sandbox with direct network socket denial. The currently supported labels are
`text-generation` and `local-diagnostics`; they do not grant additional access
by themselves. Filesystem access is still the explicit `read_paths` and
`write_paths` policy (plus the selected runtime profile).

Remote diagnostics and deployment profiles are intentionally not available in
the packaged runner: the service denies IP networking globally, while local
Unix sockets can still reach proxies. A future network-capable profile needs a
separate reviewed runner with scoped paths and credentials. Basic text
expansion does not require broker sandboxing.
