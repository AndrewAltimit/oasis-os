//! `bench_dense_frame` — sanity benchmark for the `SdiBatch::submit_rect_batch`
//! override on the UE5 software backend.
//!
//! The 2x WASM gate from PR 4b cannot be measured on host because the
//! win is purely in the wasm-bindgen → JS boundary count, which doesn't
//! exist for native targets. On native backends the dominant cost is
//! the actual pixel writes inside `SoftwareBuffer::fill_rect`, not the
//! per-call dispatch — so we expect roughly *parity*, not a multiplier.
//! Anything significantly slower in the override is the signal we care
//! about (it would mean the batch path has a real overhead bug). Real
//! WASM perf is validated by hand in a browser against the wasm-pack
//! output.
//!
//! Run with:
//! ```sh
//! cargo bench -p oasis-backend-ue5 --bench dense_frame
//! ```

use criterion::{Criterion, criterion_group, criterion_main};

use oasis_backend_ue5::Ue5Backend;
use oasis_types::backend::{BatchRect, Color, SdiBatch, SdiCore};

fn dense_rects(n: usize) -> Vec<BatchRect> {
    (0..n)
        .map(|i| BatchRect {
            // Mosaic the rects across a 480x272 surface so the pixel
            // writes hit different cache lines instead of repeating the
            // same hot region (which would understate the win).
            x: ((i * 13) % 470) as i32,
            y: ((i * 7) % 262) as i32,
            w: 8,
            h: 8,
            color: Color::rgba(
                (i * 11) as u8 & 0xFF,
                (i * 17) as u8 & 0xFF,
                (i * 23) as u8 & 0xFF,
                255,
            ),
        })
        .collect()
}

fn bench_rect_batch_vs_loop(c: &mut Criterion) {
    let mut group = c.benchmark_group("rect_batch");

    for &n in &[50usize, 200, 500] {
        let rects = dense_rects(n);

        group.bench_function(format!("default_loop_{n}"), |b| {
            let mut backend = Ue5Backend::new(480, 272);
            backend.init(480, 272).unwrap();
            b.iter(|| {
                for r in &rects {
                    backend.fill_rect(r.x, r.y, r.w, r.h, r.color).unwrap();
                }
            });
        });

        group.bench_function(format!("submit_batch_{n}"), |b| {
            let mut backend = Ue5Backend::new(480, 272);
            backend.init(480, 272).unwrap();
            b.iter(|| {
                backend.submit_rect_batch(&rects).unwrap();
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_rect_batch_vs_loop);
criterion_main!(benches);
