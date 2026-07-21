//! Criterion benchmarks for the software shader shade pass.
//!
//! Measures one full `render_shader` call (low-res shade + nearest-
//! neighbour upscale) at the desktop default resolution (1280x720 →
//! 427x240 internal cells) for every built-in shader. The SDL bridge
//! runs this at 30 Hz, so per-call time here is the dominant CPU cost
//! of an animated shader wallpaper.
//!
//! Run with `--features parallel` to measure the rayon row-parallel
//! path used by the desktop backends.

use criterion::{Criterion, criterion_group, criterion_main};
use oasis_shader::ShaderParams;
use oasis_shader::software::SoftwareShaderRenderer;
use std::hint::black_box;

const SHADERS: [&str; 8] = [
    "balatro",
    "voronoi",
    "city_lights",
    "ocean_waves",
    "calm_waves",
    "starfield",
    "plasma",
    "matrix_rain",
];

fn bench_shade_720p(c: &mut Criterion) {
    let mut group = c.benchmark_group("shader_720p");
    group.sample_size(20);
    let params = ShaderParams::default();
    for name in SHADERS {
        let mut renderer = SoftwareShaderRenderer::new(1280, 720);
        // Vary time per iteration so no shader can win via a degenerate
        // t=const fast path; the sequence is deterministic.
        let mut frame = 0u32;
        group.bench_function(name, |b| {
            b.iter(|| {
                frame = frame.wrapping_add(1);
                let t = frame as f32 / 30.0;
                black_box(renderer.render_shader(name, t, &params));
            })
        });
    }
    group.finish();
}

criterion_group!(benches, bench_shade_720p);
criterion_main!(benches);
