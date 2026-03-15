//! Advanced CSS pseudo-class selector matching.
//!
//! Provides pure functions for evaluating structural and UI-state
//! pseudo-class selectors against the arena-based DOM from
//! [`crate::html::dom`]. Each function takes a node slice and a target
//! node index, returning whether the pseudo-class matches.
//!
//! The cascade delegates to [`matches_pseudo_class`] and
//! [`matches_pseudo_class_fn`] for structural pseudo-class evaluation.

use crate::html::dom::{Node, NodeKind};

// -------------------------------------------------------------------
// An+B notation
// -------------------------------------------------------------------

/// Represents the `an+b` formula used by `:nth-child` and friends.
///
/// The formula matches a 1-based index `i` when there exists a
/// non-negative integer `n` such that `a*n + b == i`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnB {
    /// The step size (`a` in `an+b`).
    pub a: i32,
    /// The offset (`b` in `an+b`).
    pub b: i32,
}

impl AnB {
    /// Parse an `an+b` expression string.
    ///
    /// Recognised forms: `"odd"`, `"even"`, `"3"`, `"2n+1"`,
    /// `"-n+3"`, `"n"`, `"3n"`, `"-2n+5"`, `"0n+3"`, `"+3n-1"`.
    ///
    /// Returns `None` if the expression cannot be parsed.
    pub fn parse(expr: &str) -> Option<AnB> {
        let s = expr.trim().to_ascii_lowercase();
        if s.is_empty() {
            return None;
        }
        if s == "odd" {
            return Some(AnB { a: 2, b: 1 });
        }
        if s == "even" {
            return Some(AnB { a: 2, b: 0 });
        }

        if let Some(n_pos) = s.find('n') {
            let a_part = s[..n_pos].trim();
            let a = if a_part.is_empty() || a_part == "+" {
                1
            } else if a_part == "-" {
                -1
            } else {
                a_part.parse::<i32>().ok()?
            };

            let rest = s[n_pos + 1..].trim();
            let b = if rest.is_empty() {
                0
            } else {
                // Remove spaces around +/- signs: "  + 3 " -> "+3"
                let compacted: String = rest.chars().filter(|c| !c.is_ascii_whitespace()).collect();
                compacted.parse::<i32>().ok()?
            };

            Some(AnB { a, b })
        } else {
            // Pure integer: "3" matches only position 3.
            let b = s.parse::<i32>().ok()?;
            Some(AnB { a: 0, b })
        }
    }

    /// Check whether a 1-based `index` satisfies this `an+b` formula.
    ///
    /// Returns `true` when there exists a non-negative integer `n`
    /// such that `a*n + b == index`.
    pub fn matches(&self, index: i32) -> bool {
        if self.a == 0 {
            return index == self.b;
        }
        let diff = index - self.b;
        // diff must be divisible by a and the quotient non-negative.
        if diff % self.a != 0 {
            return false;
        }
        diff / self.a >= 0
    }
}

// -------------------------------------------------------------------
// Helper functions
// -------------------------------------------------------------------

/// Returns the tag name of an element node, or `None` if not an
/// element.
pub fn element_tag(nodes: &[Node], node: usize) -> Option<&str> {
    match &nodes[node].kind {
        NodeKind::Element(data) => Some(data.tag.as_str()),
        _ => None,
    }
}

/// Returns all element-child indices of the given node's parent, in
/// document order.
///
/// If the node has no parent, returns an empty `Vec`.
pub fn element_siblings(nodes: &[Node], node: usize) -> Vec<usize> {
    let parent = match nodes[node].parent {
        Some(p) => p,
        None => return Vec::new(),
    };
    nodes[parent]
        .children
        .iter()
        .copied()
        .filter(|&id| matches!(nodes[id].kind, NodeKind::Element(_)))
        .collect()
}

/// Check whether an element has an attribute with the given name.
pub fn has_attribute(nodes: &[Node], node: usize, attr: &str) -> bool {
    match &nodes[node].kind {
        NodeKind::Element(data) => data.attributes.iter().any(|a| a.name == attr),
        _ => false,
    }
}

/// Returns `true` if the node is an element.
fn is_element(nodes: &[Node], node: usize) -> bool {
    matches!(nodes[node].kind, NodeKind::Element(_))
}

/// Tags that are considered form elements for `:enabled`/`:disabled`.
const FORM_TAGS: &[&str] = &["input", "button", "select", "textarea"];

/// Returns `true` if the element is a form element that can be
/// enabled or disabled.
fn is_form_element(nodes: &[Node], node: usize) -> bool {
    element_tag(nodes, node).is_some_and(|tag| FORM_TAGS.contains(&tag))
}

// -------------------------------------------------------------------
// Pseudo-class matching functions
// -------------------------------------------------------------------

/// Matches `:first-child` -- the node is the first element child of
/// its parent.
pub fn matches_first_child(nodes: &[Node], node: usize) -> bool {
    if !is_element(nodes, node) {
        return false;
    }
    let parent = match nodes[node].parent {
        Some(p) => p,
        None => return false,
    };
    for &child in &nodes[parent].children {
        if is_element(nodes, child) {
            return child == node;
        }
    }
    false
}

/// Matches `:last-child` -- the node is the last element child of
/// its parent.
pub fn matches_last_child(nodes: &[Node], node: usize) -> bool {
    if !is_element(nodes, node) {
        return false;
    }
    let parent = match nodes[node].parent {
        Some(p) => p,
        None => return false,
    };
    for &child in nodes[parent].children.iter().rev() {
        if is_element(nodes, child) {
            return child == node;
        }
    }
    false
}

/// Matches `:first-of-type` -- the node is the first sibling with
/// the same tag name.
pub fn matches_first_of_type(nodes: &[Node], node: usize) -> bool {
    let tag = match element_tag(nodes, node) {
        Some(t) => t,
        None => return false,
    };
    let siblings = element_siblings(nodes, node);
    for &sib in &siblings {
        if element_tag(nodes, sib) == Some(tag) {
            return sib == node;
        }
    }
    false
}

/// Matches `:last-of-type` -- the node is the last sibling with the
/// same tag name.
pub fn matches_last_of_type(nodes: &[Node], node: usize) -> bool {
    let tag = match element_tag(nodes, node) {
        Some(t) => t,
        None => return false,
    };
    let siblings = element_siblings(nodes, node);
    for &sib in siblings.iter().rev() {
        if element_tag(nodes, sib) == Some(tag) {
            return sib == node;
        }
    }
    false
}

/// Matches `:only-child` -- the node is the only element child of
/// its parent.
pub fn matches_only_child(nodes: &[Node], node: usize) -> bool {
    if !is_element(nodes, node) {
        return false;
    }
    let siblings = element_siblings(nodes, node);
    siblings.len() == 1 && siblings[0] == node
}

/// Matches `:only-of-type` -- the node is the only sibling with its
/// tag name.
pub fn matches_only_of_type(nodes: &[Node], node: usize) -> bool {
    let tag = match element_tag(nodes, node) {
        Some(t) => t,
        None => return false,
    };
    let siblings = element_siblings(nodes, node);
    let same_type_count = siblings
        .iter()
        .filter(|&&sib| element_tag(nodes, sib) == Some(tag))
        .count();
    same_type_count == 1
}

/// Matches `:empty` -- the element has no child elements and no
/// non-whitespace text content.
///
/// Per the CSS spec, whitespace-only text nodes do not prevent an
/// element from matching `:empty`. Comments are also ignored.
pub fn matches_empty(nodes: &[Node], node: usize) -> bool {
    if !is_element(nodes, node) {
        return false;
    }
    for &child in &nodes[node].children {
        match &nodes[child].kind {
            NodeKind::Element(_) => return false,
            NodeKind::Text(text) if !text.trim().is_empty() => {
                return false;
            },
            // Comments and Document nodes don't prevent :empty.
            _ => {},
        }
    }
    true
}

/// Matches `:nth-child(an+b)` -- 1-based position among element
/// siblings.
pub fn matches_nth_child(nodes: &[Node], node: usize, expr: &str) -> bool {
    if !is_element(nodes, node) {
        return false;
    }
    let anb = match AnB::parse(expr) {
        Some(v) => v,
        None => return false,
    };
    let siblings = element_siblings(nodes, node);
    let pos = siblings.iter().position(|&s| s == node);
    match pos {
        Some(i) => anb.matches(i as i32 + 1),
        None => false,
    }
}

/// Matches `:nth-last-child(an+b)` -- position counting from the
/// end among element siblings.
pub fn matches_nth_last_child(nodes: &[Node], node: usize, expr: &str) -> bool {
    if !is_element(nodes, node) {
        return false;
    }
    let anb = match AnB::parse(expr) {
        Some(v) => v,
        None => return false,
    };
    let siblings = element_siblings(nodes, node);
    let pos = siblings.iter().position(|&s| s == node);
    match pos {
        Some(i) => {
            let from_end = (siblings.len() - i) as i32;
            anb.matches(from_end)
        },
        None => false,
    }
}

/// Matches `:nth-of-type(an+b)` -- 1-based position among siblings
/// of the same tag type.
pub fn matches_nth_of_type(nodes: &[Node], node: usize, expr: &str) -> bool {
    let tag = match element_tag(nodes, node) {
        Some(t) => t,
        None => return false,
    };
    let anb = match AnB::parse(expr) {
        Some(v) => v,
        None => return false,
    };
    let siblings = element_siblings(nodes, node);
    let same_type: Vec<usize> = siblings
        .iter()
        .copied()
        .filter(|&s| element_tag(nodes, s) == Some(tag))
        .collect();
    let pos = same_type.iter().position(|&s| s == node);
    match pos {
        Some(i) => anb.matches(i as i32 + 1),
        None => false,
    }
}

/// Matches `:nth-last-of-type(an+b)` -- position counting from the
/// end among siblings of the same tag type.
pub fn matches_nth_last_of_type(nodes: &[Node], node: usize, expr: &str) -> bool {
    let tag = match element_tag(nodes, node) {
        Some(t) => t,
        None => return false,
    };
    let anb = match AnB::parse(expr) {
        Some(v) => v,
        None => return false,
    };
    let siblings = element_siblings(nodes, node);
    let same_type: Vec<usize> = siblings
        .iter()
        .copied()
        .filter(|&s| element_tag(nodes, s) == Some(tag))
        .collect();
    let pos = same_type.iter().position(|&s| s == node);
    match pos {
        Some(i) => {
            let from_end = (same_type.len() - i) as i32;
            anb.matches(from_end)
        },
        None => false,
    }
}

/// Matches `:root` -- the element is the document root.
///
/// The root element is one whose parent is a `Document` node (not
/// another `Element`).
pub fn matches_root(nodes: &[Node], node: usize) -> bool {
    if !is_element(nodes, node) {
        return false;
    }
    match nodes[node].parent {
        Some(pid) => matches!(nodes[pid].kind, NodeKind::Document),
        // No parent at all -- treat as root.
        None => true,
    }
}

/// Matches `:enabled` -- a form element without the `disabled`
/// attribute.
pub fn matches_enabled(nodes: &[Node], node: usize) -> bool {
    is_form_element(nodes, node) && !has_attribute(nodes, node, "disabled")
}

/// Matches `:disabled` -- a form element with the `disabled`
/// attribute.
pub fn matches_disabled(nodes: &[Node], node: usize) -> bool {
    is_form_element(nodes, node) && has_attribute(nodes, node, "disabled")
}

/// Matches `:checked` -- an input or option element with the
/// `checked` or `selected` attribute.
pub fn matches_checked(nodes: &[Node], node: usize) -> bool {
    let tag = match element_tag(nodes, node) {
        Some(t) => t,
        None => return false,
    };
    match tag {
        "input" => has_attribute(nodes, node, "checked"),
        "option" => has_attribute(nodes, node, "selected"),
        _ => false,
    }
}

// -------------------------------------------------------------------
// Dispatcher functions
// -------------------------------------------------------------------

/// Dispatch a simple (non-functional) pseudo-class by name.
///
/// Handles structural pseudo-classes: `first-child`, `last-child`,
/// `first-of-type`, `last-of-type`, `only-child`, `only-of-type`,
/// `empty`, `root`, `enabled`, `disabled`, `checked`.
///
/// Returns `false` for unrecognised names. Stateful pseudo-classes
/// like `:hover`, `:visited`, `:link` are handled by the cascade
/// directly since they require external context.
pub fn matches_pseudo_class(nodes: &[Node], node: usize, name: &str) -> bool {
    match name {
        "first-child" => matches_first_child(nodes, node),
        "last-child" => matches_last_child(nodes, node),
        "first-of-type" => matches_first_of_type(nodes, node),
        "last-of-type" => matches_last_of_type(nodes, node),
        "only-child" => matches_only_child(nodes, node),
        "only-of-type" => matches_only_of_type(nodes, node),
        "empty" => matches_empty(nodes, node),
        "root" => matches_root(nodes, node),
        "enabled" => matches_enabled(nodes, node),
        "disabled" => matches_disabled(nodes, node),
        "checked" => matches_checked(nodes, node),
        _ => false,
    }
}

/// Dispatch a functional pseudo-class by name and argument.
///
/// Handles: `nth-child`, `nth-last-child`, `nth-of-type`,
/// `nth-last-of-type`.
///
/// Returns `false` for unrecognised names or unparseable arguments.
pub fn matches_pseudo_class_fn(nodes: &[Node], node: usize, name: &str, arg: &str) -> bool {
    match name {
        "nth-child" => matches_nth_child(nodes, node, arg),
        "nth-last-child" => matches_nth_last_child(nodes, node, arg),
        "nth-of-type" => matches_nth_of_type(nodes, node, arg),
        "nth-last-of-type" => matches_nth_last_of_type(nodes, node, arg),
        _ => false,
    }
}

// -------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::dom::{Attribute, ElementData, TagName};

    // ---------------------------------------------------------------
    // Test DOM builder helpers
    // ---------------------------------------------------------------

    /// Build a document node at index 0.
    fn doc_node() -> Node {
        Node {
            kind: NodeKind::Document,
            parent: None,
            children: Vec::new(),
        }
    }

    /// Build an element node (not yet linked).
    fn elem(tag: &str) -> Node {
        Node {
            kind: NodeKind::Element(ElementData {
                tag: TagName::from_str(tag),
                attributes: Vec::new(),
            }),
            parent: None,
            children: Vec::new(),
        }
    }

    /// Build an element node with attributes.
    fn elem_with_attrs(tag: &str, attrs: &[(&str, &str)]) -> Node {
        Node {
            kind: NodeKind::Element(ElementData {
                tag: TagName::from_str(tag),
                attributes: attrs
                    .iter()
                    .map(|(n, v)| Attribute {
                        name: n.to_string(),
                        value: v.to_string(),
                    })
                    .collect(),
            }),
            parent: None,
            children: Vec::new(),
        }
    }

    /// Build a text node.
    fn text(s: &str) -> Node {
        Node {
            kind: NodeKind::Text(s.to_string()),
            parent: None,
            children: Vec::new(),
        }
    }

    /// Build a comment node.
    fn comment(s: &str) -> Node {
        Node {
            kind: NodeKind::Comment(s.to_string()),
            parent: None,
            children: Vec::new(),
        }
    }

    /// Link `child` as a child of `parent` in the node slice.
    fn link(nodes: &mut Vec<Node>, parent: usize, child: usize) {
        nodes[parent].children.push(child);
        nodes[child].parent = Some(parent);
    }

    /// Build a typical small DOM:
    ///
    /// ```text
    /// 0: Document
    /// 1:   <html>
    /// 2:     <div>
    /// 3:       <p>        (first child of div)
    /// 4:       <span>     (second child)
    /// 5:       <p>        (third child, second p)
    /// ```
    fn build_basic_dom() -> Vec<Node> {
        let mut nodes = vec![
            doc_node(),   // 0
            elem("html"), // 1
            elem("div"),  // 2
            elem("p"),    // 3
            elem("span"), // 4
            elem("p"),    // 5
        ];
        link(&mut nodes, 0, 1);
        link(&mut nodes, 1, 2);
        link(&mut nodes, 2, 3);
        link(&mut nodes, 2, 4);
        link(&mut nodes, 2, 5);
        nodes
    }

    // ===============================================================
    // AnB parsing tests
    // ===============================================================

    #[test]
    fn anb_parse_odd() {
        let anb = AnB::parse("odd").unwrap();
        assert_eq!(anb, AnB { a: 2, b: 1 });
    }

    #[test]
    fn anb_parse_even() {
        let anb = AnB::parse("even").unwrap();
        assert_eq!(anb, AnB { a: 2, b: 0 });
    }

    #[test]
    fn anb_parse_plain_number() {
        let anb = AnB::parse("3").unwrap();
        assert_eq!(anb, AnB { a: 0, b: 3 });
    }

    #[test]
    fn anb_parse_2n_plus_1() {
        let anb = AnB::parse("2n+1").unwrap();
        assert_eq!(anb, AnB { a: 2, b: 1 });
    }

    #[test]
    fn anb_parse_neg_n_plus_3() {
        let anb = AnB::parse("-n+3").unwrap();
        assert_eq!(anb, AnB { a: -1, b: 3 });
    }

    #[test]
    fn anb_parse_n_alone() {
        let anb = AnB::parse("n").unwrap();
        assert_eq!(anb, AnB { a: 1, b: 0 });
    }

    #[test]
    fn anb_parse_3n() {
        let anb = AnB::parse("3n").unwrap();
        assert_eq!(anb, AnB { a: 3, b: 0 });
    }

    #[test]
    fn anb_parse_neg_2n_plus_5() {
        let anb = AnB::parse("-2n+5").unwrap();
        assert_eq!(anb, AnB { a: -2, b: 5 });
    }

    #[test]
    fn anb_parse_0n_plus_3() {
        let anb = AnB::parse("0n+3").unwrap();
        assert_eq!(anb, AnB { a: 0, b: 3 });
    }

    #[test]
    fn anb_parse_empty_returns_none() {
        assert!(AnB::parse("").is_none());
    }

    #[test]
    fn anb_parse_garbage_returns_none() {
        assert!(AnB::parse("abc").is_none());
    }

    #[test]
    fn anb_parse_with_spaces() {
        let anb = AnB::parse("  2n + 1  ").unwrap();
        assert_eq!(anb, AnB { a: 2, b: 1 });
    }

    #[test]
    fn anb_parse_negative_b() {
        let anb = AnB::parse("3n-2").unwrap();
        assert_eq!(anb, AnB { a: 3, b: -2 });
    }

    // ===============================================================
    // AnB matching tests
    // ===============================================================

    #[test]
    fn anb_matches_odd() {
        let anb = AnB { a: 2, b: 1 };
        assert!(anb.matches(1));
        assert!(!anb.matches(2));
        assert!(anb.matches(3));
        assert!(!anb.matches(4));
        assert!(anb.matches(5));
    }

    #[test]
    fn anb_matches_even() {
        let anb = AnB { a: 2, b: 0 };
        assert!(!anb.matches(1));
        assert!(anb.matches(2));
        assert!(!anb.matches(3));
        assert!(anb.matches(4));
    }

    #[test]
    fn anb_matches_plain_number() {
        let anb = AnB { a: 0, b: 3 };
        assert!(!anb.matches(1));
        assert!(!anb.matches(2));
        assert!(anb.matches(3));
        assert!(!anb.matches(4));
    }

    #[test]
    fn anb_matches_neg_n_plus_3() {
        // -n+3 matches 3, 2, 1 (n=0 -> 3, n=1 -> 2, n=2 -> 1)
        let anb = AnB { a: -1, b: 3 };
        assert!(anb.matches(1));
        assert!(anb.matches(2));
        assert!(anb.matches(3));
        assert!(!anb.matches(4));
        assert!(!anb.matches(5));
    }

    #[test]
    fn anb_matches_3n() {
        // 3n matches 3, 6, 9, ...
        let anb = AnB { a: 3, b: 0 };
        assert!(!anb.matches(1));
        assert!(!anb.matches(2));
        assert!(anb.matches(3));
        assert!(!anb.matches(4));
        assert!(anb.matches(6));
        assert!(anb.matches(9));
    }

    #[test]
    fn anb_matches_zero_index() {
        // An+B with index 0 (not a valid 1-based index).
        let anb = AnB { a: 1, b: 0 };
        // n=0 -> 0, which matches index 0.
        assert!(anb.matches(0));
    }

    #[test]
    fn anb_matches_neg_2n_plus_5() {
        // -2n+5 matches 5, 3, 1
        let anb = AnB { a: -2, b: 5 };
        assert!(anb.matches(5)); // n=0
        assert!(anb.matches(3)); // n=1
        assert!(anb.matches(1)); // n=2
        assert!(!anb.matches(2));
        assert!(!anb.matches(4));
        assert!(!anb.matches(6));
    }

    // ===============================================================
    // :first-child tests
    // ===============================================================

    #[test]
    fn first_child_basic() {
        let nodes = build_basic_dom();
        // Node 3 (<p>) is first element child of <div>.
        assert!(matches_first_child(&nodes, 3));
        assert!(!matches_first_child(&nodes, 4));
        assert!(!matches_first_child(&nodes, 5));
    }

    #[test]
    fn first_child_skips_text() {
        let mut nodes = vec![doc_node(), elem("div"), text("hello"), elem("p")];
        link(&mut nodes, 0, 1);
        link(&mut nodes, 1, 2); // text
        link(&mut nodes, 1, 3); // <p>
        // <p> is first element child even though text comes first.
        assert!(matches_first_child(&nodes, 3));
    }

    #[test]
    fn first_child_no_parent() {
        let nodes = vec![elem("div")];
        assert!(!matches_first_child(&nodes, 0));
    }

    // ===============================================================
    // :last-child tests
    // ===============================================================

    #[test]
    fn last_child_basic() {
        let nodes = build_basic_dom();
        // Node 5 (<p>) is last element child of <div>.
        assert!(!matches_last_child(&nodes, 3));
        assert!(!matches_last_child(&nodes, 4));
        assert!(matches_last_child(&nodes, 5));
    }

    #[test]
    fn last_child_skips_text() {
        let mut nodes = vec![doc_node(), elem("div"), elem("p"), text("trailing")];
        link(&mut nodes, 0, 1);
        link(&mut nodes, 1, 2); // <p>
        link(&mut nodes, 1, 3); // text
        // <p> is last element child.
        assert!(matches_last_child(&nodes, 2));
    }

    #[test]
    fn last_child_no_parent() {
        let nodes = vec![elem("div")];
        assert!(!matches_last_child(&nodes, 0));
    }

    // ===============================================================
    // :first-of-type tests
    // ===============================================================

    #[test]
    fn first_of_type_basic() {
        let nodes = build_basic_dom();
        // Node 3 is first <p>, node 4 is first (and only) <span>.
        assert!(matches_first_of_type(&nodes, 3));
        assert!(matches_first_of_type(&nodes, 4));
        assert!(!matches_first_of_type(&nodes, 5)); // second <p>
    }

    #[test]
    fn first_of_type_mixed() {
        let mut nodes = vec![doc_node(), elem("ul"), elem("li"), elem("span"), elem("li")];
        link(&mut nodes, 0, 1);
        link(&mut nodes, 1, 2);
        link(&mut nodes, 1, 3);
        link(&mut nodes, 1, 4);
        assert!(matches_first_of_type(&nodes, 2)); // first <li>
        assert!(matches_first_of_type(&nodes, 3)); // first <span>
        assert!(!matches_first_of_type(&nodes, 4)); // second <li>
    }

    // ===============================================================
    // :last-of-type tests
    // ===============================================================

    #[test]
    fn last_of_type_basic() {
        let nodes = build_basic_dom();
        // Node 5 is last <p>, node 4 is last (only) <span>.
        assert!(!matches_last_of_type(&nodes, 3)); // first <p>
        assert!(matches_last_of_type(&nodes, 4));
        assert!(matches_last_of_type(&nodes, 5)); // last <p>
    }

    #[test]
    fn last_of_type_mixed() {
        let mut nodes = vec![doc_node(), elem("ul"), elem("li"), elem("span"), elem("li")];
        link(&mut nodes, 0, 1);
        link(&mut nodes, 1, 2);
        link(&mut nodes, 1, 3);
        link(&mut nodes, 1, 4);
        assert!(!matches_last_of_type(&nodes, 2)); // first <li>
        assert!(matches_last_of_type(&nodes, 3)); // last <span>
        assert!(matches_last_of_type(&nodes, 4)); // last <li>
    }

    // ===============================================================
    // :only-child tests
    // ===============================================================

    #[test]
    fn only_child_true() {
        let mut nodes = vec![doc_node(), elem("div"), elem("p")];
        link(&mut nodes, 0, 1);
        link(&mut nodes, 1, 2);
        assert!(matches_only_child(&nodes, 2));
    }

    #[test]
    fn only_child_false() {
        let nodes = build_basic_dom();
        // <div> has 3 element children.
        assert!(!matches_only_child(&nodes, 3));
        assert!(!matches_only_child(&nodes, 4));
        assert!(!matches_only_child(&nodes, 5));
    }

    #[test]
    fn only_child_ignores_text_siblings() {
        let mut nodes = vec![
            doc_node(),
            elem("div"),
            text("hello"),
            elem("p"),
            text("world"),
        ];
        link(&mut nodes, 0, 1);
        link(&mut nodes, 1, 2);
        link(&mut nodes, 1, 3);
        link(&mut nodes, 1, 4);
        // <p> is the only element child.
        assert!(matches_only_child(&nodes, 3));
    }

    // ===============================================================
    // :only-of-type tests
    // ===============================================================

    #[test]
    fn only_of_type_true() {
        let nodes = build_basic_dom();
        // <span> (node 4) is the only span.
        assert!(matches_only_of_type(&nodes, 4));
    }

    #[test]
    fn only_of_type_false() {
        let nodes = build_basic_dom();
        // Two <p> elements exist (3 and 5).
        assert!(!matches_only_of_type(&nodes, 3));
        assert!(!matches_only_of_type(&nodes, 5));
    }

    // ===============================================================
    // :empty tests
    // ===============================================================

    #[test]
    fn empty_truly_empty() {
        let mut nodes = vec![doc_node(), elem("div")];
        link(&mut nodes, 0, 1);
        assert!(matches_empty(&nodes, 1));
    }

    #[test]
    fn empty_with_whitespace_text() {
        let mut nodes = vec![doc_node(), elem("div"), text("   \n\t  ")];
        link(&mut nodes, 0, 1);
        link(&mut nodes, 1, 2);
        // Whitespace-only text counts as empty per CSS spec.
        assert!(matches_empty(&nodes, 1));
    }

    #[test]
    fn empty_with_real_text() {
        let mut nodes = vec![doc_node(), elem("div"), text("hello")];
        link(&mut nodes, 0, 1);
        link(&mut nodes, 1, 2);
        assert!(!matches_empty(&nodes, 1));
    }

    #[test]
    fn empty_with_child_element() {
        let mut nodes = vec![doc_node(), elem("div"), elem("span")];
        link(&mut nodes, 0, 1);
        link(&mut nodes, 1, 2);
        assert!(!matches_empty(&nodes, 1));
    }

    #[test]
    fn empty_with_comment_only() {
        let mut nodes = vec![doc_node(), elem("div"), comment("a comment")];
        link(&mut nodes, 0, 1);
        link(&mut nodes, 1, 2);
        // Comments don't prevent :empty matching.
        assert!(matches_empty(&nodes, 1));
    }

    // ===============================================================
    // :nth-child tests
    // ===============================================================

    #[test]
    fn nth_child_odd() {
        let nodes = build_basic_dom();
        // Element children of <div>: p(3)=1st, span(4)=2nd, p(5)=3rd
        assert!(matches_nth_child(&nodes, 3, "odd")); // 1st
        assert!(!matches_nth_child(&nodes, 4, "odd")); // 2nd
        assert!(matches_nth_child(&nodes, 5, "odd")); // 3rd
    }

    #[test]
    fn nth_child_even() {
        let nodes = build_basic_dom();
        assert!(!matches_nth_child(&nodes, 3, "even")); // 1st
        assert!(matches_nth_child(&nodes, 4, "even")); // 2nd
        assert!(!matches_nth_child(&nodes, 5, "even")); // 3rd
    }

    #[test]
    fn nth_child_specific_number() {
        let nodes = build_basic_dom();
        assert!(matches_nth_child(&nodes, 3, "1"));
        assert!(matches_nth_child(&nodes, 4, "2"));
        assert!(matches_nth_child(&nodes, 5, "3"));
        assert!(!matches_nth_child(&nodes, 3, "2"));
    }

    #[test]
    fn nth_child_formula() {
        let nodes = build_basic_dom();
        // 2n+1 = odd
        assert!(matches_nth_child(&nodes, 3, "2n+1"));
        assert!(!matches_nth_child(&nodes, 4, "2n+1"));
        assert!(matches_nth_child(&nodes, 5, "2n+1"));
    }

    // ===============================================================
    // :nth-last-child tests
    // ===============================================================

    #[test]
    fn nth_last_child_basic() {
        let nodes = build_basic_dom();
        // From end: p(5)=1st, span(4)=2nd, p(3)=3rd
        assert!(matches_nth_last_child(&nodes, 5, "1"));
        assert!(matches_nth_last_child(&nodes, 4, "2"));
        assert!(matches_nth_last_child(&nodes, 3, "3"));
    }

    #[test]
    fn nth_last_child_odd() {
        let nodes = build_basic_dom();
        assert!(matches_nth_last_child(&nodes, 5, "odd")); // 1st from end
        assert!(!matches_nth_last_child(&nodes, 4, "odd")); // 2nd from end
        assert!(matches_nth_last_child(&nodes, 3, "odd")); // 3rd from end
    }

    // ===============================================================
    // :nth-of-type tests
    // ===============================================================

    #[test]
    fn nth_of_type_basic() {
        let nodes = build_basic_dom();
        // <p> elements: node 3 is 1st <p>, node 5 is 2nd <p>.
        assert!(matches_nth_of_type(&nodes, 3, "1"));
        assert!(matches_nth_of_type(&nodes, 5, "2"));
        assert!(!matches_nth_of_type(&nodes, 3, "2"));
    }

    #[test]
    fn nth_of_type_odd() {
        let nodes = build_basic_dom();
        assert!(matches_nth_of_type(&nodes, 3, "odd")); // 1st <p>
        assert!(matches_nth_of_type(&nodes, 5, "even")); // 2nd <p>
    }

    #[test]
    fn nth_of_type_only_span() {
        let nodes = build_basic_dom();
        // <span> is the only one, so it's 1st of type.
        assert!(matches_nth_of_type(&nodes, 4, "1"));
        assert!(!matches_nth_of_type(&nodes, 4, "2"));
    }

    // ===============================================================
    // :nth-last-of-type tests
    // ===============================================================

    #[test]
    fn nth_last_of_type_basic() {
        let nodes = build_basic_dom();
        // <p> elements from end: node 5 = 1st, node 3 = 2nd.
        assert!(matches_nth_last_of_type(&nodes, 5, "1"));
        assert!(matches_nth_last_of_type(&nodes, 3, "2"));
        assert!(!matches_nth_last_of_type(&nodes, 5, "2"));
    }

    #[test]
    fn nth_last_of_type_even() {
        let nodes = build_basic_dom();
        // 2 <p> elements. From end: 5 is 1st (odd), 3 is 2nd (even).
        assert!(!matches_nth_last_of_type(&nodes, 5, "even"));
        assert!(matches_nth_last_of_type(&nodes, 3, "even"));
    }

    // ===============================================================
    // :root tests
    // ===============================================================

    #[test]
    fn root_matches_html() {
        let nodes = build_basic_dom();
        // Node 1 (<html>) has Document (node 0) as parent.
        assert!(matches_root(&nodes, 1));
    }

    #[test]
    fn root_does_not_match_child() {
        let nodes = build_basic_dom();
        // Node 2 (<div>) has <html> as parent, not Document.
        assert!(!matches_root(&nodes, 2));
        assert!(!matches_root(&nodes, 3));
    }

    #[test]
    fn root_no_parent() {
        let nodes = vec![elem("html")];
        // No parent at all, treated as root.
        assert!(matches_root(&nodes, 0));
    }

    // ===============================================================
    // :enabled / :disabled tests
    // ===============================================================

    #[test]
    fn enabled_input() {
        let mut nodes = vec![doc_node(), elem("input")];
        link(&mut nodes, 0, 1);
        assert!(matches_enabled(&nodes, 1));
        assert!(!matches_disabled(&nodes, 1));
    }

    #[test]
    fn disabled_input() {
        let mut nodes = vec![doc_node(), elem_with_attrs("input", &[("disabled", "")])];
        link(&mut nodes, 0, 1);
        assert!(!matches_enabled(&nodes, 1));
        assert!(matches_disabled(&nodes, 1));
    }

    #[test]
    fn enabled_not_form_element() {
        let mut nodes = vec![doc_node(), elem("div")];
        link(&mut nodes, 0, 1);
        // <div> is not a form element.
        assert!(!matches_enabled(&nodes, 1));
        assert!(!matches_disabled(&nodes, 1));
    }

    #[test]
    fn enabled_button() {
        let mut nodes = vec![doc_node(), elem("button")];
        link(&mut nodes, 0, 1);
        assert!(matches_enabled(&nodes, 1));
    }

    #[test]
    fn disabled_select() {
        let mut nodes = vec![doc_node(), elem_with_attrs("select", &[("disabled", "")])];
        link(&mut nodes, 0, 1);
        assert!(matches_disabled(&nodes, 1));
    }

    #[test]
    fn disabled_textarea() {
        let mut nodes = vec![doc_node(), elem_with_attrs("textarea", &[("disabled", "")])];
        link(&mut nodes, 0, 1);
        assert!(matches_disabled(&nodes, 1));
    }

    // ===============================================================
    // :checked tests
    // ===============================================================

    #[test]
    fn checked_input() {
        let mut nodes = vec![
            doc_node(),
            elem_with_attrs("input", &[("type", "checkbox"), ("checked", "")]),
        ];
        link(&mut nodes, 0, 1);
        assert!(matches_checked(&nodes, 1));
    }

    #[test]
    fn unchecked_input() {
        let mut nodes = vec![
            doc_node(),
            elem_with_attrs("input", &[("type", "checkbox")]),
        ];
        link(&mut nodes, 0, 1);
        assert!(!matches_checked(&nodes, 1));
    }

    #[test]
    fn checked_option() {
        let mut nodes = vec![doc_node(), elem_with_attrs("option", &[("selected", "")])];
        link(&mut nodes, 0, 1);
        assert!(matches_checked(&nodes, 1));
    }

    #[test]
    fn checked_non_input() {
        let mut nodes = vec![doc_node(), elem("div")];
        link(&mut nodes, 0, 1);
        assert!(!matches_checked(&nodes, 1));
    }

    // ===============================================================
    // Dispatcher tests
    // ===============================================================

    #[test]
    fn dispatcher_simple_pseudo_classes() {
        let nodes = build_basic_dom();
        assert!(matches_pseudo_class(&nodes, 3, "first-child"));
        assert!(!matches_pseudo_class(&nodes, 5, "first-child"));
        assert!(matches_pseudo_class(&nodes, 5, "last-child"));
        assert!(matches_pseudo_class(&nodes, 4, "only-of-type"));
        assert!(matches_pseudo_class(&nodes, 1, "root"));
    }

    #[test]
    fn dispatcher_unknown_returns_false() {
        let nodes = build_basic_dom();
        assert!(!matches_pseudo_class(&nodes, 3, "nonexistent"));
    }

    #[test]
    fn dispatcher_fn_nth_child() {
        let nodes = build_basic_dom();
        assert!(matches_pseudo_class_fn(&nodes, 3, "nth-child", "1"));
        assert!(matches_pseudo_class_fn(&nodes, 4, "nth-child", "even"));
    }

    #[test]
    fn dispatcher_fn_nth_last_child() {
        let nodes = build_basic_dom();
        assert!(matches_pseudo_class_fn(&nodes, 5, "nth-last-child", "1"));
    }

    #[test]
    fn dispatcher_fn_nth_of_type() {
        let nodes = build_basic_dom();
        assert!(matches_pseudo_class_fn(&nodes, 5, "nth-of-type", "2"));
    }

    #[test]
    fn dispatcher_fn_nth_last_of_type() {
        let nodes = build_basic_dom();
        assert!(matches_pseudo_class_fn(&nodes, 3, "nth-last-of-type", "2"));
    }

    #[test]
    fn dispatcher_fn_unknown_returns_false() {
        let nodes = build_basic_dom();
        assert!(!matches_pseudo_class_fn(&nodes, 3, "nonexistent", "1"));
    }

    // ===============================================================
    // Edge case tests
    // ===============================================================

    #[test]
    fn single_child_is_first_and_last() {
        let mut nodes = vec![doc_node(), elem("div"), elem("p")];
        link(&mut nodes, 0, 1);
        link(&mut nodes, 1, 2);
        assert!(matches_first_child(&nodes, 2));
        assert!(matches_last_child(&nodes, 2));
        assert!(matches_only_child(&nodes, 2));
    }

    #[test]
    fn text_node_never_matches() {
        let mut nodes = vec![doc_node(), elem("div"), text("hello")];
        link(&mut nodes, 0, 1);
        link(&mut nodes, 1, 2);
        assert!(!matches_first_child(&nodes, 2));
        assert!(!matches_last_child(&nodes, 2));
        assert!(!matches_empty(&nodes, 2));
        assert!(!matches_root(&nodes, 2));
    }

    #[test]
    fn has_attribute_helper() {
        let nodes = vec![elem_with_attrs(
            "input",
            &[("type", "text"), ("disabled", "")],
        )];
        assert!(has_attribute(&nodes, 0, "type"));
        assert!(has_attribute(&nodes, 0, "disabled"));
        assert!(!has_attribute(&nodes, 0, "checked"));
    }

    #[test]
    fn element_tag_helper() {
        let nodes = vec![elem("div"), text("hello")];
        assert_eq!(element_tag(&nodes, 0), Some("div"));
        assert_eq!(element_tag(&nodes, 1), None);
    }

    #[test]
    fn element_siblings_helper() {
        let nodes = build_basic_dom();
        let sibs = element_siblings(&nodes, 3);
        assert_eq!(sibs, vec![3, 4, 5]);
    }

    #[test]
    fn element_siblings_no_parent() {
        let nodes = vec![elem("div")];
        assert!(element_siblings(&nodes, 0).is_empty());
    }

    #[test]
    fn nth_child_with_interleaved_text() {
        let mut nodes = vec![
            doc_node(),
            elem("ul"),
            text("  "),
            elem("li"), // 3
            text("  "),
            elem("li"), // 5
            text("  "),
            elem("li"), // 7
        ];
        link(&mut nodes, 0, 1);
        link(&mut nodes, 1, 2);
        link(&mut nodes, 1, 3);
        link(&mut nodes, 1, 4);
        link(&mut nodes, 1, 5);
        link(&mut nodes, 1, 6);
        link(&mut nodes, 1, 7);
        // Text nodes are skipped; element positions are 1, 2, 3.
        assert!(matches_nth_child(&nodes, 3, "1"));
        assert!(matches_nth_child(&nodes, 5, "2"));
        assert!(matches_nth_child(&nodes, 7, "3"));
    }

    #[test]
    fn empty_with_whitespace_and_comment() {
        let mut nodes = vec![doc_node(), elem("div"), text("  "), comment("note")];
        link(&mut nodes, 0, 1);
        link(&mut nodes, 1, 2);
        link(&mut nodes, 1, 3);
        // Both whitespace text and comments are ignored.
        assert!(matches_empty(&nodes, 1));
    }

    #[test]
    fn first_of_type_no_parent() {
        let nodes = vec![elem("div")];
        assert!(!matches_first_of_type(&nodes, 0));
    }

    #[test]
    fn last_of_type_no_parent() {
        let nodes = vec![elem("div")];
        assert!(!matches_last_of_type(&nodes, 0));
    }

    #[test]
    fn only_of_type_no_parent() {
        let nodes = vec![elem("div")];
        assert!(!matches_only_of_type(&nodes, 0));
    }
}
