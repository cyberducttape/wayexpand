# IME and Composition-Aware Input Roadmap

**Status: planned architecture work; not implemented and not a release promise.**

WayExpand currently matches direct keyboard input and text after an input
method has committed it. Active preedit/composition is unsupported. See the
[support matrix](SUPPORT_MATRIX.md) before deploying WayExpand in an IME-heavy
workflow.

This is an explicit product boundary and a planned engineering direction, not
an indefinitely deferred “compatibility bug.” Until the gates below are met,
WayExpand must not claim universal Linux text-expander or active-IME support.

## Architecture direction

Composition must be represented as protocol state, not guessed from key
sequences or reconstructed from evdev. The current input-method-v2 source
already receives compositor-managed surrounding text and sensitive content
purpose, and its `TextInjector::replace` submits deletion and replacement in
one input-method commit. It remains experimental and compositor-dependent;
that protocol operation is not evidence of end-to-end rollback or universal
atomicity.

The next architectural step is to separate effective guarantees by role:

- capture-source guarantees: exclusive capture, field-purpose signal,
  preedit visibility, committed-text semantics, and key pass-through;
- output guarantees: Unicode fidelity, replacement transaction semantics,
  cursor placement, and bounded completion;
- pair/session guarantees: the guarantees actually negotiated for a connected
  capture/output pair on this compositor.

`InjectorCapabilities` is an existing foundation, but it currently combines
some pair-level claims (such as `sensitive_focus`) with output properties. A
future contract should make those sources explicit and conservative by
default. Policy must evaluate negotiated pair capabilities, not backend names.
Evdev must continue to report no field-purpose awareness and best-effort,
non-exclusive replacement; it is a compatibility fallback, not the safety
flagship.

The Wayland [text-input-v3 protocol](https://wayland.app/protocols/text-input-unstable-v3)
describes text-input state associated with a seat and text-entry focus, with
preedit and commit events. It is not, by itself, a universal passive global
keyboard-observation API. The [input-method-v2 protocol](https://wayland.app/protocols/xx-input-method-v2)
defines compositor/input-method interaction and ordered commit state. Protocol
selection must be based on which role WayExpand implements and what each
compositor/client actually exposes; protocol names alone are not a support
claim.

## Delivery gates

1. **Protocol-neutral state model.** Add explicit enter/leave, activate/deactivate,
   preedit update/cancel, committed text, surrounding-text revision, content
   purpose, and protocol serial/state transitions. Preedit must never enter
   the committed matcher buffer. Every event must be generation/focus-bound.
2. **Fail-closed behavior.** Unknown or sensitive content purpose suspends
   expansion. Focus loss, deactivation, protocol reset, disconnect, and
   cancellation clear preedit and invalidate pending matches without deleting
   application text. Unsupported compositions pass through or remain untouched;
   WayExpand must not consume them speculatively.
3. **Capability and policy integration.** Report composition-awareness and
   sensitive-field semantics separately from text insertion properties.
   Add policy requirements that reject a pair before capture starts when the
   configured guarantee is unavailable. `doctor`, setup, GUI health, and
   certification output must describe the negotiated state and its caveats.
4. **Real-client certification.** Exercise activation/deactivation, preedit
   edits, candidate selection, commit/cancel, dead keys, Compose, AltGr,
   language switching, password/PIN fields, focus races, reconnect, and
   compositor restart across the exact client/toolkit/compositor combinations
   in the certification matrix. Include Fcitx and IBus engines (including
   Rime where available); a protocol unit test is not sufficient.
5. **Promotion.** Keep composition-aware support experimental until the
   automated and manual evidence is reproducible and published. A passing
   committed-text test or support for WayExpand's own IBus engine does not
   satisfy this gate.

## Non-goals until these gates pass

- Do not infer field sensitivity from application name, title, or evdev.
- Do not advertise CJK, Fcitx, Rime, IBus interoperation, dead-key, or Compose
  workflows as supported merely because committed Unicode text works.
- Do not weaken systemd hardening or grant broader device access to compensate
  for missing composition semantics.
- Do not make evdev the default or describe quiet-period mitigations as
  transactionality.
