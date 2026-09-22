//! Per-frame render cost of the Text Editor on a large source file.
//!
//! Scenario: a 10k-line C file scrolled to the end (Ctrl+End), redrawn
//! every frame without edits. The old renderer re-highlighted every line
//! above the viewport each frame to recover block-comment state; the cache
//! makes steady-state frames independent of the scroll position.

use criterion::{Criterion, criterion_group, criterion_main};
use oasis_app_core::App;
use oasis_app_text_editor::TextEditorApp;
use oasis_skin::ActiveTheme;
use oasis_types::backend::{
    Color, SdiAlpha, SdiBatch, SdiClipTransform, SdiCore, SdiGradients, SdiShapes, SdiText,
    SdiTextures, SdiVector, TextureId,
};
use oasis_types::error::Result;
use oasis_types::input::{Key, Modifiers};
use oasis_vfs::MemoryVfs;

/// A no-op backend: isolates the editor's own per-frame work.
struct NullBackend;

impl SdiCore for NullBackend {
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

impl SdiShapes for NullBackend {}
impl SdiGradients for NullBackend {}
impl SdiAlpha for NullBackend {}
impl SdiText for NullBackend {}
impl SdiTextures for NullBackend {}
impl SdiClipTransform for NullBackend {}
impl SdiVector for NullBackend {}
impl SdiBatch for NullBackend {}
impl oasis_types::backend::SdiRenderTarget for NullBackend {}

/// A C source file of `lines` lines with block comments, strings,
/// preprocessor directives and numbers.
fn c_source(lines: usize) -> String {
    let mut out = String::with_capacity(lines * 40);
    let mut i = 0;
    while i < lines {
        match i % 10 {
            0 => out.push_str("#include <stdio.h>\n"),
            1 => out.push_str("/* Block comment opening on this line\n"),
            2 => out.push_str("   still inside the comment */\n"),
            3 => out.push_str(&format!("static int value_{i} = {i};\n")),
            4 => out.push_str(&format!("int func_{i}(int a, char *s) {{\n")),
            5 => out.push_str("    printf(\"%d %s\\n\", a, s); // print\n"),
            6 => out.push_str("    for (int k = 0; k < 10; k++) { a += k * 3; }\n"),
            7 => out.push_str("    return a > 0 ? a : -a;\n"),
            8 => out.push_str("}\n"),
            _ => out.push('\n'),
        }
        i += 1;
    }
    out
}

fn bench_render(c: &mut Criterion) {
    let at = ActiveTheme::default();
    let vfs = MemoryVfs::new();
    let mut app = TextEditorApp::open_file("/bench.c", &c_source(10_000));
    let mut backend = NullBackend;
    // Draw once so the editor learns its viewport, then jump to the end.
    let _ = app.draw_windowed(0, 0, 800, 600, &mut backend, &at);
    let _ = app.handle_key(&Key::End, Modifiers::CTRL, &vfs);

    c.bench_function("text_editor_frame_10k_c_scrolled_to_end", |b| {
        b.iter(|| {
            app.draw_windowed(0, 0, 800, 600, &mut backend, &at)
                .expect("draw");
        });
    });
}

criterion_group!(benches, bench_render);
criterion_main!(benches);
