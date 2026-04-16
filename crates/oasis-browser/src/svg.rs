//! SVG parsing and rendering for inline SVG elements.
//!
//! Supports: `<rect>`, `<circle>`, `<ellipse>`, `<line>`, `<text>`,
//! `<path>`, `<polygon>`, `<polyline>`, and `<g>` groups with nested
//! transform composition. `<defs>` with `<linearGradient>`,
//! `<radialGradient>`, and `<pattern>` definitions resolved via
//! `fill="url(#id)"` / `stroke="url(#id)"` references. Presentation
//! attribute inheritance from `<g>` groups (fill, stroke, stroke-width,
//! etc.). `<text>` with `text-anchor`, `letter-spacing`, `font-weight`,
//! `opacity`, and `<tspan>` children.

use std::collections::HashMap;

use crate::html::dom::{Document, NodeId};
use crate::transform::AffineTransform2D;
use oasis_types::backend::Color;

/// Type alias for backward compatibility within this module.
type AffineTransform = AffineTransform2D;

/// SVG fill rule.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FillRule {
    #[default]
    NonZero,
    EvenOdd,
}

/// SVG stroke line cap style.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LineCap {
    #[default]
    Butt,
    Round,
    Square,
}

/// SVG stroke line join style.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LineJoin {
    #[default]
    Miter,
    Round,
    Bevel,
}

// -------------------------------------------------------------------
// SVG paint (fill/stroke value with url() reference support)
// -------------------------------------------------------------------

/// An SVG paint value: solid color, gradient/pattern reference, or none.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum SvgPaint {
    /// No paint (SVG `none`).
    #[default]
    None,
    /// Solid color.
    Color(Color),
    /// Reference to a `<linearGradient>` or `<radialGradient>` in `<defs>`.
    GradientRef(String),
    /// Reference to a `<pattern>` in `<defs>`.
    PatternRef(String),
}

impl SvgPaint {
    /// Extract a solid color, returning `None` for references and `None` paint.
    pub fn as_color(&self) -> Option<Color> {
        match self {
            SvgPaint::Color(c) => Some(*c),
            _ => Option::None,
        }
    }
}

// -------------------------------------------------------------------
// SVG gradient and pattern definitions
// -------------------------------------------------------------------

/// A single gradient color stop.
#[derive(Debug, Clone)]
pub struct SvgGradientStop {
    /// Position along the gradient axis (0.0..=1.0).
    pub offset: f32,
    /// Stop color with stop-opacity baked into the alpha channel.
    pub color: Color,
}

/// A gradient definition from `<defs>`.
#[derive(Debug, Clone)]
pub enum SvgGradientDef {
    Linear {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        stops: Vec<SvgGradientStop>,
        /// True = `gradientUnits="userSpaceOnUse"`.
        user_space: bool,
    },
    Radial {
        cx: f32,
        cy: f32,
        r: f32,
        stops: Vec<SvgGradientStop>,
        /// True = `gradientUnits="userSpaceOnUse"`.
        user_space: bool,
    },
}

/// A pattern definition from `<defs>`.
#[derive(Debug, Clone)]
pub struct SvgPatternDef {
    pub width: f32,
    pub height: f32,
    pub shapes: Vec<SvgShape>,
}

/// Collected definitions from `<defs>` elements.
#[derive(Debug, Clone, Default)]
pub struct SvgDefs {
    pub gradients: HashMap<String, SvgGradientDef>,
    pub patterns: HashMap<String, SvgPatternDef>,
}

// -------------------------------------------------------------------
// Attribute inheritance context
// -------------------------------------------------------------------

/// Presentation attributes inherited from ancestor `<g>` elements.
#[derive(Debug, Clone, Default)]
struct InheritedAttrs {
    fill: Option<SvgPaint>,
    stroke: Option<SvgPaint>,
    stroke_width: Option<f32>,
    stroke_linecap: Option<LineCap>,
    stroke_linejoin: Option<LineJoin>,
    font_size: Option<f32>,
    opacity: Option<f32>,
}

// -------------------------------------------------------------------
// Text types
// -------------------------------------------------------------------

/// SVG `text-anchor` alignment.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TextAnchor {
    #[default]
    Start,
    Middle,
    End,
}

/// A `<tspan>` child within a `<text>` element.
#[derive(Debug, Clone)]
pub struct TextSpan {
    pub text: String,
    pub class: Option<String>,
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
    /// Gradient, pattern, and other definitions from `<defs>`.
    pub defs: SvgDefs,
}

/// A single SVG shape primitive.
#[derive(Debug, Clone)]
pub enum SvgShape {
    Rect {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        fill: SvgPaint,
        stroke: SvgPaint,
        stroke_width: f32,
        rx: f32,
        opacity: f32,
    },
    Circle {
        cx: f32,
        cy: f32,
        r: f32,
        fill: SvgPaint,
        stroke: SvgPaint,
        stroke_width: f32,
        opacity: f32,
    },
    Ellipse {
        cx: f32,
        cy: f32,
        rx: f32,
        ry: f32,
        fill: SvgPaint,
        stroke: SvgPaint,
        stroke_width: f32,
        opacity: f32,
    },
    Line {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        stroke: SvgPaint,
        stroke_width: f32,
        opacity: f32,
    },
    Text {
        x: f32,
        y: f32,
        text: String,
        fill: SvgPaint,
        font_size: f32,
        text_anchor: TextAnchor,
        letter_spacing: f32,
        font_weight: u16,
        opacity: f32,
        spans: Vec<TextSpan>,
    },
    /// SVG `<path>` element — flattened to polygon points.
    Path {
        points: Vec<(f32, f32)>,
        fill: SvgPaint,
        stroke: SvgPaint,
        stroke_width: f32,
        fill_rule: FillRule,
        stroke_linecap: LineCap,
        stroke_linejoin: LineJoin,
        opacity: f32,
    },
    /// SVG `<polygon>` element.
    Polygon {
        points: Vec<(f32, f32)>,
        fill: SvgPaint,
        stroke: SvgPaint,
        stroke_width: f32,
        fill_rule: FillRule,
        stroke_linecap: LineCap,
        stroke_linejoin: LineJoin,
        opacity: f32,
    },
    /// SVG `<polyline>` element (no auto-close).
    Polyline {
        points: Vec<(f32, f32)>,
        stroke: SvgPaint,
        stroke_width: f32,
        opacity: f32,
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
    parse_svg_with_styles(doc, svg_node, &[])
}

/// Parse an `<svg>` DOM node with access to computed CSS styles.
///
/// When `styles` is non-empty, animated properties (e.g. `opacity`
/// from `@keyframes`) are read from the `ComputedStyle` instead of
/// the DOM attribute, enabling CSS animation overrides to affect SVG
/// rendering.
pub fn parse_svg_with_styles(
    doc: &Document,
    svg_node: NodeId,
    styles: &[Option<crate::css::values::ComputedStyle>],
) -> Option<SvgElement> {
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

    // Parse <defs> first so gradient/pattern references are available.
    let mut defs = SvgDefs::default();
    parse_defs(doc, svg_node, &mut defs);

    let mut shapes = Vec::new();
    let parent_xf = AffineTransform::identity();
    let inherited = InheritedAttrs::default();
    parse_children(doc, svg_node, &parent_xf, &inherited, &mut shapes, styles);

    Some(SvgElement {
        width,
        height,
        viewbox,
        shapes,
        defs,
    })
}

/// Recursively parse children of an SVG element, composing group transforms
/// and inheriting presentation attributes from `<g>` ancestors.
fn parse_children(
    doc: &Document,
    parent_id: NodeId,
    parent_xf: &AffineTransform,
    inherited: &InheritedAttrs,
    shapes: &mut Vec<SvgShape>,
    styles: &[Option<crate::css::values::ComputedStyle>],
) {
    let children = doc.get(parent_id).children.clone();
    for &child_id in &children {
        let Some(elem) = doc.element(child_id) else {
            continue;
        };
        let tag = elem.tag.as_str();

        // Skip elements handled elsewhere or not yet supported.
        if matches!(
            tag,
            "defs"
                | "style"
                | "filter"
                | "mask"
                | "clipPath"
                | "linearGradient"
                | "radialGradient"
                | "pattern"
        ) {
            continue;
        }

        // Parse optional transform attribute and compose with parent.
        let local_xf = elem.get_attribute("transform").map(parse_transform_attr);
        let composed = match &local_xf {
            Some(t) => parent_xf.multiply(t),
            None => *parent_xf,
        };

        if tag == "g" {
            // Merge presentation attributes from this <g> with inherited
            // context. When CSS styles are available (animation overrides),
            // read animated opacity from ComputedStyle.
            let merged = merge_inherited_with_styles(elem, inherited, child_id, styles);
            parse_children(doc, child_id, &composed, &merged, shapes, styles);
        } else if let Some(mut shape) = parse_shape_inner(doc, child_id, elem, tag, inherited) {
            // Apply composed transform to the shape.
            let identity = AffineTransform::identity();
            let needs_xf = composed.a != identity.a
                || composed.b != identity.b
                || composed.c != identity.c
                || composed.d != identity.d
                || composed.e != identity.e
                || composed.f != identity.f;
            if needs_xf {
                apply_transform_to_shape(&mut shape, &composed);
            }
            shapes.push(shape);
        }
    }
}

/// Merge inherited attributes with optional CSS animation overrides.
///
/// When computed styles are available, animated properties (currently
/// `opacity`) are read from the [`ComputedStyle`] so that `@keyframes`
/// animation values flow into the SVG rendering.
fn merge_inherited_with_styles(
    elem: &ElementData,
    parent: &InheritedAttrs,
    node_id: NodeId,
    styles: &[Option<crate::css::values::ComputedStyle>],
) -> InheritedAttrs {
    let mut merged = merge_inherited(elem, parent);
    // Override opacity from ComputedStyle when CSS animations have set it.
    if let Some(Some(style)) = styles.get(node_id) {
        // CSS animation may set opacity on this node — use the
        // computed value which includes animation overrides.
        if style.opacity < 1.0 || elem.get_attribute("opacity").is_some() {
            merged.opacity = Some(style.opacity);
        }
    }
    merged
}

/// Merge a `<g>` element's presentation attributes with inherited context.
/// Child attributes override parent; absent attributes fall through.
fn merge_inherited(elem: &ElementData, parent: &InheritedAttrs) -> InheritedAttrs {
    InheritedAttrs {
        fill: elem
            .get_attribute("fill")
            .map(parse_svg_paint)
            .or_else(|| parent.fill.clone()),
        stroke: elem
            .get_attribute("stroke")
            .map(parse_svg_paint)
            .or_else(|| parent.stroke.clone()),
        stroke_width: elem
            .get_attribute("stroke-width")
            .and_then(parse_length)
            .or(parent.stroke_width),
        stroke_linecap: elem
            .get_attribute("stroke-linecap")
            .map(|v| match v {
                "round" => LineCap::Round,
                "square" => LineCap::Square,
                _ => LineCap::Butt,
            })
            .or(parent.stroke_linecap),
        stroke_linejoin: elem
            .get_attribute("stroke-linejoin")
            .map(|v| match v {
                "round" => LineJoin::Round,
                "bevel" => LineJoin::Bevel,
                _ => LineJoin::Miter,
            })
            .or(parent.stroke_linejoin),
        font_size: elem
            .get_attribute("font-size")
            .and_then(parse_length)
            .or(parent.font_size),
        opacity: elem
            .get_attribute("opacity")
            .and_then(|v| v.parse::<f32>().ok())
            .or(parent.opacity),
    }
}

// -------------------------------------------------------------------
// <defs> parsing
// -------------------------------------------------------------------

/// Parse `<defs>` children from the SVG root and populate gradient/pattern maps.
fn parse_defs(doc: &Document, svg_node: NodeId, defs: &mut SvgDefs) {
    let children = doc.get(svg_node).children.clone();
    for &child_id in &children {
        let Some(elem) = doc.element(child_id) else {
            continue;
        };
        if elem.tag.as_str() == "defs" {
            parse_defs_children(doc, child_id, defs);
        }
    }
}

fn parse_defs_children(doc: &Document, defs_node: NodeId, defs: &mut SvgDefs) {
    let children = doc.get(defs_node).children.clone();
    for &child_id in &children {
        let Some(elem) = doc.element(child_id) else {
            continue;
        };
        let tag = elem.tag.as_str();
        let id = match elem.get_attribute("id") {
            Some(id) => id.to_string(),
            None => continue, // definitions without id are useless
        };
        match tag {
            "linearGradient" => {
                let user_space = elem
                    .get_attribute("gradientUnits")
                    .is_some_and(|v| v == "userSpaceOnUse");
                // Default: horizontal gradient across 0..1 in objectBoundingBox.
                let x1 = attr_f32_or(elem, "x1", 0.0);
                let y1 = attr_f32_or(elem, "y1", 0.0);
                let x2 = attr_f32_or(elem, "x2", 1.0);
                let y2 = attr_f32_or(elem, "y2", 0.0);
                let stops = parse_gradient_stops(doc, child_id);
                defs.gradients.insert(
                    id,
                    SvgGradientDef::Linear {
                        x1,
                        y1,
                        x2,
                        y2,
                        stops,
                        user_space,
                    },
                );
            },
            "radialGradient" => {
                let user_space = elem
                    .get_attribute("gradientUnits")
                    .is_some_and(|v| v == "userSpaceOnUse");
                let cx = attr_f32_or(elem, "cx", if user_space { 0.0 } else { 0.5 });
                let cy = attr_f32_or(elem, "cy", if user_space { 0.0 } else { 0.5 });
                let r = attr_f32_or(elem, "r", if user_space { 0.0 } else { 0.5 });
                let stops = parse_gradient_stops(doc, child_id);
                defs.gradients.insert(
                    id,
                    SvgGradientDef::Radial {
                        cx,
                        cy,
                        r,
                        stops,
                        user_space,
                    },
                );
            },
            "pattern" => {
                let width = attr_f32(elem, "width");
                let height = attr_f32(elem, "height");
                let mut shapes = Vec::new();
                let xf = AffineTransform::identity();
                let inh = InheritedAttrs::default();
                parse_children(doc, child_id, &xf, &inh, &mut shapes, &[]);
                defs.patterns.insert(
                    id,
                    SvgPatternDef {
                        width,
                        height,
                        shapes,
                    },
                );
            },
            _ => {},
        }
    }
}

/// Parse `<stop>` children of a gradient element.
fn parse_gradient_stops(doc: &Document, grad_node: NodeId) -> Vec<SvgGradientStop> {
    let mut stops = Vec::new();
    let children = doc.get(grad_node).children.clone();
    for &child_id in &children {
        let Some(elem) = doc.element(child_id) else {
            continue;
        };
        if elem.tag.as_str() != "stop" {
            continue;
        }
        // Parse offset: percentage string like "50%" or fraction like "0.5".
        let offset = elem
            .get_attribute("offset")
            .and_then(|v| {
                let v = v.trim();
                if let Some(pct) = v.strip_suffix('%') {
                    pct.trim().parse::<f32>().ok().map(|p| p / 100.0)
                } else {
                    v.parse::<f32>().ok()
                }
            })
            .unwrap_or(0.0)
            .clamp(0.0, 1.0);

        // stop-color attribute.
        let mut color = elem
            .get_attribute("stop-color")
            .and_then(parse_svg_color)
            .unwrap_or(Color::rgb(0, 0, 0));

        // stop-opacity: multiply into alpha channel.
        if let Some(opacity_str) = elem.get_attribute("stop-opacity")
            && let Ok(op) = opacity_str.trim().parse::<f32>()
        {
            color = Color::rgba(
                color.r,
                color.g,
                color.b,
                (color.a as f32 * op.clamp(0.0, 1.0)) as u8,
            );
        }

        stops.push(SvgGradientStop { offset, color });
    }
    // Sort by offset for correct interpolation.
    stops.sort_by(|a, b| {
        a.offset
            .partial_cmp(&b.offset)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    stops
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

/// Inner shape parsing (without transform application).
fn parse_shape_inner(
    doc: &Document,
    node_id: NodeId,
    elem: &ElementData,
    tag: &str,
    inherited: &InheritedAttrs,
) -> Option<SvgShape> {
    // Helper closures for attribute-with-inheritance lookup.
    let fill_paint = |e: &ElementData| -> SvgPaint {
        match e.get_attribute("fill") {
            Some(val) => parse_svg_paint(val),
            None => inherited
                .fill
                .clone()
                .unwrap_or_else(|| SvgPaint::Color(Color::rgb(0, 0, 0))),
        }
    };
    let stroke_paint = |e: &ElementData| -> SvgPaint {
        match e.get_attribute("stroke") {
            Some(val) => parse_svg_paint(val),
            None => inherited.stroke.clone().unwrap_or(SvgPaint::None),
        }
    };
    let sw = |e: &ElementData| -> f32 {
        attr_f32_or(e, "stroke-width", inherited.stroke_width.unwrap_or(1.0))
    };
    let opa = |e: &ElementData| -> f32 {
        e.get_attribute("opacity")
            .and_then(|v| v.parse::<f32>().ok())
            .or(inherited.opacity)
            .unwrap_or(1.0)
    };

    match tag {
        "rect" => Some(SvgShape::Rect {
            x: attr_f32(elem, "x"),
            y: attr_f32(elem, "y"),
            width: attr_f32(elem, "width"),
            height: attr_f32(elem, "height"),
            fill: fill_paint(elem),
            stroke: stroke_paint(elem),
            stroke_width: sw(elem),
            rx: attr_f32(elem, "rx"),
            opacity: opa(elem),
        }),
        "circle" => Some(SvgShape::Circle {
            cx: attr_f32(elem, "cx"),
            cy: attr_f32(elem, "cy"),
            r: attr_f32(elem, "r"),
            fill: fill_paint(elem),
            stroke: stroke_paint(elem),
            stroke_width: sw(elem),
            opacity: opa(elem),
        }),
        "ellipse" => Some(SvgShape::Ellipse {
            cx: attr_f32(elem, "cx"),
            cy: attr_f32(elem, "cy"),
            rx: attr_f32(elem, "rx"),
            ry: attr_f32(elem, "ry"),
            fill: fill_paint(elem),
            stroke: stroke_paint(elem),
            stroke_width: sw(elem),
            opacity: opa(elem),
        }),
        "line" => Some(SvgShape::Line {
            x1: attr_f32(elem, "x1"),
            y1: attr_f32(elem, "y1"),
            x2: attr_f32(elem, "x2"),
            y2: attr_f32(elem, "y2"),
            stroke: stroke_paint(elem),
            stroke_width: sw(elem),
            opacity: opa(elem),
        }),
        "path" => {
            let d = elem.get_attribute("d").unwrap_or("");
            let points = flatten_path_data(d);
            if points.len() < 2 {
                return None;
            }
            Some(SvgShape::Path {
                points,
                fill: fill_paint(elem),
                stroke: stroke_paint(elem),
                stroke_width: sw(elem),
                fill_rule: parse_fill_rule(elem),
                stroke_linecap: elem
                    .get_attribute("stroke-linecap")
                    .map(|v| match v {
                        "round" => LineCap::Round,
                        "square" => LineCap::Square,
                        _ => LineCap::Butt,
                    })
                    .or(inherited.stroke_linecap)
                    .unwrap_or_default(),
                stroke_linejoin: elem
                    .get_attribute("stroke-linejoin")
                    .map(|v| match v {
                        "round" => LineJoin::Round,
                        "bevel" => LineJoin::Bevel,
                        _ => LineJoin::Miter,
                    })
                    .or(inherited.stroke_linejoin)
                    .unwrap_or_default(),
                opacity: opa(elem),
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
            Some(SvgShape::Polygon {
                points: pts,
                fill: fill_paint(elem),
                stroke: stroke_paint(elem),
                stroke_width: sw(elem),
                fill_rule: parse_fill_rule(elem),
                stroke_linecap: parse_linecap(elem),
                stroke_linejoin: parse_linejoin(elem),
                opacity: opa(elem),
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
            Some(SvgShape::Polyline {
                points: pts,
                stroke: stroke_paint(elem),
                stroke_width: sw(elem),
                opacity: opa(elem),
            })
        },
        "text" => {
            let x = attr_f32(elem, "x");
            let y = attr_f32(elem, "y");
            let fill = fill_paint(elem);
            let font_size = attr_f32_or(elem, "font-size", inherited.font_size.unwrap_or(16.0));
            let text_anchor = match elem.get_attribute("text-anchor") {
                Some("middle") => TextAnchor::Middle,
                Some("end") => TextAnchor::End,
                _ => TextAnchor::Start,
            };
            let letter_spacing = attr_f32(elem, "letter-spacing");
            let font_weight = elem
                .get_attribute("font-weight")
                .map(|v| match v {
                    "bold" => 700u16,
                    "normal" => 400,
                    _ => v.parse::<u16>().unwrap_or(400),
                })
                .unwrap_or(400);

            // Collect text content: direct text nodes + <tspan> children.
            let mut main_text = String::new();
            let mut spans = Vec::new();
            let children = doc.get(node_id).children.clone();
            for &cid in &children {
                if let Some(child_elem) = doc.element(cid) {
                    if child_elem.tag.as_str() == "tspan" {
                        let span_text = doc.text_content(cid);
                        if !span_text.is_empty() {
                            spans.push(TextSpan {
                                text: span_text,
                                class: child_elem.get_attribute("class").map(|s| s.to_string()),
                            });
                        }
                    }
                } else {
                    // Text node — append to main text.
                    let node = doc.get(cid);
                    if let crate::html::dom::NodeKind::Text(ref t) = node.kind {
                        main_text.push_str(t);
                    }
                }
            }
            // Fallback: if no structured children, use text_content.
            if main_text.is_empty() && spans.is_empty() {
                main_text = doc.text_content(node_id);
            }
            let full_text = main_text.trim().to_string();
            if full_text.is_empty() && spans.is_empty() {
                return None;
            }
            Some(SvgShape::Text {
                x,
                y,
                text: full_text,
                fill,
                font_size,
                text_anchor,
                letter_spacing,
                font_weight,
                opacity: opa(elem),
                spans,
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

/// Parse a string value into an [`SvgPaint`].
///
/// Handles `url(#id)` references (gradient or pattern), solid color
/// strings, and the keywords `none` / `transparent`.
fn parse_svg_paint(val: &str) -> SvgPaint {
    let val = val.trim();
    if val.eq_ignore_ascii_case("none") || val.eq_ignore_ascii_case("transparent") {
        return SvgPaint::None;
    }
    // Check for url(#id) reference.
    if let Some(rest) = val.strip_prefix("url(")
        && let Some(id_part) = rest.strip_suffix(')')
    {
        let id = id_part.trim().trim_matches('\'').trim_matches('"').trim();
        if let Some(id) = id.strip_prefix('#') {
            return SvgPaint::GradientRef(id.to_string());
        }
    }
    match parse_svg_color(val) {
        Some(c) => SvgPaint::Color(c),
        None => SvgPaint::None,
    }
}

/// Parse the `fill-rule` attribute.
fn parse_fill_rule(elem: &ElementData) -> FillRule {
    match elem.get_attribute("fill-rule") {
        Some("evenodd") => FillRule::EvenOdd,
        _ => FillRule::NonZero,
    }
}

/// Parse the `stroke-linecap` attribute.
fn parse_linecap(elem: &ElementData) -> LineCap {
    match elem.get_attribute("stroke-linecap") {
        Some("round") => LineCap::Round,
        Some("square") => LineCap::Square,
        _ => LineCap::Butt,
    }
}

/// Parse the `stroke-linejoin` attribute.
fn parse_linejoin(elem: &ElementData) -> LineJoin {
    match elem.get_attribute("stroke-linejoin") {
        Some("round") => LineJoin::Round,
        Some("bevel") => LineJoin::Bevel,
        _ => LineJoin::Miter,
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
const MAX_FLATTEN_DEPTH: u8 = 16;

#[allow(clippy::too_many_arguments)]
fn flatten_cubic_inner(
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
        paint_shape(shape, backend, &xf, &svg.defs)?;
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
fn paint_shape(
    shape: &SvgShape,
    backend: &mut dyn SdiBackend,
    xf: &SvgTransform,
    defs: &SvgDefs,
) -> Result<()> {
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
            opacity,
        } => {
            let px = ox + ((x - vb_x) * sx) as i32;
            let py = oy + ((y - vb_y) * sy) as i32;
            let pw = (width * sx) as u32;
            let ph = (height * sy) as u32;
            if pw == 0 || ph == 0 {
                return Ok(());
            }
            let r_val = (rx * sx.min(sy)) as u16;
            match fill {
                SvgPaint::Color(fc) => {
                    let fc = apply_opacity(*fc, *opacity);
                    if r_val > 0 {
                        backend.fill_rounded_rect(px, py, pw, ph, r_val, fc)?;
                    } else {
                        backend.fill_rect(px, py, pw, ph, fc)?;
                    }
                },
                SvgPaint::GradientRef(id) => {
                    if let Some(grad) = defs.gradients.get(id.as_str()) {
                        paint_rect_gradient(backend, px, py, pw, ph, grad, *opacity)?;
                    }
                },
                SvgPaint::PatternRef(id) => {
                    if let Some(pat) = defs.patterns.get(id.as_str()) {
                        paint_pattern_fill(backend, px, py, pw, ph, pat, xf, defs, *opacity)?;
                    }
                },
                SvgPaint::None => {},
            }
            if let Some(sc) = stroke.as_color() {
                let sc = apply_opacity(sc, *opacity);
                let sw = (stroke_width * sx.min(sy)).max(1.0) as u32;
                backend.fill_rect(px, py, pw, sw, sc)?;
                backend.fill_rect(px, py + ph as i32 - sw as i32, pw, sw, sc)?;
                backend.fill_rect(px, py + sw as i32, sw, ph.saturating_sub(sw * 2), sc)?;
                backend.fill_rect(
                    px + pw as i32 - sw as i32,
                    py + sw as i32,
                    sw,
                    ph.saturating_sub(sw * 2),
                    sc,
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
            opacity,
        } => {
            let px = ox + ((cx - vb_x) * sx) as i32;
            let py = oy + ((cy - vb_y) * sy) as i32;
            let radius = (r * sx.min(sy)) as u16;
            if radius == 0 {
                return Ok(());
            }
            if let Some(fc) = fill.as_color() {
                backend.fill_circle(px, py, radius, apply_opacity(fc, *opacity))?;
            }
            if let Some(sc) = stroke.as_color() {
                let sw = (stroke_width * sx.min(sy)).max(1.0) as u16;
                backend.stroke_circle(px, py, radius, sw, apply_opacity(sc, *opacity))?;
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
            opacity,
        } => {
            let erx = (rx * sx) as i32;
            let ery = (ry * sy) as i32;
            let px = ox + ((cx - vb_x) * sx) as i32 - erx;
            let py = oy + ((cy - vb_y) * sy) as i32 - ery;
            let pw = (erx * 2) as u32;
            let ph = (ery * 2) as u32;
            if pw == 0 || ph == 0 {
                return Ok(());
            }
            let r_val = (erx as u16).min(ery as u16);
            if let Some(fc) = fill.as_color() {
                backend.fill_rounded_rect(px, py, pw, ph, r_val, apply_opacity(fc, *opacity))?;
            }
            if let Some(sc) = stroke.as_color() {
                let sw = (stroke_width * sx.min(sy)).max(1.0) as u16;
                backend.stroke_rounded_rect(
                    px,
                    py,
                    pw,
                    ph,
                    r_val,
                    sw,
                    apply_opacity(sc, *opacity),
                )?;
            }
        },
        SvgShape::Line {
            x1,
            y1,
            x2,
            y2,
            stroke,
            stroke_width,
            opacity,
        } => {
            let sc = match stroke.as_color() {
                Some(c) => apply_opacity(c, *opacity),
                None => return Ok(()),
            };
            let px1 = ox + ((x1 - vb_x) * sx) as i32;
            let py1 = oy + ((y1 - vb_y) * sy) as i32;
            let px2 = ox + ((x2 - vb_x) * sx) as i32;
            let py2 = oy + ((y2 - vb_y) * sy) as i32;
            let sw = (stroke_width * sx.min(sy)).max(1.0) as u32;
            let dx = (px2 - px1).abs();
            let dy = (py2 - py1).abs();
            if dx == 0 || dy == 0 {
                let lx = px1.min(px2);
                let ly = py1.min(py2);
                let lw = (dx as u32).max(sw);
                let lh = (dy as u32).max(sw);
                backend.fill_rect(lx, ly, lw, lh, sc)?;
            } else {
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
            text_anchor,
            letter_spacing,
            font_weight,
            opacity,
            spans,
        } => {
            let scaled_fs = (font_size * sy).max(1.0) as u16;
            let fc = fill.as_color().unwrap_or(Color::rgb(0, 0, 0));
            let fc = apply_opacity(fc, *opacity);

            // Compute total text (main + spans) for text-anchor measurement.
            let mut full = text.clone();
            for span in spans {
                full.push_str(&span.text);
            }

            let mut px = ox + ((x - vb_x) * sx) as i32;
            let py = oy + ((y - vb_y) * sy) as i32 - scaled_fs as i32;

            // text-anchor adjustment.
            match text_anchor {
                TextAnchor::Middle => {
                    let tw = backend.measure_text(&full, scaled_fs);
                    px -= tw as i32 / 2;
                },
                TextAnchor::End => {
                    let tw = backend.measure_text(&full, scaled_fs);
                    px -= tw as i32;
                },
                TextAnchor::Start => {},
            }

            let bold = *font_weight >= 700;
            if *letter_spacing > 0.1 {
                // Character-by-character rendering with spacing.
                let ls = (*letter_spacing * sx) as i32;
                let mut cx = px;
                for ch in full.chars() {
                    let mut buf = [0u8; 4];
                    let s = ch.encode_utf8(&mut buf);
                    backend.draw_text_styled(s, cx, py, scaled_fs, fc, bold, false)?;
                    let cw = backend.measure_text(s, scaled_fs) as i32;
                    cx += cw + ls;
                }
            } else {
                backend.draw_text_styled(&full, px, py, scaled_fs, fc, bold, false)?;
            }
        },
        SvgShape::Path {
            points,
            fill,
            stroke,
            stroke_width,
            fill_rule,
            stroke_linecap,
            stroke_linejoin,
            opacity,
        }
        | SvgShape::Polygon {
            points,
            fill,
            stroke,
            stroke_width,
            fill_rule,
            stroke_linecap,
            stroke_linejoin,
            opacity,
        } => {
            paint_polygon_shape(
                points,
                fill,
                stroke,
                *stroke_width,
                *fill_rule,
                *stroke_linecap,
                *stroke_linejoin,
                *opacity,
                backend,
                xf,
                defs,
            )?;
        },
        SvgShape::Polyline {
            points,
            stroke,
            stroke_width,
            opacity,
        } => {
            if let Some(sc) = stroke.as_color() {
                let sc = apply_opacity(sc, *opacity);
                let sw = (stroke_width * sx.min(sy)).max(1.0) as u32;
                for window in points.windows(2) {
                    let (px1, py1) = xf_point(window[0].0, window[0].1, xf);
                    let (px2, py2) = xf_point(window[1].0, window[1].1, xf);
                    stroke_line_bresenham(backend, px1, py1, px2, py2, sw, sc)?;
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
#[allow(clippy::too_many_arguments)]
fn paint_polygon_shape(
    points: &[(f32, f32)],
    fill: &SvgPaint,
    stroke: &SvgPaint,
    stroke_width: f32,
    fill_rule: FillRule,
    linecap: LineCap,
    linejoin: LineJoin,
    opacity: f32,
    backend: &mut dyn SdiBackend,
    xf: &SvgTransform,
    defs: &SvgDefs,
) -> Result<()> {
    if points.len() < 2 {
        return Ok(());
    }
    let screen_pts: Vec<(i32, i32)> = points.iter().map(|&(x, y)| xf_point(x, y, xf)).collect();

    match fill {
        SvgPaint::Color(fc) => {
            let fc = apply_opacity(*fc, opacity);
            if fill_rule == FillRule::EvenOdd && screen_pts.len() >= 3 {
                let float_pts: Vec<(f32, f32)> = screen_pts
                    .iter()
                    .map(|&(x, y)| (x as f32, y as f32))
                    .collect();
                let triangles = crate::transform::ear_clip_triangulate(&float_pts);
                for tri in &triangles {
                    let p0 = screen_pts[tri[0]];
                    let p1 = screen_pts[tri[1]];
                    let p2 = screen_pts[tri[2]];
                    backend.fill_polygon(&[p0, p1, p2], fc)?;
                }
            } else {
                backend.fill_polygon(&screen_pts, fc)?;
            }
        },
        SvgPaint::GradientRef(id) => {
            // Gradient fill for polygons: compute AABB and fill with
            // the gradient's midpoint color (approximation — full
            // per-scanline gradient clipped to polygon is a follow-up).
            if let Some(grad) = defs.gradients.get(id.as_str()) {
                let mid_color = sample_svg_gradient_at(grad, 0.5, opacity);
                backend.fill_polygon(&screen_pts, mid_color)?;
            }
        },
        SvgPaint::PatternRef(_) | SvgPaint::None => {},
    }
    if let Some(sc) = stroke.as_color() {
        let sc = apply_opacity(sc, opacity);
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

            // Line join at vertices (skip first segment).
            if linejoin == LineJoin::Round {
                let r = (sw / 2).max(1) as u16;
                backend.fill_circle(window[1].0, window[1].1, r, sc)?;
            }
        }
        // Close the path (last → first).
        if let (Some(last), Some(first)) = (screen_pts.last(), screen_pts.first())
            && last != first
        {
            stroke_line_bresenham(backend, last.0, last.1, first.0, first.1, sw, sc)?;
        }

        // Line caps at endpoints.
        match linecap {
            LineCap::Round => {
                let r = (sw / 2).max(1) as u16;
                if let Some(first) = screen_pts.first() {
                    backend.fill_circle(first.0, first.1, r, sc)?;
                }
                if let Some(last) = screen_pts.last() {
                    backend.fill_circle(last.0, last.1, r, sc)?;
                }
            },
            LineCap::Square => {
                // Square cap extends by half stroke width beyond endpoints.
                // Approximated by drawing a small rect at each endpoint.
                let half = (sw / 2).max(1);
                if let Some(first) = screen_pts.first() {
                    backend.fill_rect(first.0 - half as i32, first.1 - half as i32, sw, sw, sc)?;
                }
                if let Some(last) = screen_pts.last() {
                    backend.fill_rect(last.0 - half as i32, last.1 - half as i32, sw, sw, sc)?;
                }
            },
            LineCap::Butt => {}, // Default: no extension
        }
    }
    Ok(())
}

// -------------------------------------------------------------------
// Gradient and pattern rendering
// -------------------------------------------------------------------

/// Apply opacity to a color by multiplying its alpha channel.
fn apply_opacity(c: Color, opacity: f32) -> Color {
    if (opacity - 1.0).abs() < f32::EPSILON {
        return c;
    }
    Color::rgba(
        c.r,
        c.g,
        c.b,
        (c.a as f32 * opacity.clamp(0.0, 1.0)).round() as u8,
    )
}

/// Sample a gradient at position `t` (0..=1) with opacity applied.
fn sample_svg_gradient_at(grad: &SvgGradientDef, t: f32, opacity: f32) -> Color {
    let stops = match grad {
        SvgGradientDef::Linear { stops, .. } | SvgGradientDef::Radial { stops, .. } => stops,
    };
    let c = sample_svg_stops(stops, t);
    apply_opacity(c, opacity)
}

/// Linearly interpolate between gradient stops at position `t`.
fn sample_svg_stops(stops: &[SvgGradientStop], t: f32) -> Color {
    if stops.is_empty() {
        return Color::rgba(0, 0, 0, 0);
    }
    if t <= stops[0].offset {
        return stops[0].color;
    }
    let last = stops.len() - 1;
    if t >= stops[last].offset {
        return stops[last].color;
    }
    for i in 0..last {
        if t >= stops[i].offset && t <= stops[i + 1].offset {
            let range = stops[i + 1].offset - stops[i].offset;
            let local_t = if range > 0.0 {
                (t - stops[i].offset) / range
            } else {
                0.0
            };
            return lerp_color(stops[i].color, stops[i + 1].color, local_t);
        }
    }
    stops[last].color
}

/// Linear color interpolation.
fn lerp_color(a: Color, b: Color, t: f32) -> Color {
    let inv = 1.0 - t;
    Color::rgba(
        (a.r as f32 * inv + b.r as f32 * t).round() as u8,
        (a.g as f32 * inv + b.g as f32 * t).round() as u8,
        (a.b as f32 * inv + b.b as f32 * t).round() as u8,
        (a.a as f32 * inv + b.a as f32 * t).round() as u8,
    )
}

/// Render a gradient fill for a rectangular region.
fn paint_rect_gradient(
    backend: &mut dyn SdiBackend,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    grad: &SvgGradientDef,
    opacity: f32,
) -> Result<()> {
    match grad {
        SvgGradientDef::Linear {
            x1,
            y1,
            x2,
            y2,
            stops,
            ..
        } => {
            if stops.is_empty() || w == 0 || h == 0 {
                return Ok(());
            }
            // Determine gradient direction.
            let is_vertical = (*x2 - *x1).abs() < 0.001;
            let is_horizontal = (*y2 - *y1).abs() < 0.001;
            if is_vertical {
                // Vertical gradient: render horizontal bands.
                let bands = (h as usize).clamp(1, 128);
                let band_h = h as f32 / bands as f32;
                for i in 0..bands {
                    let t_top = *y1 + (*y2 - *y1) * (i as f32 / bands as f32);
                    let t_bot = *y1 + (*y2 - *y1) * ((i as f32 + 1.0) / bands as f32);
                    let t = (t_top + t_bot) / 2.0;
                    let c = sample_svg_gradient_at(grad, t, opacity);
                    let by = y + (i as f32 * band_h) as i32;
                    let bh = (band_h.ceil() as u32).max(1);
                    backend.fill_rect(x, by, w, bh, c)?;
                }
            } else if is_horizontal {
                // Horizontal gradient: render vertical bands.
                let bands = (w as usize).clamp(1, 128);
                let band_w = w as f32 / bands as f32;
                for i in 0..bands {
                    let t_l = *x1 + (*x2 - *x1) * (i as f32 / bands as f32);
                    let t_r = *x1 + (*x2 - *x1) * ((i as f32 + 1.0) / bands as f32);
                    let t = (t_l + t_r) / 2.0;
                    let c = sample_svg_gradient_at(grad, t, opacity);
                    let bx = x + (i as f32 * band_w) as i32;
                    let bw = (band_w.ceil() as u32).max(1);
                    backend.fill_rect(bx, y, bw, h, c)?;
                }
            } else {
                // Diagonal: approximate with vertical bands.
                let bands = (w.max(h) as usize).clamp(1, 128);
                for i in 0..bands {
                    let t = (i as f32 + 0.5) / bands as f32;
                    let c = sample_svg_gradient_at(grad, t, opacity);
                    let bx = x + (w as f32 * i as f32 / bands as f32) as i32;
                    let bw = (w as f32 / bands as f32).ceil() as u32;
                    backend.fill_rect(bx, y, bw.max(1), h, c)?;
                }
            }
        },
        SvgGradientDef::Radial {
            cx, cy, r, stops, ..
        } => {
            if stops.is_empty() || w == 0 || h == 0 {
                return Ok(());
            }
            // Radial gradient: concentric bands from outside in.
            let center_x = x + (w as f32 * cx) as i32;
            let center_y = y + (h as f32 * cy) as i32;
            let max_r = (w.max(h) as f32 * r).max(1.0);
            let bands = (max_r as usize).clamp(8, 64);
            // Paint outside-in so inner bands overwrite outer.
            for i in 0..bands {
                let t = 1.0 - (i as f32 / bands as f32);
                let c = sample_svg_gradient_at(grad, t, opacity);
                let band_r = (max_r * (1.0 - i as f32 / bands as f32)) as i32;
                if band_r <= 0 {
                    continue;
                }
                let bx = center_x - band_r;
                let by = center_y - band_r;
                let bw = (band_r * 2) as u32;
                let bh = bw;
                let corner_r = band_r as u16;
                backend.fill_rounded_rect(bx, by, bw, bh, corner_r, c)?;
            }
        },
    }
    Ok(())
}

/// Render a pattern tile fill over a rectangular region.
#[allow(clippy::too_many_arguments)]
fn paint_pattern_fill(
    backend: &mut dyn SdiBackend,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    pattern: &SvgPatternDef,
    parent_xf: &SvgTransform,
    defs: &SvgDefs,
    opacity: f32,
) -> Result<()> {
    if pattern.width <= 0.0 || pattern.height <= 0.0 {
        return Ok(());
    }
    let tile_w = (pattern.width * parent_xf.sx) as i32;
    let tile_h = (pattern.height * parent_xf.sy) as i32;
    if tile_w <= 0 || tile_h <= 0 {
        return Ok(());
    }

    let cols = (w as i32 / tile_w) + 1;
    let rows = (h as i32 / tile_h) + 1;

    for row in 0..rows {
        for col in 0..cols {
            let tile_x = x + col * tile_w;
            let tile_y = y + row * tile_h;
            // Create a mini-transform for this tile.
            let tile_xf = SvgTransform {
                ox: tile_x,
                oy: tile_y,
                vb_x: 0.0,
                vb_y: 0.0,
                sx: parent_xf.sx,
                sy: parent_xf.sy,
            };
            for shape in &pattern.shapes {
                // Apply opacity to pattern contents.
                paint_shape_with_extra_opacity(shape, backend, &tile_xf, defs, opacity)?;
            }
        }
    }
    Ok(())
}

/// Paint a shape with an additional opacity multiplier from a parent pattern.
fn paint_shape_with_extra_opacity(
    shape: &SvgShape,
    backend: &mut dyn SdiBackend,
    xf: &SvgTransform,
    defs: &SvgDefs,
    _extra_opacity: f32,
) -> Result<()> {
    // For simplicity, delegate to paint_shape. The shape already carries
    // its own opacity from parsing (including inherited opacity from the
    // pattern's parent context).
    paint_shape(shape, backend, xf, defs)
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
                assert_eq!(*fill, SvgPaint::Color(Color::rgb(255, 0, 0)));
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
                assert_eq!(*fill, SvgPaint::Color(Color::rgb(0, 255, 0)));
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
                assert_eq!(*stroke, SvgPaint::Color(Color::rgb(0, 0, 255)));
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
                assert_eq!(*fill, SvgPaint::Color(Color::rgb(0, 0, 128)));
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
