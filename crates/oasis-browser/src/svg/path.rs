//! SVG path data (`d`, `points`): tokenizer, command parser and Bezier
//! flattening into polylines.

use super::*;

// -------------------------------------------------------------------
// SVG path `d` attribute parser
// -------------------------------------------------------------------

/// Parse an SVG `points` attribute (e.g. "100,10 40,198 190,78").
pub(super) fn parse_point_list(s: &str) -> Vec<(f32, f32)> {
    let nums: Vec<f32> = s
        .split(|c: char| c == ',' || c.is_ascii_whitespace())
        .filter(|p| !p.is_empty())
        .filter_map(|p| p.parse::<f32>().ok())
        .collect();
    nums.as_chunks::<2>()
        .0
        .iter()
        .map(|c| (c[0], c[1]))
        .collect()
}

/// Parse SVG path `d` attribute into commands, then flatten curves
/// into a polygon point list suitable for `fill_polygon`.
pub(super) fn flatten_path_data(d: &str) -> Vec<(f32, f32)> {
    let cmds = parse_path_commands(d);
    flatten_commands(&cmds)
}

/// Tokenize and parse SVG path `d` data into [`PathCmd`] list.
///
/// Supports: M/m L/l H/h V/v C/c S/s Q/q T/t Z/z.
pub(super) fn parse_path_commands(d: &str) -> Vec<PathCmd> {
    let mut cmds = Vec::new();
    let nums = tokenize_path(d);
    let mut i = 0;
    let mut last_cmd = b'M';
    let mut cx = 0.0f32;
    let mut cy = 0.0f32;

    while i < nums.len() {
        if let PathToken::Cmd(c) = nums[i] {
            last_cmd = c;
            i += 1;
        }

        let relative = last_cmd.is_ascii_lowercase();
        let cmd_upper = last_cmd.to_ascii_uppercase();

        match cmd_upper {
            b'M' => {
                let (x, y) = (take_num(&nums, &mut i), take_num(&nums, &mut i));
                let (ax, ay) = if relative { (cx + x, cy + y) } else { (x, y) };
                cmds.push(PathCmd::MoveTo(ax, ay));
                cx = ax;
                cy = ay;
                // Subsequent coordinates after M are implicit LineTo.
                last_cmd = if relative { b'l' } else { b'L' };
            },
            b'L' => {
                let (x, y) = (take_num(&nums, &mut i), take_num(&nums, &mut i));
                let (ax, ay) = if relative { (cx + x, cy + y) } else { (x, y) };
                cmds.push(PathCmd::LineTo(ax, ay));
                cx = ax;
                cy = ay;
            },
            b'H' => {
                let x = take_num(&nums, &mut i);
                let ax = if relative { cx + x } else { x };
                cmds.push(PathCmd::HorizTo(ax));
                cx = ax;
            },
            b'V' => {
                let y = take_num(&nums, &mut i);
                let ay = if relative { cy + y } else { y };
                cmds.push(PathCmd::VertTo(ay));
                cy = ay;
            },
            b'C' => {
                let (x1, y1) = (take_num(&nums, &mut i), take_num(&nums, &mut i));
                let (x2, y2) = (take_num(&nums, &mut i), take_num(&nums, &mut i));
                let (x, y) = (take_num(&nums, &mut i), take_num(&nums, &mut i));
                let (off_x, off_y) = if relative { (cx, cy) } else { (0.0, 0.0) };
                cmds.push(PathCmd::CubicTo(
                    x1 + off_x,
                    y1 + off_y,
                    x2 + off_x,
                    y2 + off_y,
                    x + off_x,
                    y + off_y,
                ));
                cx = x + off_x;
                cy = y + off_y;
            },
            b'S' => {
                let (x2, y2) = (take_num(&nums, &mut i), take_num(&nums, &mut i));
                let (x, y) = (take_num(&nums, &mut i), take_num(&nums, &mut i));
                let (off_x, off_y) = if relative { (cx, cy) } else { (0.0, 0.0) };
                cmds.push(PathCmd::SmoothCubicTo(
                    x2 + off_x,
                    y2 + off_y,
                    x + off_x,
                    y + off_y,
                ));
                cx = x + off_x;
                cy = y + off_y;
            },
            b'Q' => {
                let (x1, y1) = (take_num(&nums, &mut i), take_num(&nums, &mut i));
                let (x, y) = (take_num(&nums, &mut i), take_num(&nums, &mut i));
                let (off_x, off_y) = if relative { (cx, cy) } else { (0.0, 0.0) };
                cmds.push(PathCmd::QuadTo(
                    x1 + off_x,
                    y1 + off_y,
                    x + off_x,
                    y + off_y,
                ));
                cx = x + off_x;
                cy = y + off_y;
            },
            b'T' => {
                let (x, y) = (take_num(&nums, &mut i), take_num(&nums, &mut i));
                let (ax, ay) = if relative { (cx + x, cy + y) } else { (x, y) };
                cmds.push(PathCmd::SmoothQuadTo(ax, ay));
                cx = ax;
                cy = ay;
            },
            b'Z' => {
                cmds.push(PathCmd::Close);
                // Reset last_cmd so stray numbers after Z don't
                // re-trigger the Z handler in an infinite loop.
                last_cmd = b'M';
            },
            _ => {
                i += 1; // skip unknown
            },
        }
    }
    cmds
}

/// Token from SVG path data: either a command letter or a number.
#[derive(Debug, Clone)]
pub(super) enum PathToken {
    Cmd(u8),
    Num(f32),
}

/// Tokenize SVG path `d` data into command letters and numbers.
pub(super) fn tokenize_path(d: &str) -> Vec<PathToken> {
    let mut tokens = Vec::new();
    let bytes = d.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_whitespace() || c == b',' {
            i += 1;
            continue;
        }
        if c.is_ascii_alphabetic() {
            tokens.push(PathToken::Cmd(c));
            i += 1;
            continue;
        }
        // Parse number (including sign, decimal point, exponent).
        let start = i;
        if c == b'-' || c == b'+' {
            i += 1;
        }
        while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
            i += 1;
        }
        // Exponent
        if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
            i += 1;
            if i < bytes.len() && (bytes[i] == b'-' || bytes[i] == b'+') {
                i += 1;
            }
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
        }
        if i > start {
            if let Ok(n) = d[start..i].parse::<f32>() {
                tokens.push(PathToken::Num(n));
            }
        } else {
            i += 1; // skip unparseable
        }
    }
    tokens
}

/// Extract the next number from tokens, advancing the index.
pub(super) fn take_num(tokens: &[PathToken], i: &mut usize) -> f32 {
    if *i < tokens.len() {
        match &tokens[*i] {
            PathToken::Num(n) => {
                let v = *n;
                *i += 1;
                return v;
            },
            PathToken::Cmd(_) => return 0.0,
        }
    }
    0.0
}

/// Flatten path commands into a list of polygon points.
///
/// Bezier curves are approximated with line segments (adaptive
/// subdivision based on curve length).
pub(super) fn flatten_commands(cmds: &[PathCmd]) -> Vec<(f32, f32)> {
    let mut points: Vec<(f32, f32)> = Vec::new();
    let mut cx = 0.0f32;
    let mut cy = 0.0f32;
    let mut start_x = 0.0f32;
    let mut start_y = 0.0f32;
    let mut last_cp_x = 0.0f32;
    let mut last_cp_y = 0.0f32;
    let mut last_was_cubic = false;
    let mut last_was_quad = false;

    for cmd in cmds {
        match *cmd {
            PathCmd::MoveTo(x, y) => {
                cx = x;
                cy = y;
                start_x = x;
                start_y = y;
                points.push((x, y));
                last_was_cubic = false;
                last_was_quad = false;
            },
            PathCmd::LineTo(x, y) => {
                cx = x;
                cy = y;
                points.push((x, y));
                last_was_cubic = false;
                last_was_quad = false;
            },
            PathCmd::HorizTo(x) => {
                cx = x;
                points.push((cx, cy));
                last_was_cubic = false;
                last_was_quad = false;
            },
            PathCmd::VertTo(y) => {
                cy = y;
                points.push((cx, cy));
                last_was_cubic = false;
                last_was_quad = false;
            },
            PathCmd::CubicTo(x1, y1, x2, y2, x, y) => {
                flatten_cubic(&mut points, cx, cy, x1, y1, x2, y2, x, y);
                last_cp_x = x2;
                last_cp_y = y2;
                cx = x;
                cy = y;
                last_was_cubic = true;
                last_was_quad = false;
            },
            PathCmd::SmoothCubicTo(x2, y2, x, y) => {
                let (x1, y1) = if last_was_cubic {
                    (2.0 * cx - last_cp_x, 2.0 * cy - last_cp_y)
                } else {
                    (cx, cy)
                };
                flatten_cubic(&mut points, cx, cy, x1, y1, x2, y2, x, y);
                last_cp_x = x2;
                last_cp_y = y2;
                cx = x;
                cy = y;
                last_was_cubic = true;
                last_was_quad = false;
            },
            PathCmd::QuadTo(x1, y1, x, y) => {
                flatten_quad(&mut points, cx, cy, x1, y1, x, y);
                last_cp_x = x1;
                last_cp_y = y1;
                cx = x;
                cy = y;
                last_was_quad = true;
                last_was_cubic = false;
            },
            PathCmd::SmoothQuadTo(x, y) => {
                let (x1, y1) = if last_was_quad {
                    (2.0 * cx - last_cp_x, 2.0 * cy - last_cp_y)
                } else {
                    (cx, cy)
                };
                flatten_quad(&mut points, cx, cy, x1, y1, x, y);
                last_cp_x = x1;
                last_cp_y = y1;
                cx = x;
                cy = y;
                last_was_quad = true;
                last_was_cubic = false;
            },
            PathCmd::Close => {
                if (cx - start_x).abs() > 0.01 || (cy - start_y).abs() > 0.01 {
                    points.push((start_x, start_y));
                }
                cx = start_x;
                cy = start_y;
                last_was_cubic = false;
                last_was_quad = false;
            },
        }
    }
    points
}

/// Flatten a cubic bezier curve into line segments.
///
/// Arguments: `(x0, y0)` start, `(x1, y1)` control 1, `(x2, y2)` control 2, `(x3, y3)` end.
#[allow(clippy::too_many_arguments)]
pub(crate) fn flatten_cubic(
    points: &mut Vec<(f32, f32)>,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    x3: f32,
    y3: f32,
) {
    flatten_cubic_inner(points, x0, y0, x1, y1, x2, y2, x3, y3, 0);
}

/// Maximum recursion depth for Bezier curve flattening. 16 levels of
/// subdivision produce up to 2^16 segments which is more than sufficient
/// for any practical curve, while preventing stack overflow from
/// pathological coordinates.
pub(super) const MAX_FLATTEN_DEPTH: u8 = 16;

#[allow(clippy::too_many_arguments)]
pub(super) fn flatten_cubic_inner(
    points: &mut Vec<(f32, f32)>,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    x3: f32,
    y3: f32,
    depth: u8,
) {
    // Adaptive subdivision: estimate flatness.
    let dx = x3 - x0;
    let dy = y3 - y0;
    let d = ((x1 - x3) * dy - (y1 - y3) * dx).abs() + ((x2 - x3) * dy - (y2 - y3) * dx).abs();
    let len_sq = dx * dx + dy * dy;
    // Tolerance: 0.5 pixels.
    if d * d <= 0.25 * len_sq || len_sq < 1.0 || depth >= MAX_FLATTEN_DEPTH {
        points.push((x3, y3));
        return;
    }
    // De Casteljau subdivision at t=0.5.
    let m01x = (x0 + x1) * 0.5;
    let m01y = (y0 + y1) * 0.5;
    let m12x = (x1 + x2) * 0.5;
    let m12y = (y1 + y2) * 0.5;
    let m23x = (x2 + x3) * 0.5;
    let m23y = (y2 + y3) * 0.5;
    let m012x = (m01x + m12x) * 0.5;
    let m012y = (m01y + m12y) * 0.5;
    let m123x = (m12x + m23x) * 0.5;
    let m123y = (m12y + m23y) * 0.5;
    let mx = (m012x + m123x) * 0.5;
    let my = (m012y + m123y) * 0.5;
    flatten_cubic_inner(points, x0, y0, m01x, m01y, m012x, m012y, mx, my, depth + 1);
    flatten_cubic_inner(points, mx, my, m123x, m123y, m23x, m23y, x3, y3, depth + 1);
}

/// Flatten a quadratic bezier curve into line segments.
pub(crate) fn flatten_quad(
    points: &mut Vec<(f32, f32)>,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
) {
    // Convert to cubic: CP1 = P0 + 2/3*(P1-P0), CP2 = P2 + 2/3*(P1-P2)
    let cx1 = x0 + (2.0 / 3.0) * (x1 - x0);
    let cy1 = y0 + (2.0 / 3.0) * (y1 - y0);
    let cx2 = x2 + (2.0 / 3.0) * (x1 - x2);
    let cy2 = y2 + (2.0 / 3.0) * (y1 - y2);
    flatten_cubic(points, x0, y0, cx1, cy1, cx2, cy2, x2, y2);
}
