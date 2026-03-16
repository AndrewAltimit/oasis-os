//! Basic SVG parsing and rendering for inline SVG elements.
//!
//! Supports a minimal subset of SVG: `<rect>`, `<circle>`, `<line>`,
//! `<text>`, and `<ellipse>`. No transforms, gradients, or CSS styling --
//! only presentation attributes (`fill`, `stroke`, `stroke-width`, etc.).

use crate::html::dom::{Document, NodeId};
use oasis_types::backend::Color;

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
        stroke: Color,
        stroke_width: f32,
    },
    Text {
        x: f32,
        y: f32,
        text: String,
        fill: Color,
        font_size: f32,
    },
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

/// Parse a single SVG child element into a shape.
fn parse_shape(doc: &Document, node_id: NodeId) -> Option<SvgShape> {
    let elem = doc.element(node_id)?;
    let tag = elem.tag.as_str();

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
            let stroke = attr_color(elem, "stroke").unwrap_or(Color::rgb(0, 0, 0));
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
                backend.fill_rect(px + pw as i32 - sw as i32, py + sw as i32, sw, ph.saturating_sub(sw * 2), *sc)?;
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
                // Approximate stroke as a slightly larger circle minus fill.
                // For simplicity, draw the outline circle.
                let outer = radius + sw;
                backend.fill_circle(px, py, outer, *sc)?;
                if let Some(fc) = fill {
                    backend.fill_circle(px, py, radius, *fc)?;
                }
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
                let sw = (stroke_width * sx.min(sy)).max(1.0) as u32;
                // Top
                backend.fill_rect(px, py, pw, sw, *sc)?;
                // Bottom
                backend.fill_rect(px, py + ph as i32 - sw as i32, pw, sw, *sc)?;
                // Left (between top and bottom to avoid corner overlap)
                backend.fill_rect(px, py + sw as i32, sw, ph.saturating_sub(sw * 2), *sc)?;
                // Right (between top and bottom to avoid corner overlap)
                backend.fill_rect(px + pw as i32 - sw as i32, py + sw as i32, sw, ph.saturating_sub(sw * 2), *sc)?;
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
                backend.fill_rect(lx, ly, lw, lh, *stroke)?;
            } else {
                // Diagonal: plot 1px rects along the dominant axis
                // (Bresenham-like approximation).
                let steps = dx.max(dy);
                for s in 0..=steps {
                    let t = s as f32 / steps.max(1) as f32;
                    let px = px1 + ((px2 - px1) as f32 * t) as i32;
                    let py = py1 + ((py2 - py1) as f32 * t) as i32;
                    backend.fill_rect(px, py, sw, sw, *stroke)?;
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
                assert_eq!(*stroke, Color::rgb(0, 0, 255));
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
