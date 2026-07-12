//! Benchmarks for the SDI draw path.
//!
//! Measures the per-frame CPU cost of `SdiRegistry::draw` and its variants
//! against a no-op backend, so regressions in z-sorting, name lookups, and
//! per-object dispatch are caught independently of any real renderer.

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use oasis_sdi::registry::SdiRegistry;
use oasis_types::backend::{
    Color, SdiAlpha, SdiBatch, SdiClipTransform, SdiCore, SdiGradients, SdiRenderTarget, SdiShapes,
    SdiText, SdiTextures, SdiVector, TextureId, bitmap_measure_text,
};
use oasis_types::error::Result;

/// A backend that accepts every draw call and does nothing, so the bench
/// measures registry-side cost only.
struct NullBackend;

impl SdiCore for NullBackend {
    fn init(&mut self, _width: u32, _height: u32) -> Result<()> {
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
    fn load_texture(&mut self, _width: u32, _height: u32, _rgba_data: &[u8]) -> Result<TextureId> {
        Ok(TextureId(1))
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
        bitmap_measure_text(text, font_size)
    }
    fn read_pixels(&self, _x: i32, _y: i32, w: u32, h: u32) -> Result<Vec<u8>> {
        Ok(vec![0u8; (w as usize) * (h as usize) * 4])
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
impl SdiRenderTarget for NullBackend {}

/// Build a dashboard-like scene: a mix of flat rects, gradient/rounded
/// panels, text labels, textured objects, and overlay-layer chrome in
/// roughly the proportions the shell produces (~9 objects per icon).
fn build_scene(n: usize) -> SdiRegistry {
    let mut reg = SdiRegistry::new();
    for i in 0..n {
        let name = format!("obj_{i}");
        let obj = reg.create(&name);
        obj.x = ((i * 37) % 480) as i32;
        obj.y = ((i * 53) % 272) as i32;
        obj.w = 40;
        obj.h = 40;
        obj.color = Color::rgb((i % 255) as u8, 128, 200);
        match i % 10 {
            // Gradient + rounded panel (icon bodies, cards).
            0 | 1 => {
                obj.border_radius = Some(4);
                obj.gradient_top = Some(Color::rgb(80, 90, 120));
                obj.gradient_bottom = Some(Color::rgb(40, 45, 60));
            },
            // Text labels (icon labels, bar text).
            2 | 3 => {
                obj.text = Some(format!("Label {i}"));
                obj.font_size = 8;
            },
            // Stroked + shadowed outline (selection, focus).
            4 => {
                obj.stroke_width = Some(1);
                obj.stroke_color = Some(Color::rgb(255, 255, 255));
                obj.shadow_level = Some(2);
            },
            // Textured blit (wallpaper tiles, icons).
            5 => {
                obj.texture = Some(TextureId(1));
            },
            // Overlay chrome (bars are drawn in the overlay pass).
            6 => {
                obj.overlay = true;
                obj.gradient_top = Some(Color::rgb(30, 40, 70));
                obj.gradient_bottom = Some(Color::rgb(20, 25, 45));
            },
            // Plain fills.
            _ => {},
        }
    }
    reg
}

fn bench_draw(c: &mut Criterion) {
    let mut group = c.benchmark_group("sdi_draw");
    let mut backend = NullBackend;

    for n in [120usize, 1_000] {
        let mut reg = build_scene(n);
        let label = format!("{n}");

        // Steady-state frame: z-cache is warm, nothing changed.
        group.bench_function(BenchmarkId::new("full", &label), |b| {
            b.iter(|| reg.draw(&mut backend).unwrap());
        });

        // Split passes as used by the dashboard/desktop render path.
        group.bench_function(BenchmarkId::new("split", &label), |b| {
            b.iter(|| {
                reg.draw_base_layer(&mut backend).unwrap();
                reg.draw_overlay_layer(&mut backend).unwrap();
            });
        });

        // Frame with a z-order change (forces re-sort each iteration).
        group.bench_function(BenchmarkId::new("z_dirty", &label), |b| {
            b.iter(|| {
                let _ = reg.move_to_top("obj_0");
                reg.draw(&mut backend).unwrap();
            });
        });
    }

    group.finish();
}

fn bench_draw_excluding_prefixes(c: &mut Criterion) {
    let mut group = c.benchmark_group("sdi_draw_excluding");
    let mut backend = NullBackend;

    // Window-compositing path: scene plus 3 windows' worth of prefixed
    // chrome objects, drawn with the windows excluded (as the WM does).
    let mut reg = build_scene(120);
    for w in 0..3 {
        for suffix in [
            "frame",
            "titlebar",
            "title_text",
            "btn_close",
            "btn_min",
            "btn_max",
            "content",
        ] {
            let obj = reg.create(&format!("win{w}.{suffix}"));
            obj.w = 100;
            obj.h = 80;
        }
    }
    let prefixes: Vec<String> = (0..3).map(|w| format!("win{w}.")).collect();
    let prefix_refs: Vec<&str> = prefixes.iter().map(String::as_str).collect();

    group.bench_function("3_windows_120_objects", |b| {
        b.iter(|| {
            reg.draw_base_excluding_prefixes(&mut backend, &prefix_refs)
                .unwrap();
            reg.draw_overlay_excluding_prefixes(&mut backend, &prefix_refs)
                .unwrap();
        });
    });

    group.finish();
}

criterion_group!(benches, bench_draw, bench_draw_excluding_prefixes);
criterion_main!(benches);
