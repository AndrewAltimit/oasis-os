//! Real-world HTML corpus tests.
//!
//! Unlike `browser_integration.rs` which exercises hand-written minimal
//! snippets, this suite runs the full pipeline (parse → style → layout →
//! paint) against checked-in fixtures that resemble real pages: a
//! Wikipedia-style article, a news homepage, a personal blog post, and an
//! adversarially malformed page.
//!
//! The goals are:
//!
//! 1. **Smoke test** — ensure the pipeline doesn't panic or return an
//!    error on pages that aren't hand-crafted for the engine.
//! 2. **Regression guard** — if someone breaks the tree builder, CSS
//!    cascade, layout, or paint, these will fire.
//! 3. **Adversarial coverage** — the `adversarial_malformed.html` fixture
//!    hits common HTML-parser recovery rules (unclosed formatting,
//!    stray table cells, missing head, etc.) — the parser must survive
//!    without hanging or losing the document root.
//!
//! Fixtures live under `tests/fixtures/`.

use std::collections::HashMap;
use std::path::PathBuf;

use oasis_browser::internals::{
    BoxType, CascadeContext, ComputedStyle, LayoutBox, NodeKind, PaintViewport, Stylesheet,
    TagName, TextMeasurer, Tokenizer, TreeBuilder, build_layout_tree, default_stylesheet,
    paint_page, style_tree,
};
use oasis_test_backend::MockSdiCore;

/// Monospace measurer — 6px per character.
struct FixedMeasurer;

impl TextMeasurer for FixedMeasurer {
    fn measure_text(&self, text: &str, _font_size: u16) -> u32 {
        (text.len() as u32) * 6
    }
}

/// Load a fixture file from `tests/fixtures/`.
fn load_fixture(name: &str) -> String {
    let path: PathBuf = [env!("CARGO_MANIFEST_DIR"), "tests", "fixtures", name]
        .iter()
        .collect();
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read fixture {}: {e}", path.display()))
}

/// Full pipeline: parse → cascade → layout → paint to a recording mock.
///
/// Returns the built layout tree and the mock backend so the caller can
/// assert on draw-call counts, layout topology, etc.
fn run_pipeline_sized(
    html: &str,
    width: f32,
    height: f32,
) -> (
    LayoutBox,
    MockSdiCore,
    Vec<Option<ComputedStyle>>,
    Vec<NodeKind>,
) {
    let tokens = Tokenizer::new(html).tokenize();
    let doc = TreeBuilder::build(tokens);

    let ua = default_stylesheet();
    let sheets: Vec<&Stylesheet> = vec![&ua];
    let ctx = CascadeContext::default();
    let styles = style_tree(&doc, &sheets, &[], &ctx);

    let layout = build_layout_tree(
        &doc,
        &styles,
        &FixedMeasurer,
        width,
        height,
        None,
        &HashMap::new(),
    );

    let mut backend = MockSdiCore::new(width as u32, height as u32);
    let vp = PaintViewport {
        scroll_y: 0.0,
        scroll_x: 0.0,
        x: 0,
        y: 0,
        width,
        height,
        visible_height: height,
    };
    let link_map = HashMap::new();
    paint_page(&layout, &mut backend, vp, &link_map).expect("paint must not error");

    let kinds: Vec<NodeKind> = doc.nodes.iter().map(|n| n.kind.clone()).collect();
    (layout, backend, styles, kinds)
}

fn run_pipeline(
    html: &str,
) -> (
    LayoutBox,
    MockSdiCore,
    Vec<Option<ComputedStyle>>,
    Vec<NodeKind>,
) {
    run_pipeline_sized(html, 800.0, 600.0)
}

/// Count boxes of a given type in the layout tree.
fn count_boxes<F: Fn(&LayoutBox) -> bool>(root: &LayoutBox, pred: &F) -> usize {
    let mut n = if pred(root) { 1 } else { 0 };
    for child in &root.children {
        n += count_boxes(child, pred);
    }
    n
}

fn has_tag(kinds: &[NodeKind], tag: TagName) -> bool {
    kinds
        .iter()
        .any(|k| matches!(k, NodeKind::Element(d) if d.tag == tag))
}

// ===================================================================
// Fixture: Wikipedia-style article
// ===================================================================

#[test]
fn wikipedia_article_parses_and_paints() {
    let html = load_fixture("wikipedia_article.html");
    let (layout, backend, _styles, kinds) = run_pipeline(&html);

    // Tree builder must recover head/body split.
    assert!(has_tag(&kinds, TagName::Html), "missing <html>");
    assert!(has_tag(&kinds, TagName::Body), "missing <body>");
    assert!(has_tag(&kinds, TagName::Head), "missing <head>");

    // The article has 3 h2s + 2 h3s; at minimum we expect headings and
    // paragraphs to survive cascade + layout.
    assert!(
        count_boxes(&layout, &|b| matches!(b.box_type, BoxType::Block)) > 5,
        "expected multiple block-level boxes"
    );

    // Paint should have issued draw calls.
    assert!(
        !backend.calls().is_empty(),
        "paint pass should have emitted draw calls"
    );
    let text_calls = backend
        .calls()
        .iter()
        .filter(|(m, _)| m == "draw_text")
        .count();
    assert!(text_calls > 10, "expected many text runs, got {text_calls}");
}

// ===================================================================
// Fixture: News homepage with grid/flex layout
// ===================================================================

#[test]
fn news_homepage_parses_and_paints() {
    let html = load_fixture("news_homepage.html");
    let (layout, backend, _styles, kinds) = run_pipeline(&html);

    assert!(has_tag(&kinds, TagName::Body));
    // Should contain <table> under the article.
    assert!(
        has_tag(&kinds, TagName::Table),
        "expected <table> to survive parsing"
    );

    // Paint must succeed without errors (run_pipeline would panic otherwise).
    assert!(!backend.calls().is_empty());
    // Layout tree must not be empty.
    assert!(count_boxes(&layout, &|_| true) > 20);
}

// ===================================================================
// Fixture: Blog post (simple long-form content)
// ===================================================================

#[test]
fn blog_post_parses_and_paints() {
    let html = load_fixture("blog_post.html");
    let (layout, backend, _styles, kinds) = run_pipeline(&html);

    // Blog has an <article> element — WHATWG maps it like a block.
    assert!(has_tag(&kinds, TagName::Body));

    // At least one blockquote, one pre, one ordered list.
    assert!(has_tag(&kinds, TagName::Blockquote));
    assert!(has_tag(&kinds, TagName::Pre));
    assert!(has_tag(&kinds, TagName::Ol));

    // Content should produce text calls.
    let text_calls = backend
        .calls()
        .iter()
        .filter(|(m, _)| m == "draw_text")
        .count();
    assert!(text_calls > 10, "expected many text runs, got {text_calls}");

    // Layout tree shouldn't be degenerate.
    assert!(count_boxes(&layout, &|_| true) > 15);
}

// ===================================================================
// Fixture: Adversarial malformed HTML
// ===================================================================

#[test]
fn adversarial_malformed_does_not_crash() {
    let html = load_fixture("adversarial_malformed.html");
    // The core contract: parser + layout + paint must not panic, must
    // not error, and must produce *something* non-empty.
    let (layout, backend, _styles, kinds) = run_pipeline(&html);

    // Even with missing close tags, we must have an <html> root.
    assert!(has_tag(&kinds, TagName::Html));
    assert!(has_tag(&kinds, TagName::Body));
    // The explicit <table> block must survive even with the other chaos.
    assert!(
        has_tag(&kinds, TagName::Table),
        "explicit table element must survive error recovery"
    );
    // Paragraphs must survive too — the adoption-agency algorithm should
    // reconstruct active formatting elements across unclosed tags.
    assert!(has_tag(&kinds, TagName::P));

    // Paint must succeed and emit draw calls.
    assert!(!backend.calls().is_empty());

    // Layout must have at least a few boxes (we just need to not lose
    // the document — exact tree shape under error recovery is not
    // pinned here).
    assert!(count_boxes(&layout, &|_| true) > 3);
}

// ===================================================================
// All fixtures: paint produces no errors under narrow viewports too
// ===================================================================

#[test]
fn all_fixtures_paint_at_narrow_viewport() {
    for name in &[
        "wikipedia_article.html",
        "news_homepage.html",
        "blog_post.html",
        "adversarial_malformed.html",
    ] {
        let html = load_fixture(name);
        let (_layout, backend, _styles, _kinds) = run_pipeline_sized(&html, 320.0, 240.0);
        assert!(
            !backend.calls().is_empty(),
            "{name} produced no paint calls at narrow viewport"
        );
    }
}
