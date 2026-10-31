//! Tests for the software shader renderer.

use super::*;

#[test]
fn software_renderer_produces_pixels() {
    let mut renderer = SoftwareShaderRenderer::new(16, 16);
    let params = ShaderParams::default();
    let pixels = renderer.render_balatro(1.0, &params);
    assert_eq!(pixels.len(), 16 * 16 * 4);
    let has_color = pixels
        .chunks(4)
        .any(|px| px[0] > 0 || px[1] > 0 || px[2] > 0);
    assert!(has_color, "software renderer should produce colored pixels");
}

#[test]
fn software_renderer_resize() {
    let mut renderer = SoftwareShaderRenderer::new(8, 8);
    assert_eq!(renderer.pixel_buf.len(), 8 * 8 * 4);
    renderer.resize(16, 16);
    assert_eq!(renderer.pixel_buf.len(), 16 * 16 * 4);
}

#[test]
fn render_scale_stays_minimum_up_to_1440x810() {
    // Everything at or below the cap boundary keeps the fixed 1/3
    // scale — including the golden-checksum sizes below, whose
    // outputs must not change.
    assert_eq!(render_scale_for(64, 64), 3);
    assert_eq!(render_scale_for(241, 135), 3);
    assert_eq!(render_scale_for(480, 272), 3);
    assert_eq!(render_scale_for(1280, 720), 3);
    assert_eq!(render_scale_for(1440, 810), 3);
}

#[test]
fn render_scale_caps_lo_buffer_for_large_outputs() {
    // 1080p: scale 4 → 480x270 cells.
    assert_eq!(render_scale_for(1920, 1080), 4);
    // 4K: scale 8 → 480x270 cells, not 9x the 720p cost.
    assert_eq!(render_scale_for(3840, 2160), 8);
    let r = SoftwareShaderRenderer::new(3840, 2160);
    let (iw, ih, scale_f) = r.lo_dims();
    assert_eq!((iw, ih), (480, 270));
    assert_eq!(scale_f, 8.0);
}

#[test]
fn adaptive_scale_resize_updates_lo_dims() {
    let mut r = SoftwareShaderRenderer::new(1280, 720);
    assert_eq!(r.lo_dims().2, 3.0);
    r.resize(2560, 1440);
    let (iw, ih, scale_f) = r.lo_dims();
    assert_eq!(scale_f, 6.0);
    assert!(iw <= 480 && ih <= 270, "lo buffer exceeds cap: {iw}x{ih}");
    // Output buffer still matches the full resolution.
    let pixels = r.render_shader("plasma", 1.0, &ShaderParams::default());
    assert_eq!(pixels.len(), 2560 * 1440 * 4);
}

#[test]
fn software_renderer_deterministic() {
    let mut r1 = SoftwareShaderRenderer::new(8, 8);
    let mut r2 = SoftwareShaderRenderer::new(8, 8);
    let params = ShaderParams::default();
    let p1 = r1.render_balatro(0.5, &params).to_vec();
    let p2 = r2.render_balatro(0.5, &params).to_vec();
    assert_eq!(p1, p2, "same inputs should produce same output");
}

#[test]
fn voronoi_produces_pixels() {
    let mut r = SoftwareShaderRenderer::new(16, 16);
    let pixels = r.render_voronoi(1.0, &ShaderParams::default());
    assert_eq!(pixels.len(), 16 * 16 * 4);
    let has_color = pixels
        .chunks(4)
        .any(|px| px[0] > 0 || px[1] > 0 || px[2] > 0);
    assert!(has_color);
}

#[test]
fn city_lights_produces_pixels() {
    let mut r = SoftwareShaderRenderer::new(16, 16);
    let pixels = r.render_city_lights(1.0, &ShaderParams::default());
    assert_eq!(pixels.len(), 16 * 16 * 4);
    let has_color = pixels
        .chunks(4)
        .any(|px| px[0] > 0 || px[1] > 0 || px[2] > 0);
    assert!(has_color);
}

#[test]
fn ocean_waves_produces_pixels() {
    let mut r = SoftwareShaderRenderer::new(16, 16);
    let pixels = r.render_ocean_waves(1.0, &ShaderParams::default());
    assert_eq!(pixels.len(), 16 * 16 * 4);
    let has_color = pixels
        .chunks(4)
        .any(|px| px[0] > 0 || px[1] > 0 || px[2] > 0);
    assert!(has_color);
}

#[test]
fn calm_waves_produces_pixels() {
    let mut r = SoftwareShaderRenderer::new(16, 16);
    let pixels = r.render_calm_waves(1.0, &ShaderParams::default());
    assert_eq!(pixels.len(), 16 * 16 * 4);
    let has_color = pixels
        .chunks(4)
        .any(|px| px[0] > 0 || px[1] > 0 || px[2] > 0);
    assert!(has_color);
}

#[test]
fn starfield_produces_pixels() {
    let mut r = SoftwareShaderRenderer::new(16, 16);
    let pixels = r.render_starfield(1.0, &ShaderParams::default());
    assert_eq!(pixels.len(), 16 * 16 * 4);
}

#[test]
fn plasma_produces_pixels() {
    let mut r = SoftwareShaderRenderer::new(16, 16);
    let pixels = r.render_plasma(1.0, &ShaderParams::default());
    assert_eq!(pixels.len(), 16 * 16 * 4);
    let has_color = pixels
        .chunks(4)
        .any(|px| px[0] > 0 || px[1] > 0 || px[2] > 0);
    assert!(has_color);
}

#[test]
fn matrix_rain_produces_pixels() {
    let mut r = SoftwareShaderRenderer::new(16, 16);
    let pixels = r.render_matrix_rain(1.0, &ShaderParams::default());
    assert_eq!(pixels.len(), 16 * 16 * 4);
    let has_color = pixels
        .chunks(4)
        .any(|px| px[0] > 0 || px[1] > 0 || px[2] > 0);
    assert!(has_color);
}

#[test]
fn render_shader_dispatches_correctly() {
    let mut r = SoftwareShaderRenderer::new(8, 8);
    let params = ShaderParams::default();
    // Each shader should produce some output without panicking.
    for name in &[
        "balatro",
        "voronoi",
        "city_lights",
        "ocean_waves",
        "calm_waves",
        "starfield",
        "plasma",
        "matrix_rain",
    ] {
        let pixels = r.render_shader(name, 0.5, &params);
        assert_eq!(pixels.len(), 8 * 8 * 4);
    }
}

// -----------------------------------------------------------------------
// PSP-specific shader validation
// -----------------------------------------------------------------------
// These tests validate shaders at the 64x64 resolution used by the PSP
// backend, with skin-matched parameters, verifying correctness and
// performance without requiring the PSP target.

/// PSP shader skin configurations — mirrors `PspSkinPreset::shader_config()`
/// from oasis-backend-psp so we can test without the PSP crate dependency.
fn psp_shader_configs() -> Vec<(&'static str, &'static str, ShaderParams)> {
    use std::collections::HashMap;
    fn hex(r: u8, g: u8, b: u8) -> [f32; 4] {
        [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0]
    }
    vec![
        (
            "Balatro",
            "balatro",
            ShaderParams {
                colors: vec![
                    hex(0x00, 0xF0, 0xFF),
                    hex(0x00, 0x6B, 0xB4),
                    hex(0x16, 0x23, 0x25),
                ],
                floats: HashMap::from([
                    ("speed".into(), 1.0),
                    ("contrast".into(), 3.5),
                    ("spin_speed".into(), 1.0),
                    ("spin_amount".into(), 0.25),
                    ("pixel_filter".into(), 745.0),
                    ("lighting".into(), 0.4),
                    ("spin_ease".into(), 1.0),
                ]),
            },
        ),
        (
            "RetroCga",
            "voronoi",
            ShaderParams {
                colors: vec![hex(0x55, 0xFF, 0x55), hex(0xFF, 0x55, 0xFF)],
                floats: HashMap::from([("speed".into(), 0.5), ("size".into(), 6.0)]),
            },
        ),
        (
            "Solarized",
            "ocean_waves",
            ShaderParams {
                colors: vec![
                    hex(0x00, 0x2B, 0x36),
                    hex(0x00, 0xE0, 0xE0),
                    hex(0x58, 0x6E, 0x75),
                ],
                floats: HashMap::from([("speed".into(), 0.6)]),
            },
        ),
        (
            "Terminal",
            "matrix_rain",
            ShaderParams {
                colors: vec![hex(0x00, 0xFF, 0x00), hex(0x00, 0x02, 0x00)],
                floats: HashMap::from([("speed".into(), 0.8)]),
            },
        ),
        (
            "Altimit",
            "starfield",
            ShaderParams {
                colors: vec![hex(0x00, 0xCC, 0x88), hex(0x08, 0x08, 0x16)],
                floats: HashMap::from([("speed".into(), 0.6)]),
            },
        ),
        (
            "Tactical",
            "plasma",
            ShaderParams {
                colors: vec![
                    hex(0xCC, 0x88, 0x00),
                    hex(0x66, 0x33, 0x00),
                    hex(0x33, 0x1A, 0x00),
                ],
                floats: HashMap::from([("speed".into(), 0.4)]),
            },
        ),
    ]
}

#[test]
fn psp_shaders_render_at_64x64() {
    let mut r = SoftwareShaderRenderer::new(64, 64);
    for (skin, name, params) in psp_shader_configs() {
        let pixels = r.render_shader(name, 1.0, &params);
        assert_eq!(
            pixels.len(),
            64 * 64 * 4,
            "{skin} ({name}): wrong buffer size"
        );
    }
}

#[test]
fn psp_shaders_produce_visible_output() {
    let mut r = SoftwareShaderRenderer::new(64, 64);
    for (skin, name, params) in psp_shader_configs() {
        let pixels = r.render_shader(name, 1.0, &params);
        let has_color = pixels
            .chunks(4)
            .any(|px| px[0] > 10 || px[1] > 10 || px[2] > 10);
        assert!(has_color, "{skin} ({name}): all-black output");
    }
}

#[test]
fn psp_shaders_animate_over_time() {
    let mut r = SoftwareShaderRenderer::new(64, 64);
    for (skin, name, params) in psp_shader_configs() {
        let p0 = r.render_shader(name, 0.0, &params).to_vec();
        let p5 = r.render_shader(name, 5.0, &params).to_vec();
        assert_ne!(p0, p5, "{skin} ({name}): no animation between t=0 and t=5");
    }
}

#[test]
fn psp_shaders_deterministic() {
    let params = psp_shader_configs();
    for (skin, name, p) in &params {
        let mut r1 = SoftwareShaderRenderer::new(64, 64);
        let mut r2 = SoftwareShaderRenderer::new(64, 64);
        let a = r1.render_shader(name, 2.5, p).to_vec();
        let b = r2.render_shader(name, 2.5, p).to_vec();
        assert_eq!(a, b, "{skin} ({name}): non-deterministic output");
    }
}

#[test]
fn psp_shaders_no_nan_pixels() {
    let mut r = SoftwareShaderRenderer::new(64, 64);
    for (skin, name, params) in psp_shader_configs() {
        // Test at several time points including edge cases.
        for &t in &[0.0, 0.001, 1.0, 10.0, 100.0] {
            let pixels = r.render_shader(name, t, &params);
            // Alpha should be 255 for all pixels (opaque).
            let all_opaque = pixels.chunks(4).all(|px| px[3] == 255);
            assert!(
                all_opaque,
                "{skin} ({name}) t={t}: not all pixels are opaque"
            );
        }
    }
}

#[test]
fn psp_shader_render_performance() {
    // Gate on the *minimum* per-frame time rather than the mean:
    // scheduler preemption on a busy (shared/self-hosted CI) runner
    // only ever adds time, so the fastest of 30 frames approximates
    // the true cost while the mean can blow up 2-3x under load.
    // Debug builds get a wider budget — this test guards against
    // catastrophic (algorithmic) regressions, not codegen quality.
    let budget_us: u128 = if cfg!(debug_assertions) { 8000 } else { 2000 };
    let mut r = SoftwareShaderRenderer::new(64, 64);
    for (skin, name, params) in psp_shader_configs() {
        // Warm up.
        let _ = r.render_shader(name, 0.0, &params);

        let frames = 30;
        let mut min_us = u128::MAX;
        for i in 0..frames {
            let start = std::time::Instant::now();
            let _ = r.render_shader(name, i as f32 / 30.0, &params);
            min_us = min_us.min(start.elapsed().as_micros());
        }

        // On host, each 64x64 frame should be well under 1ms in
        // release. PSP is ~10-20x slower, so budget 2ms host =
        // ~20-40ms PSP. At 30fps shader update, 33ms budget per frame.
        assert!(
            min_us < budget_us,
            "{skin} ({name}): fastest frame {min_us}us exceeds {budget_us}us host budget \
             (~20ms PSP estimate, 33ms budget at 30fps)",
        );
    }
}

/// FNV-1a over an RGBA buffer, for pixel-exact golden comparisons.
#[cfg(all(target_arch = "x86_64", target_os = "linux"))]
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Pixel-exact golden checksums, captured before the loop-invariant
/// hoisting / per-cell memoization / row-parallel refactor. Any
/// optimization of the shade loops must keep these outputs
/// bit-identical. 241x135 exercises the `div_ceil` partial edge
/// cell; 64x64 matches the PSP path.
///
/// The values depend on the platform libm's `sin`/`cos` (glibc and the
/// MSVC CRT differ in the last bit for some inputs, which the hash
/// shaders amplify), so the test only runs where the goldens were
/// captured: x86_64 Linux, i.e. the CI container. The `matrix_rain`
/// rows were re-captured when its fall direction was fixed (the rain
/// used to rise).
#[test]
#[cfg(all(target_arch = "x86_64", target_os = "linux"))]
fn golden_output_checksums() {
    #[rustfmt::skip]
    let golden: &[(u32, u32, &str, f32, u64)] = &[
        (241, 135, "balatro", 0.0, 0xe683273248fbb563),
        (241, 135, "balatro", 1.7, 0x599b552b17fc1aa0),
        (241, 135, "voronoi", 0.0, 0x134812c5bae9ebbb),
        (241, 135, "voronoi", 1.7, 0xb27f2d3a263e33e1),
        (241, 135, "city_lights", 0.0, 0x825ee4062f67f80f),
        (241, 135, "city_lights", 1.7, 0x80ef6e80d0efb168),
        (241, 135, "ocean_waves", 0.0, 0x99f4708b04fc8143),
        (241, 135, "ocean_waves", 1.7, 0xa48f2758a91c1d67),
        (241, 135, "calm_waves", 0.0, 0xd8ad25f237f7b56c),
        (241, 135, "calm_waves", 1.7, 0x4c44798775101aef),
        (241, 135, "starfield", 0.0, 0xc021031ba10c5906),
        (241, 135, "starfield", 1.7, 0x904b735e365f7d00),
        (241, 135, "plasma", 0.0, 0x810a16afc41d5d28),
        (241, 135, "plasma", 1.7, 0xa5399775c2dccdfd),
        (241, 135, "matrix_rain", 0.0, 0xaf05b81b6b195823),
        (241, 135, "matrix_rain", 1.7, 0xc29e0bde924410f1),
        (64, 64, "balatro", 0.0, 0x780a6abc3968051a),
        (64, 64, "balatro", 1.7, 0x6fef1da00cbf99e0),
        (64, 64, "voronoi", 0.0, 0x42d9eb449b82e9aa),
        (64, 64, "voronoi", 1.7, 0xff9584adb8fccee3),
        (64, 64, "city_lights", 0.0, 0x454467ba33118c7d),
        (64, 64, "city_lights", 1.7, 0x95a3c1f11c325ef8),
        (64, 64, "ocean_waves", 0.0, 0x14bf42f68ce6d2e7),
        (64, 64, "ocean_waves", 1.7, 0xf33c870531ea0a98),
        (64, 64, "calm_waves", 0.0, 0xb929ccf472509a87),
        (64, 64, "calm_waves", 1.7, 0x786122825e84babd),
        (64, 64, "starfield", 0.0, 0x9d8047079aa4c57b),
        (64, 64, "starfield", 1.7, 0xf8c72c091c63d3ba),
        (64, 64, "plasma", 0.0, 0x65e84b38260695b4),
        (64, 64, "plasma", 1.7, 0x9b6b56f561f587ef),
        (64, 64, "matrix_rain", 0.0, 0x91b024d0f01281fd),
        (64, 64, "matrix_rain", 1.7, 0x1cf453612ea3b2d9),
    ];
    // Report every mismatch at once (easier to re-capture deliberately).
    let mut changed = Vec::new();
    for &(w, h, name, t, want) in golden {
        let mut r = SoftwareShaderRenderer::new(w, h);
        let got = fnv1a(r.render_shader(name, t, &ShaderParams::default()));
        if got != want {
            changed.push(format!("({w}, {h}, \"{name}\", {t:?}, {got:#018x}),"));
        }
    }
    assert!(
        changed.is_empty(),
        "output changed:\n{}",
        changed.join("\n")
    );
}

#[test]
fn psp_shader_multi_frame_transition() {
    // Simulate 5 seconds of shader wallpaper at 30fps (every other
    // frame at 60fps). Verify each frame produces valid output and
    // the overall animation progresses.
    let mut r = SoftwareShaderRenderer::new(64, 64);
    for (skin, name, params) in psp_shader_configs() {
        let mut prev: Option<Vec<u8>> = None;
        let mut change_count = 0u32;

        for frame in (0..300).step_by(2) {
            let time = frame as f32 / 60.0;
            let pixels = r.render_shader(name, time, &params);
            assert_eq!(pixels.len(), 64 * 64 * 4);

            if let Some(ref p) = prev
                && pixels != p.as_slice()
            {
                change_count += 1;
            }
            prev = Some(pixels.to_vec());
        }

        // Over 150 shader frames (5s), expect at least 50% to differ
        // from the previous frame (animation is continuous).
        assert!(
            change_count > 75,
            "{skin} ({name}): only {change_count}/150 frames changed \
             (expected >75 for smooth animation)",
        );
    }
}

#[test]
fn matrix_rain_falls_down_the_screen() {
    use super::procedural::rain_fall;
    // Regression: the head moved toward smaller `cell_y` (the rain rose).
    let head_row = |progress: f32| {
        (0..40)
            .find(|&y| rain_fall(progress, y as f32) == 0.0)
            .expect("one head per cycle")
    };
    assert_eq!(head_row(5.0), 5);
    assert_eq!(head_row(6.0), 6, "head must move down as time advances");
    // Trail above the head (lit, small `fall`), dark below it.
    assert_eq!(rain_fall(6.0, 5.0), 1.0);
    assert_eq!(rain_fall(6.0, 7.0), 39.0);
    // Stays in 0..40 for any sign, like GLSL `mod`.
    assert!((0.0..40.0).contains(&rain_fall(-3.5, 100.0)));
}

#[test]
fn non_positive_cell_size_uses_default() {
    // A zero/negative `size` divided by zero (NaN -> black frame); both
    // renderers now fall back to the default 30 cells.
    for name in ["voronoi", "city_lights"] {
        let mut r = SoftwareShaderRenderer::new(48, 32);
        let reference = r
            .render_shader(name, 0.7, &ShaderParams::default())
            .to_vec();
        for size in [0.0, -4.0, f32::NAN] {
            let params = ShaderParams {
                colors: Vec::new(),
                floats: std::collections::HashMap::from([("size".into(), size)]),
            };
            assert_eq!(
                r.render_shader(name, 0.7, &params),
                reference.as_slice(),
                "{name} size={size}"
            );
        }
    }
}

/// `uniform <type> <name>;` names declared by a GLSL source.
fn glsl_uniforms(src: &str) -> Vec<&str> {
    src.lines()
        .filter_map(|l| l.trim().strip_prefix("uniform "))
        .filter_map(|rest| rest.split_whitespace().nth(1))
        .map(|name| name.trim_end_matches(';'))
        .collect()
}

/// Every GLSL source declares exactly the inputs the CPU port consumes:
/// `u_time`, `u_resolution`, one `u_colorN` per default color slot and
/// one `u_<key>` per float parameter -- the same table the CPU reads.
#[test]
fn glsl_uniforms_match_cpu_parameters() {
    for shader in BuiltinShader::ALL {
        let mut want: Vec<String> = vec!["u_time".into(), "u_resolution".into()];
        want.extend((1..=shader.default_colors().len()).map(|i| format!("u_color{i}")));
        want.extend(shader.float_params().iter().map(|(k, _)| format!("u_{k}")));
        want.sort();
        let mut got: Vec<String> = glsl_uniforms(shader.glsl())
            .into_iter()
            .map(String::from)
            .collect();
        got.sort();
        assert_eq!(got, want, "{shader}: GLSL uniforms vs CPU parameter table");
    }
}

/// The CPU renderer is y-down; every GLSL source must flip
/// `gl_FragCoord.y` once and use only the flipped coordinate.
#[test]
fn glsl_sources_use_top_left_origin() {
    for shader in BuiltinShader::ALL {
        let src = shader.glsl();
        assert!(
            src.contains("vec2 fragCoord = vec2(gl_FragCoord.x, u_resolution.y - gl_FragCoord.y);"),
            "{shader}: missing the y-down fragCoord"
        );
        assert_eq!(
            src.matches("gl_FragCoord").count(),
            2,
            "{shader}: raw gl_FragCoord use"
        );
    }
}

/// Every declared parameter really drives the CPU output (so the GLSL
/// uniform of the same name is not the only consumer).
#[test]
fn every_declared_parameter_affects_cpu_output() {
    let (w, h, t) = (96, 64, 1.3);
    for shader in BuiltinShader::ALL {
        let mut r = SoftwareShaderRenderer::new(w, h);
        let base = r
            .render_builtin(shader, t, &ShaderParams::default())
            .to_vec();
        for &(key, default) in shader.float_params() {
            // `spin_speed` only drives the spin while `is_rotate` is on.
            let ctx: &[(&str, f32)] = if key == "spin_speed" {
                &[("is_rotate", 1.0)]
            } else {
                &[]
            };
            let with = |extra: Option<f32>| ShaderParams {
                colors: Vec::new(),
                floats: ctx
                    .iter()
                    .map(|&(k, v)| (k.to_string(), v))
                    .chain(extra.map(|v| (key.to_string(), v)))
                    .collect(),
            };
            let reference = r.render_builtin(shader, t, &with(None)).to_vec();
            let v = if default == 0.0 { 1.0 } else { default * 1.7 };
            assert_ne!(
                r.render_builtin(shader, t, &with(Some(v))),
                reference.as_slice(),
                "{shader}: float `{key}` has no effect"
            );
        }
        for slot in 0..shader.default_colors().len() {
            let mut colors = shader.default_colors().to_vec();
            colors[slot] = [0.93, 0.11, 0.57, 1.0];
            let params = ShaderParams {
                colors,
                floats: Default::default(),
            };
            assert_ne!(
                r.render_builtin(shader, t, &params),
                base.as_slice(),
                "{shader}: color slot {slot} has no effect"
            );
        }
    }
}

#[test]
fn zero_sized_outputs_render_nothing() {
    for (w, h) in [(0, 0), (0, 9), (9, 0)] {
        let mut r = SoftwareShaderRenderer::new(w, h);
        for shader in BuiltinShader::ALL {
            let px = r.render_builtin(shader, 1.0, &ShaderParams::default());
            assert!(px.is_empty(), "{w}x{h} {}", shader.name());
        }
    }
}

#[test]
fn time_wraps_at_the_period_and_ignores_non_finite() {
    use crate::{TIME_WRAP_S, wrap_time};
    assert_eq!(wrap_time(12.5), 12.5);
    assert_eq!(wrap_time(TIME_WRAP_S + 2.0), 2.0);
    assert!((0.0..TIME_WRAP_S).contains(&wrap_time(-1.0)));
    assert_eq!(wrap_time(f32::NAN), 0.0);
    assert_eq!(wrap_time(f32::INFINITY), 0.0);
    // A day in, the frame matches the wrapped one (not a frozen image).
    let mut a = SoftwareShaderRenderer::new(32, 32);
    let mut b = SoftwareShaderRenderer::new(32, 32);
    let p = ShaderParams::default();
    for shader in BuiltinShader::ALL {
        let day = 24.0 * TIME_WRAP_S + 7.25;
        let wrapped = a.render_builtin(shader, 7.25, &p).to_vec();
        assert_eq!(b.render_builtin(shader, day, &p), wrapped.as_slice());
    }
}

#[test]
fn upscale_matches_per_pixel_nearest_neighbour() {
    // Reference: the straightforward per-pixel loop the fast path replaced.
    fn reference(lo: &[[u8; 4]], w: u32, h: u32, iw: u32, ih: u32, scale: u32) -> Vec<u8> {
        let mut out = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let sx = (x / scale).min(iw - 1);
                let sy = (y / scale).min(ih - 1);
                let i = ((y * w + x) * 4) as usize;
                out[i..i + 4].copy_from_slice(&lo[(sy * iw + sx) as usize]);
            }
        }
        out
    }
    for (w, h, scale) in [(16, 16, 3), (17, 5, 3), (8, 31, 4), (1, 1, 3), (10, 7, 8)] {
        let (iw, ih) = (u32::div_ceil(w, scale), u32::div_ceil(h, scale));
        let lo: Vec<[u8; 4]> = (0..iw * ih)
            .map(|i| [i as u8, (i * 7) as u8, (i * 13) as u8, 255])
            .collect();
        let mut out = vec![0u8; (w * h * 4) as usize];
        upscale_nn(&lo, &mut out, w, h, iw, ih, scale);
        assert_eq!(out, reference(&lo, w, h, iw, ih, scale), "{w}x{h}/{scale}");
    }
}
