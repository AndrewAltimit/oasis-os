//! Benchmarks for the `SoftwareBuffer` rasterizer hot paths.
//!
//! Covers the texture blit variants (1:1 and scaled), bitmap text, the
//! gradient fills, and translucent rounded rects -- the operations the UE5,
//! PSP-fallback and compositor paths lean on every frame.

use criterion::{Criterion, criterion_group, criterion_main};
use oasis_rasterize::SoftwareBuffer;
use oasis_types::backend::Color;
use std::hint::black_box;

/// Deterministic RGBA texture. `opaque` forces every alpha byte to 255.
fn texture(w: u32, h: u32, opaque: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    let mut s = 0x1234_5678u32;
    for _ in 0..w * h {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        let [r, g, b, a] = s.to_le_bytes();
        out.extend_from_slice(&[r, g, b, if opaque { 255 } else { a }]);
    }
    out
}

fn bench_blits(c: &mut Criterion) {
    let mut group = c.benchmark_group("blit");
    let opaque = texture(256, 256, true);
    let translucent = texture(256, 256, false);
    let small = texture(128, 128, true);
    let mut buf = SoftwareBuffer::new(480, 320);

    group.bench_function("1to1_opaque_256", |b| {
        b.iter(|| buf.blit_texture(black_box(&opaque), 256, 256, 10, 10, 256, 256));
    });
    group.bench_function("1to1_translucent_256", |b| {
        b.iter(|| buf.blit_texture(black_box(&translucent), 256, 256, 10, 10, 256, 256));
    });
    group.bench_function("scaled_128_to_256", |b| {
        b.iter(|| buf.blit_texture(black_box(&small), 128, 128, 10, 10, 256, 256));
    });
    group.bench_function("scaled_256_to_300x200", |b| {
        b.iter(|| buf.blit_texture(black_box(&opaque), 256, 256, 10, 10, 300, 200));
    });
    group.bench_function("tinted_scaled_128_to_256", |b| {
        let tint = Color::rgba(200, 180, 255, 230);
        b.iter(|| buf.blit_texture_tinted(black_box(&small), 128, 128, 10, 10, 256, 256, tint));
    });
    group.bench_function("sub_1to1_opaque_128", |b| {
        b.iter(|| buf.blit_texture_sub(black_box(&opaque), 256, 64, 64, 128, 128, 5, 5, 128, 128));
    });
    group.bench_function("flipped_h_256", |b| {
        b.iter(|| {
            buf.blit_texture_flipped(black_box(&opaque), 256, 256, 0, 0, 256, 256, true, false)
        });
    });
    group.finish();
}

fn bench_shapes(c: &mut Criterion) {
    let mut group = c.benchmark_group("shapes");
    let mut buf = SoftwareBuffer::new(480, 272);
    let text = "The quick brown fox jumps over the lazy dog 0123456789";

    group.bench_function("bitmap_text_16px", |b| {
        b.iter(|| {
            buf.draw_text(
                black_box(text),
                4,
                40,
                16,
                Color::rgba(255, 255, 255, 255),
                false,
                false,
            )
        });
    });
    group.bench_function("rounded_rect_translucent_300x200_r24", |b| {
        b.iter(|| buf.fill_rounded_rect(20, 20, 300, 200, 24, Color::rgba(40, 80, 160, 128)));
    });
    group.bench_function("hgradient_256", |b| {
        let (l, r) = (Color::rgb(255, 0, 0), Color::rgb(0, 0, 255));
        b.iter(|| buf.fill_rect_horizontal_gradient(0, 0, 256, 256, l, r));
    });
    group.bench_function("four_corner_gradient_256", |b| {
        let (a, bb) = (Color::rgb(255, 0, 0), Color::rgb(0, 255, 0));
        let (cc, d) = (Color::rgb(0, 0, 255), Color::rgb(255, 255, 0));
        b.iter(|| buf.fill_rect_four_corner_gradient(0, 0, 256, 256, a, bb, cc, d));
    });
    group.bench_function("clear_480x272", |b| {
        b.iter(|| buf.clear(black_box(Color::rgb(10, 20, 30))));
    });
    group.bench_function("fill_rect_opaque_480x272", |b| {
        b.iter(|| buf.fill_rect(0, 0, 480, 272, black_box(Color::rgb(10, 20, 30))));
    });
    group.bench_function("fill_rect_translucent_480x272", |b| {
        b.iter(|| buf.fill_rect(0, 0, 480, 272, black_box(Color::rgba(10, 20, 30, 128))));
    });
    group.bench_function("fill_circle_translucent_r100", |b| {
        b.iter(|| buf.fill_circle(240, 136, 100, Color::rgba(200, 40, 40, 160)));
    });
    group.bench_function("stroke_circle_r100_w4", |b| {
        b.iter(|| buf.stroke_circle(240, 136, 100, 4, Color::rgb(200, 40, 40)));
    });
    group.bench_function("draw_line_diag_w3", |b| {
        b.iter(|| buf.draw_line(0, 0, 479, 271, 3, Color::rgb(255, 255, 0)));
    });
    group.bench_function("fill_triangle_large", |b| {
        b.iter(|| buf.fill_triangle((10, 10), (470, 40), (200, 260), Color::rgb(0, 200, 0)));
    });
    group.bench_function("rounded_rect_hgradient_300x200_r16", |b| {
        let g = oasis_types::backend::GradientStyle::Horizontal {
            left: Color::rgb(255, 0, 0),
            right: Color::rgb(0, 0, 255),
        };
        b.iter(|| buf.fill_rounded_rect_gradient(20, 20, 300, 200, 16, black_box(&g)));
    });
    group.bench_function("rounded_rect_4corner_300x200_r16", |b| {
        let g = oasis_types::backend::GradientStyle::FourCorner {
            top_left: Color::rgb(255, 0, 0),
            top_right: Color::rgb(0, 255, 0),
            bottom_left: Color::rgb(0, 0, 255),
            bottom_right: Color::rgb(255, 255, 0),
        };
        b.iter(|| buf.fill_rounded_rect_gradient(20, 20, 300, 200, 16, black_box(&g)));
    });
    group.bench_function("bitmap_text_8px_screen", |b| {
        // A screenful of 8 px terminal text: 30 lines of 80 columns.
        let line = "drwxr-xr-x  2 oasis oasis  4096 Oct 31 12:00 some_directory_name_here";
        b.iter(|| {
            for row in 0..30 {
                buf.draw_text(black_box(line), 2, row * 9, 8, Color::WHITE, false, false);
            }
        });
    });
    group.finish();
}

criterion_group!(benches, bench_blits, bench_shapes);
criterion_main!(benches);
