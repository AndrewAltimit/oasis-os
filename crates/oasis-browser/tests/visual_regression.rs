//! Visual regression tests for the real-world corpus.
//!
//! The epic on `feat/browser-realworld-compat-epic` ships a
//! display-list golden harness as the "visual regression" layer for
//! `oasis-browser`. For each checked-in fixture under
//! `tests/fixtures/`, we run the full pipeline (parse → cascade →
//! layout → paint) into a `MockSdiCore` that records every draw call,
//! canonicalize the resulting stream to a deterministic text format,
//! and diff against a golden file under `tests/goldens/`.
//!
//! Why a display-list dump instead of a PNG? Three reasons:
//!
//! 1. **Reviewable diffs.** A PNG regression shows up as "these two
//!    binary blobs differ" in code review. A text dump shows up as a
//!    meaningful `fill_rect` / `draw_text` diff that a reviewer can
//!    read without running the harness.
//! 2. **Deterministic across backends.** The paint pass feeds an
//!    `SdiCore` implementation, and the draw-call stream is what any
//!    backend would rasterize. Capturing it directly avoids the
//!    per-backend anti-aliasing drift a PNG pipeline would suffer.
//! 3. **No binary blobs in the repo.** Goldens land as plain `.txt`
//!    files, small enough to check in without LFS.
//!
//! To refresh goldens after an intentional paint-path change, run with
//! `UPDATE_GOLDENS=1 cargo test -p oasis-browser --test visual_regression`.

use std::collections::HashMap;
use std::path::PathBuf;

use oasis_browser::internals::{
    CascadeContext, Document, MediaViewport, NodeKind, PaintViewport, Stylesheet, TagName,
    TextMeasurer, Tokenizer, TreeBuilder, build_layout_tree, default_stylesheet, paint_page,
    parse_inline_style, style_tree,
};
use oasis_test_backend::MockSdiCore;

type NodeId = usize;

/// Walk the DOM and collect text content of `<style>` elements, parsing
/// each into a `Stylesheet`. Mirrors `widget::pipeline::collect_style_sheets`.
///
/// `viewport` is threaded into the parser so `@media (min/max-width)` and
/// `@media (prefers-color-scheme)` evaluate against the test viewport
/// rather than the 480x272 default.
fn collect_style_sheets(doc: &Document, viewport: MediaViewport) -> Vec<Stylesheet> {
    let mut sheets = Vec::new();
    for (id, node) in doc.nodes.iter().enumerate() {
        if let NodeKind::Element(elem) = &node.kind
            && elem.tag == TagName::Style
        {
            let css_text = doc.text_content(id);
            if !css_text.is_empty() {
                sheets.push(Stylesheet::parse_with_viewport(&css_text, viewport));
            }
        }
    }
    sheets
}

/// Walk the DOM to collect inline `style=""` attributes. Mirrors
/// `widget::pipeline::collect_inline_styles`.
fn collect_inline_styles(
    doc: &Document,
) -> Vec<(NodeId, Vec<oasis_browser::internals::ParsedDeclaration>)> {
    let mut result = Vec::new();
    for (id, node) in doc.nodes.iter().enumerate() {
        if let NodeKind::Element(elem) = &node.kind
            && let Some(style_attr) = elem.get_attribute("style")
        {
            let decls = parse_inline_style(style_attr);
            if !decls.is_empty() {
                result.push((id, decls));
            }
        }
    }
    result
}

/// Fixed-width measurer so the golden stream is layout-stable.
struct FixedMeasurer;

impl TextMeasurer for FixedMeasurer {
    fn measure_text(&self, text: &str, _font_size: u16) -> u32 {
        (text.len() as u32) * 6
    }
}

/// The corpus used by the visual regression harness.
///
/// Any new fixture added under `tests/fixtures/*.html` that should be
/// regression-guarded goes here. On first run with no golden present,
/// the harness fails loudly instead of silently accepting new output —
/// use `UPDATE_GOLDENS=1` to seed the golden.
//
// KNOWN-WRONG (tracked, do not regenerate away):
//   - `wikipedia_portal_480x272.txt` line 48..=54: the floated `<button
//     type="submit">` renders at (348, 190, 96×60) instead of its
//     pre-PR approximation of (378, 122, 64×20). The width is now
//     correct (intrinsic via `replaced_dimensions()` from PR #141); the
//     y-position and height are still wrong because the containing-block
//     and margin computation upstream of `shrink_float_to_fit` does not
//     yet pin replaced-float descendants against their static block
//     position. Fixing this requires a corrective second layout pass
//     and is deliberately out of scope for PR #141. If a future
//     layout change moves the button back toward (378, 122, 64×20) the
//     golden *should* be regenerated — that is a fix, not a regression.
const FIXTURES: &[&str] = &[
    "wikipedia_article.html",
    "wikipedia_portal.html",
    "news_homepage.html",
    "blog_post.html",
    "adversarial_malformed.html",
    "hackernews_frontpage.html",
    "github_readme.html",
    "rust_std_docs.html",
    "forum_thread.html",
    "commerce_product.html",
    "substack_post.html",
    "reddit_listing.html",
    "reddit_comments.html",
    "google_homepage.html",
];

fn fixtures_dir() -> PathBuf {
    [env!("CARGO_MANIFEST_DIR"), "tests", "fixtures"]
        .iter()
        .collect()
}

fn goldens_dir() -> PathBuf {
    [env!("CARGO_MANIFEST_DIR"), "tests", "goldens"]
        .iter()
        .collect()
}

/// Render a fixture and produce its canonical display-list dump.
fn render_display_list(html: &str, width: f32, height: f32) -> String {
    let tokens = Tokenizer::new(html).tokenize();
    let doc = TreeBuilder::build(tokens);

    // Mirror production's stylesheet plumbing: extract `<style>` element
    // contents and inline `style=""` attributes so fixtures render with
    // their author CSS applied, not just the UA default sheet.
    let viewport = MediaViewport {
        width,
        height,
        dark_mode: false,
        prefers_reduced_motion: false,
        hover: true,
        pointer: "fine",
    };
    let author_sheets = collect_style_sheets(&doc, viewport);
    let inline_styles = collect_inline_styles(&doc);
    let ua = default_stylesheet();
    let mut sheets: Vec<&Stylesheet> = vec![&ua];
    for sheet in &author_sheets {
        sheets.push(sheet);
    }
    let ctx = CascadeContext::default();
    let styles = style_tree(&doc, &sheets, &inline_styles, &ctx);

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
        focused_node: None,
        counter_styles: Vec::new(),
    };
    paint_page(&layout, &mut backend, vp, &HashMap::new()).expect("paint must not error");

    canonicalize_calls(backend.calls(), width as u32, height as u32)
}

/// Canonicalize the raw `(method, params)` stream into a stable dump.
///
/// The dump intentionally drops calls that carry noise (texture ids,
/// byte counts, `swap_buffers`, the `init` call) and keeps the calls
/// that would actually land as pixels. This is what the visual
/// regression test compares.
fn canonicalize_calls(calls: &[(String, String)], width: u32, height: u32) -> String {
    let mut out = String::new();
    out.push_str(&format!("# viewport {width}x{height}\n"));
    let mut rect_count = 0usize;
    let mut text_count = 0usize;
    let mut other_count = 0usize;
    for (method, params) in calls {
        match method.as_str() {
            "init" | "swap_buffers" | "shutdown" | "load_texture" | "destroy_texture" => {
                // Non-visual bookkeeping — skip.
            },
            "fill_rect" => {
                rect_count += 1;
                out.push_str(&format!("fill_rect {}\n", params.trim_end()));
            },
            "draw_text" => {
                text_count += 1;
                out.push_str(&format!("draw_text {}\n", params.trim_end()));
            },
            "clear" | "set_clip_rect" | "reset_clip_rect" | "blit" => {
                other_count += 1;
                let tail = params.trim_end();
                if tail.is_empty() {
                    out.push_str(&format!("{method}\n"));
                } else {
                    out.push_str(&format!("{method} {tail}\n"));
                }
            },
            _ => {
                // Unknown extension-trait call — record verbatim so
                // new backend paths show up as diffs rather than being
                // silently swallowed.
                other_count += 1;
                let tail = params.trim_end();
                if tail.is_empty() {
                    out.push_str(&format!("{method}\n"));
                } else {
                    out.push_str(&format!("{method} {tail}\n"));
                }
            },
        }
    }
    out.push_str(&format!(
        "# totals fill_rect={rect_count} draw_text={text_count} other={other_count}\n"
    ));
    out
}

/// Run one fixture at one viewport size and compare to its golden.
fn check_fixture(name: &str, width: f32, height: f32) {
    let fixture_path = fixtures_dir().join(name);
    let html = std::fs::read_to_string(&fixture_path)
        .unwrap_or_else(|e| panic!("failed to read fixture {}: {e}", fixture_path.display()));

    let dump = render_display_list(&html, width, height);

    let golden_name = format!(
        "{}_{}x{}.txt",
        name.trim_end_matches(".html"),
        width as u32,
        height as u32
    );
    let golden_path = goldens_dir().join(&golden_name);

    let update = std::env::var("UPDATE_GOLDENS").is_ok();
    if update || !golden_path.exists() {
        std::fs::create_dir_all(goldens_dir()).expect("create goldens dir");
        std::fs::write(&golden_path, &dump).expect("write golden");
        if !update {
            panic!(
                "golden {} did not exist; wrote a fresh one. Re-run to confirm it matches.",
                golden_path.display()
            );
        }
        return;
    }

    let expected = std::fs::read_to_string(&golden_path)
        .unwrap_or_else(|e| panic!("failed to read golden {}: {e}", golden_path.display()));

    if expected != dump {
        let diff_summary = summarize_diff(&expected, &dump);
        panic!(
            "visual regression on {name} @ {width}x{height}:\n{diff_summary}\n\n\
             To accept the new output, re-run with UPDATE_GOLDENS=1."
        );
    }
}

/// Produce a compact summary of where two display-list dumps diverge.
fn summarize_diff(expected: &str, actual: &str) -> String {
    let exp_lines: Vec<&str> = expected.lines().collect();
    let act_lines: Vec<&str> = actual.lines().collect();
    let mut out = String::new();
    out.push_str(&format!(
        "  expected {} lines, got {} lines\n",
        exp_lines.len(),
        act_lines.len()
    ));
    let max = exp_lines.len().max(act_lines.len());
    let mut shown = 0usize;
    for i in 0..max {
        let e = exp_lines.get(i).copied().unwrap_or("<eof>");
        let a = act_lines.get(i).copied().unwrap_or("<eof>");
        if e != a {
            out.push_str(&format!("  line {i}:\n    - {e}\n    + {a}\n"));
            shown += 1;
            if shown >= 5 {
                out.push_str("  ... (further differences suppressed)\n");
                break;
            }
        }
    }
    out
}

// ===================================================================
// Desktop viewport — 800x600 — catches the common regression cases.
// ===================================================================

#[test]
fn corpus_visual_regression_desktop() {
    for name in FIXTURES {
        check_fixture(name, 800.0, 600.0);
    }
}

// ===================================================================
// PSP viewport — 480x272 — catches narrow-layout regressions.
// ===================================================================

#[test]
fn corpus_visual_regression_psp() {
    for name in FIXTURES {
        check_fixture(name, 480.0, 272.0);
    }
}
