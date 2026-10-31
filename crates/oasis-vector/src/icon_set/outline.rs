//! `outline` preset: stroked 2px glyphs on the 24x24 grid.
use super::*;

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
        IconCategory::Terminal => outline_terminal(color),
        IconCategory::Packages => outline_packages(color),
        IconCategory::Monitor => outline_monitor(color),
        IconCategory::Calculator => outline_calculator(color),
        IconCategory::Paint => outline_paint(color),
        IconCategory::TextEditor => outline_text_editor(color),
        IconCategory::Games => outline_games(color),
        IconCategory::Generic => outline_generic(color),
    }
}

/// Terminal: stroked window rect + ">" prompt + underscore cursor.
fn outline_terminal(color: Color) -> IconDef {
    IconDef {
        name: "outline_terminal",
        ops: vec![
            VectorOp::StrokeRoundedRect {
                x: 2,
                y: 5,
                w: 20,
                h: 14,
                radius: 2,
                width: STROKE,
                color,
            },
            // ">" prompt.
            VectorOp::StrokePolygon {
                points: vec![(6, 10), (9, 12), (6, 14)],
                width: STROKE,
                color,
            },
            // Underscore cursor.
            VectorOp::Line {
                x1: 11,
                y1: 14,
                x2: 16,
                y2: 14,
                width: STROKE,
                color,
            },
        ],
        width: SIZE,
        height: SIZE,
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
    // Arcs span roughly 206° to 334° (upper half, centred on the dot).
    // Radii are sized so the outer arc's ends (and its stroke) stay inside
    // the 24x24 box.
    let start = core::f32::consts::PI + 0.45;
    let end = core::f32::consts::TAU - 0.45;
    IconDef {
        name: "outline_network",
        ops: vec![
            VectorOp::StrokeArc {
                cx: 12,
                cy: 18,
                radius: 11,
                start_angle: start,
                end_angle: end,
                width: STROKE,
                color,
            },
            VectorOp::StrokeArc {
                cx: 12,
                cy: 18,
                radius: 7,
                start_angle: start,
                end_angle: end,
                width: STROKE,
                color,
            },
            VectorOp::StrokeArc {
                cx: 12,
                cy: 18,
                radius: 3,
                start_angle: start,
                end_angle: end,
                width: STROKE,
                color,
            },
            VectorOp::FillCircle {
                cx: 12,
                cy: 18,
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

/// Package: isometric box outline with the lid seams.
fn outline_packages(color: Color) -> IconDef {
    IconDef {
        name: "outline_packages",
        ops: vec![
            VectorOp::StrokePolygon {
                points: vec![(12, 2), (21, 7), (21, 17), (12, 22), (3, 17), (3, 7)],
                width: STROKE,
                color,
            },
            line(3, 7, 12, 12, STROKE, color),
            line(12, 12, 21, 7, STROKE, color),
            line(12, 12, 12, 22, STROKE, color),
        ],
        width: SIZE,
        height: SIZE,
    }
}

/// System monitor: display with a heartbeat trace and a stand.
fn outline_monitor(color: Color) -> IconDef {
    IconDef {
        name: "outline_monitor",
        ops: vec![
            VectorOp::StrokeRoundedRect {
                x: 2,
                y: 3,
                w: 20,
                h: 14,
                radius: 2,
                width: STROKE,
                color,
            },
            line(5, 10, 8, 10, STROKE, color),
            line(8, 10, 10, 6, STROKE, color),
            line(10, 6, 13, 14, STROKE, color),
            line(13, 14, 15, 10, STROKE, color),
            line(15, 10, 19, 10, STROKE, color),
            line(12, 17, 12, 20, STROKE, color),
            line(8, 21, 16, 21, STROKE, color),
        ],
        width: SIZE,
        height: SIZE,
    }
}

/// Calculator: body outline, display bar, 3x3 key dots.
fn outline_calculator(color: Color) -> IconDef {
    let mut ops = vec![
        VectorOp::StrokeRoundedRect {
            x: 5,
            y: 2,
            w: 14,
            h: 20,
            radius: 2,
            width: STROKE,
            color,
        },
        line(8, 6, 16, 6, STROKE, color),
    ];
    for row in 0..3 {
        for col in 0..3 {
            ops.push(VectorOp::FillCircle {
                cx: 9 + col * 3,
                cy: 11 + row * 3,
                radius: 1,
                color,
            });
        }
    }
    IconDef {
        name: "outline_calculator",
        ops,
        width: SIZE,
        height: SIZE,
    }
}

/// Paint: artist's palette ring with paint dots and a thumb hole.
fn outline_paint(color: Color) -> IconDef {
    IconDef {
        name: "outline_paint",
        ops: vec![
            VectorOp::StrokeCircle {
                cx: 12,
                cy: 12,
                radius: 9,
                width: STROKE,
                color,
            },
            VectorOp::FillCircle {
                cx: 8,
                cy: 9,
                radius: 1,
                color,
            },
            VectorOp::FillCircle {
                cx: 12,
                cy: 7,
                radius: 1,
                color,
            },
            VectorOp::FillCircle {
                cx: 16,
                cy: 9,
                radius: 1,
                color,
            },
            VectorOp::StrokeCircle {
                cx: 14,
                cy: 15,
                radius: 2,
                width: STROKE,
                color,
            },
        ],
        width: SIZE,
        height: SIZE,
    }
}

/// Text editor: page with a folded corner and ruled text lines.
fn outline_text_editor(color: Color) -> IconDef {
    IconDef {
        name: "outline_text_editor",
        ops: vec![
            VectorOp::StrokePolygon {
                points: vec![(5, 2), (14, 2), (19, 7), (19, 22), (5, 22)],
                width: STROKE,
                color,
            },
            line(14, 2, 14, 7, STROKE, color),
            line(14, 7, 19, 7, STROKE, color),
            line(8, 11, 16, 11, STROKE, color),
            line(8, 15, 16, 15, STROKE, color),
            line(8, 19, 13, 19, STROKE, color),
        ],
        width: SIZE,
        height: SIZE,
    }
}

/// Games: gamepad outline with a d-pad and two face buttons.
fn outline_games(color: Color) -> IconDef {
    IconDef {
        name: "outline_games",
        ops: vec![
            VectorOp::StrokeRoundedRect {
                x: 2,
                y: 7,
                w: 20,
                h: 11,
                radius: 5,
                width: STROKE,
                color,
            },
            line(5, 12, 10, 12, STROKE, color),
            line(7, 10, 7, 15, STROKE, color),
            VectorOp::FillCircle {
                cx: 15,
                cy: 11,
                radius: 1,
                color,
            },
            VectorOp::FillCircle {
                cx: 18,
                cy: 14,
                radius: 1,
                color,
            },
        ],
        width: SIZE,
        height: SIZE,
    }
}
