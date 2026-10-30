//! `pixel` preset: Windows 2000-style pixel art on a 32x32 grid.
use super::*;

// ---------------------------------------------------------------------------
// PIXEL SET — 32x32 Windows 2000 pixel-art icons with baked-in window chrome.
// ---------------------------------------------------------------------------

/// Design grid for the pixel set. Matches the source SVG viewBox.
pub(super) const PIXEL_SIZE: u32 = 32;

/// Dark border around the chrome (equiv. `#333333`).
pub(super) const PIXEL_BORDER: Color = Color::rgb(0x33, 0x33, 0x33);
/// Silver tile body (equiv. `#C0C0C0`).
const PIXEL_BODY: Color = Color::rgb(0xC0, 0xC0, 0xC0);
/// Win2K title-band blue (equiv. `#5A61A8`).
pub(super) const PIXEL_BAND: Color = Color::rgb(0x5A, 0x61, 0xA8);

/// Return the pixel-style icon for a category.
///
/// Each icon is self-contained: it paints its own window-chrome container and
/// the glyph inside. The `color` argument tints the glyph; the chrome palette
/// is fixed so tiles read as consistent Win2K windows regardless of app.
pub fn pixel_icon(category: IconCategory, color: Color) -> IconDef {
    match category {
        IconCategory::Browser => pixel_browser(color),
        IconCategory::Files => pixel_files(color),
        IconCategory::Audio => pixel_audio(color),
        IconCategory::Tv => pixel_tv(color),
        IconCategory::Radio => pixel_radio(color),
        IconCategory::Settings => pixel_settings(color),
        IconCategory::Video => pixel_video(color),
        IconCategory::Home => pixel_home(color),
        IconCategory::Network => pixel_network(color),
        IconCategory::Power => pixel_power(color),
        IconCategory::Gallery => pixel_gallery(color),
        IconCategory::Weather => pixel_weather(color),
        IconCategory::Terminal => pixel_terminal(color),
        IconCategory::Packages => pixel_packages(color),
        IconCategory::Monitor => pixel_monitor(color),
        IconCategory::Calculator => pixel_calculator(color),
        IconCategory::Paint => pixel_paint(color),
        IconCategory::TextEditor => pixel_text_editor(color),
        IconCategory::Games => pixel_games(color),
        IconCategory::Generic => pixel_generic(color),
    }
}

/// Four rects that form the Win2K window-chrome container: 1px dark border,
/// silver body, blue title band across the top, and a 1px separator under
/// the band.
fn pixel_container() -> [VectorOp; 4] {
    [
        VectorOp::FillRect {
            x: 1,
            y: 1,
            w: 30,
            h: 30,
            color: PIXEL_BORDER,
        },
        VectorOp::FillRect {
            x: 2,
            y: 2,
            w: 28,
            h: 28,
            color: PIXEL_BODY,
        },
        VectorOp::FillRect {
            x: 2,
            y: 2,
            w: 28,
            h: 3,
            color: PIXEL_BAND,
        },
        VectorOp::FillRect {
            x: 2,
            y: 5,
            w: 28,
            h: 1,
            color: PIXEL_BORDER,
        },
    ]
}

/// Shorthand for a filled rect in the glyph coordinate system.
fn px_rect(x: i32, y: i32, w: u32, h: u32, color: Color) -> VectorOp {
    VectorOp::FillRect { x, y, w, h, color }
}

/// Folder with stepped tab cut into the top edge.
pub(super) fn pixel_files(color: Color) -> IconDef {
    let mut ops = pixel_container().to_vec();
    // M6 9 h8 v1 h1 v1 h1 v1 h10 v13 h-20 z
    ops.push(VectorOp::FillPolygon {
        points: vec![
            (6, 9),
            (14, 9),
            (14, 10),
            (15, 10),
            (15, 11),
            (16, 11),
            (16, 12),
            (26, 12),
            (26, 25),
            (6, 25),
        ],
        color,
    });
    IconDef {
        name: "pixel_files",
        ops,
        width: PIXEL_SIZE,
        height: PIXEL_SIZE,
    }
}

/// Pixel globe: rounded outer silhouette + inner continent-shaped knockout.
fn pixel_browser(color: Color) -> IconDef {
    let mut ops = pixel_container().to_vec();
    // Outer blue blob.
    ops.push(VectorOp::FillPolygon {
        points: vec![
            (12, 8),
            (20, 8),
            (20, 10),
            (24, 10),
            (24, 14),
            (26, 14),
            (26, 22),
            (24, 22),
            (24, 26),
            (20, 26),
            (20, 28),
            (12, 28),
            (12, 26),
            (8, 26),
            (8, 22),
            (6, 22),
            (6, 14),
            (8, 14),
            (8, 10),
            (12, 10),
        ],
        color,
    });
    // Inner "continent" knockout, first subpath.
    ops.push(VectorOp::FillPolygon {
        points: vec![
            (12, 10),
            (16, 10),
            (16, 12),
            (18, 12),
            (18, 18),
            (14, 18),
            (14, 20),
            (10, 20),
            (10, 16),
            (8, 16),
            (8, 12),
            (10, 12),
        ],
        color: PIXEL_BODY,
    });
    // Second knockout rect (the small island on the right).
    ops.push(px_rect(20, 14, 2, 6, PIXEL_BODY));
    IconDef {
        name: "pixel_browser",
        ops,
        width: PIXEL_SIZE,
        height: PIXEL_SIZE,
    }
}

/// Quarter note: stem + flag + single note head.
fn pixel_audio(color: Color) -> IconDef {
    let mut ops = pixel_container().to_vec();
    ops.push(px_rect(18, 9, 2, 12, color));
    ops.push(px_rect(20, 9, 4, 3, color));
    ops.push(px_rect(12, 19, 6, 5, color));
    ops.push(px_rect(10, 20, 2, 3, color));
    IconDef {
        name: "pixel_audio",
        ops,
        width: PIXEL_SIZE,
        height: PIXEL_SIZE,
    }
}

/// Tube TV: rabbit-ear antennas + casing + screen knockout + side knobs.
fn pixel_tv(color: Color) -> IconDef {
    let mut ops = pixel_container().to_vec();
    // Left antenna polygon: M10 7 h2 v1 h1 v1 h1 v2 h-4 z
    ops.push(VectorOp::FillPolygon {
        points: vec![
            (10, 7),
            (12, 7),
            (12, 8),
            (13, 8),
            (13, 9),
            (14, 9),
            (14, 11),
            (10, 11),
        ],
        color,
    });
    // Right antenna polygon: M22 7 h-2 v1 h-1 v1 h-1 v2 h4 z
    ops.push(VectorOp::FillPolygon {
        points: vec![
            (22, 7),
            (20, 7),
            (20, 8),
            (19, 8),
            (19, 9),
            (18, 9),
            (18, 11),
            (22, 11),
        ],
        color,
    });
    // Casing, screen, glare, knobs.
    ops.push(px_rect(6, 11, 20, 14, color));
    ops.push(px_rect(8, 13, 12, 10, PIXEL_BODY));
    ops.push(px_rect(10, 15, 2, 2, color));
    ops.push(px_rect(22, 14, 2, 2, PIXEL_BODY));
    ops.push(px_rect(22, 18, 2, 2, PIXEL_BODY));
    IconDef {
        name: "pixel_tv",
        ops,
        width: PIXEL_SIZE,
        height: PIXEL_SIZE,
    }
}

/// Boombox: handle nubs + body with tuning band, dial, and speakers.
fn pixel_radio(color: Color) -> IconDef {
    let mut ops = pixel_container().to_vec();
    ops.push(px_rect(8, 8, 16, 2, color));
    ops.push(px_rect(8, 10, 2, 2, color));
    ops.push(px_rect(22, 10, 2, 2, color));
    ops.push(px_rect(5, 12, 22, 12, color));
    ops.push(px_rect(7, 14, 18, 1, PIXEL_BODY));
    ops.push(px_rect(14, 13, 1, 3, PIXEL_BODY));
    ops.push(px_rect(7, 17, 6, 5, PIXEL_BODY));
    ops.push(px_rect(19, 17, 6, 5, PIXEL_BODY));
    IconDef {
        name: "pixel_radio",
        ops,
        width: PIXEL_SIZE,
        height: PIXEL_SIZE,
    }
}

/// Chunky 4-arm gear with corner teeth and a centre hole.
fn pixel_settings(color: Color) -> IconDef {
    let mut ops = pixel_container().to_vec();
    ops.push(px_rect(12, 8, 8, 16, color));
    ops.push(px_rect(8, 12, 16, 8, color));
    ops.push(px_rect(14, 6, 4, 2, color));
    ops.push(px_rect(14, 24, 4, 2, color));
    ops.push(px_rect(6, 14, 2, 4, color));
    ops.push(px_rect(24, 14, 2, 4, color));
    ops.push(px_rect(9, 9, 3, 3, color));
    ops.push(px_rect(20, 9, 3, 3, color));
    ops.push(px_rect(9, 20, 3, 3, color));
    ops.push(px_rect(20, 20, 3, 3, color));
    ops.push(px_rect(14, 14, 4, 4, PIXEL_BODY));
    IconDef {
        name: "pixel_settings",
        ops,
        width: PIXEL_SIZE,
        height: PIXEL_SIZE,
    }
}

/// Film strip with sprocket holes and a stepped play triangle.
fn pixel_video(color: Color) -> IconDef {
    let mut ops = pixel_container().to_vec();
    ops.push(px_rect(6, 8, 20, 16, color));
    for x in [8, 15, 22] {
        ops.push(px_rect(x, 10, 2, 2, PIXEL_BODY));
        ops.push(px_rect(x, 20, 2, 2, PIXEL_BODY));
    }
    // Stepped play triangle knockout.
    ops.push(VectorOp::FillPolygon {
        points: vec![
            (12, 13),
            (14, 13),
            (14, 14),
            (15, 14),
            (15, 15),
            (16, 15),
            (16, 16),
            (17, 16),
            (17, 18),
            (16, 18),
            (16, 19),
            (15, 19),
            (15, 20),
            (14, 20),
            (14, 21),
            (12, 21),
        ],
        color: PIXEL_BODY,
    });
    IconDef {
        name: "pixel_video",
        ops,
        width: PIXEL_SIZE,
        height: PIXEL_SIZE,
    }
}

/// Two overlapping CRT monitors, each with stand and base.
fn pixel_network(color: Color) -> IconDef {
    let mut ops = pixel_container().to_vec();
    // Back monitor.
    ops.push(px_rect(14, 8, 12, 10, color));
    ops.push(px_rect(16, 10, 8, 6, PIXEL_BODY));
    ops.push(px_rect(19, 18, 2, 2, color));
    ops.push(px_rect(17, 20, 6, 2, color));
    // Erase block — separates the two monitors visually.
    ops.push(px_rect(7, 13, 14, 14, PIXEL_BODY));
    // Front monitor.
    ops.push(px_rect(8, 14, 12, 10, color));
    ops.push(px_rect(10, 16, 8, 6, PIXEL_BODY));
    ops.push(px_rect(13, 24, 2, 2, color));
    ops.push(px_rect(11, 26, 6, 2, color));
    IconDef {
        name: "pixel_network",
        ops,
        width: PIXEL_SIZE,
        height: PIXEL_SIZE,
    }
}

/// Command window with a ">_" prompt drawn from single pixels.
fn pixel_terminal(color: Color) -> IconDef {
    let mut ops = pixel_container().to_vec();
    ops.push(px_rect(6, 9, 20, 14, color));
    ops.push(px_rect(8, 13, 16, 8, PIXEL_BODY));
    // ">" prompt.
    ops.push(px_rect(10, 15, 1, 1, color));
    ops.push(px_rect(11, 16, 1, 1, color));
    ops.push(px_rect(10, 17, 1, 1, color));
    // Cursor underscore.
    ops.push(px_rect(13, 17, 3, 1, color));
    IconDef {
        name: "pixel_terminal",
        ops,
        width: PIXEL_SIZE,
        height: PIXEL_SIZE,
    }
}

/// Stepped-pixel house roof with walls, a door, and two windows.
fn pixel_home(color: Color) -> IconDef {
    let mut ops = pixel_container().to_vec();
    // Roof steps (each row one pixel wider).
    ops.push(px_rect(15, 7, 2, 1, color));
    ops.push(px_rect(13, 8, 6, 1, color));
    ops.push(px_rect(11, 9, 10, 1, color));
    ops.push(px_rect(9, 10, 14, 1, color));
    ops.push(px_rect(7, 11, 18, 1, color));
    ops.push(px_rect(5, 12, 22, 2, color));
    // House body.
    ops.push(px_rect(7, 14, 18, 10, color));
    // Door + windows knocked out.
    ops.push(px_rect(14, 17, 4, 7, PIXEL_BODY));
    ops.push(px_rect(9, 16, 3, 3, PIXEL_BODY));
    ops.push(px_rect(20, 16, 3, 3, PIXEL_BODY));
    IconDef {
        name: "pixel_home",
        ops,
        width: PIXEL_SIZE,
        height: PIXEL_SIZE,
    }
}

/// Power: vertical bar above an open C-ring approximated with rects.
fn pixel_power(color: Color) -> IconDef {
    let mut ops = pixel_container().to_vec();
    // Ring (open at the top).
    ops.push(px_rect(10, 13, 12, 2, color));
    ops.push(px_rect(8, 15, 2, 8, color));
    ops.push(px_rect(22, 15, 2, 8, color));
    ops.push(px_rect(10, 23, 12, 2, color));
    // Vertical bar.
    ops.push(px_rect(15, 8, 2, 8, color));
    IconDef {
        name: "pixel_power",
        ops,
        width: PIXEL_SIZE,
        height: PIXEL_SIZE,
    }
}

/// Picture frame with a pixel mountain silhouette and a sun dot.
fn pixel_gallery(color: Color) -> IconDef {
    let mut ops = pixel_container().to_vec();
    ops.push(px_rect(6, 9, 20, 14, color));
    ops.push(px_rect(8, 11, 16, 10, PIXEL_BODY));
    // Sun.
    ops.push(px_rect(10, 13, 2, 2, color));
    // Mountain (stepped triangle inside frame).
    ops.push(px_rect(9, 19, 14, 2, color));
    ops.push(px_rect(11, 17, 10, 2, color));
    ops.push(px_rect(13, 15, 6, 2, color));
    ops.push(px_rect(15, 13, 2, 2, color));
    IconDef {
        name: "pixel_gallery",
        ops,
        width: PIXEL_SIZE,
        height: PIXEL_SIZE,
    }
}

/// Sun with a small stepped cloud in the lower right.
fn pixel_weather(color: Color) -> IconDef {
    let mut ops = pixel_container().to_vec();
    // Sun disc (rect-approximation).
    ops.push(px_rect(10, 9, 6, 6, color));
    ops.push(px_rect(11, 8, 4, 1, color));
    ops.push(px_rect(11, 15, 4, 1, color));
    ops.push(px_rect(9, 10, 1, 4, color));
    ops.push(px_rect(16, 10, 1, 4, color));
    // Rays.
    ops.push(px_rect(7, 11, 1, 2, color));
    ops.push(px_rect(18, 11, 1, 2, color));
    ops.push(px_rect(12, 6, 2, 1, color));
    ops.push(px_rect(12, 17, 2, 1, color));
    // Cloud (stepped).
    ops.push(px_rect(14, 19, 10, 4, color));
    ops.push(px_rect(16, 17, 6, 2, color));
    ops.push(px_rect(18, 16, 2, 1, color));
    IconDef {
        name: "pixel_weather",
        ops,
        width: PIXEL_SIZE,
        height: PIXEL_SIZE,
    }
}

/// Document with ruled text lines — a safe fallback glyph.
fn pixel_generic(color: Color) -> IconDef {
    let mut ops = pixel_container().to_vec();
    // Document outline with a folded corner (polygon).
    ops.push(VectorOp::FillPolygon {
        points: vec![(8, 8), (20, 8), (24, 12), (24, 25), (8, 25)],
        color,
    });
    // Fold triangle highlight.
    ops.push(VectorOp::FillPolygon {
        points: vec![(20, 8), (24, 12), (20, 12)],
        color: PIXEL_BODY,
    });
    // Text lines.
    ops.push(px_rect(10, 14, 10, 1, PIXEL_BODY));
    ops.push(px_rect(10, 17, 12, 1, PIXEL_BODY));
    ops.push(px_rect(10, 20, 12, 1, PIXEL_BODY));
    ops.push(px_rect(10, 23, 8, 1, PIXEL_BODY));
    IconDef {
        name: "pixel_generic",
        ops,
        width: PIXEL_SIZE,
        height: PIXEL_SIZE,
    }
}

/// Cardboard box with a lid band and packing tape.
fn pixel_packages(color: Color) -> IconDef {
    let mut ops = pixel_container().to_vec();
    ops.push(px_rect(7, 12, 18, 13, color));
    ops.push(px_rect(6, 9, 20, 4, color));
    ops.push(px_rect(6, 13, 20, 1, PIXEL_BORDER));
    ops.push(px_rect(15, 9, 2, 16, PIXEL_BODY));
    IconDef {
        name: "pixel_packages",
        ops,
        width: PIXEL_SIZE,
        height: PIXEL_SIZE,
    }
}

/// CRT monitor showing a bar chart, on a stand.
fn pixel_monitor(color: Color) -> IconDef {
    let mut ops = pixel_container().to_vec();
    ops.push(px_rect(6, 8, 20, 14, color));
    ops.push(px_rect(8, 10, 16, 10, PIXEL_BORDER));
    ops.push(px_rect(10, 16, 2, 3, color));
    ops.push(px_rect(13, 13, 2, 6, color));
    ops.push(px_rect(16, 15, 2, 4, color));
    ops.push(px_rect(19, 11, 2, 8, color));
    ops.push(px_rect(14, 22, 4, 2, color));
    ops.push(px_rect(11, 24, 10, 2, color));
    IconDef {
        name: "pixel_monitor",
        ops,
        width: PIXEL_SIZE,
        height: PIXEL_SIZE,
    }
}

/// Pocket calculator: display strip over a 3x3 keypad.
fn pixel_calculator(color: Color) -> IconDef {
    let mut ops = pixel_container().to_vec();
    ops.push(px_rect(9, 8, 14, 19, color));
    ops.push(px_rect(11, 10, 10, 4, PIXEL_BODY));
    for row in 0..3 {
        for col in 0..3 {
            ops.push(px_rect(11 + col * 4, 16 + row * 3, 2, 2, PIXEL_BODY));
        }
    }
    IconDef {
        name: "pixel_calculator",
        ops,
        width: PIXEL_SIZE,
        height: PIXEL_SIZE,
    }
}

/// Stepped palette blob with three paint wells and a thumb hole.
fn pixel_paint(color: Color) -> IconDef {
    let mut ops = pixel_container().to_vec();
    ops.push(px_rect(9, 8, 14, 2, color));
    ops.push(px_rect(7, 10, 18, 14, color));
    ops.push(px_rect(9, 24, 14, 2, color));
    ops.push(px_rect(10, 12, 3, 3, PIXEL_BAND));
    ops.push(px_rect(15, 11, 3, 3, Color::rgb(0xCC, 0x33, 0x33)));
    ops.push(px_rect(20, 13, 3, 3, Color::rgb(0x33, 0x99, 0x33)));
    ops.push(px_rect(11, 19, 3, 3, PIXEL_BODY));
    IconDef {
        name: "pixel_paint",
        ops,
        width: PIXEL_SIZE,
        height: PIXEL_SIZE,
    }
}

/// Notepad page with dark ruled lines.
fn pixel_text_editor(color: Color) -> IconDef {
    let mut ops = pixel_container().to_vec();
    ops.push(px_rect(8, 8, 16, 19, color));
    ops.push(px_rect(8, 8, 16, 2, PIXEL_BAND));
    ops.push(px_rect(10, 13, 12, 1, PIXEL_BORDER));
    ops.push(px_rect(10, 16, 12, 1, PIXEL_BORDER));
    ops.push(px_rect(10, 19, 12, 1, PIXEL_BORDER));
    ops.push(px_rect(10, 22, 8, 1, PIXEL_BORDER));
    IconDef {
        name: "pixel_text_editor",
        ops,
        width: PIXEL_SIZE,
        height: PIXEL_SIZE,
    }
}

/// Gamepad with grips, a d-pad and two face buttons.
fn pixel_games(color: Color) -> IconDef {
    let mut ops = pixel_container().to_vec();
    ops.push(px_rect(6, 13, 20, 9, color));
    ops.push(px_rect(7, 22, 5, 3, color));
    ops.push(px_rect(20, 22, 5, 3, color));
    ops.push(px_rect(9, 17, 5, 1, PIXEL_BORDER));
    ops.push(px_rect(11, 15, 1, 5, PIXEL_BORDER));
    ops.push(px_rect(19, 15, 2, 2, PIXEL_BAND));
    ops.push(px_rect(22, 18, 2, 2, PIXEL_BAND));
    IconDef {
        name: "pixel_games",
        ops,
        width: PIXEL_SIZE,
        height: PIXEL_SIZE,
    }
}
