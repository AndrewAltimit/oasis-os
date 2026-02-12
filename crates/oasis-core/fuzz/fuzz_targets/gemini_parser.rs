#![no_main]

use libfuzzer_sys::fuzz_target;
use oasis_core::browser::gemini::parser::GeminiDocument;

fuzz_target!(|data: &[u8]| {
    if let Ok(input) = std::str::from_utf8(data) {
        // Must not panic on any text/gemini input.
        let _doc = GeminiDocument::parse(input);
    }
});
