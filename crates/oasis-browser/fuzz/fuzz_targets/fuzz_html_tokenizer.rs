#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Feed arbitrary bytes into the HTML tokenizer.
    // Catches panics from malformed HTML (unclosed tags, broken entities,
    // deeply nested elements, pathological attribute values, etc.).
    if let Ok(html) = std::str::from_utf8(data) {
        let mut tokenizer = oasis_browser::internals::Tokenizer::new(html);
        while tokenizer.next_token().is_some() {}
    }
});
