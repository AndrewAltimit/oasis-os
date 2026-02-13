//! CSS cascade and selector matching.
//!
//! Implements the CSS cascade algorithm: for each element in a DOM tree,
//! collect matching rules from all stylesheets, sort by specificity and
//! source order, then apply declarations to produce computed styles.

use super::parser::{
    Combinator, CompoundSelector, CssValue, Declaration, Rule, SimpleSelector, Specificity,
    Stylesheet,
};
use super::values::ComputedStyle;
use crate::html::dom::{Document, ElementData, NodeId, NodeKind};

// -----------------------------------------------------------------------
// Public API
// -----------------------------------------------------------------------

/// Style a DOM tree by applying stylesheets and inline styles.
///
/// Returns a `Vec` indexed by `NodeId`. Elements get `Some(style)`;
/// non-element nodes (text, comments, document root) get `None`.
pub fn style_tree(
    doc: &Document,
    stylesheets: &[&Stylesheet],
    inline_styles: &[(NodeId, Vec<Declaration>)],
) -> Vec<Option<ComputedStyle>> {
    let mut styles: Vec<Option<ComputedStyle>> = vec![None; doc.nodes.len()];
    style_subtree(doc, doc.root, stylesheets, inline_styles, &mut styles);
    styles
}

/// Recursively compute styles depth-first so that children can inherit
/// from their (already-computed) parent.
fn style_subtree(
    doc: &Document,
    node_id: NodeId,
    stylesheets: &[&Stylesheet],
    inline_styles: &[(NodeId, Vec<Declaration>)],
    styles: &mut [Option<ComputedStyle>],
) {
    let node = &doc.nodes[node_id];

    // Only elements get computed styles.
    if let NodeKind::Element(_) = &node.kind {
        let parent_style = node.parent.and_then(|pid| styles[pid].as_ref());
        let style = compute_style(doc, node_id, parent_style, stylesheets, inline_styles);
        styles[node_id] = Some(style);
    }

    // Recurse into children. Iterate by index to avoid cloning the Vec.
    let num_children = doc.nodes[node_id].children.len();
    for i in 0..num_children {
        let child_id = doc.nodes[node_id].children[i];
        style_subtree(doc, child_id, stylesheets, inline_styles, styles);
    }
}

/// Compute the final style for a single element.
fn compute_style(
    doc: &Document,
    node_id: NodeId,
    parent_style: Option<&ComputedStyle>,
    stylesheets: &[&Stylesheet],
    inline_styles: &[(NodeId, Vec<Declaration>)],
) -> ComputedStyle {
    // Start from inherited values if we have a parent, else defaults.
    let mut style = match parent_style {
        Some(parent) => ComputedStyle::inherit(parent),
        None => ComputedStyle::default(),
    };

    let parent_font_size = parent_style.map_or(super::values::ROOT_FONT_SIZE, |p| p.font_size);

    // Collect all matching declarations with their origin info.
    let mut matched = collect_matched_declarations(doc, node_id, stylesheets, inline_styles);

    // Sort by cascade order: specificity, then source order.
    // `!important` declarations come after normal ones.
    matched.sort_by(|a, b| {
        a.important
            .cmp(&b.important)
            .then_with(|| a.origin.cmp(&b.origin))
            .then_with(|| a.specificity.cmp(&b.specificity))
            .then_with(|| a.source_order.cmp(&b.source_order))
    });

    // Apply in sorted order (lowest priority first, last wins).
    for entry in &matched {
        style.apply_declaration(&entry.property, &entry.value, parent_font_size);
    }

    style
}

// -----------------------------------------------------------------------
// Matched declaration collection
// -----------------------------------------------------------------------

/// The origin of a declaration for cascade ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Origin {
    /// From a `<link>` or `<style>` stylesheet.
    Stylesheet,
    /// From the element's `style=""` attribute.
    Inline,
}

/// A single declaration together with its cascade metadata.
#[derive(Debug, Clone)]
struct MatchedDeclaration {
    property: String,
    value: CssValue,
    important: bool,
    origin: Origin,
    specificity: Specificity,
    source_order: usize,
}

/// Gather every declaration that applies to `node_id` from all
/// stylesheets and inline styles.
fn collect_matched_declarations(
    doc: &Document,
    node_id: NodeId,
    stylesheets: &[&Stylesheet],
    inline_styles: &[(NodeId, Vec<Declaration>)],
) -> Vec<MatchedDeclaration> {
    let mut result = Vec::new();
    let mut order: usize = 0;

    // Walk stylesheets in order (user-agent first, author last).
    for stylesheet in stylesheets {
        for rule in &stylesheet.rules {
            let best_specificity = matching_specificity(doc, node_id, rule);
            if let Some(specificity) = best_specificity {
                for decl in &rule.declarations {
                    result.push(MatchedDeclaration {
                        property: decl.property.clone(),
                        value: decl.value.clone(),
                        important: decl.important,
                        origin: Origin::Stylesheet,
                        specificity,
                        source_order: order,
                    });
                    order += 1;
                }
            }
        }
    }

    // Inline styles have the highest non-important specificity.
    let inline_spec = Specificity {
        inline: 1,
        ids: 0,
        classes: 0,
        types: 0,
    };
    for (nid, decls) in inline_styles {
        if *nid == node_id {
            for decl in decls {
                result.push(MatchedDeclaration {
                    property: decl.property.clone(),
                    value: decl.value.clone(),
                    important: decl.important,
                    origin: Origin::Inline,
                    specificity: inline_spec,
                    source_order: order,
                });
                order += 1;
            }
        }
    }

    result
}

/// Return the highest specificity among the rule's selectors that match
/// `node_id`, or `None` if no selector matches.
fn matching_specificity(doc: &Document, node_id: NodeId, rule: &Rule) -> Option<Specificity> {
    let mut best: Option<Specificity> = None;
    for selector in &rule.selectors.selectors {
        if matches_selector(doc, node_id, selector) {
            let spec = selector.specificity();
            best = Some(match best {
                Some(prev) if prev >= spec => prev,
                _ => spec,
            });
        }
    }
    best
}

// -----------------------------------------------------------------------
// Selector matching
// -----------------------------------------------------------------------

/// Check if a parsed selector matches a given element in the DOM.
///
/// A `Selector` stores its parts left-to-right: the first compound is
/// the leftmost in the source, and the last compound is the *subject*
/// (the element being tested). Combinators link compounds and are
/// stored as `Option<Combinator>` where `None` marks the first entry.
fn matches_selector(doc: &Document, node_id: NodeId, selector: &super::parser::Selector) -> bool {
    let parts = &selector.parts;
    if parts.is_empty() {
        return false;
    }

    // The last compound is the subject -- it must match node_id.
    let last_idx = parts.len() - 1;
    if !matches_compound(doc, node_id, &parts[last_idx].0) {
        return false;
    }

    // Walk remaining parts right-to-left (from subject towards root).
    let mut current = node_id;
    for i in (0..last_idx).rev() {
        let (ref compound, _) = parts[i];
        // The combinator that connects `parts[i]` to `parts[i+1]` is
        // stored in `parts[i+1].1`.
        let combinator = parts[i + 1].1.as_ref();
        match combinator {
            Some(Combinator::Child) => match parent_element(doc, current) {
                Some(pid) if matches_compound(doc, pid, compound) => {
                    current = pid;
                },
                _ => return false,
            },
            Some(Combinator::Descendant) | None => {
                // Walk up ancestors until one matches.
                let mut found = false;
                let mut ancestor = parent_element(doc, current);
                while let Some(anc_id) = ancestor {
                    if matches_compound(doc, anc_id, compound) {
                        current = anc_id;
                        found = true;
                        break;
                    }
                    ancestor = parent_element(doc, anc_id);
                }
                if !found {
                    return false;
                }
            },
        }
    }

    true
}

/// Check if a compound selector matches a given node.
fn matches_compound(doc: &Document, node_id: NodeId, compound: &CompoundSelector) -> bool {
    compound
        .parts
        .iter()
        .all(|simple| matches_simple(doc, node_id, simple))
}

/// Check if a single simple selector matches a node.
fn matches_simple(doc: &Document, node_id: NodeId, simple: &SimpleSelector) -> bool {
    let elem = match &doc.nodes[node_id].kind {
        NodeKind::Element(e) => e,
        _ => return false,
    };

    match simple {
        SimpleSelector::Universal => true,
        SimpleSelector::Type(tag_name) => elem.tag.as_str().eq_ignore_ascii_case(tag_name),
        SimpleSelector::Class(cls) => elem.has_class(cls),
        SimpleSelector::Id(id) => elem.get_attribute("id").is_some_and(|v| v == id),
        SimpleSelector::PseudoClass(pseudo) => match_pseudo_class(doc, node_id, elem, pseudo),
    }
}

/// Match structural pseudo-classes.
fn match_pseudo_class(doc: &Document, node_id: NodeId, _elem: &ElementData, pseudo: &str) -> bool {
    match pseudo {
        "first-child" => {
            if let Some(pid) = doc.nodes[node_id].parent {
                let siblings = &doc.nodes[pid].children;
                for &sid in siblings {
                    if matches!(doc.nodes[sid].kind, NodeKind::Element(_)) {
                        return sid == node_id;
                    }
                }
            }
            false
        },
        "last-child" => {
            if let Some(pid) = doc.nodes[node_id].parent {
                let siblings = &doc.nodes[pid].children;
                for &sid in siblings.iter().rev() {
                    if matches!(doc.nodes[sid].kind, NodeKind::Element(_)) {
                        return sid == node_id;
                    }
                }
            }
            false
        },
        // Stateful pseudo-classes (:hover, :focus, :visited) are not
        // handled during static cascade. Return false.
        _ => false,
    }
}

// -----------------------------------------------------------------------
// DOM traversal helpers
// -----------------------------------------------------------------------

/// Find the nearest ancestor that is an element node.
fn parent_element(doc: &Document, node_id: NodeId) -> Option<NodeId> {
    let mut current = doc.nodes[node_id].parent;
    while let Some(pid) = current {
        if matches!(doc.nodes[pid].kind, NodeKind::Element(_)) {
            return Some(pid);
        }
        current = doc.nodes[pid].parent;
    }
    None
}

// -----------------------------------------------------------------------
// Default (user-agent) stylesheet
// -----------------------------------------------------------------------

/// Return the built-in user-agent stylesheet.
///
/// This is the CSS2.1 default stylesheet for HTML elements. It
/// participates in the normal cascade so author/skin stylesheets can
/// override any rule using standard specificity rules.
pub fn default_stylesheet() -> Stylesheet {
    Stylesheet::parse(UA_CSS)
}

/// Minimal user-agent stylesheet following CSS 2.1 defaults.
const UA_CSS: &str = r#"
html, body, div, main, section, article, nav, aside,
header, footer, figure, figcaption, address, details,
summary, blockquote, fieldset, form {
    display: block;
}

p {
    display: block;
    margin-top: 1em;
    margin-bottom: 1em;
}

h1 {
    display: block;
    font-size: 2em;
    font-weight: bold;
    margin-top: 0.67em;
    margin-bottom: 0.67em;
}
h2 {
    display: block;
    font-size: 1.5em;
    font-weight: bold;
    margin-top: 0.83em;
    margin-bottom: 0.83em;
}
h3 {
    display: block;
    font-size: 1.17em;
    font-weight: bold;
    margin-top: 1em;
    margin-bottom: 1em;
}
h4 {
    display: block;
    font-size: 1em;
    font-weight: bold;
    margin-top: 1.33em;
    margin-bottom: 1.33em;
}
h5 {
    display: block;
    font-size: 0.83em;
    font-weight: bold;
    margin-top: 1.67em;
    margin-bottom: 1.67em;
}
h6 {
    display: block;
    font-size: 0.67em;
    font-weight: bold;
    margin-top: 2.33em;
    margin-bottom: 2.33em;
}

ul, ol {
    display: block;
    margin-top: 1em;
    margin-bottom: 1em;
    padding-left: 40px;
}
li {
    display: list-item;
    list-style-type: disc;
}

pre {
    display: block;
    white-space: pre;
    font-family: monospace;
    margin-top: 1em;
    margin-bottom: 1em;
}
code, kbd, samp {
    font-family: monospace;
}

hr {
    display: block;
    margin-top: 8px;
    margin-bottom: 8px;
    border-top-width: 1px;
    border-top-style: solid;
    border-top-color: #808080;
}

b, strong { font-weight: bold; }
i, em, cite { font-style: italic; }
u, ins { text-decoration: underline; }
s, del { text-decoration: line-through; }

a {
    color: #0000ee;
    text-decoration: underline;
}

table { display: table; }
tr { display: table-row; }
td { display: table-cell; }
th {
    display: table-cell;
    font-weight: bold;
    text-align: center;
}

br, img, input, button, select, textarea {
    display: inline;
}

head, script, style, link, meta, title, noscript {
    display: none;
}
"#;

// -----------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::super::parser::{
        Combinator, CompoundSelector, CssValue, Declaration, Rule, Selector, SelectorList,
        SimpleSelector, Stylesheet,
    };
    use super::super::values::{Display, FontWeight};
    use super::*;
    use crate::html::dom::{Attribute, Document, ElementData, Node, NodeKind, TagName};
    use oasis_types::backend::Color;

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

        Document { nodes, root: 0 }
    }

    fn make_rule(selectors: Vec<Selector>, declarations: Vec<Declaration>) -> Rule {
        Rule {
            selectors: SelectorList { selectors },
            declarations,
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
            property: property.to_string(),
            value,
            important,
        }
    }

    // -- Tests ----------------------------------------------------------

    #[test]
    fn type_selector_matching() {
        let doc = make_doc(vec![(TagName::P, vec![]), (TagName::Div, vec![])]);
        let sel = simple_type_selector("p");
        // Node 3 is <p>, node 4 is <div>.
        assert!(matches_selector(&doc, 3, &sel));
        assert!(!matches_selector(&doc, 4, &sel));
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
        assert!(matches_selector(&doc, 3, &sel));
        assert!(!matches_selector(&doc, 4, &sel));
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
        assert!(matches_selector(&doc, 3, &sel));

        let wrong = simple_id_selector("other");
        assert!(!matches_selector(&doc, 3, &wrong));
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
            matches_selector(&doc, p_id, &sel),
            "p inside div should match `div p`"
        );

        // <p> directly in <body> should NOT match `div p`.
        let doc2 = make_doc(vec![(TagName::P, vec![])]);
        assert!(
            !matches_selector(&doc2, 3, &sel),
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
        };
        let styles = style_tree(&doc, &[&sheet], &[]);

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
        let sheet = Stylesheet { rules: vec![rule] };
        let styles = style_tree(&doc, &[&sheet], &[]);

        let p_style = styles[p_id].as_ref().expect("p should have style");
        assert_eq!(p_style.color, Color::rgb(255, 0, 0));
        assert_eq!(p_style.font_weight, FontWeight::Bold);
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
        };
        let styles = style_tree(&doc, &[&sheet], &[]);
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
        };
        let sheet2 = Stylesheet {
            rules: vec![make_rule(
                vec![simple_type_selector("p")],
                vec![decl("font-weight", CssValue::Number(700.0), false)],
            )],
        };

        let styles = style_tree(&doc, &[&sheet1, &sheet2], &[]);
        let style = styles[3].as_ref().expect("p should have style");
        assert_eq!(style.color, Color::rgb(255, 0, 0));
        assert_eq!(style.font_weight, FontWeight::Bold);
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
        };

        // Inline style says color: blue.
        let inline = vec![(
            3_usize,
            vec![decl("color", CssValue::Keyword("blue".to_string()), false)],
        )];

        let styles = style_tree(&doc, &[&sheet], &inline);
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
        let styles = style_tree(&doc, &[&ua], &[]);

        let p_style = styles[3].as_ref().unwrap();
        assert_eq!(p_style.display, Display::Block);

        let h1_style = styles[4].as_ref().unwrap();
        assert_eq!(h1_style.display, Display::Block);
        assert_eq!(h1_style.font_weight, FontWeight::Bold);
        // h1 = 2em * ROOT_FONT_SIZE
        assert!(
            (h1_style.font_size - crate::css::values::ROOT_FONT_SIZE * 2.0).abs() < f32::EPSILON
        );

        let a_style = styles[5].as_ref().unwrap();
        assert_eq!(a_style.color, Color::rgb(0, 0, 238));
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

        let doc = Document { nodes, root: 0 };
        let sheet = Stylesheet { rules: vec![] };
        let styles = style_tree(&doc, &[&sheet], &[]);

        assert!(styles[0].is_none(), "Document node");
        assert!(styles[1].is_some(), "html element");
        assert!(styles[2].is_none(), "Text node");
    }
}
