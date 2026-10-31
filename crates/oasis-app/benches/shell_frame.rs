//! Whole-shell frame benchmarks on the UE5 software framebuffer (the
//! headless harness backend, i.e. the CPU rasterizer UE5 and the e2e
//! tests present through).
//!
//! * `render/*` -- one full frame (`Shell::render`: wallpaper, dashboard
//!   or desktop, bars, windows, present) with nothing elided.
//! * `idle_step/*` -- one main-loop iteration on an idle screen: input,
//!   app ticks, SDI sync and the idle-frame elision check; the frame is
//!   rendered only when the shell asks for it.
//!
//! Run with:
//! ```sh
//! cargo bench -p oasis-app --bench shell_frame
//! ```

// Benchmark setup panics on failure, like a test.
#![allow(clippy::unwrap_used)]

use criterion::{Criterion, criterion_group, criterion_main};
use oasis_app::harness::{Harness, HarnessOptions};

fn boot(skin: &str, res: (u32, u32), apps: &[&str]) -> Harness {
    let mut opts = HarnessOptions::new(skin);
    opts.resolution = Some(res);
    let mut h = Harness::with_options(opts).unwrap();
    for app in apps {
        assert!(h.open_app(app), "open {app}");
    }
    h.settle();
    h
}

type Case = (
    &'static str,
    &'static str,
    (u32, u32),
    &'static [&'static str],
);

const CASES: [Case; 4] = [
    ("classic_480x272_dashboard", "classic", (480, 272), &[]),
    ("classic_1024x768_dashboard", "classic", (1024, 768), &[]),
    ("xp_1024x768_desktop", "xp", (1024, 768), &[]),
    (
        "xp_1024x768_two_windows",
        "xp",
        (1024, 768),
        &["Terminal", "File Manager"],
    ),
];

fn bench_shell_frame(c: &mut Criterion) {
    let mut group = c.benchmark_group("shell_frame");
    group.sample_size(30);
    for (label, skin, res, apps) in CASES {
        let mut h = boot(skin, res, apps);
        group.bench_function(format!("render/{label}"), |b| b.iter(|| h.render_now()));
        group.bench_function(format!("idle_step/{label}"), |b| b.iter(|| h.step(&[])));
    }
    group.finish();
}

criterion_group!(benches, bench_shell_frame);
criterion_main!(benches);
