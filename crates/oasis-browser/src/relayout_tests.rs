//! Differential tests: relaying out an existing tree must give exactly
//! the geometry of a freshly built one.
//!
//! The browser lays a built tree out again in place after images decode
//! ([`layout::block::relayout_tree`]), and layout itself lays subtrees
//! out more than once (a column flex item at its flexed height). Any
//! state a layout leaves behind that the next one reads (inline line
//! fragments, sizes written into the style) makes the two diverge.
//! These run every corpus fixture through such mutations and compare
//! every box against a fresh build.

use std::collections::HashMap;
use std::path::PathBuf;

use super::*;
use crate::layout::box_model::LayoutBox;

/// The `(width, height)` windows the corpus is checked at.
const WINDOWS: [(u32, u32); 2] = [(800, 600), (480, 272)];

fn fixture_names() -> Vec<String> {
    let dir: PathBuf = [env!("CARGO_MANIFEST_DIR"), "tests", "fixtures"]
        .iter()
        .collect();
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .expect("fixtures dir")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".html"))
        .collect();
    names.sort();
    assert!(names.len() >= 10, "corpus fixtures found: {names:?}");
    names
}

fn fixture(name: &str) -> String {
    let path: PathBuf = [env!("CARGO_MANIFEST_DIR"), "tests", "fixtures", name]
        .iter()
        .collect();
    std::fs::read_to_string(path).expect("fixture")
}

/// A browser that has loaded `html` in a `w` x `h` window.
fn loaded(html: &str, w: u32, h: u32) -> BrowserWidget {
    let mut config = BrowserConfig::default();
    config.features.home_url = "vfs://none".to_string();
    let mut b = BrowserWidget::new(config);
    b.set_window(0, 0, w, h);
    b.load_html(html, "vfs://corpus/page.html");
    b
}

/// The layout viewport of `b`'s window.
fn viewport(b: &BrowserWidget) -> (f32, f32) {
    let content_h = b.config.content_height(b.window_h);
    (
        b.window_w as f32 / b.config.zoom_level,
        content_h as f32 / b.config.zoom_level,
    )
}

/// One line per box (pre-order): type, node, geometry, text.
fn geometry(root: &LayoutBox) -> Vec<String> {
    fn walk(b: &LayoutBox, depth: usize, out: &mut Vec<String>) {
        let d = &b.dimensions;
        out.push(format!(
            "{depth} {:?} {:?} content {:.2},{:.2} {:.2}x{:.2} margin {:.2} {:.2} {:.2} {:.2} \
             text {:?}",
            std::mem::discriminant(&b.box_type),
            b.node,
            d.content.x,
            d.content.y,
            d.content.width,
            d.content.height,
            d.margin.top,
            d.margin.right,
            d.margin.bottom,
            d.margin.left,
            b.text,
        ));
        for c in &b.children {
            walk(c, depth + 1, out);
        }
    }
    let mut out = Vec::new();
    walk(root, 0, &mut out);
    out
}

#[track_caller]
fn assert_same_geometry(case: &str, expected: &LayoutBox, actual: &LayoutBox) {
    let (e, a) = (geometry(expected), geometry(actual));
    if let Some(i) = (0..e.len().max(a.len())).find(|&i| e.get(i) != a.get(i)) {
        panic!(
            "{case}: relayout diverged from a fresh layout at box {i} of {} / {}\n  \
             fresh:    {}\n  relayout: {}",
            e.len(),
            a.len(),
            e.get(i).map_or("<none>", String::as_str),
            a.get(i).map_or("<none>", String::as_str),
        );
    }
}

/// Relaying a built tree out, also after a detour through another
/// viewport width, reproduces the fresh layout box for box.
#[test]
fn corpus_relayout_matches_fresh_layout() {
    for name in fixture_names() {
        let html = fixture(&name);
        for (w, h) in WINDOWS {
            let b = loaded(&html, w, h);
            let fresh = b.layout_root.clone().expect("laid out");
            let (vw, vh) = viewport(&b);

            let mut again = fresh.clone();
            layout::block::relayout_tree(&mut again, &SimpleTextMeasurer, vw, vh);
            assert_same_geometry(&format!("{name} {w}x{h} relayout"), &fresh, &again);

            // Another width first: line breaks, flex and grid sizes and
            // shrink-to-fit widths all change, then must come back.
            let mut detour = fresh.clone();
            layout::block::relayout_tree(&mut detour, &SimpleTextMeasurer, vw * 0.55, vh);
            layout::block::relayout_tree(&mut detour, &SimpleTextMeasurer, vw, vh);
            assert_same_geometry(&format!("{name} {w}x{h} width detour"), &fresh, &detour);
        }
    }
}

/// Every `<img>` src in `doc`, decoded at a fixed size.
fn fake_image_info(doc: &html::dom::Document, base: &str) -> HashMap<String, (u32, u32)> {
    let base = loader::Url::parse(base).expect("base url");
    let mut info = HashMap::new();
    for node in &doc.nodes {
        if let html::dom::NodeKind::Element(e) = &node.kind
            && e.tag == html::dom::TagName::Img
            && let Some(src) = e.src()
            && let Some(url) = base.resolve(src)
        {
            info.insert(url.to_string(), (64, 48));
        }
    }
    info
}

/// The image-decode relayout (sizes patched into the built tree, then
/// laid out again) matches building the tree with the sizes known.
fn assert_image_relayout_matches(case: &str, html: &str, w: u32, h: u32) -> bool {
    let url = "vfs://corpus/page.html";
    let b = loaded(html, w, h);
    let (vw, vh) = viewport(&b);
    let doc = b.document.as_ref().expect("document");
    let info = fake_image_info(doc, url);
    assert!(!info.is_empty(), "{case}: has images");

    let mut patched = b.layout_root.clone().expect("laid out");
    // (False when every image already has both size attributes.)
    let changed =
        crate::widget::images::update_image_dimensions(&mut patched, doc, Some(url), &info);
    layout::block::relayout_tree(&mut patched, &SimpleTextMeasurer, vw, vh);

    let fresh = layout::block::build_layout_tree(
        doc,
        &b.styles,
        &SimpleTextMeasurer,
        vw,
        vh,
        Some(url),
        &info,
    );
    assert_same_geometry(case, &fresh, &patched);
    changed
}

#[test]
fn corpus_image_relayout_matches_fresh_layout() {
    let mut checked = 0;
    for name in fixture_names() {
        let html = fixture(&name);
        if !html.contains("<img") {
            continue;
        }
        for (w, h) in WINDOWS {
            assert_image_relayout_matches(&format!("{name} {w}x{h}"), &html, w, h);
            checked += 1;
        }
    }
    assert!(checked > 0, "no corpus fixture has an <img>");
}

/// Images in the contexts most sensitive to a second layout: inline
/// text that wraps, a flex row, a column flex item that grows, a table
/// cell, a float and an inline-block.
#[test]
fn image_relayout_matches_fresh_layout_in_every_context() {
    let html = "<html><body>\
        <p>Some words before <img src='a.png'> and a long run of words after it that \
        has to wrap onto another line in a narrow window.</p>\
        <div style='display:flex;gap:4px'><img src='b.png'><div>beside the image \
        text</div><div style='flex:1'>grows <img src='c.png' width='20'></div></div>\
        <div style='display:flex;flex-direction:column;height:300px'>\
        <div style='flex-grow:1'>column <img src='d.png'> item</div><div>tail</div></div>\
        <table><tr><td><img src='e.png'></td><td>cell text</td></tr></table>\
        <div style='float:left'><img src='f.png' height='10'></div>\
        <span style='display:inline-block'>ib <img src='g.png'></span>\
        <pre>pre line one\npre <img src='h.png'> line two</pre>\
        </body></html>";
    for (w, h) in WINDOWS {
        assert!(
            assert_image_relayout_matches(&format!("contexts {w}x{h}"), html, w, h),
            "image sizes changed"
        );
    }
}
