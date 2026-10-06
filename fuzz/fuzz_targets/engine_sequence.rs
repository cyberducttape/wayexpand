//! Stateful engine fuzzing: arbitrary sequences of typing, focus and
//! sensitive-field changes, composition, pause, window identity, undo, and
//! configuration reloads. Invariants are checked by
//! `wayexpand_core::fuzzing::check_engine_sequence`, which the core test
//! suite also runs over seeded sequences on stable.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    wayexpand_core::fuzzing::check_engine_sequence(data);
});
