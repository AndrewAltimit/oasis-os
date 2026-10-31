#![no_main]

//! The browser loader's other hostile-input parsers: HPACK header blocks
//! (HTTP/2), Gemini responses, CSP headers, cookie headers / dates,
//! URLs and `data:` URLs. None may panic on any input.

use libfuzzer_sys::fuzz_target;
use oasis_core::browser::loader::hpack::{DynamicTable, decode_block};
use oasis_core::browser::loader::{Url, cookies, csp, gemini, parse_data_uri};

fuzz_target!(|data: &[u8]| {
    let mut table = DynamicTable::new(4096);
    let _ = decode_block(data, &mut table, 4096);
    let _ = gemini::parse_response(data);

    if let Ok(text) = std::str::from_utf8(data) {
        let _ = csp::parse_csp(text);
        let _ = cookies::parse_cookie_date(text);
        let _ = parse_data_uri(text);
        if let Some(url) = Url::parse(text) {
            let _ = url.resolve("../a?b#c");
            let _ = url.request_target();
            let _ = url.origin();
        }
        if let Some(base) = Url::parse("https://example.com/a/b") {
            let _ = base.resolve(text);
            let mut jar = cookies::CookieJar::new();
            jar.set_cookies(&base, &[("set-cookie".to_string(), text.to_string())]);
            let _ = jar.cookie_header(&base);
        }
    }
});
