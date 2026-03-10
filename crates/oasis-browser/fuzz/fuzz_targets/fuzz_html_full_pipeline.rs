#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Run the full HTML pipeline: tokenize -> build DOM -> parse CSS ->
    // cascade styles -> build layout tree.
    // This exercises the complete parsing and layout pipeline end-to-end.
    if let Ok(html) = std::str::from_utf8(data) {
        // Limit input size to prevent excessive runtime per iteration.
        if html.len() > 64 * 1024 {
            return;
        }

        let doc = oasis_browser::internals::TreeBuilder::parse(html);

        // Extract and parse inline styles + <style> blocks.
        let default_sheet = oasis_browser::internals::default_stylesheet();
        let mut ctx = oasis_browser::internals::CascadeContext::new();
        ctx.add_stylesheet(&default_sheet);
        let styles = oasis_browser::internals::style_tree(&doc, &ctx);

        // Build layout tree (requires a text measurer).
        struct FuzzMeasurer;
        impl oasis_browser::internals::TextMeasurer for FuzzMeasurer {
            fn measure_text(&self, text: &str, _font_size: u16) -> u32 {
                // Approximate: 7px per character.
                (text.len() * 7) as u32
            }
        }

        let image_info = std::collections::HashMap::new();
        let measurer =
            oasis_browser::internals::CachingMeasurer::new(&FuzzMeasurer);
        let _layout = oasis_browser::internals::build_layout_tree(
            &doc,
            &styles,
            &measurer,
            480.0,
            272.0,
            None,
            &image_info,
        );
    }
});
