//! Benchmark for a full dashboard frame: animation tick, SDI object sync
//! and registry draw. This is the shell's hottest per-frame path and the
//! regression gate for dashboard/theming performance work.

// Benchmark setup panics on failure, like a test.
#![allow(clippy::unwrap_used)]

use criterion::{Criterion, criterion_group, criterion_main};
use oasis_core::dashboard::{AppEntry, DashboardConfig, DashboardState};
use oasis_sdi::SdiRegistry;
use oasis_skin::{ActiveTheme, SkinFeatures, SkinTheme};
use oasis_test_backend::NullBackend;
use oasis_types::backend::Color;

fn make_apps(n: usize) -> Vec<AppEntry> {
    (0..n)
        .map(|i| AppEntry {
            title: format!("App Number {i}"),
            path: format!("/apps/app_{i}"),
            icon_png: Vec::new(),
            color: Color::rgb((i * 20 % 255) as u8, 120, 200),
        })
        .collect()
}

/// One dashboard frame as the desktop main loop runs it: tick animations,
/// sync every icon's SDI objects, then draw base + overlay layers.
fn bench_dashboard_frame(c: &mut Criterion) {
    let features = SkinFeatures::default(); // 5x3 grid, 15 icons/page
    let at = ActiveTheme::from_skin(&SkinTheme::default());
    let config = DashboardConfig::from_features(&features, &at);
    let mut dash = DashboardState::new(config, make_apps(45)); // 3 pages
    let mut sdi = SdiRegistry::new();
    let mut backend = NullBackend::default();

    c.bench_function("dashboard_frame/steady_state", |b| {
        b.iter(|| {
            dash.tick_animation();
            dash.update_sdi(&mut sdi, &at);
            sdi.draw_base_layer(&mut backend).unwrap();
            sdi.draw_overlay_layer(&mut backend).unwrap();
        });
    });

    // Frame during a page-slide animation (every icon moves every frame).
    c.bench_function("dashboard_frame/page_slide", |b| {
        b.iter(|| {
            // Restart the slide each iteration so every measured frame is
            // mid-animation with per-icon offset math active.
            dash.next_page();
            dash.tick_animation();
            dash.update_sdi(&mut sdi, &at);
            sdi.draw_base_layer(&mut backend).unwrap();
            sdi.draw_overlay_layer(&mut backend).unwrap();
        });
    });
}

/// The full per-frame Dashboard-mode SDI work the desktop main loop does:
/// the hide passes for terminal/app objects (which run every frame while
/// those modes are inactive), the dashboard sync, and the idle-frame
/// elision dirty check. On an idle dashboard this should end with a clean
/// scene and no signature hash.
fn bench_dashboard_frame_full(c: &mut Criterion) {
    use oasis_app_core::ContentState;
    use oasis_core::apps::{AppRegistry, AppRunner};
    use oasis_core::terminal_sdi;

    let features = SkinFeatures::default();
    let at = ActiveTheme::from_skin(&SkinTheme::default()).with_screen_size(800, 600);
    let config = DashboardConfig::from_features(&features, &at);
    let mut dash = DashboardState::new(config, make_apps(45));
    let mut sdi = SdiRegistry::new();
    let mut backend = NullBackend::default();

    // Populate the terminal and app object pools the hide passes walk.
    let lines: Vec<String> = (0..200).map(|i| format!("line {i}")).collect();
    terminal_sdi::setup_terminal_objects(&mut sdi, &lines, "/", "", 0, &at, true);
    let mut content = ContentState::new("Files", "/apps/files");
    content.lines = lines.clone();
    oasis_app_core::render::render_content_sdi(&content, &mut sdi, &at);

    let mut last_sig = None;
    c.bench_function("dashboard_frame/steady_state_full", |b| {
        b.iter(|| {
            terminal_sdi::set_terminal_visible(&mut sdi, false);
            AppRunner::hide_sdi(&AppRegistry::new(), &mut sdi);
            dash.tick_animation();
            dash.update_sdi(&mut sdi, &at);
            let changed = if sdi.take_scene_dirty() {
                let sig = sdi.scene_signature();
                let changed = last_sig != Some(sig);
                last_sig = Some(sig);
                changed
            } else {
                false
            };
            if changed {
                sdi.draw_base_layer(&mut backend).unwrap();
                sdi.draw_overlay_layer(&mut backend).unwrap();
            }
        });
    });
}

criterion_group!(benches, bench_dashboard_frame, bench_dashboard_frame_full);
criterion_main!(benches);
