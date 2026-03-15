#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Convert arbitrary bytes to a string, replacing invalid UTF-8 with
    // the replacement character.  The CSS parser accepts `&str`.
    let input = String::from_utf8_lossy(data);

    // Parse the input as a full CSS stylesheet.
    // This exercises the CSS tokenizer, selector parsing, declaration
    // parsing, shorthand expansion, @media evaluation, color parsing,
    // and all error-recovery paths.
    let _stylesheet = oasis_browser::internals::Stylesheet::parse(&input);
});
