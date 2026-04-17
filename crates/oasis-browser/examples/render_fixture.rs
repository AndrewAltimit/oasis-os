//! Render a browser fixture to PNG for visual inspection.
//!
//! Usage:
//!   cargo run -p oasis-browser --example render_fixture -- \
//!       tests/fixtures/reddit_listing.html /tmp/out.png 800 600
//!
//! Uses the UE5 backend's software RGBA rasterizer so we get real
//! pixels, not just a display list.

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::PathBuf;

use oasis_backend_ue5::Ue5Backend;
use oasis_browser::SimpleTextMeasurer;
use oasis_browser::internals::{
    CascadeContext, Document, MediaViewport, NodeKind, PaintViewport, Stylesheet, TagName,
    Tokenizer, TreeBuilder, build_layout_tree, default_stylesheet, paint_page, parse_inline_style,
    style_tree,
};
use oasis_types::backend::SdiCore;

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

fn collect_inline_styles(
    doc: &Document,
) -> Vec<(usize, Vec<oasis_browser::internals::ParsedDeclaration>)> {
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    let fixture_path = PathBuf::from(
        args.get(1)
            .cloned()
            .unwrap_or_else(|| "crates/oasis-browser/tests/fixtures/reddit_listing.html".into()),
    );
    let out_path = PathBuf::from(
        args.get(2)
            .cloned()
            .unwrap_or_else(|| "/tmp/out.png".to_string()),
    );
    let width: u32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(800);
    let height: u32 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(600);

    let html = fs::read_to_string(&fixture_path)?;

    let tokens = Tokenizer::new(&html).tokenize();
    let doc = TreeBuilder::build(tokens);

    let viewport = MediaViewport {
        width: width as f32,
        height: height as f32,
        dark_mode: false,
        prefers_reduced_motion: false,
        hover: true,
        pointer: "fine",
    };
    let author_sheets = collect_style_sheets(&doc, viewport);
    let inline_styles = collect_inline_styles(&doc);
    let ua = default_stylesheet();
    let mut sheets: Vec<&Stylesheet> = vec![ua];
    for sheet in &author_sheets {
        sheets.push(sheet);
    }
    let ctx = CascadeContext::default();
    let styles = style_tree(&doc, &sheets, &inline_styles, &ctx);

    let measurer = SimpleTextMeasurer;
    let layout = build_layout_tree(
        &doc,
        &styles,
        &measurer,
        width as f32,
        height as f32,
        None,
        &HashMap::new(),
    );

    let mut backend = Ue5Backend::new(width, height);
    // Paint the page background first so transparent regions don't stay alpha=0.
    backend
        .clear(oasis_types::backend::Color {
            r: 255,
            g: 255,
            b: 255,
            a: 255,
        })
        .ok();

    let vp = PaintViewport {
        scroll_y: 0.0,
        scroll_x: 0.0,
        x: 0,
        y: 0,
        width: width as f32,
        height: height as f32,
        visible_height: height as f32,
        focused_node: None,
        counter_styles: Vec::new(),
    };
    paint_page(&layout, &mut backend, vp, &HashMap::new())?;

    // Serialize to PNG.
    let rgba = backend.buffer();
    let file = fs::File::create(&out_path)?;
    let mut encoder = png::Encoder::new(file, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(rgba)?;

    println!(
        "Rendered {} → {} ({width}x{height})",
        fixture_path.display(),
        out_path.display()
    );
    Ok(())
}
