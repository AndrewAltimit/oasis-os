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
            rect_filled_with(SvgPaint::PatternRef("a".into())),
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
