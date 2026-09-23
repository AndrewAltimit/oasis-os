//! SDL3 vs software-rasterizer conformance.
//!
//! Runs every scenario of `oasis_test_backend::conformance` on the SDL
//! backend (headless: offscreen video + software renderer, see
//! `common/mod.rs`) and on the UE5 software framebuffer (the
//! `oasis-rasterize` path the FFI embeds and the shell e2e harness renders
//! with), checks each frame against the scenario's absolute expectations,
//! and compares the two frames within the scenario's tolerance.
//!
//! On failure the frames are dumped as PNGs under
//! `target/tmp/backend-parity/` (`<scenario>-sdl.png`, `-ue5.png`,
//! `-diff.png`).

mod common;

use oasis_backend_sdl::SdlBackend;
use oasis_backend_sdl::shader_bridge::SdlShaderBridge;
use oasis_backend_ue5::Ue5Backend;
use oasis_shader::ShaderParams;
use oasis_shader::software::SoftwareShaderRenderer;
use oasis_test_backend::conformance::{
    self, CANVAS_H, CANVAS_W, Frame, SCENARIOS, Tolerance, stride_pattern,
};
use oasis_types::backend::{Color, SdiBackend, SdiCore};

use common::{diff_frame, dump_dir, save_png, with_sdl};

fn dump(name: &str, sdl: Option<&Frame>, ue5: Option<&Frame>, channel: u8) {
    let dir = dump_dir("backend-parity");
    if let Some(f) = sdl {
        save_png(&dir.join(format!("{name}-sdl.png")), f);
    }
    if let Some(f) = ue5 {
        save_png(&dir.join(format!("{name}-ue5.png")), f);
    }
    if let (Some(a), Some(b)) = (sdl, ue5) {
        save_png(
            &dir.join(format!("{name}-diff.png")),
            &diff_frame(a, b, channel),
        );
    }
}

#[test]
fn scenarios_match_software_rasterizer() {
    conformance::assert_unique_names();
    let failures = with_sdl(CANVAS_W, CANVAS_H, |sdl| {
        let mut ue5 = Ue5Backend::new(CANVAS_W, CANVAS_H);
        let mut failures = Vec::new();
        for s in SCENARIOS {
            let a = conformance::render(sdl as &mut dyn SdiBackend, s);
            let b = conformance::render(&mut ue5 as &mut dyn SdiBackend, s);
            let (a, b) = match (a, b) {
                (Ok(a), Ok(b)) => (a, b),
                (a, b) => {
                    failures.push(format!(
                        "{}: draw failed (sdl: {:?}, ue5: {:?})",
                        s.name,
                        a.err(),
                        b.err()
                    ));
                    continue;
                },
            };
            let mut bad = false;
            if let Some(check) = s.check {
                for (label, f) in [("sdl", &a), ("ue5", &b)] {
                    if let Err(e) = check(f) {
                        failures.push(format!("{} [{label}]: {e}", s.name));
                        bad = true;
                    }
                }
            }
            let d = a.diff(&b, s.tolerance);
            if !d.within(s.tolerance) {
                failures.push(format!(
                    "{}: sdl vs ue5 differ in {} px (max delta {}, allowed {:?}); \
                     first (x, y, sdl, ue5): {:?}",
                    s.name, d.mismatched, d.max_delta, s.tolerance, d.samples
                ));
                bad = true;
            }
            if bad {
                dump(s.name, Some(&a), Some(&b), s.tolerance.channel);
            }
        }
        failures
    });
    assert!(
        failures.is_empty(),
        "{} conformance failure(s) (renderer: {:?}; dumps in {}):\n{}",
        failures.len(),
        common::render_driver().unwrap_or_else(|| "software".into()),
        dump_dir("backend-parity").display(),
        failures.join("\n")
    );
}

#[test]
fn text_metrics_match_software_rasterizer() {
    let (sdl_m, ue5_m) = with_sdl(64, 64, |sdl| {
        let ue5 = Ue5Backend::new(64, 64);
        (
            conformance::text_metrics(sdl as &dyn SdiBackend),
            conformance::text_metrics(&ue5 as &dyn SdiBackend),
        )
    });
    let diffs: Vec<String> = sdl_m
        .iter()
        .zip(&ue5_m)
        .filter(|(a, b)| a.1 != b.1)
        .map(|(a, b)| format!("{}: sdl {} vs ue5 {}", a.0, a.1, b.1))
        .collect();
    assert!(
        diffs.is_empty(),
        "text metrics differ:\n{}",
        diffs.join("\n")
    );
}

/// Read the canvas back and compare it with the RGB of `expected`
/// (`w` x `h`, drawn at the origin).
fn assert_shows(sdl: &SdlBackend, expected: &[u8], w: u32, h: u32, what: &str) {
    let got = sdl.read_pixels(0, 0, w, h).expect("read_pixels");
    assert_eq!(got.len(), expected.len(), "{what}: readback size");
    let mut bad = 0usize;
    let mut first = None;
    for (i, (g, e)) in got
        .as_chunks::<4>()
        .0
        .iter()
        .zip(expected.as_chunks::<4>().0.iter())
        .enumerate()
    {
        if g[..3] != e[..3] {
            bad += 1;
            first.get_or_insert((
                i as u32 % w,
                i as u32 / w,
                [g[0], g[1], g[2]],
                [e[0], e[1], e[2]],
            ));
        }
    }
    assert_eq!(
        bad, 0,
        "{what}: {bad} px differ, first (x, y, got, want): {first:?}"
    );
}

/// `update_texture` (the streaming refresh the shader wallpaper and video
/// paths use) must honour the locked pitch exactly like `load_texture`,
/// for every row size including ones no GPU pads to.
#[test]
fn texture_update_honours_pitch_at_odd_widths() {
    with_sdl(340, 24, |sdl| {
        for w in [1u32, 3, 37, 63, 65, 250, 333, 337] {
            let h = 7;
            let first = stride_pattern(w, h);
            let tex = sdl.load_texture(w, h, &first).expect("load");
            sdl.clear(Color::BLACK).expect("clear");
            sdl.blit(tex, 0, 0, w, h).expect("blit");
            assert_shows(sdl, &first, w, h, &format!("load_texture {w}x{h}"));

            let mut second = first.clone();
            for (i, px) in second.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                px[0] = px[0].wrapping_add(97);
                px[2] = (i * 13) as u8;
            }
            sdl.update_texture(tex, w, h, &second).expect("update");
            sdl.clear(Color::BLACK).expect("clear");
            sdl.blit(tex, 0, 0, w, h).expect("blit");
            assert_shows(sdl, &second, w, h, &format!("update_texture {w}x{h}"));

            // A size mismatch is rejected, never a sheared upload.
            assert!(
                sdl.update_texture(tex, w + 1, h, &stride_pattern(w + 1, h))
                    .is_err()
            );
            sdl.destroy_texture(tex).expect("destroy");
        }
    });
}

/// The shader wallpaper bridge streams CPU-shaded frames through
/// `load_texture` (first frame), `update_texture` (every later shade) and
/// destroy + re-create (resize). Every path must put the shader's exact
/// pixels on screen at odd sizes.
#[test]
fn shader_bridge_streams_exact_pixels() {
    let (w, h) = (333u32, 61u32);
    with_sdl(340, 80, |sdl| {
        let params = ShaderParams::default();
        let mut reference = SoftwareShaderRenderer::new(w, h);
        let mut bridge = SdlShaderBridge::new(w, h).expect("bridge");
        for (i, &t) in [0.0f32, 1.25, 2.5].iter().enumerate() {
            sdl.clear(Color::BLACK).expect("clear");
            bridge.render_and_blit(sdl, "balatro", t, &params);
            let want = reference.render_shader("balatro", t, &params).to_vec();
            assert_shows(sdl, &want, w, h, &format!("shade pass {i} (t={t})"));
        }
        // Resize to another odd size: the stale texture is dropped and a
        // new one uploaded at the new pitch.
        let (w2, h2) = (250u32, 33u32);
        bridge.resize(w2, h2);
        let mut reference = SoftwareShaderRenderer::new(w2, h2);
        sdl.clear(Color::BLACK).expect("clear");
        bridge.render_and_blit(sdl, "balatro", 3.0, &params);
        let want = reference.render_shader("balatro", 3.0, &params).to_vec();
        assert_shows(sdl, &want, w2, h2, "after resize");
        bridge.destroy(sdl);
    });
}

/// `read_pixels` returns exactly `w * h * 4` bytes even when the region
/// hangs off the canvas (callers index it as a `w`-wide image).
#[test]
fn read_pixels_partial_region_matches() {
    let (a, b) = with_sdl(CANVAS_W, CANVAS_H, |sdl| {
        let mut ue5 = Ue5Backend::new(CANVAS_W, CANVAS_H);
        let draw = |be: &mut dyn SdiBackend| {
            be.clear(Color::rgb(10, 20, 30)).expect("clear");
            be.fill_rect(0, 0, 8, 8, Color::rgb(250, 0, 0))
                .expect("fill");
            be.fill_rect(
                CANVAS_W as i32 - 8,
                CANVAS_H as i32 - 8,
                8,
                8,
                Color::rgb(0, 250, 0),
            )
            .expect("fill");
        };
        draw(sdl);
        draw(&mut ue5);
        let regions = [
            (-4, -4, 12u32, 12u32),
            (CANVAS_W as i32 - 6, CANVAS_H as i32 - 6, 12, 12),
            (4, 4, 5, 3),
        ];
        let a: Vec<Vec<u8>> = regions
            .iter()
            .map(|&(x, y, w, h)| sdl.read_pixels(x, y, w, h).expect("sdl read"))
            .collect();
        let b: Vec<Vec<u8>> = regions
            .iter()
            .map(|&(x, y, w, h)| ue5.read_pixels(x, y, w, h).expect("ue5 read"))
            .collect();
        (a, b)
    });
    for (i, (ra, rb)) in a.iter().zip(&b).enumerate() {
        assert_eq!(ra.len(), rb.len(), "region {i}: readback length");
        let rgb = |v: &Vec<u8>| -> Vec<[u8; 3]> {
            v.as_chunks::<4>()
                .0
                .iter()
                .map(|p| [p[0], p[1], p[2]])
                .collect()
        };
        assert_eq!(rgb(ra), rgb(rb), "region {i}: readback pixels");
    }
}

/// Tolerances above zero must be justified in the scenario itself.
#[test]
fn inexact_scenarios_document_why() {
    for s in SCENARIOS {
        if s.tolerance != Tolerance::EXACT {
            assert!(!s.note.is_empty(), "{}: tolerance without a note", s.name);
        }
    }
}
