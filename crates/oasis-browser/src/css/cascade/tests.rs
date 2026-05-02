//! Tests for the CSS cascade module.

use super::super::parser::{
    AttrOp, Combinator, CompoundSelector, CssValue, Declaration, PropertyId, Rule, Selector,
    SelectorList, SimpleSelector, Stylesheet,
};
use super::super::values::{ComputedStyle, Display, FontWeight};
use super::*;
use crate::html::dom::{Attribute, Document, ElementData, Node, NodeKind, TagName};
use oasis_types::backend::Color;

/// Default cascade context for tests (no hover, no visited URLs).
fn ctx() -> CascadeContext<'static> {
    CascadeContext::default()
}

// -- Test DOM helpers -----------------------------------------------

/// Build a minimal document: <html><body>...</body></html>.
fn make_doc(body_children: Vec<(TagName, Vec<Attribute>)>) -> Document {
    let mut nodes = Vec::new();

    // 0: Document root
    nodes.push(Node {
        kind: NodeKind::Document,
        parent: None,
        children: vec![1],
    });

    // 1: <html>
    nodes.push(Node {
        kind: NodeKind::Element(ElementData {
            tag: TagName::Html,
            attributes: vec![],
        }),
        parent: Some(0),
        children: vec![2],
    });

    // 2: <body>
    let body_child_ids: Vec<NodeId> = (3..3 + body_children.len()).collect();
    nodes.push(Node {
        kind: NodeKind::Element(ElementData {
            tag: TagName::Body,
            attributes: vec![],
        }),
        parent: Some(1),
        children: body_child_ids,
    });

    // Body children.
    for (tag, attrs) in body_children {
        nodes.push(Node {
            kind: NodeKind::Element(ElementData {
                tag,
                attributes: attrs,
            }),
            parent: Some(2),
            children: vec![],
        });
    }

    Document::from_nodes(nodes, 0)
}

fn make_rule(selectors: Vec<Selector>, declarations: Vec<Declaration>) -> Rule {
    Rule {
        selectors: SelectorList { selectors },
        declarations,
        layer: None,
        container: None,
        scope: None,
    }
}

/// Create a type selector: `tag`.
fn simple_type_selector(tag: &str) -> Selector {
    Selector {
        parts: vec![(
            CompoundSelector {
                parts: vec![SimpleSelector::Type(tag.to_string())],
            },
            None,
        )],
    }
}

/// Create a class selector: `.cls`.
fn simple_class_selector(cls: &str) -> Selector {
    Selector {
        parts: vec![(
            CompoundSelector {
                parts: vec![SimpleSelector::Class(cls.to_string())],
            },
            None,
        )],
    }
}

/// Create an ID selector: `#id`.
fn simple_id_selector(id: &str) -> Selector {
    Selector {
        parts: vec![(
            CompoundSelector {
                parts: vec![SimpleSelector::Id(id.to_string())],
            },
            None,
        )],
    }
}

/// Create a descendant selector: `ancestor descendant`.
fn descendant_selector(ancestor_tag: &str, descendant_tag: &str) -> Selector {
    // Parts stored left-to-right: ancestor first, descendant last.
    Selector {
        parts: vec![
            (
                CompoundSelector {
                    parts: vec![SimpleSelector::Type(ancestor_tag.to_string())],
                },
                None,
            ),
            (
                CompoundSelector {
                    parts: vec![SimpleSelector::Type(descendant_tag.to_string())],
                },
                Some(Combinator::Descendant),
            ),
        ],
    }
}

fn decl(property: &str, value: CssValue, important: bool) -> Declaration {
    Declaration {
        property_id: PropertyId::from_name(property),
        property: property.to_string(),
        value,
        important,
    }
}

/// Return the built-in user-agent stylesheet (test helper).
fn default_stylesheet() -> &'static Stylesheet {
    super::super::default::default_stylesheet()
}

// -- Tests ----------------------------------------------------------

#[test]
fn type_selector_matching() {
    let doc = make_doc(vec![(TagName::P, vec![]), (TagName::Div, vec![])]);
    let sel = simple_type_selector("p");
    // Node 3 is <p>, node 4 is <div>.
    assert!(matching::matches_selector(&doc, 3, &sel, &ctx()));
    assert!(!matching::matches_selector(&doc, 4, &sel, &ctx()));
}

#[test]
fn class_selector_matching() {
    let doc = make_doc(vec![
        (
            TagName::P,
            vec![Attribute {
                name: "class".to_string(),
                value: "highlight important".to_string(),
            }],
        ),
        (TagName::P, vec![]),
    ]);
    let sel = simple_class_selector("highlight");
    assert!(matching::matches_selector(&doc, 3, &sel, &ctx()));
    assert!(!matching::matches_selector(&doc, 4, &sel, &ctx()));
}

#[test]
fn id_selector_matching() {
    let doc = make_doc(vec![(
        TagName::Div,
        vec![Attribute {
            name: "id".to_string(),
            value: "main".to_string(),
        }],
    )]);
    let sel = simple_id_selector("main");
    assert!(matching::matches_selector(&doc, 3, &sel, &ctx()));

    let wrong = simple_id_selector("other");
    assert!(!matching::matches_selector(&doc, 3, &wrong, &ctx()));
}

#[test]
fn descendant_selector_matching() {
    // <body> > <div> > <p>
    let mut doc = make_doc(vec![(TagName::Div, vec![])]);
    // Add <p> as child of <div> (node 3).
    let p_id = doc.nodes.len();
    doc.nodes.push(Node {
        kind: NodeKind::Element(ElementData {
            tag: TagName::P,
            attributes: vec![],
        }),
        parent: Some(3),
        children: vec![],
    });
    doc.nodes[3].children.push(p_id);

    let sel = descendant_selector("div", "p");
    assert!(
        matching::matches_selector(&doc, p_id, &sel, &ctx()),
        "p inside div should match `div p`"
    );

    // <p> directly in <body> should NOT match `div p`.
    let doc2 = make_doc(vec![(TagName::P, vec![])]);
    assert!(
        !matching::matches_selector(&doc2, 3, &sel, &ctx()),
        "p in body should not match `div p`"
    );
}

#[test]
fn specificity_ordering() {
    // An ID selector (#main) should beat a class (.cls).
    let doc = make_doc(vec![(
        TagName::Div,
        vec![
            Attribute {
                name: "id".to_string(),
                value: "main".to_string(),
            },
            Attribute {
                name: "class".to_string(),
                value: "cls".to_string(),
            },
        ],
    )]);

    let rule_class = make_rule(
        vec![simple_class_selector("cls")],
        vec![decl("color", CssValue::Keyword("red".to_string()), false)],
    );
    let rule_id = make_rule(
        vec![simple_id_selector("main")],
        vec![decl("color", CssValue::Keyword("blue".to_string()), false)],
    );

    // Class rule comes first, ID rule second.
    let sheet = Stylesheet {
        rules: vec![rule_class, rule_id],
        keyframes: vec![],
        layers: vec![],
        counter_styles: vec![],
        properties: vec![],
        font_faces: vec![],
    };
    let styles = style_tree(&doc, &[&sheet], &[], &ctx());

    let style = styles[3].as_ref().expect("div should have style");
    // Blue wins because #main has higher specificity.
    assert_eq!(style.color, Color::rgb(0, 0, 255));
}

#[test]
fn inheritance_of_color_and_font() {
    // Parent <div> sets color: red, font-weight: bold (as Number).
    // Child <p> should inherit those.
    let mut doc = make_doc(vec![(TagName::Div, vec![])]);
    let p_id = doc.nodes.len();
    doc.nodes.push(Node {
        kind: NodeKind::Element(ElementData {
            tag: TagName::P,
            attributes: vec![],
        }),
        parent: Some(3),
        children: vec![],
    });
    doc.nodes[3].children.push(p_id);

    let rule = make_rule(
        vec![simple_type_selector("div")],
        vec![
            decl("color", CssValue::Keyword("red".to_string()), false),
            decl("font-weight", CssValue::Number(700.0), false),
        ],
    );
    let sheet = Stylesheet {
        rules: vec![rule],
        keyframes: vec![],
        layers: vec![],
        counter_styles: vec![],
        properties: vec![],
        font_faces: vec![],
    };
    let styles = style_tree(&doc, &[&sheet], &[], &ctx());

    let p_style = styles[p_id].as_ref().expect("p should have style");
    assert_eq!(p_style.color, Color::rgb(255, 0, 0));
    assert_eq!(p_style.font_weight, FontWeight::BOLD);
}

#[test]
fn important_overrides_specificity() {
    let doc = make_doc(vec![(
        TagName::Div,
        vec![Attribute {
            name: "id".to_string(),
            value: "main".to_string(),
        }],
    )]);

    // Normal ID rule: color blue.
    let rule_id = make_rule(
        vec![simple_id_selector("main")],
        vec![decl("color", CssValue::Keyword("blue".to_string()), false)],
    );
    // Type rule with !important: color green.
    let rule_type = make_rule(
        vec![simple_type_selector("div")],
        vec![decl("color", CssValue::Keyword("green".to_string()), true)],
    );

    let sheet = Stylesheet {
        rules: vec![rule_id, rule_type],
        keyframes: vec![],
        layers: vec![],
        counter_styles: vec![],
        properties: vec![],
        font_faces: vec![],
    };
    let styles = style_tree(&doc, &[&sheet], &[], &ctx());
    let style = styles[3].as_ref().expect("div should have style");
    // Green wins because !important beats higher specificity.
    assert_eq!(style.color, Color::rgb(0, 128, 0));
}

#[test]
fn multiple_stylesheets_merged() {
    let doc = make_doc(vec![(TagName::P, vec![])]);

    let sheet1 = Stylesheet {
        rules: vec![make_rule(
            vec![simple_type_selector("p")],
            vec![decl("color", CssValue::Keyword("red".to_string()), false)],
        )],
        keyframes: vec![],
        layers: vec![],
        counter_styles: vec![],
        properties: vec![],
        font_faces: vec![],
    };
    let sheet2 = Stylesheet {
        rules: vec![make_rule(
            vec![simple_type_selector("p")],
            vec![decl("font-weight", CssValue::Number(700.0), false)],
        )],
        keyframes: vec![],
        layers: vec![],
        counter_styles: vec![],
        properties: vec![],
        font_faces: vec![],
    };

    let styles = style_tree(&doc, &[&sheet1, &sheet2], &[], &ctx());
    let style = styles[3].as_ref().expect("p should have style");
    assert_eq!(style.color, Color::rgb(255, 0, 0));
    assert_eq!(style.font_weight, FontWeight::BOLD);
}

#[test]
fn inline_style_override() {
    let doc = make_doc(vec![(TagName::P, vec![])]);

    // Stylesheet says color: red.
    let sheet = Stylesheet {
        rules: vec![make_rule(
            vec![simple_type_selector("p")],
            vec![decl("color", CssValue::Keyword("red".to_string()), false)],
        )],
        keyframes: vec![],
        layers: vec![],
        counter_styles: vec![],
        properties: vec![],
        font_faces: vec![],
    };

    // Inline style says color: blue.
    let inline = vec![(
        3_usize,
        vec![decl("color", CssValue::Keyword("blue".to_string()), false)],
    )];

    let styles = style_tree(&doc, &[&sheet], &inline, &ctx());
    let style = styles[3].as_ref().expect("p should have style");
    // Inline wins over stylesheet.
    assert_eq!(style.color, Color::rgb(0, 0, 255));
}

#[test]
fn element_defaults_applied() {
    let doc = make_doc(vec![
        (TagName::P, vec![]),
        (TagName::H1, vec![]),
        (TagName::A, vec![]),
    ]);
    let ua = default_stylesheet();
    let styles = style_tree(&doc, &[ua], &[], &ctx());

    let p_style = styles[3].as_ref().unwrap();
    assert_eq!(p_style.display, Display::Block);

    let h1_style = styles[4].as_ref().unwrap();
    assert_eq!(h1_style.display, Display::Block);
    assert_eq!(h1_style.font_weight, FontWeight::BOLD);
    assert_eq!(h1_style.font_style, crate::css::values::FontStyle::Normal);
    // h1 = 2em * ROOT_FONT_SIZE
    assert!((h1_style.font_size - crate::css::values::ROOT_FONT_SIZE * 2.0).abs() < f32::EPSILON);

    let a_style = styles[5].as_ref().unwrap();
    assert_eq!(a_style.color, Color::rgb(0, 0x66, 0xcc));
}

#[test]
fn non_element_nodes_get_no_style() {
    let mut nodes = Vec::new();
    // 0: Document root
    nodes.push(Node {
        kind: NodeKind::Document,
        parent: None,
        children: vec![1],
    });
    // 1: <html>
    nodes.push(Node {
        kind: NodeKind::Element(ElementData {
            tag: TagName::Html,
            attributes: vec![],
        }),
        parent: Some(0),
        children: vec![2],
    });
    // 2: Text node
    nodes.push(Node {
        kind: NodeKind::Text("hello".to_string()),
        parent: Some(1),
        children: vec![],
    });

    let doc = Document::from_nodes(nodes, 0);
    let sheet = Stylesheet {
        rules: vec![],
        keyframes: vec![],
        layers: vec![],
        counter_styles: vec![],
        properties: vec![],
        font_faces: vec![],
    };
    let styles = style_tree(&doc, &[&sheet], &[], &ctx());

    assert!(styles[0].is_none(), "Document node");
    assert!(styles[1].is_some(), "html element");
    assert!(styles[2].is_none(), "Text node");
}

// -- New selector tests (Phase 4.3) ---------------------------------

#[test]
fn attribute_exists_selector() {
    let doc = make_doc(vec![
        (
            TagName::Div,
            vec![Attribute {
                name: "data-x".to_string(),
                value: "1".to_string(),
            }],
        ),
        (TagName::Div, vec![]),
    ]);
    let sel = Selector {
        parts: vec![(
            CompoundSelector {
                parts: vec![SimpleSelector::Attribute {
                    name: "data-x".to_string(),
                    op: AttrOp::Exists,
                    value: None,
                }],
            },
            None,
        )],
    };
    assert!(matching::matches_selector(&doc, 3, &sel, &ctx()));
    assert!(!matching::matches_selector(&doc, 4, &sel, &ctx()));
}

#[test]
fn attribute_equals_selector() {
    let doc = make_doc(vec![(
        TagName::Div,
        vec![Attribute {
            name: "lang".to_string(),
            value: "en".to_string(),
        }],
    )]);
    let sel = Selector {
        parts: vec![(
            CompoundSelector {
                parts: vec![SimpleSelector::Attribute {
                    name: "lang".to_string(),
                    op: AttrOp::Equals,
                    value: Some("en".to_string()),
                }],
            },
            None,
        )],
    };
    assert!(matching::matches_selector(&doc, 3, &sel, &ctx()));

    let wrong = Selector {
        parts: vec![(
            CompoundSelector {
                parts: vec![SimpleSelector::Attribute {
                    name: "lang".to_string(),
                    op: AttrOp::Equals,
                    value: Some("fr".to_string()),
                }],
            },
            None,
        )],
    };
    assert!(!matching::matches_selector(&doc, 3, &wrong, &ctx()));
}

#[test]
fn attribute_prefix_selector() {
    let doc = make_doc(vec![(
        TagName::A,
        vec![Attribute {
            name: "href".to_string(),
            value: "https://example.com".to_string(),
        }],
    )]);
    let sel = Selector {
        parts: vec![(
            CompoundSelector {
                parts: vec![SimpleSelector::Attribute {
                    name: "href".to_string(),
                    op: AttrOp::Prefix,
                    value: Some("https".to_string()),
                }],
            },
            None,
        )],
    };
    assert!(matching::matches_selector(&doc, 3, &sel, &ctx()));
}

#[test]
fn attribute_substring_selector() {
    let doc = make_doc(vec![(
        TagName::Div,
        vec![Attribute {
            name: "class".to_string(),
            value: "my-widget-box".to_string(),
        }],
    )]);
    let sel = Selector {
        parts: vec![(
            CompoundSelector {
                parts: vec![SimpleSelector::Attribute {
                    name: "class".to_string(),
                    op: AttrOp::Substring,
                    value: Some("widget".to_string()),
                }],
            },
            None,
        )],
    };
    assert!(matching::matches_selector(&doc, 3, &sel, &ctx()));
}

#[test]
fn not_selector() {
    let doc = make_doc(vec![(TagName::P, vec![]), (TagName::Div, vec![])]);
    let sel = Selector {
        parts: vec![(
            CompoundSelector {
                parts: vec![SimpleSelector::Not(vec![CompoundSelector {
                    parts: vec![SimpleSelector::Type("div".to_string())],
                }])],
            },
            None,
        )],
    };
    // <p> is not <div>, so it matches :not(div).
    assert!(matching::matches_selector(&doc, 3, &sel, &ctx()));
    // <div> is <div>, so it does NOT match :not(div).
    assert!(!matching::matches_selector(&doc, 4, &sel, &ctx()));
}

#[test]
fn not_selector_list_form() {
    // :not(div, p) should match an element that is neither a div nor a p.
    let doc = make_doc(vec![
        (TagName::Span, vec![]),
        (TagName::P, vec![]),
        (TagName::Div, vec![]),
    ]);
    // Parse via CSS to exercise the Level 4 list form end-to-end.
    let sheet = Stylesheet::parse("*:not(div, p) { color: red; }");
    let sel = sheet.rules[0].selectors.selectors[0].clone();
    // <span> (node 3) matches.
    assert!(matching::matches_selector(&doc, 3, &sel, &ctx()));
    // <p> (node 4) excluded by the list.
    assert!(!matching::matches_selector(&doc, 4, &sel, &ctx()));
    // <div> (node 5) excluded by the list.
    assert!(!matching::matches_selector(&doc, 5, &sel, &ctx()));
}

#[test]
fn not_selector_list_specificity_takes_max() {
    // :not(p, #header) should count one ID (the max of the two).
    let sheet = Stylesheet::parse("div:not(p, #header) { color: red; }");
    let sel = &sheet.rules[0].selectors.selectors[0];
    let spec = sel.specificity();
    // div = 1 type; #header = 1 id; no classes.
    assert_eq!(spec.ids, 1);
    assert_eq!(spec.classes, 0);
    assert_eq!(spec.types, 1);
}

#[test]
fn adjacent_sibling_selector() {
    // <body> has three children: <h1>, <p>, <div>
    let doc = make_doc(vec![
        (TagName::H1, vec![]),
        (TagName::P, vec![]),
        (TagName::Div, vec![]),
    ]);
    // h1 + p should match the <p> (node 4).
    let sel = Selector {
        parts: vec![
            (
                CompoundSelector {
                    parts: vec![SimpleSelector::Type("h1".to_string())],
                },
                None,
            ),
            (
                CompoundSelector {
                    parts: vec![SimpleSelector::Type("p".to_string())],
                },
                Some(Combinator::AdjacentSibling),
            ),
        ],
    };
    assert!(matching::matches_selector(&doc, 4, &sel, &ctx()));
    // <div> (node 5) is not immediately after <h1>.
    assert!(!matching::matches_selector(&doc, 5, &sel, &ctx()));
}

#[test]
fn general_sibling_selector() {
    let doc = make_doc(vec![
        (TagName::H1, vec![]),
        (TagName::P, vec![]),
        (TagName::Div, vec![]),
    ]);
    // h1 ~ div should match <div> (node 5) because <h1> precedes it.
    let sel = Selector {
        parts: vec![
            (
                CompoundSelector {
                    parts: vec![SimpleSelector::Type("h1".to_string())],
                },
                None,
            ),
            (
                CompoundSelector {
                    parts: vec![SimpleSelector::Type("div".to_string())],
                },
                Some(Combinator::GeneralSibling),
            ),
        ],
    };
    assert!(matching::matches_selector(&doc, 5, &sel, &ctx()));
}

#[test]
fn nth_child_matching() {
    use super::super::selectors::AnB;
    assert!(AnB { a: 2, b: 1 }.matches(1)); // odd: 1
    assert!(!AnB { a: 2, b: 1 }.matches(2)); // odd: 2 is even
    assert!(AnB { a: 2, b: 1 }.matches(3)); // odd: 3
    assert!(AnB { a: 2, b: 0 }.matches(2)); // even: 2
    assert!(!AnB { a: 2, b: 0 }.matches(1)); // even: 1 is odd
    assert!(AnB { a: 0, b: 3 }.matches(3)); // exactly 3
    assert!(!AnB { a: 0, b: 3 }.matches(4)); // not 3
    assert!(AnB { a: 3, b: 0 }.matches(6)); // 3n: 6
}

#[test]
fn parse_an_plus_b_cases() {
    use super::super::selectors::AnB;
    assert_eq!(AnB::parse("odd"), Some(AnB { a: 2, b: 1 }));
    assert_eq!(AnB::parse("even"), Some(AnB { a: 2, b: 0 }));
    assert_eq!(AnB::parse("3"), Some(AnB { a: 0, b: 3 }));
    assert_eq!(AnB::parse("2n+1"), Some(AnB { a: 2, b: 1 }));
    assert_eq!(AnB::parse("2n"), Some(AnB { a: 2, b: 0 }));
    assert_eq!(AnB::parse("n+3"), Some(AnB { a: 1, b: 3 }));
    assert_eq!(AnB::parse("-n+3"), Some(AnB { a: -1, b: 3 }));
}

#[test]
fn only_child_pseudo_class() {
    // Single child.
    let doc = make_doc(vec![(TagName::P, vec![])]);
    let __out = &doc.nodes[3].kind;
    let NodeKind::Element(e) = __out else {
        panic!("expected NodeKind::Element, got {__out:?}");
    };
    assert!(matching::match_pseudo_class(
        &doc,
        3,
        e,
        "only-child",
        &ctx()
    ));

    // Multiple children.
    let doc2 = make_doc(vec![(TagName::P, vec![]), (TagName::Div, vec![])]);
    let __out = &doc2.nodes[3].kind;
    let NodeKind::Element(e2) = __out else {
        panic!("expected NodeKind::Element, got {__out:?}");
    };
    assert!(!matching::match_pseudo_class(
        &doc2,
        3,
        e2,
        "only-child",
        &ctx()
    ));
}

#[test]
fn selector_parsing_attribute() {
    let sheet = Stylesheet::parse("[type=text] { color: red; }");
    assert!(!sheet.rules.is_empty());
    let rule = &sheet.rules[0];
    let sel = &rule.selectors.selectors[0];
    let compound = &sel.parts[0].0;
    assert!(matches!(
        &compound.parts[0],
        SimpleSelector::Attribute {
            name,
            op: AttrOp::Equals,
            value: Some(val),
        } if name == "type" && val == "text"
    ));
}

#[test]
fn selector_parsing_not() {
    let sheet = Stylesheet::parse(":not(.hidden) { display: block; }");
    assert!(!sheet.rules.is_empty());
    let sel = &sheet.rules[0].selectors.selectors[0];
    let compound = &sel.parts[0].0;
    assert!(matches!(&compound.parts[0], SimpleSelector::Not(_)));
}

#[test]
fn selector_parsing_adjacent_sibling() {
    let sheet = Stylesheet::parse("h1 + p { color: red; }");
    assert!(!sheet.rules.is_empty());
    let sel = &sheet.rules[0].selectors.selectors[0];
    assert_eq!(sel.parts.len(), 2);
    assert_eq!(sel.parts[1].1, Some(Combinator::AdjacentSibling));
}

#[test]
fn selector_parsing_general_sibling() {
    let sheet = Stylesheet::parse("h1 ~ p { color: red; }");
    assert!(!sheet.rules.is_empty());
    let sel = &sheet.rules[0].selectors.selectors[0];
    assert_eq!(sel.parts.len(), 2);
    assert_eq!(sel.parts[1].1, Some(Combinator::GeneralSibling));
}

#[test]
fn attribute_includes_selector() {
    let doc = make_doc(vec![(
        TagName::Div,
        vec![Attribute {
            name: "class".to_string(),
            value: "foo bar baz".to_string(),
        }],
    )]);
    let sel = Selector {
        parts: vec![(
            CompoundSelector {
                parts: vec![SimpleSelector::Attribute {
                    name: "class".to_string(),
                    op: AttrOp::Includes,
                    value: Some("bar".to_string()),
                }],
            },
            None,
        )],
    };
    assert!(matching::matches_selector(&doc, 3, &sel, &ctx()));
}

// -- Stateful pseudo-class tests (Phase 10) ---------------------------

#[test]
fn hover_matches_hovered_node() {
    let doc = make_doc(vec![(TagName::P, vec![])]);
    let hctx = CascadeContext {
        hover_node: Some(3),
        visited_urls: None,
        focused_node: None,
        containers: None,
        global_layers: None,
    };
    let __out = &doc.nodes[3].kind;
    let NodeKind::Element(elem) = __out else {
        panic!("expected NodeKind::Element, got {__out:?}");
    };
    assert!(matching::match_pseudo_class(&doc, 3, elem, "hover", &hctx));
    assert!(!matching::match_pseudo_class(
        &doc,
        3,
        elem,
        "hover",
        &ctx()
    ));
}

#[test]
fn hover_matches_ancestor_of_hovered_node() {
    // <body> (2) > <div> (3) > <p> (4)
    let mut doc = make_doc(vec![(TagName::Div, vec![])]);
    let p_id = doc.nodes.len();
    doc.nodes.push(Node {
        kind: NodeKind::Element(ElementData {
            tag: TagName::P,
            attributes: vec![],
        }),
        parent: Some(3),
        children: vec![],
    });
    doc.nodes[3].children.push(p_id);

    // Hover is on the <p> (inner element).
    let hctx = CascadeContext {
        hover_node: Some(p_id),
        visited_urls: None,
        focused_node: None,
        containers: None,
        global_layers: None,
    };
    // <div> (ancestor) should also match :hover.
    let __out = &doc.nodes[3].kind;
    let NodeKind::Element(div_elem) = __out else {
        panic!("expected NodeKind::Element, got {__out:?}");
    };
    assert!(matching::match_pseudo_class(
        &doc, 3, div_elem, "hover", &hctx
    ));
}

#[test]
fn visited_matches_with_visited_url() {
    let mut visited = std::collections::HashSet::new();
    visited.insert("/page1".to_string());

    let doc = make_doc(vec![(
        TagName::A,
        vec![Attribute {
            name: "href".to_string(),
            value: "/page1".to_string(),
        }],
    )]);
    let vctx = CascadeContext {
        hover_node: None,
        visited_urls: Some(&visited),
        focused_node: None,
        containers: None,
        global_layers: None,
    };
    let __out = &doc.nodes[3].kind;
    let NodeKind::Element(elem) = __out else {
        panic!("expected NodeKind::Element, got {__out:?}");
    };
    assert!(matching::match_pseudo_class(
        &doc, 3, elem, "visited", &vctx
    ));
    assert!(!matching::match_pseudo_class(&doc, 3, elem, "link", &vctx));
}

#[test]
fn link_matches_unvisited_anchor() {
    let visited = std::collections::HashSet::new();

    let doc = make_doc(vec![(
        TagName::A,
        vec![Attribute {
            name: "href".to_string(),
            value: "/page2".to_string(),
        }],
    )]);
    let vctx = CascadeContext {
        hover_node: None,
        visited_urls: Some(&visited),
        focused_node: None,
        containers: None,
        global_layers: None,
    };
    let __out = &doc.nodes[3].kind;
    let NodeKind::Element(elem) = __out else {
        panic!("expected NodeKind::Element, got {__out:?}");
    };
    assert!(matching::match_pseudo_class(&doc, 3, elem, "link", &vctx));
    assert!(!matching::match_pseudo_class(
        &doc, 3, elem, "visited", &vctx
    ));
}

#[test]
fn hover_style_applied_via_cascade() {
    let doc = make_doc(vec![(TagName::P, vec![])]);
    let sheet = Stylesheet::parse("p:hover { color: red; }");
    let hctx = CascadeContext {
        hover_node: Some(3),
        visited_urls: None,
        focused_node: None,
        containers: None,
        global_layers: None,
    };
    let styles = style_tree(&doc, &[&sheet], &[], &hctx);
    let style = styles[3].as_ref().expect("p should have style");
    assert_eq!(style.color, Color::rgb(255, 0, 0));

    // Without hover, color should be inherited default (white).
    let styles_no_hover = style_tree(&doc, &[&sheet], &[], &ctx());
    let style_no = styles_no_hover[3].as_ref().unwrap();
    assert_ne!(style_no.color, Color::rgb(255, 0, 0));
}

#[test]
fn visited_style_applied_via_cascade() {
    let mut visited = std::collections::HashSet::new();
    visited.insert("/page1".to_string());

    let doc = make_doc(vec![(
        TagName::A,
        vec![Attribute {
            name: "href".to_string(),
            value: "/page1".to_string(),
        }],
    )]);
    let sheet = Stylesheet::parse("a:visited { color: purple; }");
    let vctx = CascadeContext {
        hover_node: None,
        visited_urls: Some(&visited),
        focused_node: None,
        containers: None,
        global_layers: None,
    };
    let styles = style_tree(&doc, &[&sheet], &[], &vctx);
    let style = styles[3].as_ref().expect("a should have style");
    assert_eq!(style.color, Color::rgb(128, 0, 128));
}

// -- CSS custom properties / var() tests (var support) ----------------

#[test]
fn root_pseudo_class_matches_html() {
    let doc = make_doc(vec![(TagName::P, vec![])]);
    let NodeKind::Element(html_elem) = &doc.nodes[1].kind else {
        panic!("node 1 should be <html>");
    };
    // <html> (node 1) has parent Document (node 0) -> matches :root.
    assert!(matching::match_pseudo_class(
        &doc,
        1,
        html_elem,
        "root",
        &ctx()
    ));

    // <body> (node 2) has parent <html> (element) -> does NOT match :root.
    let NodeKind::Element(body_elem) = &doc.nodes[2].kind else {
        panic!("node 2 should be <body>");
    };
    assert!(!matching::match_pseudo_class(
        &doc,
        2,
        body_elem,
        "root",
        &ctx()
    ));
}

#[test]
fn custom_property_stored_and_inherited() {
    // :root { --color: red } p { color: var(--color) }
    let doc = make_doc(vec![(TagName::P, vec![])]);
    let css = ":root { --color: red; } p { color: var(--color); }";
    let sheet = Stylesheet::parse(css);
    let styles = style_tree(&doc, &[&sheet], &[], &ctx());

    let p_style = styles[3].as_ref().expect("p should have style");
    // var(--color) should resolve to "red" -> Color(255,0,0).
    assert_eq!(p_style.color, Color::rgb(255, 0, 0));
}

#[test]
fn var_with_fallback() {
    // No --missing defined, fallback value should be used.
    let doc = make_doc(vec![(TagName::P, vec![])]);
    let css = "p { color: var(--missing, blue); }";
    let sheet = Stylesheet::parse(css);
    let styles = style_tree(&doc, &[&sheet], &[], &ctx());

    let p_style = styles[3].as_ref().expect("p should have style");
    assert_eq!(p_style.color, Color::rgb(0, 0, 255));
}

#[test]
fn var_with_hex_fallback() {
    let doc = make_doc(vec![(TagName::P, vec![])]);
    let css = "p { color: var(--missing, #202122); }";
    let sheet = Stylesheet::parse(css);
    let styles = style_tree(&doc, &[&sheet], &[], &ctx());

    let p_style = styles[3].as_ref().expect("p should have style");
    assert_eq!(p_style.color, Color::rgb(0x20, 0x21, 0x22));
}

#[test]
fn chained_variables() {
    // --a references --b, which holds a concrete value.
    let doc = make_doc(vec![(TagName::P, vec![])]);
    let css = ":root { --b: green; --a: var(--b); } p { color: var(--a); }";
    let sheet = Stylesheet::parse(css);
    let styles = style_tree(&doc, &[&sheet], &[], &ctx());

    let p_style = styles[3].as_ref().expect("p should have style");
    assert_eq!(p_style.color, Color::rgb(0, 128, 0));
}

#[test]
fn var_in_border_shorthand() {
    let doc = make_doc(vec![(TagName::P, vec![])]);
    let css = ":root { --bc: red; } p { border: 1px solid var(--bc); }";
    let sheet = Stylesheet::parse(css);
    let styles = style_tree(&doc, &[&sheet], &[], &ctx());

    let p_style = styles[3].as_ref().expect("p should have style");
    assert_eq!(p_style.border_top_color, Color::rgb(255, 0, 0));
}

#[test]
fn var_in_background() {
    let doc = make_doc(vec![(TagName::P, vec![])]);
    let css = ":root { --bg: #ff0000; } p { background: var(--bg); }";
    let sheet = Stylesheet::parse(css);
    let styles = style_tree(&doc, &[&sheet], &[], &ctx());

    let p_style = styles[3].as_ref().expect("p should have style");
    assert_eq!(p_style.background_color, Color::rgb(255, 0, 0));
}

#[test]
fn var_margin_property() {
    let doc = make_doc(vec![(TagName::P, vec![])]);
    let css = ":root { --sp: 10px; } p { margin-top: var(--sp); }";
    let sheet = Stylesheet::parse(css);
    let styles = style_tree(&doc, &[&sheet], &[], &ctx());

    let p_style = styles[3].as_ref().expect("p should have style");
    assert!((p_style.margin_top - 10.0).abs() < 0.01);
}

#[test]
fn custom_props_inherit_to_descendants() {
    // <body> > <div> (node 3) > <p> (node 4)
    let mut doc = make_doc(vec![(TagName::Div, vec![])]);
    let p_id = doc.nodes.len();
    doc.nodes.push(Node {
        kind: NodeKind::Element(ElementData {
            tag: TagName::P,
            attributes: vec![],
        }),
        parent: Some(3),
        children: vec![],
    });
    doc.nodes[3].children.push(p_id);

    let css = ":root { --text-color: purple; } p { color: var(--text-color); }";
    let sheet = Stylesheet::parse(css);
    let styles = style_tree(&doc, &[&sheet], &[], &ctx());

    let p_style = styles[p_id].as_ref().expect("p should have style");
    assert_eq!(p_style.color, Color::rgb(128, 0, 128));
}

#[test]
fn var_background_color_from_root() {
    // Wikipedia-style pattern: :root { --bg: #fff } body { background-color: var(--bg) }
    let doc = make_doc(vec![(TagName::P, vec![])]);
    let css = ":root { --background-color-base: #fff; } \
               body { background-color: var(--background-color-base); color: var(--background-color-base); }";
    let sheet = Stylesheet::parse(css);
    let styles = style_tree(&doc, &[&sheet], &[], &ctx());

    // Node 1 = <html>, node 2 = <body>
    let body_style = styles[2].as_ref().expect("body should have style");
    assert_eq!(
        body_style.background_color,
        Color::rgb(255, 255, 255),
        "body bg should be white from var(--background-color-base), got {:?}",
        body_style.background_color
    );
    assert_eq!(
        body_style.color,
        Color::rgb(255, 255, 255),
        "body color should be white from var(--background-color-base), got {:?}",
        body_style.color
    );
}

#[test]
fn prefers_color_scheme_dark_rejected() {
    // @media (prefers-color-scheme: dark) should NOT match.
    let doc = make_doc(vec![(TagName::P, vec![])]);
    let css = ":root { --bg: #ffffff; } \
               @media (prefers-color-scheme: dark) { :root { --bg: #000000; } } \
               body { background-color: var(--bg); }";
    let sheet = Stylesheet::parse(css);
    let styles = style_tree(&doc, &[&sheet], &[], &ctx());
    let body_style = styles[2].as_ref().expect("body should have style");
    assert_eq!(
        body_style.background_color,
        Color::rgb(255, 255, 255),
        "dark mode should not apply; expected white, got {:?}",
        body_style.background_color
    );
}

#[test]
fn cyclic_var_does_not_stack_overflow() {
    // `--a` references itself -- should resolve to empty (not crash).
    let mut props = rustc_hash::FxHashMap::default();
    props.insert("--a".to_string(), "var(--a)".to_string());
    let val = CssValue::Var("--a".to_string(), None);
    let resolved = var_resolve::resolve_css_var(&val, &props);
    assert_eq!(resolved, CssValue::Keyword(String::new()));
}

#[test]
fn indirect_cyclic_var_does_not_stack_overflow() {
    // `--a` -> `var(--b)`, `--b` -> `var(--a)` -- indirect cycle.
    let mut props = rustc_hash::FxHashMap::default();
    props.insert("--a".to_string(), "var(--b)".to_string());
    props.insert("--b".to_string(), "var(--a)".to_string());
    let val = CssValue::Var("--a".to_string(), None);
    let resolved = var_resolve::resolve_css_var(&val, &props);
    assert_eq!(resolved, CssValue::Keyword(String::new()));
}

// -- Selector index tests (Phase 2) ----------------------------------

#[test]
fn selector_index_reduces_comparisons() {
    // Build a stylesheet with 3 rules: .foo, .bar, p
    let sheet =
        Stylesheet::parse(".foo { color: red; } .bar { color: blue; } p { font-weight: bold; }");
    let index = SelectorIndex::build(&[&sheet]);

    // An element with class "foo" and tag "p" should only get
    // candidates from .foo and p buckets (not .bar).
    let candidates = index.candidates("p", None, &["foo"]);
    assert!(
        candidates.len() == 2,
        "should get 2 candidates (.foo and p), got {}",
        candidates.len()
    );

    // An element with class "bar" tag "div" should only get .bar.
    let candidates = index.candidates("div", None, &["bar"]);
    assert_eq!(candidates.len(), 1, "should get 1 candidate (.bar)");
}

#[test]
fn selector_index_universal_rules() {
    let sheet = Stylesheet::parse("* { margin: 0; } .cls { color: red; }");
    let index = SelectorIndex::build(&[&sheet]);

    // Any element should get the universal rule.
    let candidates = index.candidates("div", None, &[]);
    assert_eq!(candidates.len(), 1, "universal rule");

    // Element with class "cls" gets universal + .cls.
    let candidates = index.candidates("div", None, &["cls"]);
    assert_eq!(candidates.len(), 2, "universal + class");
}

#[test]
fn selector_index_mixed_selector_list() {
    // A rule with both a keyed selector (.foo) and a non-keyed
    // selector (*) must appear in universal so that non-.foo
    // elements still match via the `*` selector.
    let sheet = Stylesheet::parse("*, .foo { color: red; }");
    let index = SelectorIndex::build(&[&sheet]);

    // Element without class "foo" should still get the rule via universal.
    let candidates = index.candidates("div", None, &[]);
    assert_eq!(
        candidates.len(),
        1,
        "non-.foo element should match via universal bucket"
    );

    // Element with class "foo" gets the rule via both .foo bucket
    // and universal, but dedup should give exactly 1.
    let candidates = index.candidates("div", None, &["foo"]);
    assert_eq!(
        candidates.len(),
        1,
        "dedup should collapse .foo + universal into 1"
    );
}

#[test]
fn pseudo_content_respects_specificity() {
    // Higher-specificity rule (.special::before) should win even
    // when it appears before a lower-specificity rule (p::before).
    let sheet =
        Stylesheet::parse(".special::before { content: \"B\"; } p::before { content: \"A\"; }");
    // Build a <p class="special"> element (node 3 in make_doc).
    let doc = make_doc(vec![(
        TagName::P,
        vec![Attribute {
            name: "class".into(),
            value: "special".into(),
        }],
    )]);
    let ctx = ctx();
    let p_id = 3; // first body child in make_doc
    let parent_style = ComputedStyle::default();
    let result =
        matching::resolve_pseudo_style(&doc, p_id, "before", &parent_style, &[&sheet], &ctx);
    assert_eq!(
        result.as_ref().and_then(|s| s.content.clone()),
        Some("B".to_string()),
        ".special::before (higher specificity) should beat p::before",
    );
}

#[test]
fn test_body_has_default_margin() {
    let ua = default_stylesheet();
    let doc = make_doc(vec![]);
    let body_id = 2; // body is node 2 in make_doc
    let ctx = ctx();
    let index = SelectorIndex::build(&[ua]);
    let inline_map = rustc_hash::FxHashMap::default();
    let mut tag_cache = rustc_hash::FxHashMap::default();
    let style = compute_style(
        &doc,
        body_id,
        None,
        &[&ua],
        &index,
        &inline_map,
        &ctx,
        &mut tag_cache,
    );
    assert!(
        (style.margin_top - 8.0).abs() < 0.01,
        "body should have 8px top margin, got {}",
        style.margin_top,
    );
    assert!(
        (style.margin_left - 8.0).abs() < 0.01,
        "body should have 8px left margin, got {}",
        style.margin_left,
    );
}

// -- :has() relational pseudo-class ---------------------------------

/// Build a document with an <article> body child that contains an <img>
/// descendant (nested inside a <div>) plus a trailing <p> sibling.
///
/// Layout:
///   0 Document
///   1 <html>
///   2 <body>
///   3   <article>
///   4     <div>
///   5       <img>
///   6   <p>
///   7   <section>
///   8     <span>  (plain span, no img)
fn make_has_doc() -> Document {
    let mut nodes = vec![
        Node {
            kind: NodeKind::Document,
            parent: None,
            children: vec![1],
        },
        Node {
            kind: NodeKind::Element(ElementData {
                tag: TagName::Html,
                attributes: vec![],
            }),
            parent: Some(0),
            children: vec![2],
        },
        Node {
            kind: NodeKind::Element(ElementData {
                tag: TagName::Body,
                attributes: vec![],
            }),
            parent: Some(1),
            children: vec![3, 6, 7],
        },
        // 3: <article> with <div><img></div>
        Node {
            kind: NodeKind::Element(ElementData {
                tag: TagName::Article,
                attributes: vec![],
            }),
            parent: Some(2),
            children: vec![4],
        },
        Node {
            kind: NodeKind::Element(ElementData {
                tag: TagName::Div,
                attributes: vec![],
            }),
            parent: Some(3),
            children: vec![5],
        },
        Node {
            kind: NodeKind::Element(ElementData {
                tag: TagName::Img,
                attributes: vec![],
            }),
            parent: Some(4),
            children: vec![],
        },
        // 6: <p>, sibling of <article>
        Node {
            kind: NodeKind::Element(ElementData {
                tag: TagName::P,
                attributes: vec![],
            }),
            parent: Some(2),
            children: vec![],
        },
        // 7: <section> with a <span>, no <img> anywhere inside
        Node {
            kind: NodeKind::Element(ElementData {
                tag: TagName::Section,
                attributes: vec![],
            }),
            parent: Some(2),
            children: vec![8],
        },
        Node {
            kind: NodeKind::Element(ElementData {
                tag: TagName::Span,
                attributes: vec![],
            }),
            parent: Some(7),
            children: vec![],
        },
    ];
    // Silence unused mut warning if no further pushes.
    let _ = &mut nodes;
    Document::from_nodes(nodes, 0)
}

/// Parse a CSS selector from source like `"article:has(img)"`.
fn parse_selector(src: &str) -> Selector {
    let full = format!("{src} {{ color: red; }}");
    let sheet = Stylesheet::parse(&full);
    sheet.rules[0].selectors.selectors[0].clone()
}

// -- @layer cascade ordering ----------------------------------------

#[test]
fn layered_rule_loses_to_unlayered_author_rule() {
    // `@layer framework { p { color: red } }` vs unlayered
    // `p { color: blue }`. Spec: unlayered author rules beat layered
    // author rules (for normal declarations). Blue should win.
    let sheet = Stylesheet::parse(
        "@layer framework { p { color: red; } } \
         p { color: blue; }",
    );
    let doc = make_doc(vec![(TagName::P, vec![])]);
    let styles = style_tree(&doc, &[&sheet], &[], &ctx());
    let style = styles[3].as_ref().expect("p has style");
    assert_eq!(style.color, Color::rgb(0, 0, 255));
}

#[test]
fn later_layer_wins_over_earlier_layer() {
    // Two layers declared via a statement to freeze their order:
    // `reset` first, then `overrides`. Both set p's color; the
    // `overrides` (later) layer should win.
    let sheet = Stylesheet::parse(
        "@layer reset, overrides; \
         @layer overrides { p { color: blue; } } \
         @layer reset { p { color: red; } }",
    );
    let doc = make_doc(vec![(TagName::P, vec![])]);
    let styles = style_tree(&doc, &[&sheet], &[], &ctx());
    let style = styles[3].as_ref().expect("p has style");
    // Blue (overrides) wins despite being declared earlier in source
    // than the `reset` block — layer order comes from the statement.
    assert_eq!(style.color, Color::rgb(0, 0, 255));
}

#[test]
fn layered_important_beats_unlayered_important() {
    // `!important` inverts the layer priority: layered `!important`
    // should beat unlayered `!important`.
    let sheet = Stylesheet::parse(
        "@layer framework { p { color: red !important; } } \
         p { color: blue !important; }",
    );
    let doc = make_doc(vec![(TagName::P, vec![])]);
    let styles = style_tree(&doc, &[&sheet], &[], &ctx());
    let style = styles[3].as_ref().expect("p has style");
    assert_eq!(style.color, Color::rgb(255, 0, 0));
}

#[test]
fn earlier_layer_important_beats_later_layer_important() {
    // With `!important`, earlier-declared layers win.
    let sheet = Stylesheet::parse(
        "@layer reset, overrides; \
         @layer reset { p { color: red !important; } } \
         @layer overrides { p { color: blue !important; } }",
    );
    let doc = make_doc(vec![(TagName::P, vec![])]);
    let styles = style_tree(&doc, &[&sheet], &[], &ctx());
    let style = styles[3].as_ref().expect("p has style");
    assert_eq!(style.color, Color::rgb(255, 0, 0));
}

#[test]
fn layer_does_not_override_specificity_within_layer() {
    // Inside the same layer, specificity still decides.
    // `.foo` (specificity 10) should beat `p` (specificity 1).
    let sheet = Stylesheet::parse(
        "@layer framework { \
           p { color: red; } \
           .foo { color: blue; } \
         }",
    );
    let doc = make_doc(vec![(
        TagName::P,
        vec![Attribute {
            name: "class".to_string(),
            value: "foo".to_string(),
        }],
    )]);
    let styles = style_tree(&doc, &[&sheet], &[], &ctx());
    let style = styles[3].as_ref().expect("p has style");
    assert_eq!(style.color, Color::rgb(0, 0, 255));
}

// -- cross-stylesheet @layer merging --------------------------------

#[test]
fn cross_stylesheet_same_layer_name_shares_ordering() {
    // Sheet 1 declares `@layer reset { p { color: red } }`.
    // Sheet 2 declares `@layer reset { p { color: blue } }`.
    // Both refer to the same global "reset" layer. Sheet 2 is later in
    // source order, so blue should win (same layer, source order breaks tie).
    let sheet1 = Stylesheet::parse("@layer reset { p { color: red; } }");
    let sheet2 = Stylesheet::parse("@layer reset { p { color: blue; } }");
    let doc = make_doc(vec![(TagName::P, vec![])]);
    let styles = style_tree(&doc, &[&sheet1, &sheet2], &[], &ctx());
    let style = styles[3].as_ref().expect("p has style");
    assert_eq!(style.color, Color::rgb(0, 0, 255));
}

#[test]
fn cross_stylesheet_layer_order_preserved() {
    // Sheet 1 declares `@layer reset, theme;`.
    // Sheet 2 puts a rule in `@layer reset`.
    // Sheet 1 also has a rule in `@layer theme`.
    // `theme` is later than `reset`, so theme wins (normal declarations).
    let sheet1 = Stylesheet::parse(
        "@layer reset, theme; \
         @layer theme { p { color: green; } }",
    );
    let sheet2 = Stylesheet::parse("@layer reset { p { color: red; } }");
    let doc = make_doc(vec![(TagName::P, vec![])]);
    let styles = style_tree(&doc, &[&sheet1, &sheet2], &[], &ctx());
    let style = styles[3].as_ref().expect("p has style");
    // `theme` (layer 1) beats `reset` (layer 0) for normal declarations.
    assert_eq!(style.color, Color::rgb(0, 128, 0));
}

// -- text-wrap parsing ---------------------------------------------

#[test]
fn text_wrap_balance_parses_and_applies() {
    use super::super::values::TextWrap;
    let sheet = Stylesheet::parse("h1 { text-wrap: balance; }");
    let doc = make_doc(vec![(TagName::H1, vec![])]);
    let styles = style_tree(&doc, &[&sheet], &[], &ctx());
    let style = styles[3].as_ref().expect("h1 has style");
    assert_eq!(style.text_wrap, TextWrap::Balance);
}

#[test]
fn text_wrap_pretty_parses_and_applies() {
    use super::super::values::TextWrap;
    let sheet = Stylesheet::parse("p { text-wrap: pretty; }");
    let doc = make_doc(vec![(TagName::P, vec![])]);
    let styles = style_tree(&doc, &[&sheet], &[], &ctx());
    let style = styles[3].as_ref().expect("p has style");
    assert_eq!(style.text_wrap, TextWrap::Pretty);
}

#[test]
fn text_wrap_defaults_to_wrap() {
    use super::super::values::TextWrap;
    let doc = make_doc(vec![(TagName::P, vec![])]);
    let sheet = Stylesheet::parse("");
    let styles = style_tree(&doc, &[&sheet], &[], &ctx());
    let style = styles[3].as_ref().expect("p has style");
    assert_eq!(style.text_wrap, TextWrap::Wrap);
}

#[test]
fn has_descendant_matches_article_with_img() {
    let doc = make_has_doc();
    let sel = parse_selector("article:has(img)");
    // <article> (node 3) has a nested <img> → match.
    assert!(matching::matches_selector(&doc, 3, &sel, &ctx()));
    // <section> (node 7) has no <img> → no match.
    let sel2 = parse_selector("section:has(img)");
    assert!(!matching::matches_selector(&doc, 7, &sel2, &ctx()));
}

#[test]
fn has_direct_child_distinguishes_depth() {
    let doc = make_has_doc();
    // :has(> img) requires an *immediate* <img> child.
    let sel = parse_selector("article:has(> img)");
    // <article>'s direct children are just <div>, not <img>.
    assert!(!matching::matches_selector(&doc, 3, &sel, &ctx()));
    // But <div> (node 4) does have <img> as a direct child.
    let sel2 = parse_selector("div:has(> img)");
    assert!(matching::matches_selector(&doc, 4, &sel2, &ctx()));
}

#[test]
fn has_next_sibling_matches() {
    let doc = make_has_doc();
    // <article>'s next element sibling is <p>.
    let sel = parse_selector("article:has(+ p)");
    assert!(matching::matches_selector(&doc, 3, &sel, &ctx()));
    // No + img sibling.
    let sel2 = parse_selector("article:has(+ img)");
    assert!(!matching::matches_selector(&doc, 3, &sel2, &ctx()));
}

#[test]
fn has_general_sibling_matches() {
    let doc = make_has_doc();
    // <article> ~ <section> — section follows later.
    let sel = parse_selector("article:has(~ section)");
    assert!(matching::matches_selector(&doc, 3, &sel, &ctx()));
}

#[test]
fn has_selector_list_any_matches() {
    let doc = make_has_doc();
    // Comma list: match if any relative selector matches.
    // <article> has no direct <img> child, but does have a nested <img>.
    let sel = parse_selector("article:has(> img, img)");
    assert!(matching::matches_selector(&doc, 3, &sel, &ctx()));
}

#[test]
fn has_inner_combinator_is_scope_bounded() {
    // Build a document where the outer <body> has class "marker", and
    // the subject <article> contains a <span>. Under a naive
    // (non-scoped) matcher, `article:has(.marker span)` would match via
    // body.marker (an ancestor of the span that is *outside* the
    // article's subtree). With scope bounding, ancestor walks for the
    // inner `.marker span` stop at `<article>` and the match fails.
    //
    // Layout:
    //   0 Document
    //   1 <html>
    //   2 <body class="marker">
    //   3   <article>
    //   4     <span>
    let nodes = vec![
        Node {
            kind: NodeKind::Document,
            parent: None,
            children: vec![1],
        },
        Node {
            kind: NodeKind::Element(ElementData {
                tag: TagName::Html,
                attributes: vec![],
            }),
            parent: Some(0),
            children: vec![2],
        },
        Node {
            kind: NodeKind::Element(ElementData {
                tag: TagName::Body,
                attributes: vec![Attribute {
                    name: "class".to_string(),
                    value: "marker".to_string(),
                }],
            }),
            parent: Some(1),
            children: vec![3],
        },
        Node {
            kind: NodeKind::Element(ElementData {
                tag: TagName::Article,
                attributes: vec![],
            }),
            parent: Some(2),
            children: vec![4],
        },
        Node {
            kind: NodeKind::Element(ElementData {
                tag: TagName::Span,
                attributes: vec![],
            }),
            parent: Some(3),
            children: vec![],
        },
    ];
    let doc = Document::from_nodes(nodes, 0);
    let sel = parse_selector("article:has(.marker span)");
    assert!(
        !matching::matches_selector(&doc, 3, &sel, &ctx()),
        "`.marker` lives outside the article's subtree and must not match"
    );
    // Sanity: if the marker is *inside* the article, it should match.
    let nodes2 = vec![
        Node {
            kind: NodeKind::Document,
            parent: None,
            children: vec![1],
        },
        Node {
            kind: NodeKind::Element(ElementData {
                tag: TagName::Html,
                attributes: vec![],
            }),
            parent: Some(0),
            children: vec![2],
        },
        Node {
            kind: NodeKind::Element(ElementData {
                tag: TagName::Body,
                attributes: vec![],
            }),
            parent: Some(1),
            children: vec![3],
        },
        Node {
            kind: NodeKind::Element(ElementData {
                tag: TagName::Article,
                attributes: vec![],
            }),
            parent: Some(2),
            children: vec![4],
        },
        Node {
            kind: NodeKind::Element(ElementData {
                tag: TagName::Div,
                attributes: vec![Attribute {
                    name: "class".to_string(),
                    value: "marker".to_string(),
                }],
            }),
            parent: Some(3),
            children: vec![5],
        },
        Node {
            kind: NodeKind::Element(ElementData {
                tag: TagName::Span,
                attributes: vec![],
            }),
            parent: Some(4),
            children: vec![],
        },
    ];
    let doc2 = Document::from_nodes(nodes2, 0);
    assert!(matching::matches_selector(&doc2, 3, &sel, &ctx()));
}

#[test]
fn has_child_multi_compound_anchors_on_first_compound() {
    // `:has(> div img)` means "has a direct child `div` containing an
    // `img`". The inner selector is multi-compound; the first compound
    // must match a direct child of the subject, not an arbitrary
    // descendant.
    let doc = make_has_doc();
    // article (3) → div (4) → img (5).
    // Direct child div contains img → match.
    let sel = parse_selector("article:has(> div img)");
    assert!(matching::matches_selector(&doc, 3, &sel, &ctx()));
    // article has no direct child span → must not match.
    let sel2 = parse_selector("article:has(> span img)");
    assert!(
        !matching::matches_selector(&doc, 3, &sel2, &ctx()),
        "first compound must match a direct child, not a deeper descendant"
    );
    // section (7) → span (8), no div child → must not match.
    assert!(!matching::matches_selector(&doc, 7, &sel, &ctx()));
}

#[test]
fn has_non_element_node_is_ignored() {
    // :has() against a non-element should be false (matches_simple
    // already guards on ElementData).
    let doc = make_has_doc();
    let sel = parse_selector("*:has(img)");
    // Node 0 is the Document, not an element.
    assert!(!matching::matches_selector(&doc, 0, &sel, &ctx()));
}

// -- Coverage gaps for the selector matcher --------------------------

/// Build a doc shaped like `<html><body><div><p/></div></body></html>`.
/// Node 3 is the div (direct body child); node 4 is the p (grandchild of
/// body). Useful for distinguishing child vs descendant combinators.
fn make_nested_doc() -> Document {
    let nodes = vec![
        Node {
            kind: NodeKind::Document,
            parent: None,
            children: vec![1],
        },
        Node {
            kind: NodeKind::Element(ElementData {
                tag: TagName::Html,
                attributes: vec![],
            }),
            parent: Some(0),
            children: vec![2],
        },
        Node {
            kind: NodeKind::Element(ElementData {
                tag: TagName::Body,
                attributes: vec![],
            }),
            parent: Some(1),
            children: vec![3],
        },
        Node {
            kind: NodeKind::Element(ElementData {
                tag: TagName::Div,
                attributes: vec![],
            }),
            parent: Some(2),
            children: vec![4],
        },
        Node {
            kind: NodeKind::Element(ElementData {
                tag: TagName::P,
                attributes: vec![],
            }),
            parent: Some(3),
            children: vec![],
        },
    ];
    Document::from_nodes(nodes, 0)
}

#[test]
fn child_combinator_matches_direct_child_only() {
    let doc = make_nested_doc();
    // `body > div` — div (node 3) is a direct child of body.
    let sel = parse_selector("body > div");
    assert!(matching::matches_selector(&doc, 3, &sel, &ctx()));
    // `body > p` — p (node 4) is a grandchild of body, not a child.
    let sel = parse_selector("body > p");
    assert!(!matching::matches_selector(&doc, 4, &sel, &ctx()));
    // Sanity: the descendant form does match the grandchild.
    let descendant = parse_selector("body p");
    assert!(matching::matches_selector(&doc, 4, &descendant, &ctx()));
}

#[test]
fn child_combinator_chain_requires_each_step_direct() {
    let doc = make_nested_doc();
    // `body > div > p` — every step a direct child; matches node 4.
    let sel = parse_selector("body > div > p");
    assert!(matching::matches_selector(&doc, 4, &sel, &ctx()));
    // `html > p` — p is a great-grandchild of html, not a child.
    let sel = parse_selector("html > p");
    assert!(!matching::matches_selector(&doc, 4, &sel, &ctx()));
}

#[test]
fn attribute_suffix_selector() {
    let doc = make_doc(vec![
        (
            TagName::A,
            vec![Attribute {
                name: "href".to_string(),
                value: "/docs/manual.pdf".to_string(),
            }],
        ),
        (
            TagName::A,
            vec![Attribute {
                name: "href".to_string(),
                value: "/docs/index.html".to_string(),
            }],
        ),
    ]);
    // Build via the parser so this also exercises `AttrOp::Suffix` parsing
    // (the matcher and parser would otherwise be tested independently).
    let sel = parse_selector("[href$=\".pdf\"]");
    assert!(matching::matches_selector(&doc, 3, &sel, &ctx()));
    assert!(!matching::matches_selector(&doc, 4, &sel, &ctx()));
}

#[test]
fn attribute_dash_match_selector() {
    // `[lang|=en]` matches "en" exactly OR "en-*" (hyphen prefix).
    let doc = make_doc(vec![
        (
            TagName::Span,
            vec![Attribute {
                name: "lang".to_string(),
                value: "en".to_string(),
            }],
        ),
        (
            TagName::Span,
            vec![Attribute {
                name: "lang".to_string(),
                value: "en-US".to_string(),
            }],
        ),
        (
            TagName::Span,
            vec![Attribute {
                name: "lang".to_string(),
                value: "english".to_string(),
            }],
        ),
        (
            TagName::Span,
            vec![Attribute {
                name: "lang".to_string(),
                value: "fr".to_string(),
            }],
        ),
    ]);
    // Build via the parser so this also exercises `AttrOp::DashMatch` parsing.
    let sel = parse_selector("[lang|=\"en\"]");
    assert!(
        matching::matches_selector(&doc, 3, &sel, &ctx()),
        "exact en"
    );
    assert!(matching::matches_selector(&doc, 4, &sel, &ctx()), "en-US");
    assert!(
        !matching::matches_selector(&doc, 5, &sel, &ctx()),
        "english is not en-prefixed by hyphen rule"
    );
    assert!(!matching::matches_selector(&doc, 6, &sel, &ctx()), "fr");
}

#[test]
fn universal_selector_matches_every_element() {
    let doc = make_doc(vec![(TagName::P, vec![]), (TagName::Div, vec![])]);
    // Route through parse_selector to also exercise the `*` parser path.
    let sel = parse_selector("*");
    // Every Element node matches; the Document root (node 0) does not
    // because matches_simple requires ElementData.
    assert!(matching::matches_selector(&doc, 1, &sel, &ctx()), "html");
    assert!(matching::matches_selector(&doc, 2, &sel, &ctx()), "body");
    assert!(matching::matches_selector(&doc, 3, &sel, &ctx()), "p");
    assert!(matching::matches_selector(&doc, 4, &sel, &ctx()), "div");
    assert!(
        !matching::matches_selector(&doc, 0, &sel, &ctx()),
        "Document node is not an element"
    );
}

#[test]
fn is_selector_matches_any_inner() {
    // `:is(div, p)` matches an element that is either <div> or <p>.
    let doc = make_doc(vec![
        (TagName::Span, vec![]),
        (TagName::P, vec![]),
        (TagName::Div, vec![]),
    ]);
    let sheet = Stylesheet::parse("*:is(div, p) { color: red; }");
    let sel = sheet.rules[0].selectors.selectors[0].clone();
    assert!(
        !matching::matches_selector(&doc, 3, &sel, &ctx()),
        "<span> is excluded"
    );
    assert!(matching::matches_selector(&doc, 4, &sel, &ctx()), "<p>");
    assert!(matching::matches_selector(&doc, 5, &sel, &ctx()), "<div>");
}

#[test]
fn is_selector_specificity_takes_max() {
    // `:is(p, #header)` should count one ID (the max of the two inner
    // compounds), mirroring `:not()`'s spec rule.
    let sheet = Stylesheet::parse("div:is(p, #header) { color: red; }");
    let spec = sheet.rules[0].selectors.selectors[0].specificity();
    // div = 1 type; #header = 1 id.
    assert_eq!(spec.ids, 1);
    assert_eq!(spec.classes, 0);
    assert_eq!(spec.types, 1);
}

#[test]
fn where_selector_matches_any_inner() {
    // `:where(div, p)` matches identically to `:is(div, p)` — only the
    // specificity contribution differs.
    let doc = make_doc(vec![
        (TagName::Span, vec![]),
        (TagName::P, vec![]),
        (TagName::Div, vec![]),
    ]);
    let sheet = Stylesheet::parse("*:where(div, p) { color: red; }");
    let sel = sheet.rules[0].selectors.selectors[0].clone();
    assert!(!matching::matches_selector(&doc, 3, &sel, &ctx()));
    assert!(matching::matches_selector(&doc, 4, &sel, &ctx()));
    assert!(matching::matches_selector(&doc, 5, &sel, &ctx()));
}

#[test]
fn where_selector_contributes_zero_specificity() {
    // `:where(#a, .b, p)` adds nothing to the host selector's
    // specificity, regardless of how specific its arguments are.
    let sheet = Stylesheet::parse("div:where(#a, .b, p) { color: red; }");
    let spec = sheet.rules[0].selectors.selectors[0].specificity();
    assert_eq!(spec.ids, 0);
    assert_eq!(spec.classes, 0);
    assert_eq!(spec.types, 1, "only the leading `div` counts");
}

#[test]
fn where_loses_to_plain_class_in_cascade() {
    // End-to-end: a `:where()`-wrapped #id selector adds zero specificity,
    // so a plain `.foo` rule (specificity 0,0,1,0) should beat
    // `p:where(#a)` (0,0,0,1 because :where contributes 0 and `p`
    // contributes 1 type — total 0,0,0,1).
    //
    // The element carries BOTH `id="a"` and `class="foo"` so that *both*
    // rules actually match — otherwise the test would only prove that a
    // non-matching selector loses, not that `:where()` demotes specificity.
    let css = ".foo { color: blue; } p:where(#a) { color: red; }";
    let sheet = Stylesheet::parse(css);
    let doc = make_doc(vec![(
        TagName::P,
        vec![
            Attribute {
                name: "id".to_string(),
                value: "a".to_string(),
            },
            Attribute {
                name: "class".to_string(),
                value: "foo".to_string(),
            },
        ],
    )]);
    let styles = style_tree(&doc, &[&sheet], &[], &ctx());
    assert_eq!(
        styles[3].as_ref().unwrap().color,
        Color::rgb(0, 0, 255),
        ".foo (1 class) outranks p:where(#a) (1 type) → blue wins"
    );
}

/// Build `<html><body><article><section><p/></section></article></body></html>`
/// for testing `@scope (root) to (limit)` boundaries.
fn make_scope_doc() -> Document {
    let nodes = vec![
        Node {
            kind: NodeKind::Document,
            parent: None,
            children: vec![1],
        },
        Node {
            kind: NodeKind::Element(ElementData {
                tag: TagName::Html,
                attributes: vec![],
            }),
            parent: Some(0),
            children: vec![2],
        },
        Node {
            kind: NodeKind::Element(ElementData {
                tag: TagName::Body,
                attributes: vec![],
            }),
            parent: Some(1),
            children: vec![3],
        },
        Node {
            kind: NodeKind::Element(ElementData {
                tag: TagName::Article,
                attributes: vec![],
            }),
            parent: Some(2),
            children: vec![4],
        },
        Node {
            kind: NodeKind::Element(ElementData {
                tag: TagName::Section,
                attributes: vec![],
            }),
            parent: Some(3),
            children: vec![5],
        },
        Node {
            kind: NodeKind::Element(ElementData {
                tag: TagName::P,
                attributes: vec![],
            }),
            parent: Some(4),
            children: vec![],
        },
    ];
    Document::from_nodes(nodes, 0)
}

#[test]
fn scope_to_limit_excludes_subtree_under_limit() {
    // `@scope (article) to (section)` — the p sits *inside* a section,
    // so it must fall outside scope and the rule should not apply.
    let css = "p { color: black; } @scope (article) to (section) { p { color: red; } }";
    let sheet = Stylesheet::parse(css);
    let doc = make_scope_doc();
    let styles = style_tree(&doc, &[&sheet], &[], &ctx());
    assert_eq!(
        styles[5].as_ref().unwrap().color,
        Color::BLACK,
        "p inside the limit subtree must not pick up the scoped color"
    );
}

#[test]
fn scope_to_limit_applies_when_limit_does_not_match() {
    // `@scope (article) to (aside)` — there's no <aside> ancestor, so the
    // limit never trips and the rule should apply.
    let css = "p { color: black; } @scope (article) to (aside) { p { color: red; } }";
    let sheet = Stylesheet::parse(css);
    let doc = make_scope_doc();
    let styles = style_tree(&doc, &[&sheet], &[], &ctx());
    assert_eq!(
        styles[5].as_ref().unwrap().color,
        Color::rgb(255, 0, 0),
        "no limit ancestor → rule applies"
    );
}

mod prop {
    use proptest::prelude::*;

    proptest! {
        /// "odd" parses to AnB(2,1) and "even" parses to AnB(2,0).
        #[test]
        fn an_plus_b_odd_even(
            input in proptest::sample::select(vec![
                "odd".to_string(), "ODD".to_string(), "Odd".to_string(),
                "even".to_string(), "EVEN".to_string(), "Even".to_string(),
            ]),
        ) {
            use super::super::super::selectors::AnB;
            let anb = AnB::parse(&input).unwrap();
            let lower = input.to_ascii_lowercase();
            if lower == "odd" {
                prop_assert_eq!((anb.a, anb.b), (2, 1));
            } else {
                prop_assert_eq!((anb.a, anb.b), (2, 0));
            }
        }

        /// A plain positive integer parses as AnB(0, n).
        #[test]
        fn an_plus_b_plain_number(n in 1i32..100) {
            use super::super::super::selectors::AnB;
            let anb = AnB::parse(&n.to_string()).unwrap();
            prop_assert_eq!(anb.a, 0);
            prop_assert_eq!(anb.b, n);
        }

        /// "An" form parses as AnB(A, 0).
        #[test]
        fn an_plus_b_an_form(coeff in 1i32..20) {
            use super::super::super::selectors::AnB;
            let input = format!("{coeff}n");
            let anb = AnB::parse(&input).unwrap();
            prop_assert_eq!(anb.a, coeff);
            prop_assert_eq!(anb.b, 0);
        }

        /// "An+B" form parses correctly.
        #[test]
        fn an_plus_b_full_form(
            coeff in 1i32..20,
            offset in 0i32..20,
        ) {
            use super::super::super::selectors::AnB;
            let input = format!("{coeff}n+{offset}");
            let anb = AnB::parse(&input).unwrap();
            prop_assert_eq!(anb.a, coeff);
            prop_assert_eq!(anb.b, offset);
        }

        /// AnB::matches: if a==0, only index==b matches.
        #[test]
        fn anb_matches_a_zero(b in 1i32..50, index in 1i32..50) {
            use super::super::super::selectors::AnB;
            let result = AnB { a: 0, b }.matches(index);
            prop_assert_eq!(result, index == b);
        }

        /// AnB::matches: index == a*1 + b always matches.
        #[test]
        fn anb_matches_first_match(a in 1i32..20, b in 0i32..10) {
            use super::super::super::selectors::AnB;
            let index = a + b;
            if index > 0 {
                prop_assert!(
                    AnB { a, b }.matches(index),
                    "{a}n+{b} should match index {index}",
                );
            }
        }

        /// AnB::parse never panics on arbitrary ASCII.
        #[test]
        fn anb_parse_never_panics(input in "[ -~]{0,30}") {
            use super::super::super::selectors::AnB;
            let _ = AnB::parse(&input);
        }
    }
}

#[test]
fn pseudo_element_inherits_color_from_rule() {
    let sheet = Stylesheet::parse(r#"p::before { content: "> "; color: red; }"#);
    let doc = make_doc(vec![(TagName::P, vec![])]);
    let ctx = ctx();
    let p_id = 3;
    let parent_style = ComputedStyle::default();
    let ps = matching::resolve_pseudo_style(&doc, p_id, "before", &parent_style, &[&sheet], &ctx)
        .expect("should produce pseudo style");
    assert_eq!(ps.content, Some("> ".to_string()));
    assert_eq!(ps.color, Color::rgb(255, 0, 0));
}

#[test]
fn pseudo_element_inherits_from_parent_when_not_set() {
    let sheet =
        Stylesheet::parse(r#"p { color: blue; } p::before { content: "*"; font-weight: bold; }"#);
    let doc = make_doc(vec![(TagName::P, vec![])]);
    let ctx = ctx();
    let p_id = 3;
    let index = SelectorIndex::build(&[&sheet]);
    let inline_map = rustc_hash::FxHashMap::default();
    let mut tag_cache = rustc_hash::FxHashMap::default();
    let parent_style = compute_style(
        &doc,
        p_id,
        None,
        &[&sheet],
        &index,
        &inline_map,
        &ctx,
        &mut tag_cache,
    );
    let ps = matching::resolve_pseudo_style(&doc, p_id, "before", &parent_style, &[&sheet], &ctx)
        .expect("should produce pseudo style");
    assert_eq!(ps.color, Color::rgb(0, 0, 255), "should inherit blue");
    assert_eq!(ps.font_weight, FontWeight::BOLD);
}

#[test]
fn pseudo_element_after_content() {
    let sheet = Stylesheet::parse(r#"p::after { content: "!"; color: green; }"#);
    let doc = make_doc(vec![(TagName::P, vec![])]);
    let ctx = ctx();
    let p_id = 3;
    let parent_style = ComputedStyle::default();
    let ps = matching::resolve_pseudo_style(&doc, p_id, "after", &parent_style, &[&sheet], &ctx)
        .expect("should produce ::after style");
    assert_eq!(ps.content, Some("!".to_string()));
    assert_eq!(ps.color, Color::rgb(0, 128, 0));
}

#[test]
fn pseudo_element_empty_content_clearfix() {
    let sheet = Stylesheet::parse(r#"p::after { content: ""; display: block; }"#);
    let doc = make_doc(vec![(TagName::P, vec![])]);
    let ctx = ctx();
    let p_id = 3;
    let parent_style = ComputedStyle::default();
    let ps = matching::resolve_pseudo_style(&doc, p_id, "after", &parent_style, &[&sheet], &ctx)
        .expect("empty content should still produce a pseudo style");
    assert_eq!(ps.content, Some(String::new()));
    assert_eq!(ps.display, Display::Block);
}

#[test]
fn pseudo_element_content_none_no_generation() {
    let sheet = Stylesheet::parse(r#"p::before { content: none; color: red; }"#);
    let doc = make_doc(vec![(TagName::P, vec![])]);
    let ctx = ctx();
    let p_id = 3;
    let parent_style = ComputedStyle::default();
    let ps = matching::resolve_pseudo_style(&doc, p_id, "before", &parent_style, &[&sheet], &ctx);
    assert!(
        ps.is_none(),
        "content:none should not generate pseudo-element"
    );
}

#[test]
fn pseudo_element_no_matching_rule() {
    let sheet = Stylesheet::parse("p { color: red; }");
    let doc = make_doc(vec![(TagName::P, vec![])]);
    let ctx = ctx();
    let p_id = 3;
    let parent_style = ComputedStyle::default();
    let ps = matching::resolve_pseudo_style(&doc, p_id, "before", &parent_style, &[&sheet], &ctx);
    assert!(ps.is_none(), "no matching rule should return None");
}

// ---------------------------------------------------------------
// Property-based tests (proptest)
// ---------------------------------------------------------------

// -- real-world CSS compliance tests ----------------------------------

#[test]
fn universal_with_descendant_combinator() {
    // `* p { color: red }` -- universal selector as ancestor.
    let mut doc = make_doc(vec![(TagName::Div, vec![])]);
    let p_id = doc.nodes.len();
    doc.nodes.push(Node {
        kind: NodeKind::Element(ElementData {
            tag: TagName::P,
            attributes: vec![],
        }),
        parent: Some(3),
        children: vec![],
    });
    doc.nodes[3].children.push(p_id);

    let sel = Selector {
        parts: vec![
            (
                CompoundSelector {
                    parts: vec![SimpleSelector::Universal],
                },
                None,
            ),
            (
                CompoundSelector {
                    parts: vec![SimpleSelector::Type("p".to_string())],
                },
                Some(Combinator::Descendant),
            ),
        ],
    };
    // <p> inside <div> (which matches *) should match `* p`.
    assert!(matching::matches_selector(&doc, p_id, &sel, &ctx()));
}

#[test]
fn multiple_class_selector_compound() {
    // `.a.b` should only match elements with both classes.
    let doc = make_doc(vec![
        (
            TagName::Div,
            vec![Attribute {
                name: "class".to_string(),
                value: "a b".to_string(),
            }],
        ),
        (
            TagName::Div,
            vec![Attribute {
                name: "class".to_string(),
                value: "a".to_string(),
            }],
        ),
    ]);
    let sel = Selector {
        parts: vec![(
            CompoundSelector {
                parts: vec![
                    SimpleSelector::Class("a".to_string()),
                    SimpleSelector::Class("b".to_string()),
                ],
            },
            None,
        )],
    };
    assert!(
        matching::matches_selector(&doc, 3, &sel, &ctx()),
        "element with classes 'a b' should match .a.b"
    );
    assert!(
        !matching::matches_selector(&doc, 4, &sel, &ctx()),
        "element with only class 'a' should not match .a.b"
    );
}

#[test]
fn pseudo_class_with_type_selector() {
    // `a:hover` -- compound type + pseudo-class.
    let doc = make_doc(vec![
        (
            TagName::A,
            vec![Attribute {
                name: "href".to_string(),
                value: "/page".to_string(),
            }],
        ),
        (TagName::P, vec![]),
    ]);
    let sheet = Stylesheet::parse("a:hover { color: red; }");
    let hctx = CascadeContext {
        hover_node: Some(3),
        visited_urls: None,
        focused_node: None,
        containers: None,
        global_layers: None,
    };
    let styles = style_tree(&doc, &[&sheet], &[], &hctx);
    let a_style = styles[3].as_ref().expect("a should have style");
    assert_eq!(a_style.color, Color::rgb(255, 0, 0));

    // <p> (node 4) should NOT get the hover style even if hovered,
    // because the selector requires `a`, not `p`.
    let hctx_p = CascadeContext {
        hover_node: Some(4),
        visited_urls: None,
        focused_node: None,
        containers: None,
        global_layers: None,
    };
    let styles2 = style_tree(&doc, &[&sheet], &[], &hctx_p);
    let p_style = styles2[4].as_ref().unwrap();
    assert_ne!(
        p_style.color,
        Color::rgb(255, 0, 0),
        "p:hover should not match a:hover rule"
    );
}

#[test]
fn specificity_id_vs_many_classes() {
    // #id should beat .a.b.c.d.e.f.g.h.i.j.k (11 classes).
    // CSS specificity: #id = (0,1,0,0), 11 classes = (0,0,11,0).
    // ID always wins per spec.
    let mut attrs = vec![Attribute {
        name: "id".to_string(),
        value: "x".to_string(),
    }];
    let classes = "a b c d e f g h i j k";
    attrs.push(Attribute {
        name: "class".to_string(),
        value: classes.to_string(),
    });
    let doc = make_doc(vec![(TagName::Div, attrs)]);

    let rule_classes = make_rule(
        vec![Selector {
            parts: vec![(
                CompoundSelector {
                    parts: vec![
                        SimpleSelector::Class("a".into()),
                        SimpleSelector::Class("b".into()),
                        SimpleSelector::Class("c".into()),
                        SimpleSelector::Class("d".into()),
                        SimpleSelector::Class("e".into()),
                        SimpleSelector::Class("f".into()),
                        SimpleSelector::Class("g".into()),
                        SimpleSelector::Class("h".into()),
                        SimpleSelector::Class("i".into()),
                        SimpleSelector::Class("j".into()),
                        SimpleSelector::Class("k".into()),
                    ],
                },
                None,
            )],
        }],
        vec![decl("color", CssValue::Keyword("red".to_string()), false)],
    );
    let rule_id = make_rule(
        vec![simple_id_selector("x")],
        vec![decl("color", CssValue::Keyword("blue".to_string()), false)],
    );

    let sheet = Stylesheet {
        rules: vec![rule_classes, rule_id],
        keyframes: vec![],
        layers: vec![],
        counter_styles: vec![],
        properties: vec![],
        font_faces: vec![],
    };
    let styles = style_tree(&doc, &[&sheet], &[], &ctx());
    let style = styles[3].as_ref().expect("div should have style");
    assert_eq!(
        style.color,
        Color::rgb(0, 0, 255),
        "#id should beat 11 classes"
    );
}

#[test]
fn important_on_inherited_vs_direct() {
    // Parent has `color: red !important`.
    // Child has direct `color: blue` (not important).
    // Direct declaration on the child should win over inherited
    // !important, because !important only affects the same element.
    let mut doc = make_doc(vec![(TagName::Div, vec![])]);
    let p_id = doc.nodes.len();
    doc.nodes.push(Node {
        kind: NodeKind::Element(ElementData {
            tag: TagName::P,
            attributes: vec![],
        }),
        parent: Some(3),
        children: vec![],
    });
    doc.nodes[3].children.push(p_id);

    let sheet = Stylesheet {
        rules: vec![
            make_rule(
                vec![simple_type_selector("div")],
                vec![decl("color", CssValue::Keyword("red".to_string()), true)],
            ),
            make_rule(
                vec![simple_type_selector("p")],
                vec![decl("color", CssValue::Keyword("blue".to_string()), false)],
            ),
        ],
        keyframes: vec![],
        layers: vec![],
        counter_styles: vec![],
        properties: vec![],
        font_faces: vec![],
    };
    let styles = style_tree(&doc, &[&sheet], &[], &ctx());
    let p_style = styles[p_id].as_ref().expect("p should have style");
    assert_eq!(
        p_style.color,
        Color::rgb(0, 0, 255),
        "direct declaration on child should beat inherited !important"
    );
}

#[test]
fn nth_child_negative_n_plus_3() {
    // :nth-child(-n+3) matches positions 1, 2, 3 only.
    use super::super::selectors::AnB;
    let anb = AnB::parse("-n+3").expect("-n+3 should parse");
    assert!(anb.matches(1), "-n+3 should match 1");
    assert!(anb.matches(2), "-n+3 should match 2");
    assert!(anb.matches(3), "-n+3 should match 3");
    assert!(!anb.matches(4), "-n+3 should not match 4");
    assert!(!anb.matches(5), "-n+3 should not match 5");
}

#[test]
fn compound_type_and_class_selector() {
    // `p.highlight` -- type + class compound.
    let doc = make_doc(vec![
        (
            TagName::P,
            vec![Attribute {
                name: "class".to_string(),
                value: "highlight".to_string(),
            }],
        ),
        (TagName::P, vec![]),
        (
            TagName::Div,
            vec![Attribute {
                name: "class".to_string(),
                value: "highlight".to_string(),
            }],
        ),
    ]);
    let sel = Selector {
        parts: vec![(
            CompoundSelector {
                parts: vec![
                    SimpleSelector::Type("p".to_string()),
                    SimpleSelector::Class("highlight".to_string()),
                ],
            },
            None,
        )],
    };
    assert!(
        matching::matches_selector(&doc, 3, &sel, &ctx()),
        "p.highlight should match <p class=highlight>"
    );
    assert!(
        !matching::matches_selector(&doc, 4, &sel, &ctx()),
        "p.highlight should not match <p> without class"
    );
    assert!(
        !matching::matches_selector(&doc, 5, &sel, &ctx()),
        "p.highlight should not match <div class=highlight>"
    );
}

mod prop_tests {
    use proptest::prelude::*;

    use super::super::super::parser::{
        CompoundSelector, CssValue, Declaration, PropertyId, Rule, Selector, SelectorList,
        SimpleSelector, Specificity, Stylesheet,
    };
    use super::style_tree;
    use crate::html::dom::{Attribute, TagName};

    /// Strategy for Specificity values with bounded components.
    fn arb_specificity() -> impl Strategy<Value = Specificity> {
        (0u8..=1, 0u8..=10, 0u8..=10, 0u8..=10).prop_map(|(inline, ids, classes, types)| {
            Specificity {
                inline,
                ids,
                classes,
                types,
            }
        })
    }

    /// Build a simple selector from counts of id/class/type parts.
    fn selector_with_counts(n_ids: u8, n_classes: u8, n_types: u8) -> Selector {
        let mut parts = Vec::new();
        for i in 0..n_ids {
            parts.push(SimpleSelector::Id(format!("id{i}")));
        }
        for i in 0..n_classes {
            parts.push(SimpleSelector::Class(format!("cls{i}")));
        }
        for i in 0..n_types {
            parts.push(SimpleSelector::Type(format!("t{i}")));
        }
        if parts.is_empty() {
            parts.push(SimpleSelector::Universal);
        }
        Selector {
            parts: vec![(CompoundSelector { parts }, None)],
        }
    }

    // -- Specificity struct ordering -----------------------------------

    proptest! {
        /// Specificity comparison is reflexive.
        #[test]
        fn specificity_reflexive(
            inline in 0u8..=1,
            ids in 0u8..=10,
            classes in 0u8..=10,
            types in 0u8..=10,
        ) {
            let s = Specificity { inline, ids, classes, types };
            prop_assert_eq!(s.cmp(&s), std::cmp::Ordering::Equal);
        }

        /// Higher inline always beats any non-inline specificity.
        #[test]
        fn inline_always_wins(
            ids in 0u8..=10,
            classes in 0u8..=10,
            types in 0u8..=10,
        ) {
            let inline = Specificity { inline: 1, ids: 0, classes: 0, types: 0 };
            let non_inline = Specificity { inline: 0, ids, classes, types };
            prop_assert!(inline > non_inline);
        }

        /// Any ID selector beats any number of class+type selectors
        /// (with no IDs and no inline).
        #[test]
        fn id_beats_classes_and_types(
            classes in 0u8..=10,
            types in 0u8..=10,
        ) {
            let with_id = Specificity { inline: 0, ids: 1, classes: 0, types: 0 };
            let without_id = Specificity { inline: 0, ids: 0, classes, types };
            prop_assert!(with_id > without_id);
        }

        /// Any class selector beats any number of type selectors
        /// (with no IDs, no inline, no classes on the other side).
        #[test]
        fn class_beats_types(types in 0u8..=10) {
            let with_class = Specificity { inline: 0, ids: 0, classes: 1, types: 0 };
            let only_types = Specificity { inline: 0, ids: 0, classes: 0, types };
            prop_assert!(with_class > only_types);
        }

        /// Specificity ordering is transitive: if a > b and b > c then a > c.
        #[test]
        fn specificity_transitive(
            a in arb_specificity(),
            b in arb_specificity(),
            c in arb_specificity(),
        ) {
            if a > b && b > c {
                prop_assert!(a > c);
            }
        }

        /// Specificity ordering is antisymmetric: if a > b then !(b > a).
        #[test]
        fn specificity_antisymmetric(
            a in arb_specificity(),
            b in arb_specificity(),
        ) {
            if a > b {
                prop_assert!(!(b > a));
            }
        }
    }

    // -- Selector::specificity() computation --------------------------

    proptest! {
        /// A selector with N id parts should have ids == N in its
        /// specificity (up to saturation).
        #[test]
        fn selector_specificity_id_count(n_ids in 1u8..=5) {
            let sel = selector_with_counts(n_ids, 0, 0);
            let spec = sel.specificity();
            prop_assert_eq!(spec.ids, n_ids);
            prop_assert_eq!(spec.classes, 0);
            prop_assert_eq!(spec.types, 0);
        }

        /// A selector with only class parts should have classes == N.
        #[test]
        fn selector_specificity_class_count(n_classes in 1u8..=5) {
            let sel = selector_with_counts(0, n_classes, 0);
            let spec = sel.specificity();
            prop_assert_eq!(spec.classes, n_classes);
            prop_assert_eq!(spec.ids, 0);
        }

        /// Adding an ID part to a selector always increases specificity.
        #[test]
        fn adding_id_increases_specificity(
            n_classes in 0u8..=3,
            n_types in 0u8..=3,
        ) {
            let without = selector_with_counts(0, n_classes, n_types);
            let with = selector_with_counts(1, n_classes, n_types);
            prop_assert!(with.specificity() > without.specificity());
        }
    }

    // -- Source order: equal specificity, later declaration wins -------

    proptest! {
        /// When two rules have equal specificity (both type selectors),
        /// the later rule's value wins in the cascade.
        #[test]
        fn source_order_later_wins(
            later_color in proptest::sample::select(vec![
                ("red",     oasis_types::backend::Color::rgb(255, 0, 0)),
                ("green",   oasis_types::backend::Color::rgb(0, 128, 0)),
                ("blue",    oasis_types::backend::Color::rgb(0, 0, 255)),
                ("yellow",  oasis_types::backend::Color::rgb(255, 255, 0)),
                ("cyan",    oasis_types::backend::Color::rgb(0, 255, 255)),
                ("magenta", oasis_types::backend::Color::rgb(255, 0, 255)),
                ("white",   oasis_types::backend::Color::rgb(255, 255, 255)),
            ]),
        ) {
            let (color_name, expected) = later_color;
            // First rule sets color to black; second to the named color.
            let sheet = Stylesheet {
                rules: vec![
                    Rule {
                        selectors: SelectorList {
                            selectors: vec![super::simple_type_selector("div")],
                        },
                        declarations: vec![Declaration {
                            property: "color".to_string(),
                            value: CssValue::Keyword("black".to_string()),
                            important: false,
                            property_id: PropertyId::from_name("color"),
                        }],
                        layer: None,
                        container: None,
                        scope: None,
                    },
                    Rule {
                        selectors: SelectorList {
                            selectors: vec![super::simple_type_selector("div")],
                        },
                        declarations: vec![Declaration {
                            property: "color".to_string(),
                            value: CssValue::Keyword(String::from(color_name)),
                            important: false,
                            property_id: PropertyId::from_name("color"),
                        }],
                        layer: None,
                        container: None,
                        scope: None,
                    },
                ],
                keyframes: vec![],
                layers: vec![],
                counter_styles: vec![],
                properties: vec![],
                font_faces: vec![],
            };
            let doc = super::make_doc(vec![(TagName::Div, vec![])]);
            let styles = style_tree(&doc, &[&sheet], &[], &super::ctx());
            let style = styles[3].as_ref().expect("div should have style");
            // The second (later) rule should win.
            prop_assert_eq!(style.color, expected);
        }
    }

    // -- Inheritance overridden by any direct declaration -------------

    proptest! {
        /// A direct type-selector rule on a child always overrides an
        /// inherited value from the parent, regardless of what the
        /// parent's color is.
        #[test]
        fn direct_declaration_overrides_inheritance(
            parent_color in proptest::sample::select(vec![
                "red", "green", "blue", "yellow", "cyan", "magenta",
            ]),
        ) {
            use crate::html::dom::{ElementData, Node, NodeKind};

            let mut doc = super::make_doc(vec![(TagName::Div, vec![])]);
            let p_id = doc.nodes.len();
            doc.nodes.push(Node {
                kind: NodeKind::Element(ElementData {
                    tag: TagName::P,
                    attributes: vec![],
                }),
                parent: Some(3),
                children: vec![],
            });
            doc.nodes[3].children.push(p_id);

            // Parent sets color to parent_color; child sets color to white.
            let sheet = Stylesheet {
                rules: vec![
                    Rule {
                        selectors: SelectorList {
                            selectors: vec![super::simple_type_selector("div")],
                        },
                        declarations: vec![Declaration {
                            property: "color".to_string(),
                            value: CssValue::Keyword(String::from(parent_color)),
                            important: false,
                            property_id: PropertyId::from_name("color"),
                        }],
                        layer: None,
                        container: None,
                        scope: None,
                    },
                    Rule {
                        selectors: SelectorList {
                            selectors: vec![super::simple_type_selector("p")],
                        },
                        declarations: vec![Declaration {
                            property: "color".to_string(),
                            value: CssValue::Keyword("white".to_string()),
                            important: false,
                            property_id: PropertyId::from_name("color"),
                        }],
                        layer: None,
                        container: None,
                        scope: None,
                    },
                ],
                keyframes: vec![],
                layers: vec![],
                counter_styles: vec![],
                properties: vec![],
                font_faces: vec![],
            };
            let styles = style_tree(&doc, &[&sheet], &[], &super::ctx());
            let p_style = styles[p_id].as_ref().expect("p should have style");
            // Child should always be white, not the inherited parent color.
            prop_assert_eq!(
                p_style.color,
                oasis_types::backend::Color::rgb(255, 255, 255),
            );
        }

        /// !important on a low-specificity selector beats a higher-
        /// specificity normal declaration.
        #[test]
        fn important_beats_higher_specificity(
            n_extra_classes in 0u8..=5,
        ) {
            // Build a selector with 1 id + N extra classes (high specificity).
            let high_spec_sel = Selector {
                parts: vec![(
                    CompoundSelector {
                        parts: {
                            let mut p = vec![SimpleSelector::Id("main".to_string())];
                            for i in 0..n_extra_classes {
                                p.push(SimpleSelector::Class(format!("c{i}")));
                            }
                            p
                        },
                    },
                    None,
                )],
            };
            // Low-specificity type selector with !important.
            let low_spec_sel = super::simple_type_selector("div");

            let sheet = Stylesheet {
                rules: vec![
                    Rule {
                        selectors: SelectorList {
                            selectors: vec![high_spec_sel],
                        },
                        declarations: vec![Declaration {
                            property: "color".to_string(),
                            value: CssValue::Keyword("red".to_string()),
                            important: false,
                            property_id: PropertyId::from_name("color"),
                        }],
                        layer: None,
                        container: None,
                        scope: None,
                    },
                    Rule {
                        selectors: SelectorList {
                            selectors: vec![low_spec_sel],
                        },
                        declarations: vec![Declaration {
                            property: "color".to_string(),
                            value: CssValue::Keyword("blue".to_string()),
                            important: true,
                            property_id: PropertyId::from_name("color"),
                        }],
                        layer: None,
                        container: None,
                        scope: None,
                    },
                ],
                keyframes: vec![],
                layers: vec![],
                counter_styles: vec![],
                properties: vec![],
                font_faces: vec![],
            };

            // Build element with id="main" and all the extra classes.
            let mut class_val = String::new();
            for i in 0..n_extra_classes {
                if !class_val.is_empty() {
                    class_val.push(' ');
                }
                class_val.push_str(&format!("c{i}"));
            }
            let mut attrs = vec![Attribute {
                name: "id".to_string(),
                value: "main".to_string(),
            }];
            if !class_val.is_empty() {
                attrs.push(Attribute {
                    name: "class".to_string(),
                    value: class_val,
                });
            }

            let doc = super::make_doc(vec![(TagName::Div, attrs)]);
            let styles = style_tree(&doc, &[&sheet], &[], &super::ctx());
            let style = styles[3].as_ref().expect("div should have style");
            // Blue (!important) should always win.
            prop_assert_eq!(style.color, oasis_types::backend::Color::rgb(0, 0, 255));
        }
    }

    // -- Fuzz-style no-panic tests for CSS parsing -----------------------

    use crate::css::helpers::{named_color, parse_hex_color};
    use crate::css::parser::parse_inline_style;
    use crate::css::tokenizer::CssTokenizer;

    proptest! {
        /// Parsing an arbitrary string as a full CSS stylesheet never panics.
        #[test]
        fn stylesheet_parse_no_panic(input in "\\PC{0,200}") {
            let _ = Stylesheet::parse(&input);
        }

        /// Parsing an arbitrary string as an inline style never panics.
        #[test]
        fn inline_style_parse_no_panic(input in "\\PC{0,200}") {
            let _ = parse_inline_style(&input);
        }

        /// Hex color parsing never panics on arbitrary input.
        #[test]
        fn hex_color_parse_no_panic(input in "\\PC{0,20}") {
            let _ = parse_hex_color(&input);
        }

        /// Named color lookup never panics on arbitrary input.
        #[test]
        fn named_color_no_panic(input in "\\PC{0,30}") {
            let _ = named_color(&input);
        }

        /// Cascading a stylesheet parsed from arbitrary CSS over a
        /// trivial document never panics.
        #[test]
        fn cascade_arbitrary_css_no_panic(css in "\\PC{0,300}") {
            let sheet = Stylesheet::parse(&css);
            let doc = super::make_doc(vec![(TagName::Div, vec![])]);
            let _ = style_tree(&doc, &[&sheet], &[], &super::ctx());
        }

        /// Tokenizing then parsing a value list never panics.
        #[test]
        fn parse_value_list_no_panic(input in "\\PC{0,100}") {
            let tokens = CssTokenizer::new(&input).tokenize();
            let _ = crate::css::parser::parse_value_list(&tokens);
        }
    }
}

// -------------------------------------------------------------------
// @container query cascade
// -------------------------------------------------------------------

#[cfg(test)]
mod container_query_cascade_tests {
    use super::*;

    /// Build a doc shaped like `<html><body><div class="card"><p/></div></body></html>`,
    /// where the div is the query container and the p is the rule subject.
    fn make_card_doc() -> (Document, NodeId, NodeId) {
        let mut doc = make_doc(vec![(TagName::Div, vec![])]);
        let div_id = 3;
        let p_id = doc.nodes.len();
        doc.nodes.push(Node {
            kind: NodeKind::Element(ElementData {
                tag: TagName::P,
                attributes: vec![],
            }),
            parent: Some(div_id),
            children: vec![],
        });
        doc.nodes[div_id].children.push(p_id);
        (doc, div_id, p_id)
    }

    fn lookup_with(div_id: NodeId, names: Vec<&str>, w: f32, h: f32) -> ContainerLookup {
        let mut l = ContainerLookup::new();
        l.insert(
            div_id,
            ContainerEntry {
                names: names.iter().map(|s| s.to_string()).collect(),
                width: w,
                height: h,
                container_type: crate::css::values::types::ContainerType::Size,
                custom_properties: rustc_hash::FxHashMap::default(),
            },
        );
        l
    }

    fn p_color_with_lookup(
        sheet: &Stylesheet,
        doc: &Document,
        p_id: NodeId,
        lookup: Option<&ContainerLookup>,
    ) -> Color {
        let ctx = CascadeContext {
            hover_node: None,
            visited_urls: None,
            focused_node: None,
            containers: lookup,
            global_layers: None,
        };
        let styles = style_tree(doc, &[sheet], &[], &ctx);
        styles[p_id].as_ref().unwrap().color
    }

    #[test]
    fn min_width_matches_when_container_wide_enough() {
        let css = "p { color: black; } @container (min-width: 400px) { p { color: red; } }";
        let sheet = Stylesheet::parse(css);
        let (doc, div_id, p_id) = make_card_doc();
        let lookup = lookup_with(div_id, vec![], 500.0, 100.0);
        assert_eq!(
            p_color_with_lookup(&sheet, &doc, p_id, Some(&lookup)),
            Color::rgb(255, 0, 0),
            "container is 500px ≥ 400px → red wins"
        );
    }

    #[test]
    fn min_width_does_not_match_when_container_too_narrow() {
        let css = "p { color: black; } @container (min-width: 400px) { p { color: red; } }";
        let sheet = Stylesheet::parse(css);
        let (doc, div_id, p_id) = make_card_doc();
        let lookup = lookup_with(div_id, vec![], 300.0, 100.0);
        assert_eq!(
            p_color_with_lookup(&sheet, &doc, p_id, Some(&lookup)),
            Color::BLACK,
            "container is 300px < 400px → black wins"
        );
    }

    #[test]
    fn rule_skipped_without_lookup() {
        // Without a container snapshot the gated rule must not contribute.
        let css = "p { color: black; } @container (min-width: 1px) { p { color: red; } }";
        let sheet = Stylesheet::parse(css);
        let (doc, _, p_id) = make_card_doc();
        assert_eq!(
            p_color_with_lookup(&sheet, &doc, p_id, None),
            Color::BLACK,
            "no lookup → container rules treated as never-matching"
        );
    }

    #[test]
    fn named_container_only_matches_matching_name() {
        let css = "@container card (min-width: 100px) { p { color: red; } }";
        let sheet = Stylesheet::parse(css);
        let (doc, div_id, p_id) = make_card_doc();

        // Wrong name → no match → falls back to UA default (Color::BLACK).
        let wrong = lookup_with(div_id, vec!["sidebar"], 500.0, 100.0);
        let style_with_wrong = p_color_with_lookup(&sheet, &doc, p_id, Some(&wrong));

        // Right name → match → red.
        let right = lookup_with(div_id, vec!["card"], 500.0, 100.0);
        let style_with_right = p_color_with_lookup(&sheet, &doc, p_id, Some(&right));

        assert_eq!(style_with_wrong, Color::BLACK);
        assert_eq!(style_with_right, Color::rgb(255, 0, 0));
    }

    #[test]
    fn max_width_predicate() {
        let css = "p { color: black; } @container (max-width: 500px) { p { color: red; } }";
        let sheet = Stylesheet::parse(css);
        let (doc, div_id, p_id) = make_card_doc();

        let narrow = lookup_with(div_id, vec![], 400.0, 100.0);
        let wide = lookup_with(div_id, vec![], 600.0, 100.0);

        assert_eq!(
            p_color_with_lookup(&sheet, &doc, p_id, Some(&narrow)),
            Color::rgb(255, 0, 0)
        );
        assert_eq!(
            p_color_with_lookup(&sheet, &doc, p_id, Some(&wide)),
            Color::BLACK
        );
    }

    #[test]
    fn empty_features_never_match() {
        // `style(...)` parses to an empty feature list; should never apply.
        let css = "p { color: black; } @container style(--x: y) { p { color: red; } }";
        let sheet = Stylesheet::parse(css);
        let (doc, div_id, p_id) = make_card_doc();
        let lookup = lookup_with(div_id, vec![], 500.0, 100.0);
        assert_eq!(
            p_color_with_lookup(&sheet, &doc, p_id, Some(&lookup)),
            Color::BLACK
        );
    }

    fn scope_p_color(css: &str) -> Color {
        let sheet = Stylesheet::parse(css);
        let (doc, _, p_id) = make_card_doc();
        let ctx = ctx();
        let styles = style_tree(&doc, &[&sheet], &[], &ctx);
        styles[p_id].as_ref().unwrap().color
    }

    #[test]
    fn scope_root_includes_descendants() {
        // Root matches the div ancestor → p inside scope → red wins.
        let css = "p { color: black; } @scope (div) { p { color: red; } }";
        assert_eq!(scope_p_color(css), Color::rgb(255, 0, 0));
    }

    #[test]
    fn scope_root_excludes_unrelated_subtrees() {
        // Root matches no ancestor → out of scope → fall back.
        let css = "p { color: black; } @scope (.absent) { p { color: red; } }";
        assert_eq!(scope_p_color(css), Color::BLACK);
    }

    #[test]
    fn scope_no_root_applies_everywhere() {
        let css = "p { color: black; } @scope { p { color: red; } }";
        assert_eq!(scope_p_color(css), Color::rgb(255, 0, 0));
    }

    #[test]
    fn property_initial_value_seeds_var_fallback() {
        let css = r#"
            @property --brand { syntax: "*"; inherits: true; initial-value: red; }
            p { color: var(--brand); }
        "#;
        let sheet = Stylesheet::parse(css);
        let (doc, _, p_id) = make_card_doc();
        let styles = style_tree(&doc, &[&sheet], &[], &ctx());
        // var(--brand) should resolve to the registered initial-value.
        assert_eq!(styles[p_id].as_ref().unwrap().color, Color::rgb(255, 0, 0));
    }

    #[test]
    fn property_explicit_value_overrides_initial() {
        let css = r#"
            @property --brand { syntax: "*"; inherits: true; initial-value: red; }
            div { --brand: blue; }
            p { color: var(--brand); }
        "#;
        let sheet = Stylesheet::parse(css);
        let (doc, _, p_id) = make_card_doc();
        let styles = style_tree(&doc, &[&sheet], &[], &ctx());
        // Inherited from div which set --brand: blue.
        assert_eq!(styles[p_id].as_ref().unwrap().color, Color::rgb(0, 0, 255));
    }

    #[test]
    fn build_container_lookup_picks_up_container_type() {
        use crate::css::values::types::ContainerType;
        use crate::layout::box_model::{BoxType, Dimensions, LayoutBox, Rect};

        let mut style = ComputedStyle::default();
        style.container_type = ContainerType::InlineSize;
        style.container_name = vec!["card".to_string()];

        let mut child = LayoutBox::new(BoxType::Block, style, Some(7));
        child.dimensions = Dimensions {
            content: Rect::new(0.0, 0.0, 320.0, 240.0),
            ..Dimensions::default()
        };

        let lookup = build_container_lookup(&child);
        let entry = lookup.get(7).expect("container should be indexed");
        assert_eq!(entry.names, vec!["card".to_string()]);
        assert!((entry.width - 320.0).abs() < 0.01);
        assert!((entry.height - 240.0).abs() < 0.01);
    }

    // -- light-dark() + color-scheme -----------------------------------

    #[test]
    fn light_dark_resolves_to_dark_under_dark_scheme() {
        use super::super::super::parser::CssColor;
        use super::super::super::values::types::ColorScheme;

        let mut style = ComputedStyle::default();
        // Simulate color-scheme: dark on the element.
        style.color_scheme = ColorScheme::Dark;
        let value =
            CssValue::LightDark(CssColor::new(255, 0, 0, 255), CssColor::new(0, 0, 255, 255));
        style.apply_declaration("color", &value, 16.0);
        assert_eq!(style.color, Color::rgba(0, 0, 255, 255));
    }

    #[test]
    fn light_dark_resolves_to_light_under_light_scheme() {
        use super::super::super::parser::CssColor;
        use super::super::super::values::types::ColorScheme;

        let mut style = ComputedStyle::default();
        style.color_scheme = ColorScheme::Light;
        let value =
            CssValue::LightDark(CssColor::new(255, 0, 0, 255), CssColor::new(0, 0, 255, 255));
        style.apply_declaration("color", &value, 16.0);
        assert_eq!(style.color, Color::rgba(255, 0, 0, 255));
    }
}
