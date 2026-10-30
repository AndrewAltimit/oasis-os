//! App-semantic icon sets: `outline` and `solid` presets with per-app-category glyphs.
//!
//! Where the `altimit` preset cycles through six decorative icons by position,
//! these presets pick an icon based on the **category** of the app (browser,
//! file manager, audio player, …). Categories are derived from the app title
//! via [`IconCategory::from_app_title`].
//!
//! Designs are transliterations of the reference SVGs in
//! `/Downloads/svg icons.txt` onto a 24x24 design grid. Curves that no
//! primitive represents exactly (e.g. the gear rim) are approximated with
//! polygons or radial primitives; the visual target is "recognisable at 32px",
//! not pixel-perfect vector identity.
//!
//! `outline` uses stroked primitives (2px), `solid` uses filled primitives.
//! Both recolour via the app's assigned accent.
//!
//! `pixel` is a Windows 2000–style pixel-art set drawn on a 32x32 grid. Each
//! icon bakes in its own window-chrome container (dark border + silver body +
//! blue title band) so callers should pair it with `icon_container = "none"`
//! to avoid stacking two backdrops.

use oasis_types::backend::Color;

use crate::icons::IconDef;
use crate::op::VectorOp;

// Design grid: 24x24 (matches the source SVG viewBox).
const SIZE: u32 = 24;
const STROKE: u16 = 2;

/// Semantic category of a dashboard app. Drives which icon a preset returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconCategory {
    /// Web browser.
    Browser,
    /// File manager / explorer.
    Files,
    /// Music / audio player.
    Audio,
    /// TV guide / broadcast video.
    Tv,
    /// Internet radio / streaming radio.
    Radio,
    /// Settings / preferences / control panel.
    Settings,
    /// Video / YouTube / media streaming.
    Video,
    /// Home / dashboard / launcher.
    Home,
    /// Network / Wi-Fi / connectivity.
    Network,
    /// Power / shutdown.
    Power,
    /// Photo gallery / image viewer.
    Gallery,
    /// Weather / cloud.
    Weather,
    /// Terminal / console / shell.
    Terminal,
    /// Package manager / app store / installer.
    Packages,
    /// System monitor / task manager / activity viewer.
    Monitor,
    /// Calculator.
    Calculator,
    /// Paint / drawing program.
    Paint,
    /// Text editor / notepad.
    TextEditor,
    /// Games / arcade.
    Games,
    /// Fallback for apps that don't match any category.
    Generic,
}

impl IconCategory {
    /// Every category, in declaration order.
    pub const ALL: [IconCategory; 20] = [
        Self::Browser,
        Self::Files,
        Self::Audio,
        Self::Tv,
        Self::Radio,
        Self::Settings,
        Self::Video,
        Self::Home,
        Self::Network,
        Self::Power,
        Self::Gallery,
        Self::Weather,
        Self::Terminal,
        Self::Packages,
        Self::Monitor,
        Self::Calculator,
        Self::Paint,
        Self::TextEditor,
        Self::Games,
        Self::Generic,
    ];

    /// Classify an app title into a semantic category. Case-insensitive,
    /// matches on substrings so "TV Guide" and "Tune Test Episode (TV)" both
    /// map to [`IconCategory::Tv`].
    ///
    /// The order of checks matters: narrower matches come first so, e.g.,
    /// "File Manager" doesn't collapse into Generic just because "Manager"
    /// appears in other titles.
    pub fn from_app_title(title: &str) -> Self {
        let t = title.to_ascii_lowercase();

        // Browser before Files so "Internet Explorer" doesn't get captured by
        // the bare `explorer` substring in the Files branch.
        if t.contains("browser") || t.contains("web") || t.contains("internet explorer") {
            return Self::Browser;
        }
        if t.contains("file") || t.contains("explorer") || t.contains("finder") {
            return Self::Files;
        }
        // `tv` alone catches "TV Guide"; a bare `guide` substring would also
        // over-fire for "User Guide" / "Help Guide" style titles.
        if t.contains("tv") || t.contains("television") {
            return Self::Tv;
        }
        if t.contains("radio") {
            return Self::Radio;
        }
        if t.contains("setting") || t.contains("preference") || t.contains("control panel") {
            return Self::Settings;
        }
        // `stream` is deliberately absent: "Audio Streamer" should route to
        // Audio (matched later via "audio"), not Video.
        if t.contains("youtube") || t.contains("video") {
            return Self::Video;
        }
        if t.contains("photo") || t.contains("gallery") || t.contains("image") {
            return Self::Gallery;
        }
        if t.contains("weather") || t.contains("forecast") || t.contains("cloud") {
            return Self::Weather;
        }
        if t.contains("network") || t.contains("wifi") || t.contains("wi-fi") {
            return Self::Network;
        }
        if t.contains("power") || t.contains("shutdown") {
            return Self::Power;
        }
        if t.contains("home") || t.contains("dashboard") || t.contains("launcher") {
            return Self::Home;
        }
        if t.contains("terminal") || t.contains("console") || t.contains("shell") {
            return Self::Terminal;
        }
        if t.contains("package") || t.contains("app store") || t.contains("installer") {
            return Self::Packages;
        }
        if t.contains("monitor") || t.contains("task manager") || t.contains("activity") {
            return Self::Monitor;
        }
        if t.contains("calc") {
            return Self::Calculator;
        }
        if t.contains("paint") || t.contains("draw") || t.contains("sketch") {
            return Self::Paint;
        }
        if t.contains("editor") || t.contains("notepad") || t.contains("text") {
            return Self::TextEditor;
        }
        if t.contains("game") || t.contains("arcade") {
            return Self::Games;
        }
        // Audio last: matches "music", "audio", "sound", "mp3".
        if t.contains("music") || t.contains("audio") || t.contains("sound") || t.contains("mp3") {
            return Self::Audio;
        }
        Self::Generic
    }
}

mod outline;
mod pixel;
mod solid;

pub use outline::outline_icon;
pub use pixel::pixel_icon;
pub use solid::solid_icon;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Shorthand for a solid line.
fn line(x1: i32, y1: i32, x2: i32, y2: i32, width: u16, color: Color) -> VectorOp {
    VectorOp::Line {
        x1,
        y1,
        x2,
        y2,
        width,
        color,
    }
}

/// Conservative pixel bounds `(min_x, min_y, max_x, max_y)` of a list of
/// ops (max exclusive), or `None` for an empty list.
///
/// Strokes are assumed centred on their path (half the stroke width on
/// each side); arcs are sampled along their sweep so a partial arc only
/// contributes the part actually drawn. Group translates are applied.
/// Used to verify glyphs stay inside their design box.
pub fn ops_bounds(ops: &[VectorOp]) -> Option<(i32, i32, i32, i32)> {
    let mut b: Option<(i32, i32, i32, i32)> = None;
    let mut add = |x0: i32, y0: i32, x1: i32, y1: i32| {
        b = Some(match b {
            None => (x0, y0, x1, y1),
            Some((a, bb, c, d)) => (a.min(x0), bb.min(y0), c.max(x1), d.max(y1)),
        });
    };
    let half = |w: u16| (w as i32 + 1) / 2;
    for op in ops {
        match op {
            VectorOp::FillRect { x, y, w, h, .. }
            | VectorOp::StrokeRect { x, y, w, h, .. }
            | VectorOp::FillRoundedRect { x, y, w, h, .. }
            | VectorOp::StrokeRoundedRect { x, y, w, h, .. }
            | VectorOp::RectGradient { x, y, w, h, .. } => {
                add(*x, *y, x + *w as i32, y + *h as i32);
            },
            VectorOp::FillPolygon { points, .. } | VectorOp::PolygonGradient { points, .. } => {
                for &(px, py) in points {
                    add(px, py, px + 1, py + 1);
                }
            },
            VectorOp::FillTriangle { points, .. } => {
                for &(px, py) in points {
                    add(px, py, px + 1, py + 1);
                }
            },
            VectorOp::StrokePolygon { points, width, .. } => {
                let h = half(*width);
                for &(px, py) in points {
                    add(px - h, py - h, px + h, py + h);
                }
            },
            VectorOp::FillCircle { cx, cy, radius, .. } => {
                let r = *radius as i32;
                add(cx - r, cy - r, cx + r + 1, cy + r + 1);
            },
            VectorOp::StrokeCircle {
                cx,
                cy,
                radius,
                width,
                ..
            } => {
                let r = *radius as i32 + half(*width);
                add(cx - r, cy - r, cx + r, cy + r);
            },
            VectorOp::FillArc {
                cx,
                cy,
                radius,
                start_angle,
                end_angle,
                ..
            }
            | VectorOp::StrokeArc {
                cx,
                cy,
                radius,
                start_angle,
                end_angle,
                ..
            } => {
                let pad = match op {
                    VectorOp::StrokeArc { width, .. } => half(*width) as f32,
                    _ => {
                        // A pie wedge also covers its centre.
                        add(*cx, *cy, cx + 1, cy + 1);
                        0.0
                    },
                };
                let r = *radius as f32 + pad;
                const STEPS: u32 = 32;
                for i in 0..=STEPS {
                    let a = start_angle + (end_angle - start_angle) * i as f32 / STEPS as f32;
                    let px = (*cx as f32 + r * a.cos()).round() as i32;
                    let py = (*cy as f32 + r * a.sin()).round() as i32;
                    add(px, py, px, py);
                }
            },
            VectorOp::Line {
                x1,
                y1,
                x2,
                y2,
                width,
                ..
            }
            | VectorOp::DashedLine {
                x1,
                y1,
                x2,
                y2,
                width,
                ..
            } => {
                let h = half(*width);
                add(
                    (*x1).min(*x2) - h,
                    (*y1).min(*y2) - h,
                    (*x1).max(*x2) + h,
                    (*y1).max(*y2) + h,
                );
            },
            VectorOp::Text {
                text,
                x,
                y,
                font_size,
                ..
            } => {
                let fs = *font_size as i32;
                add(*x, *y, x + text.chars().count() as i32 * fs, y + fs);
            },
            VectorOp::Group { ops, translate, .. } => {
                if let Some((a, bb, c, d)) = ops_bounds(ops) {
                    let (dx, dy) = *translate;
                    add(a + dx, bb + dy, c + dx, d + dy);
                }
            },
        }
    }
    b
}

/// Generate an 8-tooth gear silhouette as a polygon centred at `(cx, cy)`.
///
/// `outer_r` is the tooth tip radius; `inner_r` is the valley radius.
/// `teeth` is the number of teeth. Each tooth occupies one step and is
/// represented as four polygon vertices: two valley vertices spanning the
/// gap, and two peak vertices forming the tooth face.
fn gear_teeth_polygon(cx: i32, cy: i32, outer_r: i32, inner_r: i32, teeth: u32) -> Vec<(i32, i32)> {
    let mut pts = Vec::with_capacity((teeth * 4) as usize);
    let step = core::f32::consts::TAU / teeth as f32;
    let tooth_half = step * 0.25;
    for i in 0..teeth {
        let centre = i as f32 * step;
        // Valley entry (just before the tooth).
        let a0 = centre - step * 0.5 + tooth_half;
        // Tooth peak start.
        let a1 = centre - tooth_half;
        let a2 = centre + tooth_half;
        // Valley exit (just after the tooth).
        let a3 = centre + step * 0.5 - tooth_half;
        pts.push((
            cx + (inner_r as f32 * a0.cos()) as i32,
            cy + (inner_r as f32 * a0.sin()) as i32,
        ));
        pts.push((
            cx + (outer_r as f32 * a1.cos()) as i32,
            cy + (outer_r as f32 * a1.sin()) as i32,
        ));
        pts.push((
            cx + (outer_r as f32 * a2.cos()) as i32,
            cy + (outer_r as f32 * a2.sin()) as i32,
        ));
        pts.push((
            cx + (inner_r as f32 * a3.cos()) as i32,
            cy + (inner_r as f32 * a3.sin()) as i32,
        ));
    }
    pts
}

#[cfg(test)]
mod tests;
