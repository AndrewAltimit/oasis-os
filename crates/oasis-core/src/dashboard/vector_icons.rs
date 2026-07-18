//! Vector icon rendering for the dashboard.
//!
//! When `icon_style = "vector"`, icons are drawn using `oasis_vector`
//! primitives rather than SDI object manipulation. Each app gets a vector
//! icon from a preset (e.g. "altimit") that is recolored with the app's
//! assigned color and rendered directly to the backend.

use oasis_types::backend::{Color, SdiBackend};
use oasis_types::error::Result;
use oasis_vector::anim;
use oasis_vector::icon_set::{self, IconCategory};
use oasis_vector::icons::{self, IconDef};
use oasis_vector::op::VectorOp;
use oasis_vector::render::render_scene_at;
use oasis_vector::scene::VectorScene;

use crate::active_theme::{ActiveTheme, IconTheme};

use super::{AppEntry, DashboardState, IconGeometry, IconNames};

/// Get a vector icon for an app based on the preset, app index, and animation state.
///
/// - "altimit": cycles through the 6 Altimit-inspired icons by position.
/// - "geometric": circles, hexagons, and abstract shapes by position.
/// - "hud": military/tactical diamond/chevron icons by position.
/// - "outline": app-semantic icons, 2px strokes — picks by [`IconCategory`]
///   derived from the app title.
/// - "solid": app-semantic icons, filled shapes — picks by [`IconCategory`]
///   derived from the app title.
/// - "pixel": Windows 2000 pixel-art tiles with baked-in window chrome (pair
///   with `icon_container = "none"`).
///
/// When `frame` > 0, animated variants are used for icons that support animation.
pub(super) fn icon_for_app(
    preset: &str,
    app: &AppEntry,
    index: usize,
    frame: u32,
    anim_cfg: &IconTheme,
) -> IconDef {
    match preset {
        "altimit" => altimit_icon(app.color, index, frame, anim_cfg),
        "geometric" => geometric_icon(app.color, index),
        "hud" => hud_icon(app.color, index),
        "outline" => icon_set::outline_icon(IconCategory::from_app_title(&app.title), app.color),
        "solid" => icon_set::solid_icon(IconCategory::from_app_title(&app.title), app.color),
        "pixel" => icon_set::pixel_icon(IconCategory::from_app_title(&app.title), app.color),
        _ => altimit_icon(app.color, index, frame, anim_cfg),
    }
}

/// Select an Altimit icon by cycling through the 6 available designs.
///
/// Applies per-icon animations when enabled in the theme config.
fn altimit_icon(color: Color, index: usize, frame: u32, cfg: &IconTheme) -> IconDef {
    match index % 6 {
        0 if cfg.spin_enabled && frame > 0 => {
            let angle = frame as f32 * cfg.spin_speed;
            icons::icon_the_world_animated(color, angle)
        },
        0 => icons::icon_the_world(color),
        1 => icons::icon_mailer(color),
        2 => icons::icon_news(color),
        3 => icons::icon_accessory(color, Color::rgba(0, 0, 0, 60)),
        4 if cfg.pulse_enabled && frame > 0 => {
            let alpha = anim::pulse_alpha(frame, cfg.pulse_speed, 80);
            icons::icon_audio_animated(color, alpha)
        },
        4 => icons::icon_audio(color),
        _ if cfg.blink_enabled && frame > 0 => {
            let visible = anim::blink_visible(frame, cfg.blink_interval);
            icons::icon_data_animated(color, cfg.data_led_color, visible)
        },
        _ => icons::icon_data(color, cfg.data_led_color),
    }
}

/// Geometric icon preset — circles, hexagons, and abstract shapes.
fn geometric_icon(color: Color, index: usize) -> IconDef {
    let full = core::f32::consts::TAU;
    match index % 6 {
        0 => IconDef {
            name: "geo_circle",
            ops: vec![
                VectorOp::StrokeCircle {
                    cx: 11,
                    cy: 11,
                    radius: 10,
                    width: 2,
                    color,
                },
                VectorOp::FillCircle {
                    cx: 11,
                    cy: 11,
                    radius: 4,
                    color,
                },
            ],
            width: 22,
            height: 22,
        },
        1 => {
            // Hexagon
            let pts: Vec<(i32, i32)> = (0..6)
                .map(|i| {
                    let a = full * i as f32 / 6.0 - core::f32::consts::FRAC_PI_2;
                    (11 + (10.0 * a.cos()) as i32, 11 + (10.0 * a.sin()) as i32)
                })
                .collect();
            IconDef {
                name: "geo_hexagon",
                ops: vec![VectorOp::StrokePolygon {
                    points: pts,
                    width: 2,
                    color,
                }],
                width: 22,
                height: 22,
            }
        },
        2 => IconDef {
            name: "geo_diamond",
            ops: vec![VectorOp::StrokePolygon {
                points: vec![(11, 0), (22, 11), (11, 22), (0, 11)],
                width: 2,
                color,
            }],
            width: 22,
            height: 22,
        },
        3 => IconDef {
            name: "geo_triangle",
            ops: vec![VectorOp::StrokePolygon {
                points: vec![(11, 1), (21, 20), (1, 20)],
                width: 2,
                color,
            }],
            width: 22,
            height: 22,
        },
        4 => IconDef {
            name: "geo_square",
            ops: vec![
                VectorOp::StrokeRect {
                    x: 1,
                    y: 1,
                    w: 20,
                    h: 20,
                    width: 2,
                    color,
                },
                VectorOp::Line {
                    x1: 1,
                    y1: 1,
                    x2: 21,
                    y2: 21,
                    width: 1,
                    color,
                },
            ],
            width: 22,
            height: 22,
        },
        _ => {
            // Pentagon
            let pts: Vec<(i32, i32)> = (0..5)
                .map(|i| {
                    let a = full * i as f32 / 5.0 - core::f32::consts::FRAC_PI_2;
                    (11 + (10.0 * a.cos()) as i32, 11 + (10.0 * a.sin()) as i32)
                })
                .collect();
            IconDef {
                name: "geo_pentagon",
                ops: vec![VectorOp::StrokePolygon {
                    points: pts,
                    width: 2,
                    color,
                }],
                width: 22,
                height: 22,
            }
        },
    }
}

/// HUD/tactical icon preset — military-style diamond and chevron shapes.
fn hud_icon(color: Color, index: usize) -> IconDef {
    match index % 6 {
        0 => IconDef {
            name: "hud_diamond",
            ops: vec![
                VectorOp::StrokePolygon {
                    points: vec![(11, 0), (22, 11), (11, 22), (0, 11)],
                    width: 1,
                    color,
                },
                VectorOp::FillPolygon {
                    points: vec![(11, 4), (18, 11), (11, 18), (4, 11)],
                    color,
                },
            ],
            width: 22,
            height: 22,
        },
        1 => IconDef {
            name: "hud_chevron_up",
            ops: vec![
                VectorOp::Line {
                    x1: 2,
                    y1: 16,
                    x2: 11,
                    y2: 6,
                    width: 2,
                    color,
                },
                VectorOp::Line {
                    x1: 11,
                    y1: 6,
                    x2: 20,
                    y2: 16,
                    width: 2,
                    color,
                },
            ],
            width: 22,
            height: 22,
        },
        2 => IconDef {
            name: "hud_crosshair",
            ops: vec![
                VectorOp::StrokeCircle {
                    cx: 11,
                    cy: 11,
                    radius: 8,
                    width: 1,
                    color,
                },
                VectorOp::Line {
                    x1: 11,
                    y1: 0,
                    x2: 11,
                    y2: 22,
                    width: 1,
                    color,
                },
                VectorOp::Line {
                    x1: 0,
                    y1: 11,
                    x2: 22,
                    y2: 11,
                    width: 1,
                    color,
                },
            ],
            width: 22,
            height: 22,
        },
        3 => IconDef {
            name: "hud_bracket",
            ops: vec![
                VectorOp::Line {
                    x1: 0,
                    y1: 0,
                    x2: 6,
                    y2: 0,
                    width: 2,
                    color,
                },
                VectorOp::Line {
                    x1: 0,
                    y1: 0,
                    x2: 0,
                    y2: 6,
                    width: 2,
                    color,
                },
                VectorOp::Line {
                    x1: 16,
                    y1: 0,
                    x2: 22,
                    y2: 0,
                    width: 2,
                    color,
                },
                VectorOp::Line {
                    x1: 22,
                    y1: 0,
                    x2: 22,
                    y2: 6,
                    width: 2,
                    color,
                },
                VectorOp::Line {
                    x1: 0,
                    y1: 22,
                    x2: 6,
                    y2: 22,
                    width: 2,
                    color,
                },
                VectorOp::Line {
                    x1: 0,
                    y1: 16,
                    x2: 0,
                    y2: 22,
                    width: 2,
                    color,
                },
                VectorOp::Line {
                    x1: 16,
                    y1: 22,
                    x2: 22,
                    y2: 22,
                    width: 2,
                    color,
                },
                VectorOp::Line {
                    x1: 22,
                    y1: 16,
                    x2: 22,
                    y2: 22,
                    width: 2,
                    color,
                },
            ],
            width: 22,
            height: 22,
        },
        4 => IconDef {
            name: "hud_arrow",
            ops: vec![VectorOp::FillPolygon {
                points: vec![(11, 0), (22, 22), (11, 16), (0, 22)],
                color,
            }],
            width: 22,
            height: 22,
        },
        _ => IconDef {
            name: "hud_bars",
            ops: vec![
                VectorOp::FillRect {
                    x: 2,
                    y: 2,
                    w: 18,
                    h: 3,
                    color,
                },
                VectorOp::FillRect {
                    x: 2,
                    y: 9,
                    w: 18,
                    h: 3,
                    color,
                },
                VectorOp::FillRect {
                    x: 2,
                    y: 16,
                    w: 18,
                    h: 3,
                    color,
                },
            ],
            width: 22,
            height: 22,
        },
    }
}

/// A cached scaled icon scene plus the inputs it was built from.
///
/// Rebuilding every icon's op list each frame (tessellation-equivalent
/// work) showed up in the pipeline audit; static icons only change when
/// the preset, app color, or icon size changes, so the built scene is
/// reused until its fingerprint goes stale.
#[derive(Debug)]
pub(super) struct CachedIconScene {
    preset: String,
    color: Color,
    icon_w: u32,
    icon_h: u32,
    scene: VectorScene,
}

/// Whether an icon's glyph depends on the frame counter (spin / pulse /
/// blink variants). Only the altimit preset (and unknown presets, which
/// fall back to it) has animated variants.
fn icon_is_animated(preset: &str, global_index: usize, cfg: &IconTheme) -> bool {
    match preset {
        "geometric" | "hud" | "outline" | "solid" | "pixel" => false,
        _ => match global_index % 6 {
            0 => cfg.spin_enabled,
            4 => cfg.pulse_enabled,
            5 => cfg.blink_enabled,
            _ => false,
        },
    }
}

/// Build the scaled vector scene for one icon (glyph ops only; container
/// backdrop and effects are applied at render time). The preset comes
/// from `cfg.vector_preset`.
fn build_icon_scene(
    app: &AppEntry,
    global_index: usize,
    frame: u32,
    cfg: &IconTheme,
    icon_w: u32,
    icon_h: u32,
    flash_color: Option<Color>,
) -> VectorScene {
    let mut icon = icon_for_app(&cfg.vector_preset, app, global_index, frame, cfg);
    if let Some(c) = flash_color {
        icon.recolor(c);
    }
    // Icons are 22x22 base, scale to icon_w x icon_h.
    let scale_x = icon_w as f32 / icon.width.max(1) as f32;
    let scale_y = icon_h as f32 / icon.height.max(1) as f32;
    let scale = scale_x.min(scale_y);
    let mut scene = VectorScene::new(
        (icon.width as f32 * scale) as u32,
        (icon.height as f32 * scale) as u32,
    );
    for mut op in icon.ops {
        op.scale(scale);
        scene.push(op);
    }
    scene
}

impl DashboardState {
    /// Draw a "vector" style icon — hides SDI sub-objects and prepares a
    /// vector scene for later rendering via [`render_vector_icons`].
    pub(super) fn draw_vector_icon(
        &self,
        sdi: &mut crate::sdi::SdiRegistry,
        at: &ActiveTheme,
        names: &IconNames,
        geo: IconGeometry,
        slot: usize,
        app: &AppEntry,
    ) {
        let IconGeometry {
            ix,
            iy,
            icon_w,
            icon_h,
            cell_x,
            text_pad,
            left_align,
        } = geo;

        // Hide all SDI sub-objects (vector rendering is done directly).
        for name in [
            &names.outline,
            &names.stripe,
            &names.fold,
            &names.gfx,
            &names.icon,
        ] {
            if let Ok(obj) = sdi.get_mut(name) {
                obj.visible = false;
            }
        }

        // Draw label using the shared label renderer (still SDI-based).
        super::labels::draw_label(
            sdi,
            at,
            names,
            cell_x,
            self.cell_size().0,
            iy + icon_h as i32 + text_pad,
            &app.title,
            left_align.then_some(ix),
        );

        // Store the vector icon slot for rendering in render_vector_icons().
        // We use the slot index to find the right entry later.
        let _ = (ix, iy, icon_w, icon_h, slot);
    }

    /// Render vector icons directly to the backend.
    ///
    /// Call this between `sdi.draw_base_layer()` and `sdi.draw_overlay_layer()`
    /// to maintain correct z-order (above wallpaper, below cursor/overlays).
    ///
    /// `frame_counter` drives per-icon animations (spin, pulse, blink, float).
    /// Pass 0 for static rendering.
    pub fn render_vector_icons(
        &self,
        backend: &mut dyn SdiBackend,
        at: &ActiveTheme,
        frame_counter: u32,
    ) -> Result<()> {
        if at.icon.style != "vector" {
            return Ok(());
        }

        let page_apps = self.current_page_apps();
        if page_apps.is_empty() {
            return Ok(());
        }

        let icon_w = at.icon_width;
        let icon_h = at.icon_height;
        let preset = &at.icon.vector_preset;

        // Page-slide offset for animation.
        let slide_offset = if let Some(ref anim) = self.page_anim {
            let t = crate::transition::ease_in_out_cubic(anim.frame as f32 / anim.duration as f32);
            let w = at.screen_w as f32;
            ((1.0 - t) * w * anim.direction as f32) as i32
        } else {
            0
        };

        let per_page = self.config.icons_per_page as usize;
        let page_start = self.page * per_page;
        let origins = self.page_cell_origins();
        let (cell_w, cell_h) = self.cell_size();

        // Draw the dragged icon last so it stays on top of its siblings.
        let mut order: Vec<usize> = (0..page_apps.len()).collect();
        if let Some(drag) = self.drag_index
            && drag < order.len()
        {
            order.retain(|&i| i != drag);
            order.push(drag);
        }

        for &i in &order {
            let app = &page_apps[i];
            let (cell_x, cell_y) = (origins[i].0 + slide_offset, origins[i].1);
            let ix = cell_x + (cell_w as i32 - icon_w as i32) / 2;
            let iy = cell_y + (cell_h as i32 - icon_h as i32) / 4;

            // Get or build the scaled icon scene: static icons render from
            // the cache; animated or flashing icons rebuild each frame.
            let global_index = page_start + i;
            let flashing = self.press_flash_frame > 0 && i == self.press_flash_index;
            let fresh: Option<VectorScene> =
                if flashing || icon_is_animated(preset, global_index, &at.icon) {
                    let flash = flashing
                        .then(|| oasis_types::color::lighten(app.color, at.press_flash_lighten));
                    Some(build_icon_scene(
                        app,
                        global_index,
                        frame_counter,
                        &at.icon,
                        icon_w,
                        icon_h,
                        flash,
                    ))
                } else {
                    None
                };
            let mut cache = self.scene_cache.borrow_mut();
            let scene: &VectorScene = match fresh {
                Some(ref s) => s,
                None => {
                    let build = || CachedIconScene {
                        preset: preset.clone(),
                        color: app.color,
                        icon_w,
                        icon_h,
                        scene: build_icon_scene(
                            app,
                            global_index,
                            0,
                            &at.icon,
                            icon_w,
                            icon_h,
                            None,
                        ),
                    };
                    let entry = cache.entry(global_index).or_insert_with(build);
                    if entry.preset != *preset
                        || entry.color != app.color
                        || entry.icon_w != icon_w
                        || entry.icon_h != icon_h
                    {
                        *entry = build();
                    }
                    &entry.scene
                },
            };

            // Center the scaled icon within the cell.
            let mut ox = ix + (icon_w as i32 - scene.width as i32) / 2;
            let mut oy = iy + (icon_h as i32 - scene.height as i32) / 2;

            // Apply idle float animation (sine-wave bob).
            if at.icon.idle_float {
                oy += anim::float_offset(
                    frame_counter,
                    i,
                    at.icon.float_amplitude,
                    at.icon.float_speed,
                );
            }

            // Entrance animation. `scale_up` shrinks a transient copy so
            // the cached scene stays pristine.
            let mut alpha = 255u8;
            let mut shrunk: Option<VectorScene> = None;
            if at.entrance_style != "none"
                && let Some(elapsed) = self.entrance_elapsed_ms
            {
                let staggered = elapsed.saturating_sub(i as u32 * at.entrance_stagger_ms);
                let dur = at.entrance_duration_ms;
                match at.entrance_style.as_str() {
                    "fade_in" => {
                        alpha = anim::entrance_alpha(staggered, dur);
                    },
                    "scale_up" => {
                        let s = anim::entrance_scale(staggered, dur);
                        if s < 1.0 {
                            let mut small = scene.clone();
                            let sw = (small.width as f32 * s) as u32;
                            let sh = (small.height as f32 * s) as u32;
                            ox += (small.width as i32 - sw as i32) / 2;
                            oy += (small.height as i32 - sh as i32) / 2;
                            for op in &mut small.ops {
                                op.scale(s);
                            }
                            small.width = sw;
                            small.height = sh;
                            shrunk = Some(small);
                        }
                    },
                    "slide_up" => {
                        oy += anim::entrance_slide_y(staggered, dur, 15);
                        alpha = anim::entrance_alpha(staggered, dur);
                    },
                    _ => {},
                }
            }
            let scene: &VectorScene = shrunk.as_ref().unwrap_or(scene);

            // Container backdrop (drawn before the glyph so it sits underneath).
            // Keeps vector icons legible on busy shader wallpapers.
            //
            // The `pixel` preset bakes its own chrome into the glyph, so the
            // external backdrop is always skipped for it — otherwise you'd get
            // two stacked containers.
            //
            // `glyph_offset_y` nudges the glyph down so it sits in the tile's
            // body area below the accent band, rather than floating over it.
            let pixel_preset = preset == "pixel";
            let glyph_offset_y = if at.icon.container_style == "chip" && !pixel_preset {
                let bh = scene.height + (2 * at.icon.container_padding as u32);
                let band_h = ((bh as f32 * 0.22) as i32)
                    .max(4)
                    .min((bh as i32 - 4).max(0));
                band_h / 2
            } else {
                0
            };
            if at.icon.container_style != "none" && !pixel_preset {
                let backdrop = build_container_backdrop(at, app.color, ox, oy, scene);
                render_scene_at(backend, &backdrop, 0, 0, alpha)?;
            }

            // Focus glow ring.
            if at.focus_glow && i == self.selected_index {
                let glow_alpha = anim::pulse_alpha(frame_counter, 0.08, 100);
                let mut gc = at.focus_glow_color;
                gc.a = ((gc.a as u16 * glow_alpha as u16) / 255) as u8;
                let glow_op = VectorOp::StrokeRoundedRect {
                    x: ox - 2,
                    y: oy - 2,
                    w: scene.width + 4,
                    h: scene.height + 4,
                    radius: 4,
                    width: 1,
                    color: gc,
                };
                let glow_scene = VectorScene {
                    width: scene.width + 4,
                    height: scene.height + 4,
                    ops: vec![glow_op],
                };
                render_scene_at(backend, &glow_scene, 0, 0, alpha)?;
            }

            render_scene_at(backend, scene, ox, oy + glyph_offset_y, alpha)?;
        }

        Ok(())
    }
}

/// Build the container backdrop scene for a single icon.
///
/// "chip" paints a Windows 2000 / XP Luna-style tile:
///   1. Light body filled with `icon.body_color`.
///   2. Accent band across the top in a vertical gradient derived from
///      `app_color` (app-specific, giving each tile its identity colour).
///   3. A 1px separator line under the band.
///   4. Thin outline in `icon.outline_color`.
///
/// "circle" paints a plain disc in `body_color` with a 1px stroke.
///
/// Any other style falls through to the flat filled rect + outline.
///
/// NOTE: The returned scene bakes absolute screen coordinates derived from
/// `ox` / `oy` into every op (unlike glyph scenes which use origin-relative
/// ops). Callers MUST render it at `(0, 0)` via `render_scene_at`; passing a
/// non-zero offset will double-offset the backdrop.
fn build_container_backdrop(
    at: &ActiveTheme,
    app_color: Color,
    ox: i32,
    oy: i32,
    scene: &VectorScene,
) -> VectorScene {
    let pad = at.icon.container_padding as i32;
    let bx = ox - pad;
    let by = oy - pad;
    let bw = scene.width + (2 * pad) as u32;
    let bh = scene.height + (2 * pad) as u32;
    let radius = at.icon.border_radius;

    match at.icon.container_style.as_str() {
        "circle" => {
            let r = (bw.min(bh) / 2) as u16;
            let cx = bx + bw as i32 / 2;
            let cy = by + bh as i32 / 2;
            VectorScene {
                width: bw,
                height: bh,
                ops: vec![
                    VectorOp::FillCircle {
                        cx,
                        cy,
                        radius: r,
                        color: at.icon.body_color,
                    },
                    VectorOp::StrokeCircle {
                        cx,
                        cy,
                        radius: r,
                        width: 1,
                        color: at.icon.outline_color,
                    },
                ],
            }
        },
        "chip" => {
            // Band height: ~22% of chip height, minimum 4px so it stays
            // readable even on PSP-sized dashboards.
            let band_h = ((bh as f32 * 0.22) as u32).max(4).min(bh.saturating_sub(4));
            let band_top = oasis_types::color::lighten(app_color, 0.25);
            let band_bottom = app_color;
            let separator = oasis_types::color::darken(app_color, 0.35);

            let ops = vec![
                // Body (full rounded rect — sits underneath everything).
                VectorOp::FillRoundedRect {
                    x: bx,
                    y: by,
                    w: bw,
                    h: bh,
                    radius,
                    color: at.icon.body_color,
                },
                // Accent band: vertical gradient, inset 1px from the sides
                // so it doesn't poke past the rounded corners at the top.
                VectorOp::RectGradient {
                    x: bx + 1,
                    y: by + 1,
                    w: bw.saturating_sub(2),
                    h: band_h,
                    gradient: oasis_types::backend::GradientStyle::Vertical {
                        top: band_top,
                        bottom: band_bottom,
                    },
                },
                // 1px separator line under the band — picks up the Win2K
                // "title bar shadow" feel.
                VectorOp::Line {
                    x1: bx + 1,
                    y1: by + 1 + band_h as i32,
                    x2: bx + bw as i32 - 1,
                    y2: by + 1 + band_h as i32,
                    width: 1,
                    color: separator,
                },
                // Outline on top of everything.
                VectorOp::StrokeRoundedRect {
                    x: bx,
                    y: by,
                    w: bw,
                    h: bh,
                    radius,
                    width: 1,
                    color: at.icon.outline_color,
                },
            ];
            VectorScene {
                width: bw,
                height: bh,
                ops,
            }
        },
        _ => VectorScene {
            width: bw,
            height: bh,
            ops: vec![
                VectorOp::FillRoundedRect {
                    x: bx,
                    y: by,
                    w: bw,
                    h: bh,
                    radius,
                    color: at.icon.body_color,
                },
                VectorOp::StrokeRoundedRect {
                    x: bx,
                    y: by,
                    w: bw,
                    h: bh,
                    radius,
                    width: 1,
                    color: at.icon.outline_color,
                },
            ],
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::active_theme::ActiveTheme;

    fn default_icon_theme() -> IconTheme {
        ActiveTheme::default().icon
    }

    #[test]
    fn icon_for_app_cycles_presets() {
        let cfg = default_icon_theme();
        let app = AppEntry {
            title: "Test".to_string(),
            path: "/test".to_string(),
            icon_png: Vec::new(),
            color: Color::WHITE,
        };

        let i0 = icon_for_app("altimit", &app, 0, 0, &cfg);
        assert_eq!(i0.name, "the_world");

        let i1 = icon_for_app("altimit", &app, 1, 0, &cfg);
        assert_eq!(i1.name, "mailer");

        let i5 = icon_for_app("altimit", &app, 5, 0, &cfg);
        assert_eq!(i5.name, "data");

        // Wraps around.
        let i6 = icon_for_app("altimit", &app, 6, 0, &cfg);
        assert_eq!(i6.name, "the_world");
    }

    #[test]
    fn icon_for_app_uses_app_color() {
        let cfg = default_icon_theme();
        let app = AppEntry {
            title: "Test".to_string(),
            path: "/test".to_string(),
            icon_png: Vec::new(),
            color: Color::rgb(255, 0, 0),
        };
        let icon = icon_for_app("altimit", &app, 0, 0, &cfg);
        // First op should use the app's color.
        let __out = &icon.ops[0];
        let oasis_vector::op::VectorOp::StrokeRect { color, .. } = __out else {
            panic!("expected StrokeRect for the_world icon, got {__out:?}");
        };
        assert_eq!(*color, Color::rgb(255, 0, 0));
    }

    #[test]
    fn unknown_preset_defaults_to_altimit() {
        let cfg = default_icon_theme();
        let app = AppEntry {
            title: "Test".to_string(),
            path: "/test".to_string(),
            icon_png: Vec::new(),
            color: Color::WHITE,
        };
        let icon = icon_for_app("nonexistent", &app, 0, 0, &cfg);
        assert_eq!(icon.name, "the_world");
    }

    #[test]
    fn outline_preset_picks_icon_by_app_title() {
        let cfg = default_icon_theme();
        let browser = AppEntry {
            title: "Browser".to_string(),
            path: "/browser".to_string(),
            icon_png: Vec::new(),
            color: Color::WHITE,
        };
        let files = AppEntry {
            title: "File Manager".to_string(),
            path: "/files".to_string(),
            icon_png: Vec::new(),
            color: Color::WHITE,
        };
        let audio = AppEntry {
            title: "Music Player".to_string(),
            path: "/music".to_string(),
            icon_png: Vec::new(),
            color: Color::WHITE,
        };
        assert_eq!(
            icon_for_app("outline", &browser, 0, 0, &cfg).name,
            "outline_browser"
        );
        assert_eq!(
            icon_for_app("outline", &files, 1, 0, &cfg).name,
            "outline_files"
        );
        assert_eq!(
            icon_for_app("outline", &audio, 2, 0, &cfg).name,
            "outline_audio"
        );
    }

    #[test]
    fn solid_preset_picks_icon_by_app_title() {
        let cfg = default_icon_theme();
        let tv = AppEntry {
            title: "TV Guide".to_string(),
            path: "/tv".to_string(),
            icon_png: Vec::new(),
            color: Color::WHITE,
        };
        let radio = AppEntry {
            title: "Internet Radio".to_string(),
            path: "/radio".to_string(),
            icon_png: Vec::new(),
            color: Color::WHITE,
        };
        assert_eq!(icon_for_app("solid", &tv, 0, 0, &cfg).name, "solid_tv");
        assert_eq!(
            icon_for_app("solid", &radio, 1, 0, &cfg).name,
            "solid_radio"
        );
    }

    #[test]
    fn pixel_preset_picks_icon_by_app_title() {
        let cfg = default_icon_theme();
        let terminal = AppEntry {
            title: "Terminal".to_string(),
            path: "/terminal".to_string(),
            icon_png: Vec::new(),
            color: Color::WHITE,
        };
        let files = AppEntry {
            title: "File Manager".to_string(),
            path: "/files".to_string(),
            icon_png: Vec::new(),
            color: Color::WHITE,
        };
        assert_eq!(
            icon_for_app("pixel", &terminal, 0, 0, &cfg).name,
            "pixel_terminal"
        );
        assert_eq!(
            icon_for_app("pixel", &files, 1, 0, &cfg).name,
            "pixel_files"
        );
    }

    #[test]
    fn outline_preset_independent_of_index() {
        // Semantic presets must not depend on `index`, only the title.
        let cfg = default_icon_theme();
        let app = AppEntry {
            title: "Settings".to_string(),
            path: "/settings".to_string(),
            icon_png: Vec::new(),
            color: Color::WHITE,
        };
        for i in 0..12 {
            assert_eq!(
                icon_for_app("outline", &app, i, 0, &cfg).name,
                "outline_settings"
            );
        }
    }

    #[test]
    fn animated_icons_with_frame_counter() {
        let mut cfg = default_icon_theme();
        cfg.spin_enabled = true;
        cfg.pulse_enabled = true;
        cfg.blink_enabled = true;

        let app = AppEntry {
            title: "Test".to_string(),
            path: "/test".to_string(),
            icon_png: Vec::new(),
            color: Color::WHITE,
        };

        // the_world at frame 100 should still be "the_world"
        let icon = icon_for_app("altimit", &app, 0, 100, &cfg);
        assert_eq!(icon.name, "the_world");
        // Inner element should be a FillPolygon (rotated rect) instead of FillRect
        let __out = &icon.ops[1];
        let oasis_vector::op::VectorOp::FillPolygon { points, .. } = __out else {
            panic!("expected FillPolygon for animated the_world inner element, got {__out:?}");
        };
        assert_eq!(points.len(), 4);

        // audio at frame 50 should have pulsing alpha
        let audio = icon_for_app("altimit", &app, 4, 50, &cfg);
        assert_eq!(audio.name, "audio");

        // data at frame where LED is off (last 1/3 of interval)
        let data = icon_for_app("altimit", &app, 5, 40, &cfg);
        assert_eq!(data.name, "data");
        // At frame 40 with interval 45, phase = 40 >= 30, so LED should be off
        assert_eq!(data.ops.len(), 3); // no LED circle
    }
}
