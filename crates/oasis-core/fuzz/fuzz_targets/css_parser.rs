#![no_main]

use libfuzzer_sys::fuzz_target;
use oasis_core::browser::css::parser::{Stylesheet, parse_inline_style};

fuzz_target!(|data: &[u8]| {
    if let Ok(input) = std::str::from_utf8(data) {
        // Stylesheet parser -- must not panic on any CSS input.
        let _stylesheet = Stylesheet::parse(input);

        // Inline style parser -- same guarantee.
        let _declarations = parse_inline_style(input);
    }
});
