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
    /// Fallback for apps that don't match any category.
    Generic,
}

impl IconCategory {
    /// Classify an app title into a semantic category. Case-insensitive,
    /// matches on substrings so "TV Guide" and "Tune Test Episode (TV)" both
    /// map to [`IconCategory::Tv`].
    ///
    /// The order of checks matters: narrower matches come first so, e.g.,
    /// "File Manager" doesn't collapse into Generic just because "Manager"
    /// appears in other titles.
    pub fn from_app_title(title: &str) -> Self {
        let t = title.to_ascii_lowercase();

        // More specific patterns first.
        if t.contains("file") || t.contains("explorer") || t.contains("finder") {
            return Self::Files;
        }
        if t.contains("browser") || t.contains("web") || t.contains("internet explorer") {
            return Self::Browser;
        }
        if t.contains("tv") || t.contains("guide") || t.contains("television") {
            return Self::Tv;
        }
        if t.contains("radio") {
            return Self::Radio;
        }
        if t.contains("setting") || t.contains("preference") || t.contains("control panel") {
            return Self::Settings;
        }
        if t.contains("youtube") || t.contains("video") || t.contains("stream") {
            return Self::Video;
        }
        if t.contains("photo")
            || t.contains("gallery")
            || t.contains("image")
            || t.contains("paint")
        {
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
        // Audio last: matches "music", "audio", "player", "sound", "mp3".
        if t.contains("music") || t.contains("audio") || t.contains("sound") || t.contains("mp3") {
            return Self::Audio;
        }
        Self::Generic
    }
}

// ---------------------------------------------------------------------------
// OUTLINE SET — stroked, 2px, modern/minimal.
// ---------------------------------------------------------------------------

/// Return the outline-style icon for a category.
pub fn outline_icon(category: IconCategory, color: Color) -> IconDef {
    match category {
        IconCategory::Browser => outline_browser(color),
        IconCategory::Files => outline_files(color),
        IconCategory::Audio => outline_audio(color),
        IconCategory::Tv => outline_tv(color),
        IconCategory::Radio => outline_radio(color),
        IconCategory::Settings => outline_settings(color),
        IconCategory::Video => outline_video(color),
        IconCategory::Home => outline_home(color),
        IconCategory::Network => outline_network(color),
        IconCategory::Power => outline_power(color),
        IconCategory::Gallery => outline_gallery(color),
        IconCategory::Weather => outline_weather(color),
        IconCategory::Generic => outline_generic(color),
    }
}

/// Globe: circle + vertical ellipse (approx. via arcs) + horizontal equator.
fn outline_browser(color: Color) -> IconDef {
    IconDef {
        name: "outline_browser",
        ops: vec![
            VectorOp::StrokeCircle {
                cx: 12,
                cy: 12,
                radius: 10,
                width: STROKE,
                color,
            },
            // Vertical ellipse approximated by a tall thin polygon outline
            // (traces M12,2 curving out to ~±4 at y=12, back to 12,22).
            VectorOp::StrokePolygon {
                points: vec![
                    (12, 2),
                    (15, 5),
                    (16, 9),
                    (16, 12),
                    (16, 15),
                    (15, 19),
                    (12, 22),
                    (9, 19),
                    (8, 15),
                    (8, 12),
                    (8, 9),
                    (9, 5),
                ],
                width: STROKE,
                color,
            },
            VectorOp::Line {
                x1: 2,
                y1: 12,
                x2: 22,
                y2: 12,
                width: STROKE,
                color,
            },
        ],
        width: SIZE,
        height: SIZE,
    }
}

/// Folder: tabbed rectangle outline.
fn outline_files(color: Color) -> IconDef {
    IconDef {
        name: "outline_files",
        ops: vec![VectorOp::StrokePolygon {
            points: vec![(2, 5), (9, 5), (11, 8), (22, 8), (22, 21), (2, 21)],
            width: STROKE,
            color,
        }],
        width: SIZE,
        height: SIZE,
    }
}

/// Music notes: bent quarter-note stem + two note heads.
fn outline_audio(color: Color) -> IconDef {
    IconDef {
        name: "outline_audio",
        ops: vec![
            // Stem: M9 18 V5 L21 3 V16.
            VectorOp::Line {
                x1: 9,
                y1: 18,
                x2: 9,
                y2: 5,
                width: STROKE,
                color,
            },
            VectorOp::Line {
                x1: 9,
                y1: 5,
                x2: 21,
                y2: 3,
                width: STROKE,
                color,
            },
            VectorOp::Line {
                x1: 21,
                y1: 3,
                x2: 21,
                y2: 16,
                width: STROKE,
                color,
            },
            // Note heads.
            VectorOp::StrokeCircle {
                cx: 6,
                cy: 18,
                radius: 3,
                width: STROKE,
                color,
            },
            VectorOp::StrokeCircle {
                cx: 18,
                cy: 16,
                radius: 3,
                width: STROKE,
                color,
            },
        ],
        width: SIZE,
        height: SIZE,
    }
}

/// Classic antenna TV: screen rect + V antenna.
fn outline_tv(color: Color) -> IconDef {
    IconDef {
        name: "outline_tv",
        ops: vec![
            VectorOp::StrokeRoundedRect {
                x: 2,
                y: 7,
                w: 20,
                h: 15,
                radius: 2,
                width: STROKE,
                color,
            },
            // Antenna: 17,2 -> 12,7 -> 7,2.
            VectorOp::Line {
                x1: 17,
                y1: 2,
                x2: 12,
                y2: 7,
                width: STROKE,
                color,
            },
            VectorOp::Line {
                x1: 12,
                y1: 7,
                x2: 7,
                y2: 2,
                width: STROKE,
                color,
            },
        ],
        width: SIZE,
        height: SIZE,
    }
}

/// Vintage radio: body + tuning dial + speaker grille + handle arc.
fn outline_radio(color: Color) -> IconDef {
    IconDef {
        name: "outline_radio",
        ops: vec![
            VectorOp::StrokeRoundedRect {
                x: 2,
                y: 8,
                w: 20,
                h: 13,
                radius: 2,
                width: STROKE,
                color,
            },
            VectorOp::StrokeCircle {
                cx: 8,
                cy: 14,
                radius: 3,
                width: STROKE,
                color,
            },
            VectorOp::Line {
                x1: 14,
                y1: 12,
                x2: 18,
                y2: 12,
                width: STROKE,
                color,
            },
            VectorOp::Line {
                x1: 14,
                y1: 16,
                x2: 18,
                y2: 16,
                width: STROKE,
                color,
            },
            // Handle arc — upper half from (5,8) to (19,8) curving up to y=3.
            VectorOp::StrokeArc {
                cx: 12,
                cy: 8,
                radius: 7,
                start_angle: core::f32::consts::PI,
                end_angle: core::f32::consts::TAU,
                width: STROKE,
                color,
            },
        ],
        width: SIZE,
        height: SIZE,
    }
}

/// Settings gear: 8-tooth gear approximated via two overlapping rotated
/// squares plus a central ring.
fn outline_settings(color: Color) -> IconDef {
    let teeth = gear_teeth_polygon(12, 12, 10, 7, 8);
    IconDef {
        name: "outline_settings",
        ops: vec![
            VectorOp::StrokePolygon {
                points: teeth,
                width: STROKE,
                color,
            },
            VectorOp::StrokeCircle {
                cx: 12,
                cy: 12,
                radius: 3,
                width: STROKE,
                color,
            },
        ],
        width: SIZE,
        height: SIZE,
    }
}

/// Video / YouTube: rounded rect + centred play triangle.
fn outline_video(color: Color) -> IconDef {
    IconDef {
        name: "outline_video",
        ops: vec![
            VectorOp::StrokeRoundedRect {
                x: 2,
                y: 5,
                w: 20,
                h: 14,
                radius: 3,
                width: STROKE,
                color,
            },
            VectorOp::StrokePolygon {
                points: vec![(10, 9), (15, 12), (10, 15)],
                width: STROKE,
                color,
            },
        ],
        width: SIZE,
        height: SIZE,
    }
}

/// Home: pitched-roof pentagon + door.
fn outline_home(color: Color) -> IconDef {
    IconDef {
        name: "outline_home",
        ops: vec![
            // Roof + walls: 3,9 - 12,2 - 21,9 - 21,22 - 3,22 - 3,9.
            VectorOp::StrokePolygon {
                points: vec![(3, 9), (12, 2), (21, 9), (21, 22), (3, 22)],
                width: STROKE,
                color,
            },
            // Door: 9,22 - 9,12 - 15,12 - 15,22.
            VectorOp::StrokePolygon {
                points: vec![(9, 22), (9, 12), (15, 12), (15, 22)],
                width: STROKE,
                color,
            },
        ],
        width: SIZE,
        height: SIZE,
    }
}

/// Wi-Fi: three concentric arcs + dot.
fn outline_network(color: Color) -> IconDef {
    // Arcs span roughly 30° to 150° (upper half, centred on bottom point).
    let start = core::f32::consts::PI + 0.45;
    let end = core::f32::consts::TAU - 0.45;
    IconDef {
        name: "outline_network",
        ops: vec![
            VectorOp::StrokeArc {
                cx: 12,
                cy: 20,
                radius: 14,
                start_angle: start,
                end_angle: end,
                width: STROKE,
                color,
            },
            VectorOp::StrokeArc {
                cx: 12,
                cy: 20,
                radius: 9,
                start_angle: start,
                end_angle: end,
                width: STROKE,
                color,
            },
            VectorOp::StrokeArc {
                cx: 12,
                cy: 20,
                radius: 4,
                start_angle: start,
                end_angle: end,
                width: STROKE,
                color,
            },
            VectorOp::FillCircle {
                cx: 12,
                cy: 20,
                radius: 1,
                color,
            },
        ],
        width: SIZE,
        height: SIZE,
    }
}

/// Power: three-quarter ring + vertical stroke through the gap.
fn outline_power(color: Color) -> IconDef {
    // Arc from 30° below horizontal on both sides, wrapping around the bottom.
    let start = -core::f32::consts::FRAC_PI_2 + 0.8;
    let end = -core::f32::consts::FRAC_PI_2 + core::f32::consts::TAU - 0.8;
    IconDef {
        name: "outline_power",
        ops: vec![
            VectorOp::StrokeArc {
                cx: 12,
                cy: 13,
                radius: 9,
                start_angle: start,
                end_angle: end,
                width: STROKE,
                color,
            },
            VectorOp::Line {
                x1: 12,
                y1: 2,
                x2: 12,
                y2: 12,
                width: STROKE,
                color,
            },
        ],
        width: SIZE,
        height: SIZE,
    }
}

/// Gallery: frame + mountain polygon inside.
fn outline_gallery(color: Color) -> IconDef {
    IconDef {
        name: "outline_gallery",
        ops: vec![
            VectorOp::StrokeRoundedRect {
                x: 3,
                y: 3,
                w: 18,
                h: 18,
                radius: 2,
                width: STROKE,
                color,
            },
            // Sun.
            VectorOp::FillCircle {
                cx: 8,
                cy: 9,
                radius: 1,
                color,
            },
            // Mountain polyline: 3,18 - 8,13 - 12,17 - 16,11 - 21,18.
            VectorOp::StrokePolygon {
                points: vec![(3, 18), (8, 13), (12, 17), (16, 11), (21, 18)],
                width: STROKE,
                color,
            },
        ],
        width: SIZE,
        height: SIZE,
    }
}

/// Cloud: three overlapping circles with a flat base line.
fn outline_weather(color: Color) -> IconDef {
    IconDef {
        name: "outline_weather",
        ops: vec![
            // Body polygon roughly traces the cloud silhouette.
            VectorOp::StrokePolygon {
                points: vec![
                    (5, 18),
                    (3, 15),
                    (5, 12),
                    (8, 11),
                    (10, 8),
                    (14, 7),
                    (18, 9),
                    (20, 12),
                    (21, 15),
                    (19, 18),
                ],
                width: STROKE,
                color,
            },
        ],
        width: SIZE,
        height: SIZE,
    }
}

/// Generic / fallback: rounded square with a small centred dot.
fn outline_generic(color: Color) -> IconDef {
    IconDef {
        name: "outline_generic",
        ops: vec![
            VectorOp::StrokeRoundedRect {
                x: 3,
                y: 3,
                w: 18,
                h: 18,
                radius: 3,
                width: STROKE,
                color,
            },
            VectorOp::FillCircle {
                cx: 12,
                cy: 12,
                radius: 2,
                color,
            },
        ],
        width: SIZE,
        height: SIZE,
    }
}

// ---------------------------------------------------------------------------
// SOLID SET — filled shapes, high-contrast.
// ---------------------------------------------------------------------------

/// Return the solid-style icon for a category.
pub fn solid_icon(category: IconCategory, color: Color) -> IconDef {
    match category {
        IconCategory::Browser => solid_browser(color),
        IconCategory::Files => solid_files(color),
        IconCategory::Audio => solid_audio(color),
        IconCategory::Tv => solid_tv(color),
        IconCategory::Radio => solid_radio(color),
        IconCategory::Settings => solid_settings(color),
        IconCategory::Video => solid_video(color),
        IconCategory::Home => solid_home(color),
        IconCategory::Network => solid_network(color),
        IconCategory::Power => solid_power(color),
        IconCategory::Gallery => solid_gallery(color),
        IconCategory::Weather => solid_weather(color),
        IconCategory::Generic => solid_generic(color),
    }
}

/// Solid globe: filled disc with a longitude wedge cut by a crosshair.
fn solid_browser(color: Color) -> IconDef {
    IconDef {
        name: "solid_browser",
        ops: vec![
            VectorOp::FillCircle {
                cx: 12,
                cy: 12,
                radius: 10,
                color,
            },
            // Knock-out longitudes / equator so the filled disc reads as a globe.
            VectorOp::Line {
                x1: 2,
                y1: 12,
                x2: 22,
                y2: 12,
                width: 2,
                color: Color::rgba(0, 0, 0, 110),
            },
            VectorOp::Line {
                x1: 12,
                y1: 2,
                x2: 12,
                y2: 22,
                width: 2,
                color: Color::rgba(0, 0, 0, 110),
            },
            VectorOp::StrokeCircle {
                cx: 12,
                cy: 12,
                radius: 5,
                width: 2,
                color: Color::rgba(0, 0, 0, 110),
            },
        ],
        width: SIZE,
        height: SIZE,
    }
}

/// Solid folder: filled tabbed shape.
fn solid_files(color: Color) -> IconDef {
    IconDef {
        name: "solid_files",
        ops: vec![VectorOp::FillPolygon {
            points: vec![(2, 5), (9, 5), (11, 8), (22, 8), (22, 21), (2, 21)],
            color,
        }],
        width: SIZE,
        height: SIZE,
    }
}

/// Headphones: filled U-band + two earcups.
fn solid_audio(color: Color) -> IconDef {
    IconDef {
        name: "solid_audio",
        ops: vec![
            // Band arc (top half of an outer ring).
            VectorOp::StrokeArc {
                cx: 12,
                cy: 12,
                radius: 9,
                start_angle: core::f32::consts::PI,
                end_angle: core::f32::consts::TAU,
                width: 3,
                color,
            },
            // Left earcup.
            VectorOp::FillRoundedRect {
                x: 3,
                y: 12,
                w: 5,
                h: 9,
                radius: 2,
                color,
            },
            // Right earcup.
            VectorOp::FillRoundedRect {
                x: 16,
                y: 12,
                w: 5,
                h: 9,
                radius: 2,
                color,
            },
        ],
        width: SIZE,
        height: SIZE,
    }
}

/// Solid TV: filled casing with cut-out screen + antenna.
fn solid_tv(color: Color) -> IconDef {
    IconDef {
        name: "solid_tv",
        ops: vec![
            VectorOp::FillRoundedRect {
                x: 3,
                y: 7,
                w: 18,
                h: 14,
                radius: 2,
                color,
            },
            // Screen knock-out.
            VectorOp::FillRect {
                x: 5,
                y: 9,
                w: 11,
                h: 8,
                color: Color::rgba(0, 0, 0, 130),
            },
            // Knob dots.
            VectorOp::FillCircle {
                cx: 18,
                cy: 11,
                radius: 1,
                color: Color::rgba(0, 0, 0, 130),
            },
            VectorOp::FillCircle {
                cx: 18,
                cy: 15,
                radius: 1,
                color: Color::rgba(0, 0, 0, 130),
            },
            // Antennas.
            VectorOp::Line {
                x1: 17,
                y1: 2,
                x2: 12,
                y2: 7,
                width: 2,
                color,
            },
            VectorOp::Line {
                x1: 12,
                y1: 7,
                x2: 7,
                y2: 2,
                width: 2,
                color,
            },
        ],
        width: SIZE,
        height: SIZE,
    }
}

/// Boombox: filled body + cut-out speaker + handle strap.
fn solid_radio(color: Color) -> IconDef {
    IconDef {
        name: "solid_radio",
        ops: vec![
            // Handle strap (top).
            VectorOp::FillRoundedRect {
                x: 6,
                y: 3,
                w: 12,
                h: 4,
                radius: 1,
                color,
            },
            // Body.
            VectorOp::FillRoundedRect {
                x: 2,
                y: 6,
                w: 20,
                h: 15,
                radius: 2,
                color,
            },
            // Speaker cut-out.
            VectorOp::FillCircle {
                cx: 9,
                cy: 15,
                radius: 3,
                color: Color::rgba(0, 0, 0, 130),
            },
            VectorOp::StrokeCircle {
                cx: 9,
                cy: 15,
                radius: 1,
                width: 1,
                color,
            },
            // LED row.
            VectorOp::FillRect {
                x: 14,
                y: 12,
                w: 5,
                h: 2,
                color: Color::rgba(0, 0, 0, 130),
            },
            VectorOp::FillRect {
                x: 14,
                y: 16,
                w: 5,
                h: 2,
                color: Color::rgba(0, 0, 0, 130),
            },
        ],
        width: SIZE,
        height: SIZE,
    }
}

/// Solid gear: radial teeth + cut-out hub.
fn solid_settings(color: Color) -> IconDef {
    let teeth = gear_teeth_polygon(12, 12, 11, 8, 8);
    IconDef {
        name: "solid_settings",
        ops: vec![
            VectorOp::FillPolygon {
                points: teeth,
                color,
            },
            VectorOp::FillCircle {
                cx: 12,
                cy: 12,
                radius: 3,
                color: Color::rgba(0, 0, 0, 150),
            },
        ],
        width: SIZE,
        height: SIZE,
    }
}

/// YouTube-style play badge: filled rounded rect + white play triangle.
fn solid_video(color: Color) -> IconDef {
    IconDef {
        name: "solid_video",
        ops: vec![
            VectorOp::FillRoundedRect {
                x: 2,
                y: 5,
                w: 20,
                h: 14,
                radius: 3,
                color,
            },
            VectorOp::FillPolygon {
                points: vec![(10, 9), (16, 12), (10, 15)],
                color: Color::rgba(255, 255, 255, 230),
            },
        ],
        width: SIZE,
        height: SIZE,
    }
}

/// Solid home: filled house silhouette with chimney notch.
fn solid_home(color: Color) -> IconDef {
    IconDef {
        name: "solid_home",
        ops: vec![
            VectorOp::FillPolygon {
                points: vec![
                    (2, 12),
                    (12, 3),
                    (22, 12),
                    (19, 12),
                    (19, 21),
                    (5, 21),
                    (5, 12),
                ],
                color,
            },
            // Door knock-out.
            VectorOp::FillRect {
                x: 10,
                y: 14,
                w: 4,
                h: 7,
                color: Color::rgba(0, 0, 0, 150),
            },
        ],
        width: SIZE,
        height: SIZE,
    }
}

/// Solid Wi-Fi: three filled wedges, strongest at outer.
fn solid_network(color: Color) -> IconDef {
    let start = core::f32::consts::PI + 0.35;
    let end = core::f32::consts::TAU - 0.35;
    IconDef {
        name: "solid_network",
        ops: vec![
            // Outer wedge, clip inner two by painting smaller wedges over it in
            // background — since we don't have path subtraction, stack stroked
            // arcs with increasing width at three radii instead.
            VectorOp::StrokeArc {
                cx: 12,
                cy: 20,
                radius: 14,
                start_angle: start,
                end_angle: end,
                width: 3,
                color,
            },
            VectorOp::StrokeArc {
                cx: 12,
                cy: 20,
                radius: 9,
                start_angle: start,
                end_angle: end,
                width: 3,
                color,
            },
            VectorOp::StrokeArc {
                cx: 12,
                cy: 20,
                radius: 4,
                start_angle: start,
                end_angle: end,
                width: 3,
                color,
            },
            VectorOp::FillCircle {
                cx: 12,
                cy: 20,
                radius: 2,
                color,
            },
        ],
        width: SIZE,
        height: SIZE,
    }
}

/// Solid power: filled ring-with-gap + bar.
fn solid_power(color: Color) -> IconDef {
    let start = -core::f32::consts::FRAC_PI_2 + 0.7;
    let end = -core::f32::consts::FRAC_PI_2 + core::f32::consts::TAU - 0.7;
    IconDef {
        name: "solid_power",
        ops: vec![
            VectorOp::StrokeArc {
                cx: 12,
                cy: 13,
                radius: 9,
                start_angle: start,
                end_angle: end,
                width: 3,
                color,
            },
            VectorOp::FillRect {
                x: 11,
                y: 2,
                w: 2,
                h: 11,
                color,
            },
        ],
        width: SIZE,
        height: SIZE,
    }
}

/// Solid gallery: filled frame with cut-out mountain + sun.
fn solid_gallery(color: Color) -> IconDef {
    IconDef {
        name: "solid_gallery",
        ops: vec![
            VectorOp::FillRoundedRect {
                x: 3,
                y: 3,
                w: 18,
                h: 18,
                radius: 2,
                color,
            },
            // Sun.
            VectorOp::FillCircle {
                cx: 8,
                cy: 8,
                radius: 2,
                color: Color::rgba(255, 255, 255, 220),
            },
            // Mountain knock-out.
            VectorOp::FillPolygon {
                points: vec![(3, 20), (9, 12), (13, 16), (17, 10), (21, 20)],
                color: Color::rgba(0, 0, 0, 150),
            },
        ],
        width: SIZE,
        height: SIZE,
    }
}

/// Solid cloud: three overlapping filled circles + base rect.
fn solid_weather(color: Color) -> IconDef {
    IconDef {
        name: "solid_weather",
        ops: vec![
            VectorOp::FillCircle {
                cx: 8,
                cy: 14,
                radius: 5,
                color,
            },
            VectorOp::FillCircle {
                cx: 13,
                cy: 11,
                radius: 5,
                color,
            },
            VectorOp::FillCircle {
                cx: 17,
                cy: 14,
                radius: 4,
                color,
            },
            VectorOp::FillRect {
                x: 6,
                y: 14,
                w: 13,
                h: 5,
                color,
            },
        ],
        width: SIZE,
        height: SIZE,
    }
}

/// Solid generic: filled rounded square with a cut-out dot.
fn solid_generic(color: Color) -> IconDef {
    IconDef {
        name: "solid_generic",
        ops: vec![
            VectorOp::FillRoundedRect {
                x: 3,
                y: 3,
                w: 18,
                h: 18,
                radius: 3,
                color,
            },
            VectorOp::FillCircle {
                cx: 12,
                cy: 12,
                radius: 3,
                color: Color::rgba(0, 0, 0, 150),
            },
        ],
        width: SIZE,
        height: SIZE,
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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
mod tests {
    use super::*;

    #[test]
    fn classifier_matches_common_titles() {
        assert_eq!(
            IconCategory::from_app_title("Browser"),
            IconCategory::Browser
        );
        assert_eq!(
            IconCategory::from_app_title("File Manager"),
            IconCategory::Files
        );
        assert_eq!(
            IconCategory::from_app_title("Music Player"),
            IconCategory::Audio
        );
        assert_eq!(IconCategory::from_app_title("TV Guide"), IconCategory::Tv);
        assert_eq!(
            IconCategory::from_app_title("Internet Radio"),
            IconCategory::Radio
        );
        assert_eq!(
            IconCategory::from_app_title("Settings"),
            IconCategory::Settings
        );
        assert_eq!(IconCategory::from_app_title("YouTube"), IconCategory::Video);
        assert_eq!(
            IconCategory::from_app_title("Photo Viewer"),
            IconCategory::Gallery
        );
        assert_eq!(
            IconCategory::from_app_title("Weather"),
            IconCategory::Weather
        );
        assert_eq!(
            IconCategory::from_app_title("Network"),
            IconCategory::Network
        );
        assert_eq!(
            IconCategory::from_app_title("Shutdown"),
            IconCategory::Power
        );
        assert_eq!(IconCategory::from_app_title("Home"), IconCategory::Home);
    }

    #[test]
    fn classifier_is_case_insensitive() {
        assert_eq!(
            IconCategory::from_app_title("BROWSER"),
            IconCategory::Browser
        );
        assert_eq!(
            IconCategory::from_app_title("browser"),
            IconCategory::Browser
        );
    }

    #[test]
    fn unknown_titles_are_generic() {
        assert_eq!(
            IconCategory::from_app_title("Random"),
            IconCategory::Generic
        );
        assert_eq!(IconCategory::from_app_title(""), IconCategory::Generic);
    }

    #[test]
    fn file_check_beats_generic() {
        // "File Manager" contains "manager" but should still be Files.
        assert_eq!(
            IconCategory::from_app_title("File Manager"),
            IconCategory::Files
        );
    }

    #[test]
    fn outline_icons_produce_ops_for_all_categories() {
        let color = Color::WHITE;
        for cat in [
            IconCategory::Browser,
            IconCategory::Files,
            IconCategory::Audio,
            IconCategory::Tv,
            IconCategory::Radio,
            IconCategory::Settings,
            IconCategory::Video,
            IconCategory::Home,
            IconCategory::Network,
            IconCategory::Power,
            IconCategory::Gallery,
            IconCategory::Weather,
            IconCategory::Generic,
        ] {
            let icon = outline_icon(cat, color);
            assert!(!icon.ops.is_empty(), "no ops for {cat:?}");
            assert_eq!(icon.width, SIZE);
            assert_eq!(icon.height, SIZE);
        }
    }

    #[test]
    fn solid_icons_produce_ops_for_all_categories() {
        let color = Color::WHITE;
        for cat in [
            IconCategory::Browser,
            IconCategory::Files,
            IconCategory::Audio,
            IconCategory::Tv,
            IconCategory::Radio,
            IconCategory::Settings,
            IconCategory::Video,
            IconCategory::Home,
            IconCategory::Network,
            IconCategory::Power,
            IconCategory::Gallery,
            IconCategory::Weather,
            IconCategory::Generic,
        ] {
            let icon = solid_icon(cat, color);
            assert!(!icon.ops.is_empty(), "no ops for {cat:?}");
            assert_eq!(icon.width, SIZE);
            assert_eq!(icon.height, SIZE);
        }
    }

    #[test]
    fn gear_teeth_polygon_has_expected_vertex_count() {
        let pts = gear_teeth_polygon(12, 12, 10, 7, 8);
        assert_eq!(pts.len(), 32); // 4 vertices per tooth × 8 teeth.
    }
}
