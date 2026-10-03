//! Action Broker request frames from an untrusted local client, read through
//! buffers of varying size so UTF-8 sequences and newlines straddle reads.
#![no_main]

use libfuzzer_sys::fuzz_target;
use std::io::BufReader;

fuzz_target!(|data: &[u8]| {
    let Some((&capacity, frame)) = data.split_first() else {
        return;
    };
    let mut reader = BufReader::with_capacity(usize::from(capacity).max(1), frame);
    let _ = action_broker::decode_request_frame(&mut reader);
});
