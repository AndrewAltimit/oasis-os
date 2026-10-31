//! `<svg>` DOM subtree -> [`SvgElement`]: shapes, `<defs>`, presentation
//! attributes, inheritance from `<g>`, and colour / length parsing.

use super::*;
use crate::html::dom::ElementData;

/// Parse SVG `transform` attribute into an affine matrix.
pub(super) fn parse_transform_attr(s: &str) -> AffineTransform {
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
pub(super) fn parse_children(
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
pub(super) fn merge_inherited_with_styles(
    elem: &ElementData,
    parent: &InheritedAttrs,
    node_id: NodeId,
    styles: &[Option<crate::css::values::ComputedStyle>],
) -> InheritedAttrs {
    let mut merged = merge_inherited(elem, parent);
    // Override opacity from ComputedStyle when available. The computed
    // value carries the cascade result and any active CSS animation
    // override, including fade-ins that end at `opacity: 1.0`. Applying
    // conditionally on `style.opacity < 1.0` dropped the final keyframe
    // of fade-in animations.
    if let Some(Some(style)) = styles.get(node_id) {
        merged.opacity = Some(style.opacity);
    }
    merged
}

/// Merge a `<g>` element's presentation attributes with inherited context.
/// Child attributes override parent; absent attributes fall through.
pub(super) fn merge_inherited(elem: &ElementData, parent: &InheritedAttrs) -> InheritedAttrs {
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
pub(super) fn parse_defs(doc: &Document, svg_node: NodeId, defs: &mut SvgDefs) {
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

pub(super) fn parse_defs_children(doc: &Document, defs_node: NodeId, defs: &mut SvgDefs) {
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
pub(super) fn parse_gradient_stops(doc: &Document, grad_node: NodeId) -> Vec<SvgGradientStop> {
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

/// Apply a transform to a shape: its points move, and its sizes (rect
/// extent, radii, stroke width, font size) scale with it. Only moving the
/// key points drew `<g transform="scale(0.5)">` icons with full-size
/// circles and rects next to correctly scaled paths. A rect under a
/// rotation or skew becomes the polygon of its corners; circles and
/// ellipses keep their axes (a rotated non-uniform ellipse is
/// approximated by its axis lengths).
pub(super) fn apply_transform_to_shape(shape: &mut SvgShape, xf: &AffineTransform) {
    // Length scale of each axis and the area-preserving mean, for sizes.
    let sx = xf.a.hypot(xf.b);
    let sy = xf.c.hypot(xf.d);
    let mean = (xf.a * xf.d - xf.b * xf.c).abs().sqrt();
    let axis_aligned = xf.b.abs() < 1e-6 && xf.c.abs() < 1e-6;
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
            if axis_aligned {
                // Both corners, so a negative (mirroring) scale keeps a
                // positive extent.
                let (x0, y0) = xf.apply(*x, *y);
                let (x1, y1) = xf.apply(*x + *width, *y + *height);
                *x = x0.min(x1);
                *y = y0.min(y1);
                *width = (x1 - x0).abs();
                *height = (y1 - y0).abs();
                *rx *= xf.a.abs().min(xf.d.abs());
                *stroke_width *= mean;
            } else {
                let corners = [
                    (*x, *y),
                    (*x + *width, *y),
                    (*x + *width, *y + *height),
                    (*x, *y + *height),
                ];
                *shape = SvgShape::Polygon {
                    points: corners.iter().map(|&(px, py)| xf.apply(px, py)).collect(),
                    fill: std::mem::take(fill),
                    stroke: std::mem::take(stroke),
                    stroke_width: *stroke_width * mean,
                    fill_rule: FillRule::NonZero,
                    stroke_linecap: LineCap::Butt,
                    stroke_linejoin: LineJoin::Miter,
                    opacity: *opacity,
                };
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
            let (nx, ny) = xf.apply(*cx, *cy);
            if (sx - sy).abs() <= 1e-4 * sx.max(sy) {
                *cx = nx;
                *cy = ny;
                *r *= sx;
                *stroke_width *= mean;
            } else {
                *shape = SvgShape::Ellipse {
                    cx: nx,
                    cy: ny,
                    rx: *r * sx,
                    ry: *r * sy,
                    fill: std::mem::take(fill),
                    stroke: std::mem::take(stroke),
                    stroke_width: *stroke_width * mean,
                    opacity: *opacity,
                };
            }
        },
        SvgShape::Ellipse {
            cx,
            cy,
            rx,
            ry,
            stroke_width,
            ..
        } => {
            let (nx, ny) = xf.apply(*cx, *cy);
            *cx = nx;
            *cy = ny;
            *rx *= sx;
            *ry *= sy;
            *stroke_width *= mean;
        },
        SvgShape::Line {
            x1,
            y1,
            x2,
            y2,
            stroke_width,
            ..
        } => {
            let (nx1, ny1) = xf.apply(*x1, *y1);
            let (nx2, ny2) = xf.apply(*x2, *y2);
            *x1 = nx1;
            *y1 = ny1;
            *x2 = nx2;
            *y2 = ny2;
            *stroke_width *= mean;
        },
        SvgShape::Text {
            x,
            y,
            font_size,
            letter_spacing,
            ..
        } => {
            let (nx, ny) = xf.apply(*x, *y);
            *x = nx;
            *y = ny;
            *font_size *= mean;
            *letter_spacing *= mean;
        },
        SvgShape::Path {
            points,
            stroke_width,
            ..
        }
        | SvgShape::Polygon {
            points,
            stroke_width,
            ..
        }
        | SvgShape::Polyline {
            points,
            stroke_width,
            ..
        } => {
            for pt in points.iter_mut() {
                let (nx, ny) = xf.apply(pt.0, pt.1);
                pt.0 = nx;
                pt.1 = ny;
            }
            *stroke_width *= mean;
        },
    }
}

/// Inner shape parsing (without transform application).
pub(super) fn parse_shape_inner(
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

/// Get a float attribute, defaulting to 0.0 if missing or unparseable.
pub(super) fn attr_f32(elem: &ElementData, name: &str) -> f32 {
    elem.get_attribute(name)
        .and_then(parse_length)
        .unwrap_or(0.0)
}

/// Get a float attribute with a custom default.
pub(super) fn attr_f32_or(elem: &ElementData, name: &str, default: f32) -> f32 {
    elem.get_attribute(name)
        .and_then(parse_length)
        .unwrap_or(default)
}

/// Parse a length value, stripping optional `px` suffix.
pub(super) fn parse_length(s: &str) -> Option<f32> {
    let s = s.trim();
    let s = s.strip_suffix("px").unwrap_or(s);
    let s = s.strip_suffix("pt").unwrap_or(s);
    s.trim().parse::<f32>().ok()
}

/// Parse a viewBox attribute: "min-x min-y width height".
pub(super) fn parse_viewbox(s: &str) -> Option<(f32, f32, f32, f32)> {
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
pub(super) fn parse_svg_paint(val: &str) -> SvgPaint {
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
pub(super) fn parse_fill_rule(elem: &ElementData) -> FillRule {
    match elem.get_attribute("fill-rule") {
        Some("evenodd") => FillRule::EvenOdd,
        _ => FillRule::NonZero,
    }
}

/// Parse the `stroke-linecap` attribute.
pub(super) fn parse_linecap(elem: &ElementData) -> LineCap {
    match elem.get_attribute("stroke-linecap") {
        Some("round") => LineCap::Round,
        Some("square") => LineCap::Square,
        _ => LineCap::Butt,
    }
}

/// Parse the `stroke-linejoin` attribute.
pub(super) fn parse_linejoin(elem: &ElementData) -> LineJoin {
    match elem.get_attribute("stroke-linejoin") {
        Some("round") => LineJoin::Round,
        Some("bevel") => LineJoin::Bevel,
        _ => LineJoin::Miter,
    }
}

/// Parse an SVG / canvas colour string with the CSS colour parser.
///
/// `None` for `none`, for fully transparent colours and for anything
/// unparseable. `currentColor` has no inherited CSS colour to resolve
/// against here and paints black, the initial `color`.
pub(crate) fn parse_svg_color(val: &str) -> Option<Color> {
    let val = val.trim();
    if val.eq_ignore_ascii_case("currentcolor") {
        return Some(Color::rgb(0, 0, 0));
    }
    crate::css::parse_color(val)
        .filter(|c| c.a > 0)
        .map(|c| Color::rgba(c.r, c.g, c.b, c.a))
}
