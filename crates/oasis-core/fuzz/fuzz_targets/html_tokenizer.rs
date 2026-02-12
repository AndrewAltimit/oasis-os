#![no_main]

use libfuzzer_sys::fuzz_target;
use oasis_core::browser::html::tokenizer::Tokenizer;

fuzz_target!(|data: &[u8]| {
    if let Ok(input) = std::str::from_utf8(data) {
        // Must not panic or loop infinitely on any input.
        let mut tokenizer = Tokenizer::new(input);
        let _tokens = tokenizer.tokenize();
    }
});
