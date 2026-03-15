#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Convert arbitrary bytes to a string, replacing invalid UTF-8 with
    // the replacement character.  The tokenizer accepts `&str`.
    let input = String::from_utf8_lossy(data);

    // Create the tokenizer and consume the full token stream.
    // This exercises every state-machine transition: tags, attributes,
    // comments, DOCTYPE, character references, RAWTEXT, RCDATA, and
    // error-recovery paths.
    let mut tokenizer = oasis_browser::internals::Tokenizer::new(&input);
    let _tokens = tokenizer.tokenize();
});
