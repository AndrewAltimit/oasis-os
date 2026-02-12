#![no_main]

use libfuzzer_sys::fuzz_target;
use oasis_core::pbp::parse_pbp;

fuzz_target!(|data: &[u8]| {
    // Must not panic on any byte sequence. Errors are fine.
    let _result = parse_pbp(data);
});
