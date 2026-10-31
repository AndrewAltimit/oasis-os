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
/// - "altimit": picks an Altimit-inspired glyph by [`IconCategory`] (globe
///   for browsers, play button for audio, memory card for files, pen nib for
///   text editors — with their spin/pulse/blink animations), falling back to
///   the matching `solid` glyph for other categories and to the legacy
///   by-position cycle for uncategorised (plugin) apps.
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
    let category = IconCategory::from_app_title(&app.title);
    icon_for_category(preset, category, app.color, index, frame, anim_cfg)
}

/// [`icon_for_app`] for an already-classified app.
fn icon_for_category(
    preset: &str,
    category: IconCategory,
    color: Color,
    index: usize,
    frame: u32,
    anim_cfg: &IconTheme,
) -> IconDef {
    match preset {
        "geometric" => geometric_icon(color, index),
        "hud" => hud_icon(color, index),
        "outline" => icon_set::outline_icon(category, color),
        "solid" => icon_set::solid_icon(category, color),
        "pixel" => icon_set::pixel_icon(category, color),
        _ => altimit_icon(category, color, index, frame, anim_cfg),
    }
}

/// Which Altimit design (if any) represents an app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AltimitGlyph {
    /// Square-in-square globe (spins).
    World,
    Mailer,
    News,
    Accessory,
    /// Play button (inner triangle pulses).
    Audio,
    /// Memory card (LED blinks).
    Data,
    /// No Altimit design fits: use the category's `solid` glyph.
    Solid(IconCategory),
}

/// Map an app to its Altimit glyph by category. Uncategorised apps keep
/// the historical by-position cycle through the six designs.
fn altimit_glyph(category: IconCategory, index: usize) -> AltimitGlyph {
    match category {
        IconCategory::Browser | IconCategory::Home => AltimitGlyph::World,
        IconCategory::Audio => AltimitGlyph::Audio,
        IconCategory::Files => AltimitGlyph::Data,
        IconCategory::TextEditor => AltimitGlyph::Accessory,
        IconCategory::Generic => match index % 6 {
            0 => AltimitGlyph::World,
            1 => AltimitGlyph::Mailer,
            2 => AltimitGlyph::News,
            3 => AltimitGlyph::Accessory,
            4 => AltimitGlyph::Audio,
            _ => AltimitGlyph::Data,
        },
        other => AltimitGlyph::Solid(other),
    }
}

/// Build the Altimit icon for an app.
///
/// Applies per-icon animations when enabled in the theme config.
fn altimit_icon(
    category: IconCategory,
    color: Color,
    index: usize,
    frame: u32,
    cfg: &IconTheme,
) -> IconDef {
    match altimit_glyph(category, index) {
        AltimitGlyph::World if cfg.spin_enabled && frame > 0 => {
            let angle = frame as f32 * cfg.spin_speed;
            icons::icon_the_world_animated(color, angle)
        },
        AltimitGlyph::World => icons::icon_the_world(color),
        AltimitGlyph::Mailer => icons::icon_mailer(color),
        AltimitGlyph::News => icons::icon_news(color),
        AltimitGlyph::Accessory => icons::icon_accessory(color, Color::rgba(0, 0, 0, 60)),
        AltimitGlyph::Audio if cfg.pulse_enabled && frame > 0 => {
            let alpha = anim::pulse_alpha(frame, cfg.pulse_speed, 80);
            icons::icon_audio_animated(color, alpha)
        },
        AltimitGlyph::Audio => icons::icon_audio(color),
        AltimitGlyph::Data if cfg.blink_enabled && frame > 0 => {
            let visible = anim::blink_visible(frame, cfg.blink_interval);
            icons::icon_data_animated(color, cfg.data_led_color, visible)
        },
        AltimitGlyph::Data => icons::icon_data(color, cfg.data_led_color),
        AltimitGlyph::Solid(category) => icon_set::solid_icon(category, color),
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
    /// App title the glyph was picked for (semantic presets map by title).
    title: String,
    color: Color,
    icon_w: u32,
    icon_h: u32,
    /// Container style + padding: a chip shrinks the glyph into its body.
    container: String,
    container_padding: u16,
    scenes: IconScenes,
}

/// One icon's glyph plus the box its container is drawn around.
///
/// Without a container the glyph *is* the box. A `chip` container keeps
/// the box at the full fitted icon size but rebuilds the glyph smaller so
/// it sits inside the chip body, below the accent band (a full-size glyph
/// overlapped the band and poked out of the chip's bottom edge).
#[derive(Debug, Clone)]
struct IconScenes {
    /// Size of the icon box (the container wraps it with `container_padding`).
    frame_w: u32,
    frame_h: u32,
    /// Glyph ops, origin-relative.
    glyph: VectorScene,
    /// Glyph origin relative to the box origin.
    glyph_dx: i32,
    glyph_dy: i32,
}

impl IconScenes {
    /// Shrink everything by `s` (entrance `scale_up`), keeping the glyph's
    /// offset proportional so it stays put inside its container.
    fn scaled(&self, s: f32) -> Self {
        let mut glyph = self.glyph.clone();
        for op in &mut glyph.ops {
            op.scale(s);
        }
        glyph.width = (glyph.width as f32 * s) as u32;
        glyph.height = (glyph.height as f32 * s) as u32;
        Self {
            frame_w: (self.frame_w as f32 * s) as u32,
            frame_h: (self.frame_h as f32 * s) as u32,
            glyph,
            glyph_dx: (self.glyph_dx as f32 * s) as i32,
            glyph_dy: (self.glyph_dy as f32 * s) as i32,
        }
    }
}

/// Accent-band height of a chip container `bh` px tall: ~22% of it, at
/// least 4 px so it stays readable on PSP-sized dashboards.
fn chip_band_h(bh: u32) -> u32 {
    ((bh as f32 * 0.22) as u32).max(4).min(bh.saturating_sub(4))
}

/// Build the glyph (and container box) for one icon. A `chip` container
/// (other than with the `pixel` preset, which bakes its own chrome) fits
/// the glyph into the chip body below the accent band and separator.
fn build_icon_scenes(
    app: &AppEntry,
    global_index: usize,
    frame: u32,
    at: &ActiveTheme,
    flash_color: Option<Color>,
) -> IconScenes {
    let cfg = &at.icon;
    let full = build_icon_scene(
        app,
        global_index,
        frame,
        cfg,
        at.icon_width,
        at.icon_height,
        flash_color,
    );
    let (frame_w, frame_h) = (full.width, full.height);
    if cfg.container_style != "chip" || cfg.vector_preset == "pixel" {
        return IconScenes {
            frame_w,
            frame_h,
            glyph: full,
            glyph_dx: 0,
            glyph_dy: 0,
        };
    }
    let pad = i32::from(cfg.container_padding);
    let bw = frame_w as i32 + 2 * pad;
    let bh = frame_h as i32 + 2 * pad;
    // Body: below the band and its 1 px separator (both start 1 px inside
    // the outline), inset by a small gap on every side.
    let gap = pad.max(2);
    let body_top = 2 + chip_band_h(bh as u32) as i32 + gap;
    let avail_w = (bw - 2 * gap).max(1) as u32;
    let avail_h = (bh - body_top - gap).max(1) as u32;
    let glyph = build_icon_scene(app, global_index, frame, cfg, avail_w, avail_h, flash_color);
    // Offsets are relative to the box origin, which sits `pad` inside the
    // container's top-left corner.
    let glyph_dx = gap - pad + (avail_w as i32 - glyph.width as i32) / 2;
    let glyph_dy = body_top - pad + (avail_h as i32 - glyph.height as i32) / 2;
    IconScenes {
        frame_w,
        frame_h,
        glyph,
        glyph_dx,
        glyph_dy,
    }
}

/// Whether an icon's glyph depends on the frame counter (spin / pulse /
/// blink variants). Only the altimit preset (and unknown presets, which
/// fall back to it) has animated variants.
fn icon_is_animated(preset: &str, app: &AppEntry, global_index: usize, cfg: &IconTheme) -> bool {
    match preset {
        "geometric" | "hud" | "outline" | "solid" | "pixel" => false,
        _ => {
            let any = cfg.spin_enabled || cfg.pulse_enabled || cfg.blink_enabled;
            any && match altimit_glyph(IconCategory::from_app_title(&app.title), global_index) {
                AltimitGlyph::World => cfg.spin_enabled,
                AltimitGlyph::Audio => cfg.pulse_enabled,
                AltimitGlyph::Data => cfg.blink_enabled,
                _ => false,
            }
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
            lift,
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

        // A container backdrop extends `container_padding` below the icon
        // box; start the label under it rather than on its bottom edge.
        let container_pad = if at.icon.container_style != "none" && at.icon.vector_preset != "pixel"
        {
            i32::from(at.icon.container_padding)
        } else {
            0
        };

        // Draw label using the shared label renderer (still SDI-based).
        super::labels::draw_label(
            sdi,
            at,
            names,
            cell_x,
            self.cell_size().0,
            iy + lift + icon_h as i32 + text_pad + container_pad,
            &app.title,
            left_align.then_some(ix + icon_w as i32 / 2),
            &self.label_wrap_cache,
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

        let slide_offset = self.slide_offset(at);
        let per_page = self.config.icons_per_page as usize;
        let page_start = self.page * per_page;
        let origins = self.page_cell_origins();

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
            let Some(&(ox, oy)) = origins.get(i) else {
                continue;
            };
            // Same placement as the SDI pass (column layouts left-align).
            let (ix, iy) = self.icon_origin(i, (ox + slide_offset, oy), at);

            // Get or build the icon scenes: static icons render from the
            // cache; animated or flashing icons rebuild each frame.
            let global_index = page_start + i;
            let flashing = self.press_flash_frame > 0 && i == self.press_flash_index;
            let fresh: Option<IconScenes> =
                if flashing || icon_is_animated(preset, app, global_index, &at.icon) {
                    let flash = flashing
                        .then(|| oasis_types::color::lighten(app.color, at.press_flash_lighten));
                    Some(build_icon_scenes(
                        app,
                        global_index,
                        frame_counter,
                        at,
                        flash,
                    ))
                } else {
                    None
                };
            let mut cache = self.scene_cache.borrow_mut();
            let scenes: &IconScenes = match fresh {
                Some(ref s) => s,
                None => {
                    let build = || CachedIconScene {
                        preset: preset.clone(),
                        title: app.title.clone(),
                        color: app.color,
                        icon_w,
                        icon_h,
                        container: at.icon.container_style.clone(),
                        container_padding: at.icon.container_padding,
                        scenes: build_icon_scenes(app, global_index, 0, at, None),
                    };
                    let entry = cache.entry(global_index).or_insert_with(build);
                    if entry.preset != *preset
                        || entry.title != app.title
                        || entry.color != app.color
                        || entry.icon_w != icon_w
                        || entry.icon_h != icon_h
                        || entry.container != at.icon.container_style
                        || entry.container_padding != at.icon.container_padding
                    {
                        *entry = build();
                    }
                    &entry.scenes
                },
            };

            // Center the icon box within the cell.
            let mut ox = ix + (icon_w as i32 - scenes.frame_w as i32) / 2;
            let mut oy = iy + (icon_h as i32 - scenes.frame_h as i32) / 2;

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
            // the cached scenes stay pristine.
            let mut alpha = 255u8;
            let mut shrunk: Option<IconScenes> = None;
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
                            let small = scenes.scaled(s);
                            ox += (scenes.frame_w as i32 - small.frame_w as i32) / 2;
                            oy += (scenes.frame_h as i32 - small.frame_h as i32) / 2;
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
            let scenes: &IconScenes = shrunk.as_ref().unwrap_or(scenes);

            // Container backdrop (drawn before the glyph so it sits underneath).
            // Keeps vector icons legible on busy shader wallpapers.
            //
            // The `pixel` preset bakes its own chrome into the glyph, so the
            // external backdrop is always skipped for it — otherwise you'd get
            // two stacked containers.
            if at.icon.container_style != "none" && preset != "pixel" {
                let backdrop = build_container_backdrop(
                    at,
                    app.color,
                    ox,
                    oy,
                    (scenes.frame_w, scenes.frame_h),
                );
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
                    w: scenes.frame_w + 4,
                    h: scenes.frame_h + 4,
                    radius: 4,
                    width: 1,
                    color: gc,
                };
                let glow_scene = VectorScene {
                    width: scenes.frame_w + 4,
                    height: scenes.frame_h + 4,
                    ops: vec![glow_op],
                };
                render_scene_at(backend, &glow_scene, 0, 0, alpha)?;
            }

            render_scene_at(
                backend,
                &scenes.glyph,
                ox + scenes.glyph_dx,
                oy + scenes.glyph_dy,
                alpha,
            )?;
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
    (frame_w, frame_h): (u32, u32),
) -> VectorScene {
    let pad = at.icon.container_padding as i32;
    let bx = ox - pad;
    let by = oy - pad;
    let bw = frame_w + (2 * pad) as u32;
    let bh = frame_h + (2 * pad) as u32;
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
            let band_h = chip_band_h(bh);
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

    fn app(title: &str) -> AppEntry {
        AppEntry {
            title: title.to_string(),
            path: format!("/apps/{title}"),
            icon_png: Vec::new(),
            color: Color::WHITE,
        }
    }

    /// Every real category (each built-in app maps to one; `oasis-apps`
    /// tests that against its registry).
    fn categories() -> impl Iterator<Item = IconCategory> {
        IconCategory::ALL
            .into_iter()
            .filter(|c| *c != IconCategory::Generic)
    }

    #[test]
    fn no_category_gets_a_generic_icon() {
        let cfg = default_icon_theme();
        for cat in categories() {
            for preset in ["outline", "solid", "pixel", "altimit"] {
                let name = icon_for_category(preset, cat, Color::WHITE, 0, 0, &cfg).name;
                assert!(!name.ends_with("generic"), "{preset} {cat:?} -> {name}");
            }
        }
    }

    #[test]
    fn altimit_maps_by_category_not_position() {
        let cfg = default_icon_theme();
        // Settings used to land on the mail envelope by position.
        for i in 0..6 {
            assert_eq!(
                icon_for_app("altimit", &app("Settings"), i, 0, &cfg).name,
                "solid_settings"
            );
        }
        assert_eq!(
            icon_for_app("altimit", &app("Browser"), 3, 0, &cfg).name,
            "the_world"
        );
        assert_eq!(
            icon_for_app("altimit", &app("Music Player"), 0, 0, &cfg).name,
            "audio"
        );
        assert_eq!(
            icon_for_app("altimit", &app("File Manager"), 0, 0, &cfg).name,
            "data"
        );
        // Radio and System Monitor used to share the "N" glyph.
        let radio = icon_for_app("altimit", &app("Internet Radio"), 2, 0, &cfg).name;
        let monitor = icon_for_app("altimit", &app("System Monitor"), 2, 0, &cfg).name;
        assert_ne!(radio, monitor);
    }

    #[test]
    fn altimit_icons_are_distinct_per_category() {
        // Distinct categories get distinct Altimit icons; Home shares the
        // Browser globe by design (no built-in app is a launcher).
        let cfg = default_icon_theme();
        let mut seen = std::collections::HashMap::new();
        for (i, cat) in categories().enumerate() {
            let name = icon_for_category("altimit", cat, Color::WHITE, i, 0, &cfg).name;
            if let Some(prev) = seen.insert(name, cat) {
                assert!(
                    matches!(
                        (prev, cat),
                        (IconCategory::Browser, IconCategory::Home)
                            | (IconCategory::Home, IconCategory::Browser)
                    ),
                    "{cat:?} reuses {name} of {prev:?}"
                );
            }
        }
    }

    #[test]
    fn altimit_keeps_spin_pulse_blink_by_category() {
        let mut cfg = default_icon_theme();
        cfg.spin_enabled = true;
        cfg.pulse_enabled = true;
        cfg.blink_enabled = true;
        // Browser spins regardless of its grid position.
        assert!(icon_is_animated("altimit", &app("Browser"), 1, &cfg));
        assert!(icon_is_animated("altimit", &app("Music Player"), 1, &cfg));
        assert!(icon_is_animated("altimit", &app("File Manager"), 1, &cfg));
        assert!(!icon_is_animated("altimit", &app("Settings"), 0, &cfg));
        // Semantic presets never animate.
        assert!(!icon_is_animated("solid", &app("Browser"), 0, &cfg));
        let spun = icon_for_app("altimit", &app("Browser"), 1, 100, &cfg);
        assert!(matches!(spun.ops[1], VectorOp::FillPolygon { .. }));
    }

    #[test]
    fn builtin_icons_stay_inside_their_box() {
        // Semantic sets are strict; the legacy Altimit glyphs are drawn
        // edge-to-edge, so allow their centred 2px strokes 1px of overhang.
        let cfg = default_icon_theme();
        for cat in categories() {
            for preset in ["outline", "solid", "pixel", "altimit"] {
                let icon = icon_for_category(preset, cat, Color::WHITE, 0, 0, &cfg);
                let (x0, y0, x1, y1) = icon_set::ops_bounds(&icon.ops).expect("icon has ops");
                let legacy = matches!(
                    icon.name,
                    "the_world" | "mailer" | "news" | "accessory" | "audio" | "data"
                );
                let tol = i32::from(legacy);
                assert!(
                    x0 >= -tol
                        && y0 >= -tol
                        && x1 <= icon.width as i32 + tol
                        && y1 <= icon.height as i32 + tol,
                    "{preset} {cat:?} ({}) bounds ({x0},{y0})-({x1},{y1})",
                    icon.name
                );
            }
        }
    }

    /// A chip's glyph must sit inside the chip body: below the accent band
    /// and its separator, and above the bottom edge (Radio's and TV
    /// Guide's antennas used to overlap the band in macOS / GNOME).
    #[test]
    fn chip_glyph_sits_below_band() {
        for skin in ["macos", "gnome", "modern", "solarized"] {
            let skin = crate::skin::resolve_skin(skin).unwrap();
            let at = ActiveTheme::from_skin(&skin.theme)
                .with_screen_size(skin.manifest.screen_width, skin.manifest.screen_height);
            assert_eq!(at.icon.container_style, "chip");
            for title in ["Radio", "TV Guide", "Browser", "Terminal"] {
                let s = build_icon_scenes(&app(title), 0, 0, &at, None);
                let pad = i32::from(at.icon.container_padding);
                let bh = s.frame_h as i32 + 2 * pad;
                let bw = s.frame_w as i32 + 2 * pad;
                // Container-relative glyph bounds.
                let (x0, y0, x1, y1) = icon_set::ops_bounds(&s.glyph.ops).expect("glyph ops");
                let (gx, gy) = (s.glyph_dx + pad, s.glyph_dy + pad);
                let band_bottom = 2 + chip_band_h(bh as u32) as i32;
                assert!(
                    gy + y0 >= band_bottom,
                    "{title}: glyph top {} overlaps band ending {band_bottom}",
                    gy + y0
                );
                assert!(
                    gy + y1 <= bh - 1,
                    "{title}: glyph bottom {} past {bh}",
                    gy + y1
                );
                assert!(
                    gx + x0 >= 1 && gx + x1 <= bw - 1,
                    "{title}: glyph x out of chip"
                );
            }
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
