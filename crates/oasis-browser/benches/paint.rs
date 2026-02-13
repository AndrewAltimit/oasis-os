//! Benchmarks for the paint layer.

use std::collections::HashMap;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use oasis_browser::SimpleTextMeasurer;
use oasis_browser::css::cascade::style_tree;
use oasis_browser::css::parser::Stylesheet;
use oasis_browser::html::tokenizer::Tokenizer;
use oasis_browser::html::tree_builder::TreeBuilder;
use oasis_browser::layout::block::build_layout_tree;
use oasis_browser::paint;
use oasis_types::backend::{Color, SdiBackend, TextureId};
use oasis_types::error::Result;

/// A no-op backend that does nothing -- isolates paint logic cost from rendering.
struct NullBackend;

impl SdiBackend for NullBackend {
    fn init(&mut self, _w: u32, _h: u32) -> Result<()> {
        Ok(())
    }
    fn clear(&mut self, _color: Color) -> Result<()> {
        Ok(())
    }
    fn blit(&mut self, _tex: TextureId, _x: i32, _y: i32, _w: u32, _h: u32) -> Result<()> {
        Ok(())
    }
    fn fill_rect(&mut self, _x: i32, _y: i32, _w: u32, _h: u32, _color: Color) -> Result<()> {
        Ok(())
    }
    fn draw_text(
        &mut self,
        _text: &str,
        _x: i32,
        _y: i32,
        _font_size: u16,
        _color: Color,
    ) -> Result<()> {
        Ok(())
    }
    fn swap_buffers(&mut self) -> Result<()> {
        Ok(())
    }
    fn load_texture(&mut self, _w: u32, _h: u32, _data: &[u8]) -> Result<TextureId> {
        Ok(TextureId(0))
    }
    fn destroy_texture(&mut self, _tex: TextureId) -> Result<()> {
        Ok(())
    }
    fn set_clip_rect(&mut self, _x: i32, _y: i32, _w: u32, _h: u32) -> Result<()> {
        Ok(())
    }
    fn reset_clip_rect(&mut self) -> Result<()> {
        Ok(())
    }
    fn measure_text(&self, text: &str, font_size: u16) -> u32 {
        oasis_types::backend::bitmap_measure_text(text, font_size)
    }
    fn read_pixels(&self, _x: i32, _y: i32, w: u32, h: u32) -> Result<Vec<u8>> {
        Ok(vec![0u8; (w * h * 4) as usize])
    }
    fn shutdown(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Generate a page with various elements that produce paint commands.
fn generate_mixed_page(n: usize) -> String {
    let mut html = String::from(
        "<html><head></head><body style=\"background: #ffffff;\">\n\
         <h1 style=\"color: #333; background: #eee; padding: 8px;\">Benchmark Page</h1>\n",
    );
    for i in 0..n {
        match i % 4 {
            0 => html.push_str(&format!(
                "<div style=\"background: #{:02x}{:02x}{:02x}; padding: 4px; margin: 2px;\">\
                 <p style=\"color: #000;\">Paragraph {i} with text content.</p></div>\n",
                (i * 7) % 256,
                (i * 13) % 256,
                (i * 17) % 256,
            )),
            1 => html.push_str(&format!(
                "<ul><li>List item {i}</li><li>Another item</li></ul>\n"
            )),
            2 => html.push_str(&format!(
                "<a href=\"page{i}\">Link {i}</a> <b>bold text</b> <i>italic text</i><br>\n"
            )),
            _ => html.push_str(&format!(
                "<table><tr><td>Cell A{i}</td><td>Cell B{i}</td></tr></table>\n"
            )),
        }
    }
    html.push_str("</body></html>");
    html
}

fn bench_paint(c: &mut Criterion) {
    let mut group = c.benchmark_group("paint");

    let css = "";
    let measurer = SimpleTextMeasurer;

    for n_elements in [50, 200, 500] {
        let html = generate_mixed_page(n_elements);
        let mut tokenizer = Tokenizer::new(&html);
        let tokens = tokenizer.tokenize();
        let doc = TreeBuilder::build(tokens);
        let stylesheet = Stylesheet::parse(css);
        let styles = style_tree(&doc, &[&stylesheet], &[]);
        let layout = build_layout_tree(&doc, &styles, &measurer, 480.0, 272.0);

        let link_map: HashMap<usize, String> = HashMap::new();
        let label = format!("{n_elements}_elements");

        group.bench_with_input(
            BenchmarkId::new("paint", &label),
            &(&layout, &link_map),
            |b, (layout, link_map)| {
                let mut backend = NullBackend;
                b.iter(|| paint::paint(layout, &mut backend, 0.0, 0, 0, 480.0, 272.0, link_map));
            },
        );
    }

    group.finish();
}

fn bench_full_pipeline(c: &mut Criterion) {
    let mut group = c.benchmark_group("full_pipeline");

    let measurer = SimpleTextMeasurer;

    for n_elements in [50, 200] {
        let html = generate_mixed_page(n_elements);
        let label = format!("{n_elements}_elements");

        group.bench_with_input(
            BenchmarkId::new("parse_layout_paint", &label),
            &html,
            |b, html| {
                let mut backend = NullBackend;
                b.iter(|| {
                    let mut tokenizer = Tokenizer::new(html);
                    let tokens = tokenizer.tokenize();
                    let doc = TreeBuilder::build(tokens);
                    let stylesheet = Stylesheet::parse("");
                    let styles = style_tree(&doc, &[&stylesheet], &[]);
                    let layout = build_layout_tree(&doc, &styles, &measurer, 480.0, 272.0);
                    let link_map: HashMap<usize, String> = HashMap::new();
                    paint::paint(&layout, &mut backend, 0.0, 0, 0, 480.0, 272.0, &link_map)
                });
            },
        );
    }

    group.finish();
}

criterion_group!(benches, bench_paint, bench_full_pipeline);
criterion_main!(benches);
