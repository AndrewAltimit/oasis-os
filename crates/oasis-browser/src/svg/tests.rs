//! SVG parse-tree and paint tests.

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
    assert_eq!(
        parse_svg_color("steelblue"),
        Some(Color::rgb(0x46, 0x82, 0xB4))
    );
    assert_eq!(parse_svg_color("currentColor"), Some(Color::rgb(0, 0, 0)));
}

/// Regression: the SVG colour parser was a separate copy that sliced
/// hex digits by byte offset (panicking on `fill="#é1"`) and knew no
/// `rgba()` / `hsl()` / 8-digit hex. It now delegates to oasis-css.
#[test]
fn svg_colors_share_the_css_parser() {
    assert_eq!(parse_svg_color("#\u{e9}1"), None);
    assert_eq!(
        parse_svg_color("rgba(0, 0, 255, 0.5)"),
        Some(Color::rgba(0, 0, 255, 127))
    );
    assert_eq!(
        parse_svg_color("hsl(120, 100%, 25%)"),
        Some(Color::rgb(0, 128, 0))
    );
    assert_eq!(parse_svg_color("#ff000000"), None);
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

fn rect_filled_with(paint: SvgPaint) -> SvgShape {
    SvgShape::Rect {
        x: 0.0,
        y: 0.0,
        width: 10.0,
        height: 10.0,
        fill: paint,
        stroke: SvgPaint::None,
        stroke_width: 1.0,
        rx: 0.0,
        opacity: 1.0,
    }
}

/// Regression: a `<pattern>` whose content fills with itself (or an
/// A -> B -> A cycle) recursed until the stack overflowed.
#[test]
fn self_referencing_patterns_terminate() {
    let mut defs = SvgDefs::default();
    for (id, other) in [("p", "p"), ("a", "b"), ("b", "a")] {
        defs.patterns.insert(
            id.to_string(),
            SvgPatternDef {
                width: 5.0,
                height: 5.0,
                shapes: vec![rect_filled_with(SvgPaint::GradientRef(other.to_string()))],
            },
        );
    }
    let svg = SvgElement {
        width: 100.0,
        height: 100.0,
        viewbox: None,
        shapes: vec![
            rect_filled_with(SvgPaint::GradientRef("p".into())),
            rect_filled_with(SvgPaint::GradientRef("a".into())),
        ],
        defs,
    };
    let mut backend = crate::test_utils::MockBackend::new();
    paint_svg(&svg, &mut backend, 0, 0, 100.0, 100.0).unwrap();
}

/// Regression: `1e39` parsed to infinity, the NaN flatness test never
/// passed and every curve subdivided to the full depth; a long `d` held
/// hundreds of millions of points. Coordinates clamp and the point
/// count is budgeted.
#[test]
fn hostile_path_data_flattens_to_a_bounded_point_count() {
    let d = "M0 0 ".to_string() + &"C1e39 0 0 1e39 -1e39 -1e39 ".repeat(1000);
    let points = flatten_path_data(&d);
    assert!(
        points.len() <= MAX_PATH_POINTS + 1001,
        "{} points",
        points.len()
    );
    assert!(points.iter().all(|&(x, y)| x.is_finite() && y.is_finite()));
}

/// Regression: a stroke to `x2="1e9"` issued one rect per pixel of its
/// (off-screen) length, ~1e9 backend calls.
#[test]
fn far_off_screen_stroke_is_clipped_before_stepping() {
    let svg = SvgElement {
        width: 100.0,
        height: 100.0,
        viewbox: None,
        shapes: vec![SvgShape::Line {
            x1: 0.0,
            y1: 0.0,
            x2: 1e9,
            y2: 3e8,
            stroke: SvgPaint::Color(Color::rgb(0, 0, 0)),
            stroke_width: 1.0,
            opacity: 1.0,
        }],
        defs: SvgDefs::default(),
    };
    let mut backend = crate::test_utils::MockBackend::new();
    paint_svg(&svg, &mut backend, 0, 0, 100.0, 100.0).unwrap();
    let n = backend.fill_rect_count();
    assert!(n > 0 && n <= 2 * 16_384 + 1, "{n} rects");
}

/// A negative ellipse radius disables rendering instead of painting a
/// ~4e9-pixel rect.
#[test]
fn negative_ellipse_radius_paints_nothing() {
    let svg = SvgElement {
        width: 100.0,
        height: 100.0,
        viewbox: None,
        shapes: vec![SvgShape::Ellipse {
            cx: 50.0,
            cy: 50.0,
            rx: -10.0,
            ry: 10.0,
            fill: SvgPaint::Color(Color::rgb(0, 0, 0)),
            stroke: SvgPaint::None,
            stroke_width: 1.0,
            opacity: 1.0,
        }],
        defs: SvgDefs::default(),
    };
    let mut backend = crate::test_utils::MockBackend::new();
    paint_svg(&svg, &mut backend, 0, 0, 100.0, 100.0).unwrap();
    assert!(backend.calls.is_empty());
}

// -- Real markup through the HTML parser --

/// Parse `markup` (an `<svg id="s">` somewhere in it) with the HTML tree
/// builder, so foreign-content tag case fix-ups (`linearGradient`) and
/// nesting are exercised the way pages reach the SVG parser.
fn parse_markup(markup: &str) -> SvgElement {
    let doc = crate::html::tree_builder::TreeBuilder::parse(markup);
    let svg = doc.get_element_by_id("s").expect("svg#s");
    parse_svg(&doc, svg).expect("parses")
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}

#[test]
fn group_transforms_compose_and_presentation_attributes_inherit() {
    let svg = parse_markup(
        r#"<svg id="s" width="100" height="100">
             <g transform="translate(10 20)" fill="red" stroke="blue" stroke-width="3">
               <g transform="translate(5, 5)">
                 <circle cx="1" cy="2" r="4"/>
               </g>
               <line x1="0" y1="0" x2="10" y2="0" stroke-linecap="round"/>
               <rect x="0" y="0" width="5" height="5" fill="green"/>
             </g>
           </svg>"#,
    );
    assert_eq!(svg.shapes.len(), 3, "{:?}", svg.shapes);
    match &svg.shapes[0] {
        SvgShape::Circle {
            cx,
            cy,
            fill,
            stroke,
            stroke_width,
            ..
        } => {
            assert!(near(*cx, 16.0) && near(*cy, 27.0), "({cx}, {cy})");
            assert_eq!(fill.as_color(), Some(Color::rgb(255, 0, 0)), "fill inherited");
            assert_eq!(stroke.as_color(), Some(Color::rgb(0, 0, 255)));
            assert!(near(*stroke_width, 3.0));
        },
        other => panic!("{other:?}"),
    }
    match &svg.shapes[1] {
        SvgShape::Line { x1, y1, x2, .. } => {
            assert!(near(*x1, 10.0) && near(*y1, 20.0) && near(*x2, 20.0));
        },
        other => panic!("{other:?}"),
    }
    match &svg.shapes[2] {
        SvgShape::Rect { fill, .. } => {
            assert_eq!(fill.as_color(), Some(Color::rgb(0, 128, 0)), "own fill wins");
        },
        other => panic!("{other:?}"),
    }
}

#[test]
fn transform_attribute_functions() {
    let pt = |t: &str| parse_transform_attr(t).apply(1.0, 0.0);
    let (x, y) = pt("translate(3)");
    assert!(near(x, 4.0) && near(y, 0.0));
    let (x, y) = pt("scale(2 3)");
    assert!(near(x, 2.0) && near(y, 0.0));
    let (x, y) = pt("rotate(90)");
    assert!(near(x, 0.0) && near(y, 1.0), "degrees, clockwise in y-down: ({x}, {y})");
    // rotate(a cx cy) turns about (cx, cy).
    let (x, y) = parse_transform_attr("rotate(180 1 1)").apply(1.0, 0.0);
    assert!(near(x, 1.0) && near(y, 2.0), "({x}, {y})");
    let (x, y) = pt("matrix(1 0 0 1 5 6)");
    assert!(near(x, 6.0) && near(y, 6.0));
    // Chained: applied right to left, as SVG specifies.
    let (x, y) = pt("translate(10, 0) scale(2)");
    assert!(near(x, 12.0) && near(y, 0.0), "({x}, {y})");
    // Malformed / unknown functions are the identity, never a panic.
    for bad in ["", "skewX(30)", "matrix(1 2)", "translate(", "rotate)", "((((", "scale(NaN)"] {
        let _ = parse_transform_attr(bad).apply(1.0, 1.0);
    }
    let (x, y) = pt("bogus(5) translate(1 1)");
    assert!(near(x, 2.0) && near(y, 1.0));
}

#[test]
fn defs_gradients_and_stops_parse_from_markup() {
    let svg = parse_markup(
        r##"<svg id="s" width="10" height="10">
              <defs>
                <linearGradient id="lg" x1="0" y1="0" x2="1" y2="0"
                                gradientUnits="userSpaceOnUse">
                  <stop offset="100%" stop-color="blue"/>
                  <stop offset="0" stop-color="red" stop-opacity="0.5"/>
                </linearGradient>
                <radialGradient id="rg" cx="0.5" cy="0.5" r="0.5">
                  <stop offset="0.2" stop-color="#00ff00"/>
                </radialGradient>
              </defs>
              <rect width="10" height="10" fill="url(#lg)"/>
              <circle r="3" fill="url('#rg')"/>
            </svg>"##,
    );
    match svg.defs.gradients.get("lg") {
        Some(SvgGradientDef::Linear {
            x2,
            stops,
            user_space,
            ..
        }) => {
            assert!(near(*x2, 1.0) && *user_space);
            assert_eq!(stops.len(), 2);
            // Sorted by offset; stop-opacity folded into alpha.
            assert!(near(stops[0].offset, 0.0) && near(stops[1].offset, 1.0));
            assert_eq!(stops[0].color.r, 255);
            assert!((126..=128).contains(&stops[0].color.a), "{:?}", stops[0].color);
            assert_eq!(stops[1].color.b, 255);
        },
        other => panic!("linear gradient: {other:?} in {:?}", svg.defs),
    }
    match svg.defs.gradients.get("rg") {
        Some(SvgGradientDef::Radial { r, stops, user_space, .. }) => {
            assert!(near(*r, 0.5) && !*user_space);
            assert_eq!(stops.len(), 1);
            assert!(near(stops[0].offset, 0.2));
        },
        other => panic!("radial gradient: {other:?}"),
    }
    assert_eq!(svg.shapes.len(), 2, "defs children are not drawn");
    match &svg.shapes[0] {
        SvgShape::Rect { fill, .. } => {
            assert!(matches!(fill, SvgPaint::GradientRef(id) if id == "lg"), "{fill:?}");
        },
        other => panic!("{other:?}"),
    }
    match &svg.shapes[1] {
        SvgShape::Circle { fill, .. } => {
            assert!(matches!(fill, SvgPaint::GradientRef(id) if id == "rg"), "{fill:?}");
        },
        other => panic!("{other:?}"),
    }
}

#[test]
fn polygons_polylines_and_stroke_styles() {
    let svg = parse_markup(
        r#"<svg id="s" width="10" height="10">
             <polygon points="0,0 10,0 5,8" fill-rule="evenodd" stroke-linejoin="round"/>
             <polyline points="0 0, 1 1 2 0 3" stroke="red"/>
             <path d="M0 0 L5 5" stroke-linecap="square" stroke-linejoin="bevel"/>
           </svg>"#,
    );
    assert_eq!(svg.shapes.len(), 3, "{:?}", svg.shapes);
    match &svg.shapes[0] {
        SvgShape::Polygon {
            points,
            fill_rule,
            stroke_linejoin,
            ..
        } => {
            assert_eq!(points, &vec![(0.0, 0.0), (10.0, 0.0), (5.0, 8.0)]);
            assert_eq!(*fill_rule, FillRule::EvenOdd);
            assert_eq!(*stroke_linejoin, LineJoin::Round);
        },
        other => panic!("{other:?}"),
    }
    match &svg.shapes[1] {
        // The dangling odd coordinate is dropped.
        SvgShape::Polyline { points, .. } => {
            assert_eq!(points, &vec![(0.0, 0.0), (1.0, 1.0), (2.0, 0.0)]);
        },
        other => panic!("{other:?}"),
    }
    match &svg.shapes[2] {
        SvgShape::Path {
            stroke_linecap,
            stroke_linejoin,
            ..
        } => {
            assert_eq!(*stroke_linecap, LineCap::Square);
            assert_eq!(*stroke_linejoin, LineJoin::Bevel);
        },
        other => panic!("{other:?}"),
    }
}

#[test]
fn text_with_tspans_anchor_and_weight() {
    let svg = parse_markup(
        r#"<svg id="s" width="100" height="20">
             <text x="50" y="10" text-anchor="middle" font-weight="bold" font-size="12">
               <tspan class="a">Hello</tspan> <tspan>world</tspan>
             </text>
           </svg>"#,
    );
    match &svg.shapes[..] {
        [SvgShape::Text {
            text,
            text_anchor,
            font_weight,
            font_size,
            spans,
            ..
        }] => {
            assert_eq!(*text_anchor, TextAnchor::Middle);
            assert_eq!(*font_weight, 700);
            assert!(near(*font_size, 12.0));
            // Direct text only (here just whitespace); tspans are separate.
            assert_eq!(text, "");
            assert_eq!(spans.len(), 2, "{spans:?}");
            assert_eq!((spans[0].text.as_str(), spans[1].text.as_str()), ("Hello", "world"));
            assert_eq!(spans[0].class.as_deref(), Some("a"));
        },
        other => panic!("{other:?}"),
    }
}

/// A scaled group scales its shapes' sizes, not just their positions:
/// `<g transform="scale(0.5)">` icons used to draw full-size circles and
/// rects beside correctly scaled paths.
#[test]
fn group_scale_scales_shape_sizes() {
    let svg = parse_markup(
        r#"<svg id="s" width="24" height="24">
             <g transform="scale(0.5)" stroke-width="4">
               <rect x="4" y="8" width="20" height="10" rx="2"/>
               <circle cx="10" cy="10" r="6"/>
               <line x1="0" y1="0" x2="4" y2="0"/>
               <text x="0" y="10" font-size="20">A</text>
             </g>
             <g transform="scale(2 1)"><circle cx="0" cy="0" r="3"/></g>
             <g transform="scale(-1 1)"><rect x="1" y="0" width="2" height="2"/></g>
             <rect transform="rotate(45)" width="2" height="2"/>
           </svg>"#,
    );
    let s = &svg.shapes;
    assert_eq!(s.len(), 7, "{s:?}");
    match &s[0] {
        SvgShape::Rect {
            x,
            y,
            width,
            height,
            rx,
            stroke_width,
            ..
        } => {
            assert!(near(*x, 2.0) && near(*y, 4.0), "({x}, {y})");
            assert!(near(*width, 10.0) && near(*height, 5.0), "{width}x{height}");
            assert!(near(*rx, 1.0) && near(*stroke_width, 2.0));
        },
        other => panic!("{other:?}"),
    }
    match &s[1] {
        SvgShape::Circle { cx, r, .. } => assert!(near(*cx, 5.0) && near(*r, 3.0), "{r}"),
        other => panic!("{other:?}"),
    }
    match &s[2] {
        SvgShape::Line { x2, stroke_width, .. } => {
            assert!(near(*x2, 2.0) && near(*stroke_width, 2.0));
        },
        other => panic!("{other:?}"),
    }
    match &s[3] {
        SvgShape::Text { font_size, .. } => assert!(near(*font_size, 10.0)),
        other => panic!("{other:?}"),
    }
    // Non-uniform scale turns a circle into an ellipse.
    match &s[4] {
        SvgShape::Ellipse { rx, ry, .. } => assert!(near(*rx, 6.0) && near(*ry, 3.0)),
        other => panic!("{other:?}"),
    }
    // A mirroring scale keeps a positive extent at the mirrored place.
    match &s[5] {
        SvgShape::Rect { x, width, .. } => assert!(near(*x, -3.0) && near(*width, 2.0)),
        other => panic!("{other:?}"),
    }
    // A rotated rect is the polygon of its rotated corners.
    match &s[6] {
        SvgShape::Polygon { points, .. } => {
            assert_eq!(points.len(), 4);
            // The far corner (2, 2) lands on the y axis, 2 * sqrt(2) down.
            let h = 2.0 * std::f32::consts::SQRT_2;
            assert!(near(points[2].0, 0.0) && near(points[2].1, h), "{points:?}");
        },
        other => panic!("{other:?}"),
    }
}
