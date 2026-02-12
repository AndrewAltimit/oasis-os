#![no_main]

use libfuzzer_sys::fuzz_target;
use oasis_core::browser::loader::http::parse_response;

fuzz_target!(|data: &[u8]| {
    // Must not panic on any byte sequence. Errors are fine.
    let _result = parse_response(data);
});
