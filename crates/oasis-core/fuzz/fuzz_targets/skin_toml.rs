#![no_main]

use libfuzzer_sys::fuzz_target;
use oasis_core::skin::Skin;

fuzz_target!(|data: &[u8]| {
    if let Ok(input) = std::str::from_utf8(data) {
        // Try parsing as a manifest (single TOML file).
        // Use the same string for all 3 required files -- the parser
        // should tolerate any combination without panicking.
        let _result = Skin::from_toml(input, input, input);
    }
});
