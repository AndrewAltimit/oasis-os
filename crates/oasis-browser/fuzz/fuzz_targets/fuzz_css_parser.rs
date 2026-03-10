#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Feed arbitrary bytes into the CSS parser.
    // Catches panics from malformed CSS (broken selectors, invalid values,
    // unclosed blocks, pathological whitespace, etc.).
    if let Ok(css) = std::str::from_utf8(data) {
        let _ = oasis_browser::internals::Stylesheet::parse(css);
        let _ = oasis_browser::internals::parse_inline_style(css);
    }
});
