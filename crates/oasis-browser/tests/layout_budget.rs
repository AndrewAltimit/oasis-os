//! Layout performance budgets for the real-world corpus.
//!
//! Part of the real-world-compatibility-measurement epic: `cargo bench`
//! measures absolute layout cost and criterion gates on regressions,
//! but the "can this page still lay out at all?" question is much more
//! useful as a hard wall-clock budget that fails the normal `cargo
//! test` run rather than requiring a separate benchmark step. This
//! file provides exactly that.
//!
//! Budgets are intentionally loose (2–3× the measured release-build
//! time on CI) so they fail on catastrophic O(n²) / O(n³) regressions
//! instead of on noise from a busy CI runner. Tighten them as the
//! engine stabilises.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use oasis_browser::internals::{
    CascadeContext, Stylesheet, TextMeasurer, Tokenizer, TreeBuilder, build_layout_tree,
    default_stylesheet, style_tree,
};

struct FixedMeasurer;

impl TextMeasurer for FixedMeasurer {
    fn measure_text(&self, text: &str, _font_size: u16) -> u32 {
        (text.len() as u32) * 6
    }
}

fn load_fixture(name: &str) -> String {
    let path: PathBuf = [env!("CARGO_MANIFEST_DIR"), "tests", "fixtures", name]
        .iter()
        .collect();
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read fixture {}: {e}", path.display()))
}

/// Run the full parse → cascade → layout pipeline once and return the
/// measured wall-clock time.
fn measure_layout(html: &str, width: f32, height: f32) -> Duration {
    let start = Instant::now();
    let tokens = Tokenizer::new(html).tokenize();
    let doc = TreeBuilder::build(tokens);
    let ua = default_stylesheet();
    let sheets: Vec<&Stylesheet> = vec![&ua];
    let ctx = CascadeContext::default();
    let styles = style_tree(&doc, &sheets, &[], &ctx);
    let _layout = build_layout_tree(
        &doc,
        &styles,
        &FixedMeasurer,
        width,
        height,
        None,
        &HashMap::new(),
    );
    start.elapsed()
}

/// Budget table: (fixture, viewport-width, viewport-height, budget-ms).
///
/// Budgets are wall-clock caps on `parse + cascade + build_layout_tree`
/// end-to-end, measured on the host `cargo test` runs on. Values are
/// chosen so the debug build has headroom on a modest CI runner; the
/// release build is 5–10× faster.
///
/// `substack_post.html` carries a 750 ms budget instead of the default
/// 500 ms because it is ~3× larger than the other fixtures (long-form
/// article with nested `figure` / pull-quote markup, drop-cap, and a
/// 10+ paragraph body) — a narrow-reflow pass through it does more
/// work than the "home page" shape of the rest of the corpus. The
/// budget is set at ~1.5× its observed debug-build cost on CI.
const BUDGETS: &[(&str, f32, f32, u64)] = &[
    // Desktop viewport — 800x600 — primary budget target.
    ("wikipedia_article.html", 800.0, 600.0, 500),
    ("news_homepage.html", 800.0, 600.0, 500),
    ("blog_post.html", 800.0, 600.0, 500),
    ("adversarial_malformed.html", 800.0, 600.0, 500),
    ("hackernews_frontpage.html", 800.0, 600.0, 500),
    ("github_readme.html", 800.0, 600.0, 500),
    ("rust_std_docs.html", 800.0, 600.0, 500),
    ("forum_thread.html", 800.0, 600.0, 500),
    ("commerce_product.html", 800.0, 600.0, 500),
    ("substack_post.html", 800.0, 600.0, 750),
    ("rtl_bidi_stress.html", 800.0, 600.0, 500),
    ("responsive_grid.html", 800.0, 600.0, 500),
    // PSP viewport — 480x272 — narrow reflow target. All twelve
    // fixtures are gated here so a narrow-width regression in any
    // one of them (e.g. a `flex-basis: 0` loop or a text-wrap
    // O(n²)) fails `cargo test`, not just the four biggest pages.
    ("wikipedia_article.html", 480.0, 272.0, 500),
    ("news_homepage.html", 480.0, 272.0, 500),
    ("blog_post.html", 480.0, 272.0, 500),
    ("adversarial_malformed.html", 480.0, 272.0, 500),
    ("hackernews_frontpage.html", 480.0, 272.0, 500),
    ("github_readme.html", 480.0, 272.0, 500),
    ("rust_std_docs.html", 480.0, 272.0, 500),
    ("forum_thread.html", 480.0, 272.0, 500),
    ("commerce_product.html", 480.0, 272.0, 500),
    ("substack_post.html", 480.0, 272.0, 750),
    ("rtl_bidi_stress.html", 480.0, 272.0, 500),
    ("responsive_grid.html", 480.0, 272.0, 500),
];

#[test]
fn corpus_respects_layout_budgets() {
    let mut failures = Vec::new();
    for (name, w, h, budget_ms) in BUDGETS {
        let html = load_fixture(name);
        // Warm-up pass: the first run after crate load pays for
        // default-stylesheet parsing and initial allocator behaviour.
        // The budgeted pass is the second run so we measure steady-
        // state cost.
        let _ = measure_layout(&html, *w, *h);
        let elapsed = measure_layout(&html, *w, *h);
        let budget = Duration::from_millis(*budget_ms);
        if elapsed > budget {
            failures.push(format!(
                "  {name} @ {w}x{h}: {elapsed:?} > budget {budget:?}",
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "layout budget regressions:\n{}\n\n\
         To diagnose, re-run with `cargo test -p oasis-browser --test layout_budget \
         -- --nocapture` and profile the offending fixture with \
         `cargo bench -p oasis-browser --bench layout_engine`.",
        failures.join("\n")
    );
}
