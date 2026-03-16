//! Basic SVG parsing and rendering for inline SVG elements.
//!
//! Supports a minimal subset of SVG: `<rect>`, `<circle>`, `<line>`,
//! `<text>`, and `<ellipse>`. No transforms, gradients, or CSS styling --
//! only presentation attributes (`fill`, `stroke`, `stroke-width`, etc.).

use crate::html::dom::{Document, NodeId};
use oasis_types::backend::Color;

/// A 2D affine transform matrix [a, b, c, d, e, f] representing:
///   | a c e |
///   | b d f |
///   | 0 0 1 |
#[derive(Debug, Clone, Copy)]
struct AffineTransform {
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    e: f32,
    f: f32,
}

impl AffineTransform {
    fn identity() -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 0.0,
            f: 0.0,
        }
    }

    fn translate(tx: f32, ty: f32) -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: tx,
            f: ty,
        }
    }

    fn scale(sx: f32, sy: f32) -> Self {
        Self {
            a: sx,
            b: 0.0,
            c: 0.0,
            d: sy,
            e: 0.0,
            f: 0.0,
        }
    }

    fn rotate(angle_deg: f32) -> Self {
        let r = angle_deg * std::f32::consts::PI / 180.0;
        let (sin, cos) = (r.sin(), r.cos());
        Self {
            a: cos,
            b: sin,
            c: -sin,
            d: cos,
            e: 0.0,
            f: 0.0,
        }
    }

    fn multiply(&self, other: &Self) -> Self {
        Self {
            a: self.a * other.a + self.c * other.b,
            b: self.b * other.a + self.d * other.b,
            c: self.a * other.c + self.c * other.d,
            d: self.b * other.c + self.d * other.d,
            e: self.a * other.e + self.c * other.f + self.e,
            f: self.b * other.e + self.d * other.f + self.f,
        }
    }

    fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        (
            self.a * x + self.c * y + self.e,
            self.b * x + self.d * y + self.f,
        )
    }
}

/// Parse SVG `transform` attribute into an affine matrix.
fn parse_transform_attr(s: &str) -> AffineTransform {
    let mut result = AffineTransform::identity();
    let mut input = s;
    while let Some(paren_start) = input.find('(') {
        let func_name = input[..paren_start].trim();
        // Extract the last word (the function name) in case of chained transforms.
        let func_name = func_name
            .rsplit(|c: char| c.is_ascii_whitespace() || c == ',')
            .next()
            .unwrap_or(func_name);
        if let Some(paren_end) = input[paren_start..].find(')') {
            let args_str = &input[paren_start + 1..paren_start + paren_end];
            let args: Vec<f32> = args_str
                .split(|c: char| c == ',' || c.is_ascii_whitespace())
                .filter(|s| !s.is_empty())
                .filter_map(|s| s.parse::<f32>().ok())
                .collect();
            let t = match func_name {
                "translate" => {
                    let tx = args.first().copied().unwrap_or(0.0);
                    let ty = args.get(1).copied().unwrap_or(0.0);
                    AffineTransform::translate(tx, ty)
                },
                "scale" => {
                    let sx = args.first().copied().unwrap_or(1.0);
                    let sy = args.get(1).copied().unwrap_or(sx);
                    AffineTransform::scale(sx, sy)
                },
                "rotate" => {
                    let angle = args.first().copied().unwrap_or(0.0);
                    if args.len() >= 3 {
                        let cx = args[1];
                        let cy = args[2];
                        AffineTransform::translate(cx, cy)
                            .multiply(&AffineTransform::rotate(angle))
                            .multiply(&AffineTransform::translate(-cx, -cy))
                    } else {
                        AffineTransform::rotate(angle)
                    }
                },
                "matrix" if args.len() >= 6 => AffineTransform {
                    a: args[0],
                    b: args[1],
                    c: args[2],
                    d: args[3],
                    e: args[4],
                    f: args[5],
                },
                _ => AffineTransform::identity(),
            };
            result = result.multiply(&t);
            input = &input[paren_start + paren_end + 1..];
        } else {
            break;
        }
    }
    result
}

/// A parsed SVG element ready for rendering.
#[derive(Debug, Clone)]
pub struct SvgElement {
    /// Intrinsic width from the `width` attribute (or viewBox).
    pub width: f32,
    /// Intrinsic height from the `height` attribute (or viewBox).
    pub height: f32,
    /// Optional viewBox: (min-x, min-y, width, height).
    pub viewbox: Option<(f32, f32, f32, f32)>,
    /// Shapes parsed from the SVG children.
    pub shapes: Vec<SvgShape>,
}

/// A single SVG shape primitive.
#[derive(Debug, Clone)]
pub enum SvgShape {
    Rect {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        fill: Option<Color>,
        stroke: Option<Color>,
        stroke_width: f32,
        rx: f32,
    },
    Circle {
        cx: f32,
        cy: f32,
        r: f32,
        fill: Option<Color>,
        stroke: Option<Color>,
        stroke_width: f32,
    },
    Ellipse {
        cx: f32,
        cy: f32,
        rx: f32,
        ry: f32,
        fill: Option<Color>,
        stroke: Option<Color>,
        stroke_width: f32,
    },
    Line {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        stroke: Option<Color>,
        stroke_width: f32,
    },
    Text {
        x: f32,
        y: f32,
        text: String,
        fill: Color,
        font_size: f32,
    },
    /// SVG `<path>` element — flattened to polygon points.
    Path {
        points: Vec<(f32, f32)>,
        fill: Option<Color>,
        stroke: Option<Color>,
        stroke_width: f32,
    },
    /// SVG `<polygon>` element.
    Polygon {
        points: Vec<(f32, f32)>,
        fill: Option<Color>,
        stroke: Option<Color>,
        stroke_width: f32,
    },
    /// SVG `<polyline>` element (no auto-close).
    Polyline {
        points: Vec<(f32, f32)>,
        stroke: Option<Color>,
        stroke_width: f32,
    },
}

/// SVG path command (parsed from `d` attribute).
#[derive(Debug, Clone, Copy)]
enum PathCmd {
    MoveTo(f32, f32),
    LineTo(f32, f32),
    HorizTo(f32),
    VertTo(f32),
    CubicTo(f32, f32, f32, f32, f32, f32),
    SmoothCubicTo(f32, f32, f32, f32),
    QuadTo(f32, f32, f32, f32),
    SmoothQuadTo(f32, f32),
    Close,
}

/// Parse an `<svg>` DOM node into an [`SvgElement`].
///
/// Returns `None` if the node is not an SVG element or has no
/// parseable content.
pub fn parse_svg(doc: &Document, svg_node: NodeId) -> Option<SvgElement> {
    let elem = doc.element(svg_node)?;
    if elem.tag.as_str() != "svg" {
        return None;
    }

    // Parse viewBox first -- it provides fallback dimensions.
    let viewbox = elem.get_attribute("viewBox").and_then(parse_viewbox);

    // Width/height from attributes, falling back to viewBox, then 300x150.
    let width = elem
        .get_attribute("width")
        .and_then(parse_length)
        .or_else(|| viewbox.map(|(_, _, w, _)| w))
        .unwrap_or(300.0);
    let height = elem
        .get_attribute("height")
        .and_then(parse_length)
        .or_else(|| viewbox.map(|(_, _, _, h)| h))
        .unwrap_or(150.0);

    let mut shapes = Vec::new();
    let children = doc.get(svg_node).children.clone();
    for &child_id in &children {
        if let Some(shape) = parse_shape(doc, child_id) {
            shapes.push(shape);
        }
    }

    Some(SvgElement {
        width,
        height,
        viewbox,
        shapes,
    })
}

/// Apply a transform to a shape's coordinates (translating key points).
fn apply_transform_to_shape(shape: &mut SvgShape, xf: &AffineTransform) {
    match shape {
        SvgShape::Rect { x, y, .. } => {
            let (nx, ny) = xf.apply(*x, *y);
            *x = nx;
            *y = ny;
        },
        SvgShape::Circle { cx, cy, .. } => {
            let (nx, ny) = xf.apply(*cx, *cy);
            *cx = nx;
            *cy = ny;
        },
        SvgShape::Ellipse { cx, cy, .. } => {
            let (nx, ny) = xf.apply(*cx, *cy);
            *cx = nx;
            *cy = ny;
        },
        SvgShape::Line { x1, y1, x2, y2, .. } => {
            let (nx1, ny1) = xf.apply(*x1, *y1);
            let (nx2, ny2) = xf.apply(*x2, *y2);
            *x1 = nx1;
            *y1 = ny1;
            *x2 = nx2;
            *y2 = ny2;
        },
        SvgShape::Text { x, y, .. } => {
            let (nx, ny) = xf.apply(*x, *y);
            *x = nx;
            *y = ny;
        },
        SvgShape::Path { points, .. }
        | SvgShape::Polygon { points, .. }
        | SvgShape::Polyline { points, .. } => {
            for pt in points.iter_mut() {
                let (nx, ny) = xf.apply(pt.0, pt.1);
                pt.0 = nx;
                pt.1 = ny;
            }
        },
    }
}

/// Parse a single SVG child element into a shape.
fn parse_shape(doc: &Document, node_id: NodeId) -> Option<SvgShape> {
    let elem = doc.element(node_id)?;
    let tag = elem.tag.as_str();

    // Parse optional transform attribute.
    let transform = elem.get_attribute("transform").map(parse_transform_attr);

    let mut shape = parse_shape_inner(doc, node_id, elem, tag)?;

    if let Some(xf) = transform {
        apply_transform_to_shape(&mut shape, &xf);
    }

    Some(shape)
}

/// Inner shape parsing (without transform application).
fn parse_shape_inner(
    doc: &Document,
    node_id: NodeId,
    elem: &ElementData,
    tag: &str,
) -> Option<SvgShape> {
    match tag {
        "rect" => {
            let x = attr_f32(elem, "x");
            let y = attr_f32(elem, "y");
            let width = attr_f32(elem, "width");
            let height = attr_f32(elem, "height");
            let fill = attr_fill(elem);
            let stroke = attr_color(elem, "stroke");
            let stroke_width = attr_f32_or(elem, "stroke-width", 1.0);
            let rx = attr_f32(elem, "rx");
            Some(SvgShape::Rect {
                x,
                y,
                width,
                height,
                fill,
                stroke,
                stroke_width,
                rx,
            })
        },
        "circle" => {
            let cx = attr_f32(elem, "cx");
            let cy = attr_f32(elem, "cy");
            let r = attr_f32(elem, "r");
            let fill = attr_fill(elem);
            let stroke = attr_color(elem, "stroke");
            let stroke_width = attr_f32_or(elem, "stroke-width", 1.0);
            Some(SvgShape::Circle {
                cx,
                cy,
                r,
                fill,
                stroke,
                stroke_width,
            })
        },
        "ellipse" => {
            let cx = attr_f32(elem, "cx");
            let cy = attr_f32(elem, "cy");
            let rx = attr_f32(elem, "rx");
            let ry = attr_f32(elem, "ry");
            let fill = attr_fill(elem);
            let stroke = attr_color(elem, "stroke");
            let stroke_width = attr_f32_or(elem, "stroke-width", 1.0);
            Some(SvgShape::Ellipse {
                cx,
                cy,
                rx,
                ry,
                fill,
                stroke,
                stroke_width,
            })
        },
        "line" => {
            let x1 = attr_f32(elem, "x1");
            let y1 = attr_f32(elem, "y1");
            let x2 = attr_f32(elem, "x2");
            let y2 = attr_f32(elem, "y2");
            let stroke = attr_color(elem, "stroke");
            let stroke_width = attr_f32_or(elem, "stroke-width", 1.0);
            Some(SvgShape::Line {
                x1,
                y1,
                x2,
                y2,
                stroke,
                stroke_width,
            })
        },
        "path" => {
            let d = elem.get_attribute("d").unwrap_or("");
            let points = flatten_path_data(d);
            if points.len() < 2 {
                return None;
            }
            let fill = attr_fill(elem);
            let stroke = attr_color(elem, "stroke");
            let stroke_width = attr_f32_or(elem, "stroke-width", 1.0);
            Some(SvgShape::Path {
                points,
                fill,
                stroke,
                stroke_width,
            })
        },
        "polygon" => {
            let pts = elem
                .get_attribute("points")
                .map(parse_point_list)
                .unwrap_or_default();
            if pts.len() < 2 {
                return None;
            }
            let fill = attr_fill(elem);
            let stroke = attr_color(elem, "stroke");
            let stroke_width = attr_f32_or(elem, "stroke-width", 1.0);
            Some(SvgShape::Polygon {
                points: pts,
                fill,
                stroke,
                stroke_width,
            })
        },
        "polyline" => {
            let pts = elem
                .get_attribute("points")
                .map(parse_point_list)
                .unwrap_or_default();
            if pts.len() < 2 {
                return None;
            }
            let stroke = attr_color(elem, "stroke");
            let stroke_width = attr_f32_or(elem, "stroke-width", 1.0);
            Some(SvgShape::Polyline {
                points: pts,
                stroke,
                stroke_width,
            })
        },
        "text" => {
            let x = attr_f32(elem, "x");
            let y = attr_f32(elem, "y");
            let fill = attr_color(elem, "fill").unwrap_or(Color::rgb(0, 0, 0));
            let font_size = attr_f32_or(elem, "font-size", 16.0);
            let text = doc.text_content(node_id);
            if text.trim().is_empty() {
                return None;
            }
            Some(SvgShape::Text {
                x,
                y,
                text: text.trim().to_string(),
                fill,
                font_size,
            })
        },
        _ => None,
    }
}

// -------------------------------------------------------------------
// Attribute helpers
// -------------------------------------------------------------------

use crate::html::dom::ElementData;

/// Get a float attribute, defaulting to 0.0 if missing or unparseable.
fn attr_f32(elem: &ElementData, name: &str) -> f32 {
    elem.get_attribute(name)
        .and_then(parse_length)
        .unwrap_or(0.0)
}

/// Get a float attribute with a custom default.
fn attr_f32_or(elem: &ElementData, name: &str, default: f32) -> f32 {
    elem.get_attribute(name)
        .and_then(parse_length)
        .unwrap_or(default)
}

/// Parse a length value, stripping optional `px` suffix.
fn parse_length(s: &str) -> Option<f32> {
    let s = s.trim();
    let s = s.strip_suffix("px").unwrap_or(s);
    let s = s.strip_suffix("pt").unwrap_or(s);
    s.trim().parse::<f32>().ok()
}

/// Parse a viewBox attribute: "min-x min-y width height".
fn parse_viewbox(s: &str) -> Option<(f32, f32, f32, f32)> {
    let parts: Vec<f32> = s
        .split(|c: char| c == ',' || c.is_ascii_whitespace())
        .filter(|p| !p.is_empty())
        .filter_map(|p| p.trim().parse::<f32>().ok())
        .collect();
    if parts.len() == 4 {
        Some((parts[0], parts[1], parts[2], parts[3]))
    } else {
        None
    }
}

/// Parse a color attribute value.
///
/// Supports: named colors, `#RGB`, `#RRGGBB`, `rgb(r,g,b)`,
/// `none`, and `transparent`.
fn attr_color(elem: &ElementData, name: &str) -> Option<Color> {
    let val = elem.get_attribute(name)?;
    parse_svg_color(val)
}

/// Parse a fill attribute, distinguishing between absent (default black)
/// and explicitly set to `"none"` (no fill).
fn attr_fill(elem: &ElementData) -> Option<Color> {
    match elem.get_attribute("fill") {
        None => Some(Color::rgb(0, 0, 0)), // SVG default fill is black
        Some(val) => parse_svg_color(val), // "none" → None, color → Some
    }
}

/// Parse a CSS/SVG color string.
pub(crate) fn parse_svg_color(val: &str) -> Option<Color> {
    let val = val.trim();
    if val.eq_ignore_ascii_case("none") || val.eq_ignore_ascii_case("transparent") {
        return None;
    }
    // Hex colors
    if let Some(hex) = val.strip_prefix('#') {
        return parse_hex_color(hex);
    }
    // rgb(r, g, b)
    if let Some(inner) = val.strip_prefix("rgb(").and_then(|s| s.strip_suffix(')')) {
        let parts: Vec<&str> = inner.split(',').collect();
        if parts.len() == 3 {
            let r = parts[0].trim().parse::<u8>().ok()?;
            let g = parts[1].trim().parse::<u8>().ok()?;
            let b = parts[2].trim().parse::<u8>().ok()?;
            return Some(Color::rgb(r, g, b));
        }
        return None;
    }
    // Named colors
    named_color(val)
}

/// Parse a hex color: `RGB` or `RRGGBB`.
fn parse_hex_color(hex: &str) -> Option<Color> {
    match hex.len() {
        3 => {
            let r = u8::from_str_radix(&hex[0..1], 16).ok()?;
            let g = u8::from_str_radix(&hex[1..2], 16).ok()?;
            let b = u8::from_str_radix(&hex[2..3], 16).ok()?;
            Some(Color::rgb(r * 17, g * 17, b * 17))
        },
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some(Color::rgb(r, g, b))
        },
        _ => None,
    }
}

/// Map a named color string to a `Color`.
fn named_color(name: &str) -> Option<Color> {
    let c = match name.to_ascii_lowercase().as_str() {
        "black" => Color::rgb(0, 0, 0),
        "white" => Color::rgb(255, 255, 255),
        "red" => Color::rgb(255, 0, 0),
        "green" => Color::rgb(0, 128, 0),
        "blue" => Color::rgb(0, 0, 255),
        "yellow" => Color::rgb(255, 255, 0),
        "cyan" | "aqua" => Color::rgb(0, 255, 255),
        "magenta" | "fuchsia" => Color::rgb(255, 0, 255),
        "gray" | "grey" => Color::rgb(128, 128, 128),
        "silver" => Color::rgb(192, 192, 192),
        "maroon" => Color::rgb(128, 0, 0),
        "olive" => Color::rgb(128, 128, 0),
        "lime" => Color::rgb(0, 255, 0),
        "teal" => Color::rgb(0, 128, 128),
        "navy" => Color::rgb(0, 0, 128),
        "purple" => Color::rgb(128, 0, 128),
        "orange" => Color::rgb(255, 165, 0),
        "brown" => Color::rgb(165, 42, 42),
        "pink" => Color::rgb(255, 192, 203),
        "coral" => Color::rgb(255, 127, 80),
        "gold" => Color::rgb(255, 215, 0),
        "currentcolor" | "currentColor" => Color::rgb(0, 0, 0),
        _ => return None,
    };
    Some(c)
}

// -------------------------------------------------------------------
// SVG path `d` attribute parser
// -------------------------------------------------------------------

/// Parse an SVG `points` attribute (e.g. "100,10 40,198 190,78").
fn parse_point_list(s: &str) -> Vec<(f32, f32)> {
    let nums: Vec<f32> = s
        .split(|c: char| c == ',' || c.is_ascii_whitespace())
        .filter(|p| !p.is_empty())
        .filter_map(|p| p.parse::<f32>().ok())
        .collect();
    nums.chunks_exact(2).map(|c| (c[0], c[1])).collect()
}

/// Parse SVG path `d` attribute into commands, then flatten curves
/// into a polygon point list suitable for `fill_polygon`.
fn flatten_path_data(d: &str) -> Vec<(f32, f32)> {
    let cmds = parse_path_commands(d);
    flatten_commands(&cmds)
}

/// Tokenize and parse SVG path `d` data into [`PathCmd`] list.
///
/// Supports: M/m L/l H/h V/v C/c S/s Q/q T/t Z/z.
fn parse_path_commands(d: &str) -> Vec<PathCmd> {
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
enum PathToken {
    Cmd(u8),
    Num(f32),
}

/// Tokenize SVG path `d` data into command letters and numbers.
fn tokenize_path(d: &str) -> Vec<PathToken> {
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
fn take_num(tokens: &[PathToken], i: &mut usize) -> f32 {
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
fn flatten_commands(cmds: &[PathCmd]) -> Vec<(f32, f32)> {
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
fn flatten_cubic(
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
    // Adaptive subdivision: estimate flatness.
    let dx = x3 - x0;
    let dy = y3 - y0;
    let d = ((x1 - x3) * dy - (y1 - y3) * dx).abs() + ((x2 - x3) * dy - (y2 - y3) * dx).abs();
    let len_sq = dx * dx + dy * dy;
    // Tolerance: 0.5 pixels.
    if d * d <= 0.25 * len_sq || len_sq < 1.0 {
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
    flatten_cubic(points, x0, y0, m01x, m01y, m012x, m012y, mx, my);
    flatten_cubic(points, mx, my, m123x, m123y, m23x, m23y, x3, y3);
}

/// Flatten a quadratic bezier curve into line segments.
fn flatten_quad(
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

// -------------------------------------------------------------------
// Rendering
// -------------------------------------------------------------------

use oasis_types::backend::SdiBackend;
use oasis_types::error::Result;

/// Paint an SVG element into the given backend at the specified position.
///
/// `content_x` and `content_y` are the top-left corner of the content
/// box in screen coordinates. `content_w` and `content_h` are the
/// layout dimensions of the SVG box.
pub fn paint_svg(
    svg: &SvgElement,
    backend: &mut dyn SdiBackend,
    content_x: i32,
    content_y: i32,
    content_w: f32,
    content_h: f32,
) -> Result<()> {
    // Compute scale factors from viewBox (or intrinsic size) to layout box.
    let (vb_x, vb_y, vb_w, vb_h) = svg.viewbox.unwrap_or((0.0, 0.0, svg.width, svg.height));
    let scale_x = if vb_w > 0.0 { content_w / vb_w } else { 1.0 };
    let scale_y = if vb_h > 0.0 { content_h / vb_h } else { 1.0 };

    let xf = SvgTransform {
        ox: content_x,
        oy: content_y,
        vb_x,
        vb_y,
        sx: scale_x,
        sy: scale_y,
    };

    for shape in &svg.shapes {
        paint_shape(shape, backend, &xf)?;
    }

    Ok(())
}

/// Pre-computed transform from SVG viewBox coordinates to screen pixels.
struct SvgTransform {
    ox: i32,
    oy: i32,
    vb_x: f32,
    vb_y: f32,
    sx: f32,
    sy: f32,
}

/// Paint a single SVG shape.
fn paint_shape(shape: &SvgShape, backend: &mut dyn SdiBackend, xf: &SvgTransform) -> Result<()> {
    let (ox, oy, vb_x, vb_y, sx, sy) = (xf.ox, xf.oy, xf.vb_x, xf.vb_y, xf.sx, xf.sy);
    match shape {
        SvgShape::Rect {
            x,
            y,
            width,
            height,
            fill,
            stroke,
            stroke_width,
            rx,
        } => {
            let px = ox + ((x - vb_x) * sx) as i32;
            let py = oy + ((y - vb_y) * sy) as i32;
            let pw = (width * sx) as u32;
            let ph = (height * sy) as u32;
            if pw == 0 || ph == 0 {
                return Ok(());
            }
            if let Some(fc) = fill {
                let r = (rx * sx.min(sy)) as u16;
                if r > 0 {
                    backend.fill_rounded_rect(px, py, pw, ph, r, *fc)?;
                } else {
                    backend.fill_rect(px, py, pw, ph, *fc)?;
                }
            }
            if let Some(sc) = stroke {
                let sw = (stroke_width * sx.min(sy)).max(1.0) as u32;
                // Top
                backend.fill_rect(px, py, pw, sw, *sc)?;
                // Bottom
                backend.fill_rect(px, py + ph as i32 - sw as i32, pw, sw, *sc)?;
                // Left (between top and bottom to avoid corner overlap)
                backend.fill_rect(px, py + sw as i32, sw, ph.saturating_sub(sw * 2), *sc)?;
                // Right (between top and bottom to avoid corner overlap)
                backend.fill_rect(
                    px + pw as i32 - sw as i32,
                    py + sw as i32,
                    sw,
                    ph.saturating_sub(sw * 2),
                    *sc,
                )?;
            }
        },
        SvgShape::Circle {
            cx,
            cy,
            r,
            fill,
            stroke,
            stroke_width,
        } => {
            let px = ox + ((cx - vb_x) * sx) as i32;
            let py = oy + ((cy - vb_y) * sy) as i32;
            let radius = (r * sx.min(sy)) as u16;
            if radius == 0 {
                return Ok(());
            }
            if let Some(fc) = fill {
                backend.fill_circle(px, py, radius, *fc)?;
            }
            if let Some(sc) = stroke {
                let sw = (stroke_width * sx.min(sy)).max(1.0) as u16;
                backend.stroke_circle(px, py, radius, sw, *sc)?;
            }
        },
        SvgShape::Ellipse {
            cx,
            cy,
            rx,
            ry,
            fill,
            stroke,
            stroke_width,
        } => {
            // Approximate ellipse with a rounded rect.
            let erx = (rx * sx) as i32;
            let ery = (ry * sy) as i32;
            let px = ox + ((cx - vb_x) * sx) as i32 - erx;
            let py = oy + ((cy - vb_y) * sy) as i32 - ery;
            let pw = (erx * 2) as u32;
            let ph = (ery * 2) as u32;
            if pw == 0 || ph == 0 {
                return Ok(());
            }
            let r = (erx as u16).min(ery as u16);
            if let Some(fc) = fill {
                backend.fill_rounded_rect(px, py, pw, ph, r, *fc)?;
            }
            if let Some(sc) = stroke {
                let sw = (stroke_width * sx.min(sy)).max(1.0) as u16;
                backend.stroke_rounded_rect(px, py, pw, ph, r, sw, *sc)?;
            }
        },
        SvgShape::Line {
            x1,
            y1,
            x2,
            y2,
            stroke,
            stroke_width,
        } => {
            // SVG lines with stroke=none are invisible.
            let sc = match stroke {
                Some(c) => *c,
                None => return Ok(()),
            };
            // Render horizontal/vertical lines accurately; diagonal lines
            // are approximated as a filled rect between the two endpoints.
            let px1 = ox + ((x1 - vb_x) * sx) as i32;
            let py1 = oy + ((y1 - vb_y) * sy) as i32;
            let px2 = ox + ((x2 - vb_x) * sx) as i32;
            let py2 = oy + ((y2 - vb_y) * sy) as i32;
            let sw = (stroke_width * sx.min(sy)).max(1.0) as u32;

            let dx = (px2 - px1).abs();
            let dy = (py2 - py1).abs();
            if dx == 0 || dy == 0 {
                // Horizontal or vertical: a single rect is exact.
                let lx = px1.min(px2);
                let ly = py1.min(py2);
                let lw = (dx as u32).max(sw);
                let lh = (dy as u32).max(sw);
                backend.fill_rect(lx, ly, lw, lh, sc)?;
            } else {
                // Diagonal: plot 1px rects along the dominant axis
                // (Bresenham-like approximation).
                let steps = dx.max(dy);
                for s in 0..=steps {
                    let t = s as f32 / steps.max(1) as f32;
                    let px = px1 + ((px2 - px1) as f32 * t) as i32;
                    let py = py1 + ((py2 - py1) as f32 * t) as i32;
                    backend.fill_rect(px, py, sw, sw, sc)?;
                }
            }
        },
        SvgShape::Text {
            x,
            y,
            text,
            fill,
            font_size,
        } => {
            let px = ox + ((x - vb_x) * sx) as i32;
            // SVG text y is the baseline; adjust up by ~font_size for
            // the top-left rendering used by draw_text.
            let scaled_fs = (font_size * sy).max(1.0);
            let py = oy + ((y - vb_y) * sy) as i32 - scaled_fs as i32;
            backend.draw_text(text, px, py, scaled_fs as u16, *fill)?;
        },
        SvgShape::Path {
            points,
            fill,
            stroke,
            stroke_width,
        }
        | SvgShape::Polygon {
            points,
            fill,
            stroke,
            stroke_width,
        } => {
            paint_polygon_shape(points, *fill, *stroke, *stroke_width, backend, xf)?;
        },
        SvgShape::Polyline {
            points,
            stroke,
            stroke_width,
        } => {
            // Polyline: stroke only, no fill.
            if let Some(sc) = stroke {
                let sw = (stroke_width * sx.min(sy)).max(1.0) as u32;
                for window in points.windows(2) {
                    let (px1, py1) = xf_point(window[0].0, window[0].1, xf);
                    let (px2, py2) = xf_point(window[1].0, window[1].1, xf);
                    stroke_line_bresenham(backend, px1, py1, px2, py2, sw, *sc)?;
                }
            }
        },
    }
    Ok(())
}

/// Transform a point from SVG viewBox coordinates to screen pixels.
fn xf_point(x: f32, y: f32, xf: &SvgTransform) -> (i32, i32) {
    (
        xf.ox + ((x - xf.vb_x) * xf.sx) as i32,
        xf.oy + ((y - xf.vb_y) * xf.sy) as i32,
    )
}

/// Paint a filled and/or stroked polygon (used by Path and Polygon shapes).
fn paint_polygon_shape(
    points: &[(f32, f32)],
    fill: Option<Color>,
    stroke: Option<Color>,
    stroke_width: f32,
    backend: &mut dyn SdiBackend,
    xf: &SvgTransform,
) -> Result<()> {
    if points.len() < 2 {
        return Ok(());
    }
    let screen_pts: Vec<(i32, i32)> = points.iter().map(|&(x, y)| xf_point(x, y, xf)).collect();

    if let Some(fc) = fill {
        backend.fill_polygon(&screen_pts, fc)?;
    }
    if let Some(sc) = stroke {
        let sw = (stroke_width * xf.sx.min(xf.sy)).max(1.0) as u32;
        for window in screen_pts.windows(2) {
            stroke_line_bresenham(
                backend,
                window[0].0,
                window[0].1,
                window[1].0,
                window[1].1,
                sw,
                sc,
            )?;
        }
        // Close the path (last → first).
        if let (Some(last), Some(first)) = (screen_pts.last(), screen_pts.first())
            && last != first
        {
            stroke_line_bresenham(backend, last.0, last.1, first.0, first.1, sw, sc)?;
        }
    }
    Ok(())
}

/// Stroke a line between two points using filled rectangles (Bresenham-like).
fn stroke_line_bresenham(
    backend: &mut dyn SdiBackend,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    sw: u32,
    color: Color,
) -> Result<()> {
    let dx = (x2 - x1).abs();
    let dy = (y2 - y1).abs();
    if dx == 0 && dy == 0 {
        backend.fill_rect(x1, y1, sw, sw, color)?;
    } else if dx == 0 || dy == 0 {
        let lx = x1.min(x2);
        let ly = y1.min(y2);
        let lw = (dx as u32).max(sw);
        let lh = (dy as u32).max(sw);
        backend.fill_rect(lx, ly, lw, lh, color)?;
    } else {
        let steps = dx.max(dy);
        for s in 0..=steps {
            let t = s as f32 / steps.max(1) as f32;
            let px = x1 + ((x2 - x1) as f32 * t) as i32;
            let py = y1 + ((y2 - y1) as f32 * t) as i32;
            backend.fill_rect(px, py, sw, sw, color)?;
        }
    }
    Ok(())
}

// -------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::dom::{Attribute, Document, ElementData, NodeKind, TagName};

    fn make_svg_doc(
        svg_attrs: &[(&str, &str)],
        children_html: &[(&str, &[(&str, &str)])],
    ) -> (Document, NodeId) {
        let mut doc = Document::new();
        let mut svg_data = ElementData::new(TagName::Svg);
        for &(name, value) in svg_attrs {
            svg_data.attributes.push(Attribute {
                name: name.to_string(),
                value: value.to_string(),
            });
        }
        let svg_id = doc.add_node(NodeKind::Element(svg_data));
        doc.append_child(doc.root, svg_id);

        for &(tag, attrs) in children_html {
            let mut elem = ElementData::new(TagName::Unknown(tag.to_string()));
            for &(name, value) in attrs {
                elem.attributes.push(Attribute {
                    name: name.to_string(),
                    value: value.to_string(),
                });
            }
            let child_id = doc.add_node(NodeKind::Element(elem));
            doc.append_child(svg_id, child_id);
        }

        (doc, svg_id)
    }

    #[test]
    fn parse_svg_basic_rect() {
        let (doc, svg_id) = make_svg_doc(
            &[("width", "100"), ("height", "50")],
            &[(
                "rect",
                &[
                    ("x", "10"),
                    ("y", "5"),
                    ("width", "80"),
                    ("height", "40"),
                    ("fill", "red"),
                ],
            )],
        );
        let svg = parse_svg(&doc, svg_id).expect("should parse");
        assert_eq!(svg.width, 100.0);
        assert_eq!(svg.height, 50.0);
        assert_eq!(svg.shapes.len(), 1);
        match &svg.shapes[0] {
            SvgShape::Rect {
                x,
                y,
                width,
                height,
                fill,
                ..
            } => {
                assert_eq!(*x, 10.0);
                assert_eq!(*y, 5.0);
                assert_eq!(*width, 80.0);
                assert_eq!(*height, 40.0);
                assert_eq!(*fill, Some(Color::rgb(255, 0, 0)));
            },
            _ => panic!("expected Rect"),
        }
    }

    #[test]
    fn parse_svg_circle() {
        let (doc, svg_id) = make_svg_doc(
            &[("width", "100"), ("height", "100")],
            &[(
                "circle",
                &[("cx", "50"), ("cy", "50"), ("r", "25"), ("fill", "#0f0")],
            )],
        );
        let svg = parse_svg(&doc, svg_id).expect("should parse");
        assert_eq!(svg.shapes.len(), 1);
        match &svg.shapes[0] {
            SvgShape::Circle {
                cx, cy, r, fill, ..
            } => {
                assert_eq!(*cx, 50.0);
                assert_eq!(*cy, 50.0);
                assert_eq!(*r, 25.0);
                assert_eq!(*fill, Some(Color::rgb(0, 255, 0)));
            },
            _ => panic!("expected Circle"),
        }
    }

    #[test]
    fn parse_svg_line() {
        let (doc, svg_id) = make_svg_doc(
            &[("width", "200"), ("height", "100")],
            &[(
                "line",
                &[
                    ("x1", "0"),
                    ("y1", "0"),
                    ("x2", "200"),
                    ("y2", "100"),
                    ("stroke", "blue"),
                ],
            )],
        );
        let svg = parse_svg(&doc, svg_id).expect("should parse");
        assert_eq!(svg.shapes.len(), 1);
        match &svg.shapes[0] {
            SvgShape::Line {
                x1,
                y1,
                x2,
                y2,
                stroke,
                ..
            } => {
                assert_eq!(*x1, 0.0);
                assert_eq!(*y1, 0.0);
                assert_eq!(*x2, 200.0);
                assert_eq!(*y2, 100.0);
                assert_eq!(*stroke, Some(Color::rgb(0, 0, 255)));
            },
            _ => panic!("expected Line"),
        }
    }

    #[test]
    fn parse_svg_text() {
        let mut doc = Document::new();
        let mut svg_data = ElementData::new(TagName::Svg);
        svg_data.attributes.push(Attribute {
            name: "width".to_string(),
            value: "200".to_string(),
        });
        svg_data.attributes.push(Attribute {
            name: "height".to_string(),
            value: "50".to_string(),
        });
        let svg_id = doc.add_node(NodeKind::Element(svg_data));
        doc.append_child(doc.root, svg_id);

        let mut text_elem = ElementData::new(TagName::Unknown("text".to_string()));
        text_elem.attributes.push(Attribute {
            name: "x".to_string(),
            value: "10".to_string(),
        });
        text_elem.attributes.push(Attribute {
            name: "y".to_string(),
            value: "30".to_string(),
        });
        text_elem.attributes.push(Attribute {
            name: "fill".to_string(),
            value: "navy".to_string(),
        });
        let text_id = doc.add_node(NodeKind::Element(text_elem));
        doc.append_child(svg_id, text_id);
        let txt = doc.add_node(NodeKind::Text("Hello SVG".to_string()));
        doc.append_child(text_id, txt);

        let svg = parse_svg(&doc, svg_id).expect("should parse");
        assert_eq!(svg.shapes.len(), 1);
        match &svg.shapes[0] {
            SvgShape::Text {
                x, y, text, fill, ..
            } => {
                assert_eq!(*x, 10.0);
                assert_eq!(*y, 30.0);
                assert_eq!(text, "Hello SVG");
                assert_eq!(*fill, Color::rgb(0, 0, 128));
            },
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn parse_viewbox() {
        let (doc, svg_id) = make_svg_doc(&[("viewBox", "0 0 100 50")], &[]);
        let svg = parse_svg(&doc, svg_id).expect("should parse");
        assert_eq!(svg.width, 100.0);
        assert_eq!(svg.height, 50.0);
        assert_eq!(svg.viewbox, Some((0.0, 0.0, 100.0, 50.0)));
    }

    #[test]
    fn parse_hex_colors() {
        assert_eq!(parse_svg_color("#f00"), Some(Color::rgb(255, 0, 0)));
        assert_eq!(parse_svg_color("#00ff00"), Some(Color::rgb(0, 255, 0)));
        assert_eq!(parse_svg_color("none"), None);
        assert_eq!(parse_svg_color("transparent"), None);
    }

    #[test]
    fn parse_rgb_color() {
        assert_eq!(
            parse_svg_color("rgb(128, 64, 32)"),
            Some(Color::rgb(128, 64, 32))
        );
    }

    #[test]
    fn parse_named_colors() {
        assert_eq!(parse_svg_color("red"), Some(Color::rgb(255, 0, 0)));
        assert_eq!(parse_svg_color("White"), Some(Color::rgb(255, 255, 255)));
        assert_eq!(parse_svg_color("unknown_color"), None);
    }

    #[test]
    fn parse_length_strips_px() {
        assert_eq!(parse_length("42px"), Some(42.0));
        assert_eq!(parse_length("3.5"), Some(3.5));
        assert_eq!(parse_length("bad"), None);
    }

    #[test]
    fn parse_viewbox_comma_separated() {
        assert_eq!(
            super::parse_viewbox("0,0,100,50"),
            Some((0.0, 0.0, 100.0, 50.0))
        );
    }

    #[test]
    fn parse_svg_returns_none_for_non_svg() {
        let mut doc = Document::new();
        let div = doc.add_node(NodeKind::Element(ElementData::new(TagName::Div)));
        doc.append_child(doc.root, div);
        assert!(parse_svg(&doc, div).is_none());
    }

    #[test]
    fn svg_default_dimensions() {
        let (doc, svg_id) = make_svg_doc(&[], &[]);
        let svg = parse_svg(&doc, svg_id).expect("should parse");
        assert_eq!(svg.width, 300.0);
        assert_eq!(svg.height, 150.0);
    }

    #[test]
    fn parse_svg_ellipse() {
        let (doc, svg_id) = make_svg_doc(
            &[("width", "100"), ("height", "100")],
            &[(
                "ellipse",
                &[("cx", "50"), ("cy", "50"), ("rx", "40"), ("ry", "20")],
            )],
        );
        let svg = parse_svg(&doc, svg_id).expect("should parse");
        assert_eq!(svg.shapes.len(), 1);
        match &svg.shapes[0] {
            SvgShape::Ellipse { cx, cy, rx, ry, .. } => {
                assert_eq!(*cx, 50.0);
                assert_eq!(*cy, 50.0);
                assert_eq!(*rx, 40.0);
                assert_eq!(*ry, 20.0);
            },
            _ => panic!("expected Ellipse"),
        }
    }
}
