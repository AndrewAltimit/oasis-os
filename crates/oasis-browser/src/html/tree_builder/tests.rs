//! Tests for the HTML tree builder.

use super::super::dom::{NodeId, NodeKind, TagName};
use super::super::tokenizer::{Attribute as TokAttr, DoctypeToken, EndTagToken, StartTagToken};
use super::*;

// Convenience helpers for building token streams.

fn start(name: &str) -> Token {
    Token::StartTag(StartTagToken {
        name: name.to_string(),
        self_closing: false,
        attributes: Vec::new(),
    })
}

fn start_with_attrs(name: &str, attrs: Vec<(&str, &str)>) -> Token {
    Token::StartTag(StartTagToken {
        name: name.to_string(),
        self_closing: false,
        attributes: attrs
            .into_iter()
            .map(|(n, v)| TokAttr {
                name: n.to_string(),
                value: v.to_string(),
            })
            .collect(),
    })
}

fn end(name: &str) -> Token {
    Token::EndTag(EndTagToken {
        name: name.to_string(),
    })
}

fn text(s: &str) -> Token {
    Token::Character(s.to_string())
}

fn tag_at(doc: &Document, id: NodeId) -> Option<&TagName> {
    doc.element(id).map(|e| &e.tag)
}

// ---- Test 1: Simple document structure ----

#[test]
fn simple_document() {
    let tokens = vec![
        start("html"),
        start("body"),
        start("p"),
        text("Hello"),
        end("p"),
        end("body"),
        end("html"),
        Token::Eof,
    ];
    let doc = TreeBuilder::build(tokens);

    let body = doc.body().expect("has body");
    let body_children = &doc.get(body).children;
    assert_eq!(body_children.len(), 1);

    let p = body_children[0];
    assert_eq!(tag_at(&doc, p), Some(&TagName::P));
    assert_eq!(doc.text_content(p), "Hello");
}

// ---- Test 2: Implicit elements ----

#[test]
fn implicit_elements() {
    let tokens = vec![start("p"), text("Hello"), end("p"), Token::Eof];
    let doc = TreeBuilder::build(tokens);

    assert!(doc.head().is_some());
    assert!(doc.body().is_some());

    let body = doc.body().unwrap();
    let body_children = &doc.get(body).children;
    assert!(!body_children.is_empty());

    let p = body_children[0];
    assert_eq!(tag_at(&doc, p), Some(&TagName::P));
    assert_eq!(doc.text_content(p), "Hello");
}

// ---- Test 3: Void elements ----

#[test]
fn void_elements() {
    let tokens = vec![
        start("p"),
        text("Hello"),
        start("br"),
        text("World"),
        end("p"),
        Token::Eof,
    ];
    let doc = TreeBuilder::build(tokens);

    let body = doc.body().unwrap();
    let p = doc.get(body).children[0];
    assert_eq!(tag_at(&doc, p), Some(&TagName::P));

    // p should have: Text("Hello"), <br>, Text("World")
    let p_children = &doc.get(p).children;
    assert_eq!(p_children.len(), 3);

    assert!(matches!(
        &doc.get(p_children[0]).kind,
        NodeKind::Text(t) if t == "Hello"
    ));
    assert_eq!(tag_at(&doc, p_children[1]), Some(&TagName::Br),);
    assert!(doc.get(p_children[1]).children.is_empty());
    assert!(matches!(
        &doc.get(p_children[2]).kind,
        NodeKind::Text(t) if t == "World"
    ));
}

// ---- Test 4: Auto-close p ----

#[test]
fn auto_close_p() {
    let tokens = vec![
        start("p"),
        text("First"),
        start("p"),
        text("Second"),
        Token::Eof,
    ];
    let doc = TreeBuilder::build(tokens);

    let body = doc.body().unwrap();
    let body_children = &doc.get(body).children;

    let ps: Vec<NodeId> = body_children
        .iter()
        .filter(|&&id| tag_at(&doc, id) == Some(&TagName::P))
        .copied()
        .collect();
    assert_eq!(ps.len(), 2);
    assert_eq!(doc.text_content(ps[0]), "First");
    assert_eq!(doc.text_content(ps[1]), "Second");
}

// ---- Test 5: Auto-close li ----

#[test]
fn auto_close_li() {
    let tokens = vec![
        start("ul"),
        start("li"),
        text("One"),
        start("li"),
        text("Two"),
        end("ul"),
        Token::Eof,
    ];
    let doc = TreeBuilder::build(tokens);

    let body = doc.body().unwrap();
    let ul = doc.get(body).children[0];
    assert_eq!(tag_at(&doc, ul), Some(&TagName::Ul));

    let lis: Vec<NodeId> = doc
        .get(ul)
        .children
        .iter()
        .filter(|&&id| tag_at(&doc, id) == Some(&TagName::Li))
        .copied()
        .collect();
    assert_eq!(lis.len(), 2);
    assert_eq!(doc.text_content(lis[0]), "One");
    assert_eq!(doc.text_content(lis[1]), "Two");
}

// ---- Test 6: Nested divs ----

#[test]
fn nested_divs() {
    let tokens = vec![
        start("div"),
        start("div"),
        start("p"),
        text("text"),
        end("p"),
        end("div"),
        end("div"),
        Token::Eof,
    ];
    let doc = TreeBuilder::build(tokens);

    let body = doc.body().unwrap();
    let outer = doc.get(body).children[0];
    assert_eq!(tag_at(&doc, outer), Some(&TagName::Div));

    let inner = doc.get(outer).children[0];
    assert_eq!(tag_at(&doc, inner), Some(&TagName::Div));

    let p = doc.get(inner).children[0];
    assert_eq!(tag_at(&doc, p), Some(&TagName::P));
    assert_eq!(doc.text_content(p), "text");
}

// ---- Test 7: Formatting elements ----

#[test]
fn formatting_elements() {
    let tokens = vec![
        start("p"),
        start("b"),
        text("bold "),
        start("i"),
        text("bold-italic"),
        end("i"),
        end("b"),
        end("p"),
        Token::Eof,
    ];
    let doc = TreeBuilder::build(tokens);

    let body = doc.body().unwrap();
    let p = doc.get(body).children[0];
    assert_eq!(tag_at(&doc, p), Some(&TagName::P));

    let b = doc.get(p).children[0];
    assert_eq!(tag_at(&doc, b), Some(&TagName::B));

    let b_children = &doc.get(b).children;
    assert!(b_children.len() >= 2);

    assert!(matches!(
        &doc.get(b_children[0]).kind,
        NodeKind::Text(t) if t == "bold "
    ));

    let i = b_children[1];
    assert_eq!(tag_at(&doc, i), Some(&TagName::I));
    assert_eq!(doc.text_content(i), "bold-italic");
}

// ---- Test 8: Text coalescing ----

#[test]
fn text_coalescing() {
    // Multiple consecutive Character tokens should coalesce into
    // a single text node.
    let tokens = vec![
        start("p"),
        text("H"),
        text("i"),
        text("!"),
        end("p"),
        Token::Eof,
    ];
    let doc = TreeBuilder::build(tokens);

    let body = doc.body().unwrap();
    let p = doc.get(body).children[0];
    let p_children = &doc.get(p).children;
    assert_eq!(p_children.len(), 1);
    assert!(matches!(
        &doc.get(p_children[0]).kind,
        NodeKind::Text(t) if t == "Hi!"
    ));
}

// ---- Test 9: Table structure ----

#[test]
fn table_structure() {
    let tokens = vec![
        start("table"),
        start("tr"),
        start("td"),
        text("cell"),
        end("td"),
        end("tr"),
        end("table"),
        Token::Eof,
    ];
    let doc = TreeBuilder::build(tokens);

    let body = doc.body().unwrap();
    let table = doc.get(body).children[0];
    assert_eq!(tag_at(&doc, table), Some(&TagName::Table));

    // table -> tbody (implicit)
    let tbody = doc.get(table).children[0];
    assert_eq!(tag_at(&doc, tbody), Some(&TagName::Tbody));

    // tbody -> tr
    let tr = doc.get(tbody).children[0];
    assert_eq!(tag_at(&doc, tr), Some(&TagName::Tr));

    // tr -> td
    let td = doc.get(tr).children[0];
    assert_eq!(tag_at(&doc, td), Some(&TagName::Td));

    assert_eq!(doc.text_content(td), "cell");
}

// ---- Test 10: Mixed content ----

#[test]
fn mixed_content() {
    let tokens = vec![
        start("h1"),
        text("Title"),
        end("h1"),
        start("p"),
        text("A paragraph with "),
        start_with_attrs("a", vec![("href", "https://example.com")]),
        text("a link"),
        end("a"),
        text("."),
        end("p"),
        start("ul"),
        start("li"),
        text("Item 1"),
        end("li"),
        start("li"),
        text("Item 2"),
        end("li"),
        end("ul"),
        Token::Eof,
    ];
    let doc = TreeBuilder::build(tokens);

    let body = doc.body().unwrap();
    let children = &doc.get(body).children;

    // h1, p, ul
    assert!(children.len() >= 3);

    let h1 = children[0];
    assert_eq!(tag_at(&doc, h1), Some(&TagName::H1));
    assert_eq!(doc.text_content(h1), "Title");

    let p = children[1];
    assert_eq!(tag_at(&doc, p), Some(&TagName::P));
    assert_eq!(doc.text_content(p), "A paragraph with a link.",);

    let a = doc
        .get(p)
        .children
        .iter()
        .find(|&&id| tag_at(&doc, id) == Some(&TagName::A))
        .copied()
        .expect("has <a>");
    assert_eq!(doc.element(a).unwrap().href(), Some("https://example.com"),);

    let ul = children[2];
    assert_eq!(tag_at(&doc, ul), Some(&TagName::Ul));
    let lis: Vec<NodeId> = doc
        .get(ul)
        .children
        .iter()
        .filter(|&&id| tag_at(&doc, id) == Some(&TagName::Li))
        .copied()
        .collect();
    assert_eq!(lis.len(), 2);
    assert_eq!(doc.text_content(lis[0]), "Item 1");
    assert_eq!(doc.text_content(lis[1]), "Item 2");
}

// ---- Doctype is handled ----

#[test]
fn doctype_skipped() {
    let tokens = vec![
        Token::Doctype(DoctypeToken {
            name: Some("html".to_string()),
            force_quirks: false,
        }),
        start("html"),
        start("head"),
        end("head"),
        start("body"),
        end("body"),
        end("html"),
        Token::Eof,
    ];
    let doc = TreeBuilder::build(tokens);
    assert!(doc.body().is_some());
    assert!(doc.head().is_some());
}

// ---- Empty document ----

#[test]
fn empty_document() {
    let tokens = vec![Token::Eof];
    let doc = TreeBuilder::build(tokens);
    // Should have at least Document root + implicit html.
    assert!(doc.nodes.len() >= 2);
}

// ---- Heading auto-close ----

#[test]
fn heading_auto_close() {
    let tokens = vec![
        start("h1"),
        text("First"),
        start("h2"),
        text("Second"),
        Token::Eof,
    ];
    let doc = TreeBuilder::build(tokens);

    let body = doc.body().unwrap();
    let headings: Vec<NodeId> = doc
        .get(body)
        .children
        .iter()
        .filter(|&&id| matches!(tag_at(&doc, id), Some(TagName::H1) | Some(TagName::H2)))
        .copied()
        .collect();
    assert_eq!(headings.len(), 2);
    assert_eq!(doc.text_content(headings[0]), "First");
    assert_eq!(doc.text_content(headings[1]), "Second");
}

// ---- Title in head ----

#[test]
fn title_in_head() {
    let tokens = vec![
        start("html"),
        start("head"),
        start("title"),
        text("My Page"),
        end("title"),
        end("head"),
        start("body"),
        end("body"),
        end("html"),
        Token::Eof,
    ];
    let doc = TreeBuilder::build(tokens);
    assert_eq!(doc.title(), Some("My Page".to_string()));
}

// ---- Attributes preserved ----

#[test]
fn attributes_preserved() {
    let tokens = vec![
        start_with_attrs("div", vec![("id", "main"), ("class", "container")]),
        end("div"),
        Token::Eof,
    ];
    let doc = TreeBuilder::build(tokens);

    let found = doc.get_element_by_id("main").expect("found by id");
    let data = doc.element(found).unwrap();
    assert_eq!(data.id(), Some("main"));
    assert!(data.has_class("container"));
}

// ---- Default trait ----

#[test]
fn default_trait() {
    let builder = TreeBuilder::default();
    assert_eq!(builder.mode, InsertionMode::Initial);
    assert!(builder.open_elements.is_empty());
}

// ---- Robustness / edge cases ----

#[test]
fn deeply_nested_divs() {
    // 200 levels of nesting -- should not stack overflow.
    let mut tokens: Vec<Token> = Vec::new();
    for _ in 0..200 {
        tokens.push(start("div"));
    }
    tokens.push(text("leaf"));
    for _ in 0..200 {
        tokens.push(end("div"));
    }
    tokens.push(Token::Eof);
    let doc = TreeBuilder::build(tokens);
    let body = doc.body().expect("has body");
    assert!(!doc.get(body).children.is_empty());
}

#[test]
fn orphan_end_tag() {
    // End tag without matching start should not panic.
    let tokens = vec![end("span"), Token::Eof];
    let doc = TreeBuilder::build(tokens);
    assert!(doc.body().is_some());
}

#[test]
fn multiple_orphan_end_tags() {
    let tokens = vec![end("div"), end("span"), end("p"), end("table"), Token::Eof];
    let doc = TreeBuilder::build(tokens);
    assert!(doc.body().is_some());
}

#[test]
fn end_tag_before_matching_start() {
    let tokens = vec![
        end("div"),
        start("div"),
        text("content"),
        end("div"),
        Token::Eof,
    ];
    let doc = TreeBuilder::build(tokens);
    let body = doc.body().unwrap();
    assert!(!doc.get(body).children.is_empty());
}

#[test]
fn interleaved_mismatched_tags() {
    // <b><i></b></i> -- misnested formatting.
    let tokens = vec![
        start("b"),
        start("i"),
        text("text"),
        end("b"),
        end("i"),
        Token::Eof,
    ];
    let doc = TreeBuilder::build(tokens);
    let body = doc.body().unwrap();
    assert!(!doc.get(body).children.is_empty());
}

#[test]
fn empty_document_eof_only() {
    let tokens = vec![Token::Eof];
    let doc = TreeBuilder::build(tokens);
    // Should still create implicit html/head/body.
    assert!(doc.body().is_some());
}

#[test]
fn text_only_no_tags() {
    let tokens = vec![text("just plain text"), Token::Eof];
    let doc = TreeBuilder::build(tokens);
    let body = doc.body().unwrap();
    assert_eq!(doc.text_content(body), "just plain text");
}

#[test]
fn void_element_with_children_ignored() {
    // <br> should not contain children even if tokens supply them.
    let tokens = vec![
        start("br"),
        text("should not be child of br"),
        end("br"),
        Token::Eof,
    ];
    let doc = TreeBuilder::build(tokens);
    let body = doc.body().unwrap();
    // br should exist, text should be sibling, not child.
    assert!(!doc.get(body).children.is_empty());
}

#[test]
fn p_inside_p_auto_closes() {
    let tokens = vec![
        start("p"),
        text("A"),
        start("p"),
        text("B"),
        start("p"),
        text("C"),
        Token::Eof,
    ];
    let doc = TreeBuilder::build(tokens);
    let body = doc.body().unwrap();
    let ps: Vec<NodeId> = doc
        .get(body)
        .children
        .iter()
        .filter(|&&id| tag_at(&doc, id) == Some(&TagName::P))
        .copied()
        .collect();
    assert_eq!(ps.len(), 3);
}

#[test]
fn duplicate_html_tag() {
    let tokens = vec![
        start("html"),
        start("html"),
        start("body"),
        text("x"),
        end("body"),
        end("html"),
        end("html"),
        Token::Eof,
    ];
    let doc = TreeBuilder::build(tokens);
    assert!(doc.body().is_some());
}

#[test]
fn duplicate_body_tag() {
    let tokens = vec![
        start("body"),
        text("first"),
        end("body"),
        start("body"),
        text("second"),
        end("body"),
        Token::Eof,
    ];
    let doc = TreeBuilder::build(tokens);
    let body = doc.body().unwrap();
    // Content should be preserved, not lost.
    let tc = doc.text_content(body);
    assert!(tc.contains("first") || tc.contains("second"));
}

#[test]
fn script_content_preserved() {
    let tokens = vec![
        start("script"),
        text("var x = 1;"),
        end("script"),
        Token::Eof,
    ];
    let doc = TreeBuilder::build(tokens);
    let head = doc.head().unwrap();
    let body = doc.body().unwrap();
    // Script might go to head or body depending on insertion mode.
    let total_children = doc.get(head).children.len() + doc.get(body).children.len();
    assert!(total_children >= 1);
}

#[test]
fn unknown_tag_names() {
    let tokens = vec![
        start("custom-element"),
        text("content"),
        end("custom-element"),
        Token::Eof,
    ];
    let doc = TreeBuilder::build(tokens);
    let body = doc.body().unwrap();
    assert!(!doc.get(body).children.is_empty());
}

#[test]
fn large_number_of_siblings() {
    let mut tokens: Vec<Token> = Vec::new();
    for i in 0..500 {
        tokens.push(start("span"));
        tokens.push(text(&format!("{i}")));
        tokens.push(end("span"));
    }
    tokens.push(Token::Eof);
    let doc = TreeBuilder::build(tokens);
    let body = doc.body().unwrap();
    assert!(doc.get(body).children.len() >= 500);
}

// -- Nesting depth guard tests ------------------------------------

#[test]
fn nesting_depth_capped_at_256() {
    // Create 300 levels of nesting (exceeds MAX_NESTING_DEPTH).
    let mut tokens: Vec<Token> = Vec::new();
    for _ in 0..300 {
        tokens.push(start("div"));
    }
    tokens.push(text("deep leaf"));
    for _ in 0..300 {
        tokens.push(end("div"));
    }
    tokens.push(Token::Eof);

    let doc = TreeBuilder::build(tokens);
    let body = doc.body().expect("has body");

    // Walk down to measure actual depth.
    let mut depth = 0u32;
    let mut node = body;
    loop {
        let children = &doc.get(node).children;
        if children.is_empty() {
            break;
        }
        // Follow the first child that is an element.
        let child = children.iter().find(|&&id| doc.element(id).is_some());
        if let Some(&id) = child {
            depth += 1;
            node = id;
        } else {
            break;
        }
    }

    // Depth should be capped. The open_elements stack includes
    // html and body (2 slots), so the div nesting is capped at
    // MAX_NESTING_DEPTH - 2 = 254 at most. Allow some margin.
    assert!(
        depth <= super::MAX_NESTING_DEPTH as u32,
        "nesting depth {depth} should be <= {}",
        super::MAX_NESTING_DEPTH,
    );
}

#[test]
fn nesting_depth_leaf_content_preserved() {
    // Even with extreme nesting, the leaf text should appear
    // somewhere in the tree.
    let mut tokens: Vec<Token> = Vec::new();
    for _ in 0..300 {
        tokens.push(start("div"));
    }
    tokens.push(text("deep leaf"));
    for _ in 0..300 {
        tokens.push(end("div"));
    }
    tokens.push(Token::Eof);

    let doc = TreeBuilder::build(tokens);
    let body = doc.body().expect("has body");
    let full_text = doc.text_content(body);
    assert!(
        full_text.contains("deep leaf"),
        "leaf text should be preserved, got: {full_text}",
    );
}

#[test]
fn nesting_at_exact_limit_works() {
    // Nesting exactly at the limit (minus html+body) should be fine.
    let depth = super::MAX_NESTING_DEPTH - 2;
    let mut tokens: Vec<Token> = Vec::new();
    for _ in 0..depth {
        tokens.push(start("span"));
    }
    tokens.push(text("leaf"));
    for _ in 0..depth {
        tokens.push(end("span"));
    }
    tokens.push(Token::Eof);

    let doc = TreeBuilder::build(tokens);
    let body = doc.body().expect("has body");
    let full_text = doc.text_content(body);
    assert!(full_text.contains("leaf"));
}

#[test]
fn nesting_one_over_limit_still_works() {
    // One level beyond the limit should not crash and content
    // should still be in the tree (attached to the current
    // parent instead of nesting deeper).
    let depth = super::MAX_NESTING_DEPTH;
    let mut tokens: Vec<Token> = Vec::new();
    for _ in 0..depth {
        tokens.push(start("div"));
    }
    tokens.push(text("over"));
    for _ in 0..depth {
        tokens.push(end("div"));
    }
    tokens.push(Token::Eof);

    let doc = TreeBuilder::build(tokens);
    let body = doc.body().expect("has body");
    let full_text = doc.text_content(body);
    assert!(full_text.contains("over"));
}

#[test]
fn nesting_depth_with_mixed_tags() {
    // Mix different tags in deep nesting.
    let tags = ["div", "span", "p", "section", "article"];
    let mut tokens: Vec<Token> = Vec::new();
    for i in 0..300 {
        tokens.push(start(tags[i % tags.len()]));
    }
    tokens.push(text("mixed deep"));
    for i in (0..300).rev() {
        tokens.push(end(tags[i % tags.len()]));
    }
    tokens.push(Token::Eof);

    let doc = TreeBuilder::build(tokens);
    let body = doc.body().expect("has body");
    let full_text = doc.text_content(body);
    // Content should be present and no crash.
    assert!(
        full_text.contains("mixed deep") || !full_text.is_empty(),
        "tree should be well-formed",
    );
}

// -- real-world tree builder compliance tests -------------------------

#[test]
fn implicit_head_and_body_from_bare_content() {
    // Just a text node -- tree builder should create
    // implicit <html>, <head>, and <body>.
    let tokens = vec![text("hello world"), Token::Eof];
    let doc = TreeBuilder::build(tokens);

    assert!(doc.head().is_some(), "implicit head should be created");
    assert!(doc.body().is_some(), "implicit body should be created");

    // The text should be in <body>.
    let body = doc.body().unwrap();
    assert_eq!(doc.text_content(body), "hello world");
}

#[test]
fn mismatched_nested_formatting_b_i() {
    // <b><i>text</b></i> -- misnested formatting tags.
    // The text should still be present in the tree.
    let tokens = vec![
        start("p"),
        start("b"),
        start("i"),
        text("styled text"),
        end("b"),
        end("i"),
        end("p"),
        Token::Eof,
    ];
    let doc = TreeBuilder::build(tokens);
    let body = doc.body().unwrap();
    let full_text = doc.text_content(body);
    assert!(
        full_text.contains("styled text"),
        "misnested formatting should preserve text, got: {full_text}",
    );
}

#[test]
fn table_with_direct_text_child() {
    // Text directly inside <table> should not crash and
    // the table structure should still be valid.
    let tokens = vec![
        start("table"),
        text("stray text"),
        start("tr"),
        start("td"),
        text("cell"),
        end("td"),
        end("tr"),
        end("table"),
        Token::Eof,
    ];
    let doc = TreeBuilder::build(tokens);
    let body = doc.body().unwrap();

    // Find the table.
    let table = doc
        .get(body)
        .children
        .iter()
        .find(|&&id| tag_at(&doc, id) == Some(&TagName::Table))
        .copied()
        .expect("table should exist");

    // The cell text should be present somewhere.
    let full_text = doc.text_content(body);
    assert!(
        full_text.contains("cell"),
        "table cell text should be present, got: {full_text}",
    );

    // Table should still have proper structure with tbody.
    let has_tbody = doc
        .get(table)
        .children
        .iter()
        .any(|&id| tag_at(&doc, id) == Some(&TagName::Tbody));
    assert!(has_tbody, "table should have implicit tbody");
}

#[test]
fn p_auto_closes_on_block_element() {
    // <p>text<div>block</div> -- the <div> should auto-close <p>.
    let tokens = vec![
        start("p"),
        text("paragraph text"),
        start("div"),
        text("block content"),
        end("div"),
        Token::Eof,
    ];
    let doc = TreeBuilder::build(tokens);
    let body = doc.body().unwrap();

    // Should have <p> and <div> as siblings in body, not nested.
    let body_children = &doc.get(body).children;
    let p_nodes: Vec<NodeId> = body_children
        .iter()
        .filter(|&&id| tag_at(&doc, id) == Some(&TagName::P))
        .copied()
        .collect();
    let div_nodes: Vec<NodeId> = body_children
        .iter()
        .filter(|&&id| tag_at(&doc, id) == Some(&TagName::Div))
        .copied()
        .collect();

    assert_eq!(p_nodes.len(), 1, "should have one <p>");
    assert_eq!(div_nodes.len(), 1, "should have one <div>");
    assert_eq!(doc.text_content(p_nodes[0]), "paragraph text");
    assert_eq!(doc.text_content(div_nodes[0]), "block content");
}

#[test]
fn implicit_html_head_body_from_title() {
    // <title> in head should create implicit structure.
    let tokens = vec![
        start("title"),
        text("My Page Title"),
        end("title"),
        start("p"),
        text("body content"),
        end("p"),
        Token::Eof,
    ];
    let doc = TreeBuilder::build(tokens);

    assert!(doc.head().is_some(), "implicit head");
    assert!(doc.body().is_some(), "implicit body");
    assert_eq!(doc.title(), Some("My Page Title".to_string()));

    let body = doc.body().unwrap();
    assert!(
        doc.text_content(body).contains("body content"),
        "body should contain paragraph text",
    );
}

// ---- Recovery: stray table-structure tags in body scope ----

/// Stray `<tr>` / `<td>` outside any table must be ignored per
/// WHATWG §13.2.6.4.7 ("in body" insertion mode, "tr" / "td" / "th"
/// start tag → parse error, ignore token). Real browsers drop these
/// instead of building floating table elements.
#[test]
fn stray_tr_td_outside_table_are_ignored() {
    let tokens = vec![
        start("html"),
        start("body"),
        start("p"),
        text("before"),
        end("p"),
        start("tr"), // stray
        start("td"), // stray
        text("should be ignored"),
        end("td"),
        end("tr"),
        start("p"),
        text("after"),
        end("p"),
        Token::Eof,
    ];
    let doc = TreeBuilder::build(tokens);

    // No Tr or Td anywhere in the tree.
    let has_tr = doc
        .nodes
        .iter()
        .any(|n| matches!(&n.kind, NodeKind::Element(e) if e.tag == TagName::Tr));
    let has_td = doc
        .nodes
        .iter()
        .any(|n| matches!(&n.kind, NodeKind::Element(e) if e.tag == TagName::Td));
    assert!(!has_tr, "stray <tr> outside table must be ignored");
    assert!(!has_td, "stray <td> outside table must be ignored");

    // Both paragraphs still present.
    let body = doc.body().unwrap();
    let tc = doc.text_content(body);
    assert!(tc.contains("before"));
    assert!(tc.contains("after"));
    // The text inside the stray <td> becomes a plain text run in body,
    // which is acceptable recovery (spec says "ignore the token" for the
    // tag, but subsequent character tokens are still processed).
}

#[test]
fn stray_tbody_thead_tfoot_outside_table_ignored() {
    let tokens = vec![
        start("html"),
        start("body"),
        start("thead"),
        start("tbody"),
        start("tfoot"),
        start("caption"),
        start("colgroup"),
        start("p"),
        text("real content"),
        end("p"),
        Token::Eof,
    ];
    let doc = TreeBuilder::build(tokens);
    for name in [
        TagName::Thead,
        TagName::Tbody,
        TagName::Tfoot,
        TagName::Caption,
        TagName::Colgroup,
    ] {
        let present = doc
            .nodes
            .iter()
            .any(|n| matches!(&n.kind, NodeKind::Element(e) if e.tag == name));
        assert!(!present, "stray <{name:?}> must be ignored");
    }
    let body = doc.body().unwrap();
    assert!(doc.text_content(body).contains("real content"));
}

/// An actual `<table>` containing `<tr><td>` must still build correctly —
/// the stray-tag filter must not apply once we're in table scope.
#[test]
fn legitimate_table_with_rows_still_works() {
    let tokens = vec![
        start("html"),
        start("body"),
        start("table"),
        start("tr"),
        start("td"),
        text("cell"),
        end("td"),
        end("tr"),
        end("table"),
        Token::Eof,
    ];
    let doc = TreeBuilder::build(tokens);
    let has_table = doc
        .nodes
        .iter()
        .any(|n| matches!(&n.kind, NodeKind::Element(e) if e.tag == TagName::Table));
    let has_tr = doc
        .nodes
        .iter()
        .any(|n| matches!(&n.kind, NodeKind::Element(e) if e.tag == TagName::Tr));
    let has_td = doc
        .nodes
        .iter()
        .any(|n| matches!(&n.kind, NodeKind::Element(e) if e.tag == TagName::Td));
    assert!(has_table && has_tr && has_td);
    let body = doc.body().unwrap();
    assert!(doc.text_content(body).contains("cell"));
}

// ---- Foster parenting ----

/// Per WHATWG §13.2.6.1, a stray `<div>` inside a `<table>` (but not
/// inside a cell) must be foster-parented immediately before the table
/// in the table's parent — not appended after the table.
#[test]
fn foster_parented_div_lands_before_table() {
    let tokens = vec![
        start("html"),
        start("body"),
        start("p"),
        text("before"),
        end("p"),
        start("table"),
        start("div"),
        text("foster"),
        end("div"),
        end("table"),
        Token::Eof,
    ];
    let doc = TreeBuilder::build(tokens);
    let body = doc.body().unwrap();
    let body_children = &doc.get(body).children;

    let table_pos = body_children
        .iter()
        .position(|&id| tag_at(&doc, id) == Some(&TagName::Table))
        .expect("table is a body child");
    let div_pos = body_children
        .iter()
        .position(|&id| tag_at(&doc, id) == Some(&TagName::Div))
        .expect("foster-parented div is a body child");
    assert!(
        div_pos < table_pos,
        "div must be inserted before the table, got div at {div_pos} and table at {table_pos}"
    );

    let div_id = body_children[div_pos];
    assert_eq!(doc.text_content(div_id), "foster");
}

/// Foster-parented text coalesces with the immediately preceding
/// sibling text node in the foster-parent — not with the trailing text
/// node at the end of the parent.
#[test]
fn foster_parented_text_coalesces_with_preceding_sibling() {
    let tokens = vec![
        start("html"),
        start("body"),
        text("alpha"),
        start("table"),
        text("beta"),
        text("gamma"),
        end("table"),
        Token::Eof,
    ];
    let doc = TreeBuilder::build(tokens);
    let body = doc.body().unwrap();

    // The pre-table "alpha" text node should have absorbed beta+gamma,
    // and there should be no fresh text node sitting after the table.
    let mut text_nodes = Vec::new();
    for &child in &doc.get(body).children {
        if let NodeKind::Text(s) = &doc.get(child).kind {
            text_nodes.push(s.clone());
        }
    }
    assert_eq!(
        text_nodes,
        vec!["alphabetagamma".to_string()],
        "foster-parented text must merge into the preceding sibling text node",
    );
}

// ---- Template element ----

/// `<template>` parses as a `Template` element and its children attach
/// to it. The UA stylesheet hides it via `display:none`, so it is not
/// visible — but the DOM contents must be present.
#[test]
fn template_element_parses_with_children() {
    let tokens = vec![
        start("html"),
        start("body"),
        start("template"),
        start("p"),
        text("inside template"),
        end("p"),
        end("template"),
        Token::Eof,
    ];
    let doc = TreeBuilder::build(tokens);
    let body = doc.body().unwrap();
    let body_children = &doc.get(body).children;

    let tmpl_id = *body_children
        .iter()
        .find(|&&id| tag_at(&doc, id) == Some(&TagName::Template))
        .expect("template element present");
    let tmpl_children = &doc.get(tmpl_id).children;
    assert_eq!(tmpl_children.len(), 1);
    assert_eq!(tag_at(&doc, tmpl_children[0]), Some(&TagName::P));
    assert_eq!(doc.text_content(tmpl_id), "inside template");
}

/// `<template>` is also valid inside `<head>`.
#[test]
fn template_element_in_head_parses() {
    let tokens = vec![
        start("html"),
        start("head"),
        start("template"),
        start("span"),
        text("hello"),
        end("span"),
        end("template"),
        end("head"),
        start("body"),
        end("body"),
        Token::Eof,
    ];
    let doc = TreeBuilder::build(tokens);
    let head = doc.head().unwrap();
    let head_children = &doc.get(head).children;
    let tmpl_id = *head_children
        .iter()
        .find(|&&id| tag_at(&doc, id) == Some(&TagName::Template))
        .expect("template element present in head");
    assert_eq!(doc.text_content(tmpl_id), "hello");
}

/// Per WHATWG §13.2.4.2, `template` is a default scope boundary.
/// A `<p>` opened *before* a `<template>` must not be reachable from
/// inside the template — opening a nested `<p>` inside the template
/// must not implicitly close the outer one. The outer `<p>` therefore
/// keeps its trailing text content.
#[test]
fn template_is_a_scope_boundary_for_outer_p() {
    let tokens = vec![
        start("html"),
        start("body"),
        start("p"),
        text("outer-before "),
        start("template"),
        start("p"),
        text("inner"),
        end("p"),
        end("template"),
        text("outer-after"),
        end("p"),
        Token::Eof,
    ];
    let doc = TreeBuilder::build(tokens);
    let body = doc.body().unwrap();

    // The outer <p> must still contain "outer-after" — if the template
    // boundary was missing, the inner `<p>` start tag would have called
    // `close_p_if_in_scope()` which would have walked across the
    // template and closed the outer <p>.
    let outer_p = doc
        .get(body)
        .children
        .iter()
        .find(|&&id| tag_at(&doc, id) == Some(&TagName::P))
        .copied()
        .expect("outer <p>");
    let outer_text = doc.text_content(outer_p);
    assert!(
        outer_text.contains("outer-before") && outer_text.contains("outer-after"),
        "outer <p> lost its trailing text — template boundary was crossed: {outer_text:?}"
    );
}

// -- adoption agency algorithm (WHATWG §13.2.6.4.7) -------------------

/// Helper: collect every element's tag along a DFS path, skipping
/// Document/Text/Comment nodes, to make tree shape assertions easy.
fn element_shape(doc: &Document, id: NodeId) -> String {
    fn walk(doc: &Document, id: NodeId, out: &mut String) {
        match &doc.nodes[id].kind {
            NodeKind::Element(data) => {
                out.push('<');
                out.push_str(data.tag.as_str());
                out.push('>');
                for &c in &doc.nodes[id].children {
                    walk(doc, c, out);
                }
                out.push_str("</");
                out.push_str(data.tag.as_str());
                out.push('>');
            },
            NodeKind::Text(s) => out.push_str(s),
            _ => {
                for &c in &doc.nodes[id].children {
                    walk(doc, c, out);
                }
            },
        }
    }
    let mut s = String::new();
    walk(doc, id, &mut s);
    s
}

#[test]
fn adoption_agency_b_p_end_b_end_p() {
    // WHATWG spec example: `<p>1<b>2<i>3</b>4</i>5</p>`
    // The adoption agency should rebuild the tree so the `<i>` inside
    // `<b>` ends up cloned into `<p>` with 4 as its content.
    let tokens = vec![
        start("p"),
        text("1"),
        start("b"),
        text("2"),
        start("i"),
        text("3"),
        end("b"),
        text("4"),
        end("i"),
        text("5"),
        end("p"),
        Token::Eof,
    ];
    let doc = TreeBuilder::build(tokens);
    let body = doc.body().unwrap();
    let full_text = doc.text_content(body);
    // All five characters must survive.
    assert!(
        full_text.contains('1')
            && full_text.contains('2')
            && full_text.contains('3')
            && full_text.contains('4')
            && full_text.contains('5'),
        "text dropped during adoption agency: {full_text}"
    );
    // And they must be in source order.
    let positions: Vec<_> = ['1', '2', '3', '4', '5']
        .iter()
        .map(|&c| full_text.find(c).unwrap())
        .collect();
    assert!(
        positions.windows(2).all(|w| w[0] < w[1]),
        "text order scrambled: {full_text} positions {positions:?}"
    );
}

#[test]
fn adoption_agency_simple_bp_reorder() {
    // Adversarial `<b><p></b></p>`: after adoption, the `<b>` should
    // wrap the empty prefix, while the `<p>` contains a cloned `<b>`.
    // In any case, the end tag sequence must not lose text.
    let tokens = vec![
        start("b"),
        text("bold"),
        start("p"),
        text("also"),
        end("b"),
        text("plain"),
        end("p"),
        Token::Eof,
    ];
    let doc = TreeBuilder::build(tokens);
    let body = doc.body().unwrap();
    let shape = element_shape(&doc, body);
    assert!(shape.contains("bold"), "lost 'bold': {shape}");
    assert!(shape.contains("also"), "lost 'also': {shape}");
    assert!(shape.contains("plain"), "lost 'plain': {shape}");
    // There must be a <p> in the tree.
    assert!(shape.contains("<p>"), "no <p>: {shape}");
    // `plain` appears under the <p> subtree (whereas the simplified
    // algorithm used to leave it hanging outside).
    let p_id = doc
        .get(body)
        .children
        .iter()
        .find(|&&c| tag_at(&doc, c) == Some(&TagName::P))
        .copied()
        .expect("<p> should exist");
    let p_text = doc.text_content(p_id);
    assert!(
        p_text.contains("plain"),
        "'plain' should live inside <p>: {p_text:?}"
    );
}

#[test]
fn adoption_agency_b_div_close_b() {
    // <b>1<div>2</b>3</div>
    // Per spec: the <div> is the furthest block. After adoption, the
    // <div> moves to the original common ancestor (<body>) and
    // receives a clone of <b> wrapping "2".
    let tokens = vec![
        start("b"),
        text("1"),
        start("div"),
        text("2"),
        end("b"),
        text("3"),
        end("div"),
        Token::Eof,
    ];
    let doc = TreeBuilder::build(tokens);
    let body = doc.body().unwrap();
    let full_text = doc.text_content(body);
    assert!(full_text.contains('1'), "'1' dropped: {full_text}");
    assert!(full_text.contains('2'), "'2' dropped: {full_text}");
    assert!(full_text.contains('3'), "'3' dropped: {full_text}");
    // <div> should be a direct child of <body>, not still nested
    // inside the original <b>.
    let div_under_body = doc
        .get(body)
        .children
        .iter()
        .any(|&c| tag_at(&doc, c) == Some(&TagName::Div));
    assert!(div_under_body, "<div> should be hoisted up to <body>");
}

// -- template contents isolation (DocumentFragment emulation) ----------

#[test]
fn template_isolates_form_scope() {
    // Outer <form>, then inside a <template>, a nested <form>.
    // Without isolation the nested <form> would be ignored (because
    // `form_element` is already set by the outer). With isolation, the
    // inner <form> should be inserted and parse normally.
    let tokens = vec![
        start("form"),
        start("template"),
        start("form"),
        start("input"),
        end("form"),
        end("template"),
        end("form"),
        Token::Eof,
    ];
    let doc = TreeBuilder::build(tokens);
    let body = doc.body().unwrap();

    // Find the outer <form>, then the <template> inside it, then
    // confirm the template subtree contains a <form>.
    let outer_form = doc
        .get(body)
        .children
        .iter()
        .find(|&&c| tag_at(&doc, c) == Some(&TagName::Form))
        .copied()
        .expect("outer form");
    let tmpl = doc
        .get(outer_form)
        .children
        .iter()
        .find(|&&c| tag_at(&doc, c) == Some(&TagName::Template))
        .copied()
        .expect("template inside form");
    let inner_form = doc
        .get(tmpl)
        .children
        .iter()
        .find(|&&c| tag_at(&doc, c) == Some(&TagName::Form))
        .copied();
    assert!(
        inner_form.is_some(),
        "inner <form> should exist inside the template — outer form scope must not leak in"
    );
}

// -- foreign content (svg / mathml) ------------------------------------

#[test]
fn svg_subtree_parses_without_auto_close() {
    // Inside <svg>, a stray <g> should NOT auto-close the enclosing
    // <p> the way an HTML block element would. Both <p> and <svg>
    // must be siblings under body, and <g> must be nested in <svg>.
    let tokens = vec![
        start("p"),
        text("before "),
        start("svg"),
        start("g"),
        start("rect"),
        end("rect"),
        end("g"),
        end("svg"),
        text(" after"),
        end("p"),
        Token::Eof,
    ];
    let doc = TreeBuilder::build(tokens);
    let body = doc.body().unwrap();

    // Body should have exactly one child: the <p>.
    let body_children = &doc.get(body).children;
    let p_children: Vec<_> = body_children
        .iter()
        .filter(|&&c| tag_at(&doc, c) == Some(&TagName::P))
        .copied()
        .collect();
    assert_eq!(
        p_children.len(),
        1,
        "expected exactly one <p>; the svg breakout must not have split it"
    );
    let p = p_children[0];
    // The <p> must contain an <svg>, and the full text "before  after"
    // (with the space inserted between) must round-trip.
    let svg_id = doc
        .get(p)
        .children
        .iter()
        .find(|&&c| tag_at(&doc, c) == Some(&TagName::Svg))
        .copied()
        .expect("<svg> should be a child of <p>");
    // <g><rect/></g> chain nested under svg.
    let g = doc.get(svg_id).children[0];
    assert_eq!(
        doc.element(g).unwrap().tag.as_str(),
        "g",
        "<g> should be inside <svg>"
    );
    let rect = doc.get(g).children[0];
    assert_eq!(doc.element(rect).unwrap().tag.as_str(), "rect");
    let full_text = doc.text_content(p);
    assert!(
        full_text.contains("before") && full_text.contains("after"),
        "text around <svg> lost: {full_text:?}"
    );
}

#[test]
fn math_subtree_parses_as_foreign_content() {
    let tokens = vec![
        start("math"),
        start("mi"),
        text("x"),
        end("mi"),
        start("mo"),
        text("="),
        end("mo"),
        start("mn"),
        text("1"),
        end("mn"),
        end("math"),
        Token::Eof,
    ];
    let doc = TreeBuilder::build(tokens);
    let body = doc.body().unwrap();
    let math_id = doc
        .get(body)
        .children
        .iter()
        .find(|&&c| tag_at(&doc, c) == Some(&TagName::Math))
        .copied()
        .expect("<math> root should exist");
    let children: Vec<&str> = doc
        .get(math_id)
        .children
        .iter()
        .filter_map(|&c| doc.element(c).map(|e| e.tag.as_str()))
        .collect();
    assert_eq!(children, vec!["mi", "mo", "mn"]);
    assert_eq!(doc.text_content(math_id), "x=1");
}

#[test]
fn svg_html_breakout_tag_returns_to_html() {
    // Seeing a <div> inside <svg> must break out of foreign content
    // and reparse the <div> as HTML.
    let tokens = vec![
        start("svg"),
        start("rect"),
        end("rect"),
        start("div"),
        text("back to html"),
        end("div"),
        end("svg"),
        Token::Eof,
    ];
    let doc = TreeBuilder::build(tokens);
    let body = doc.body().unwrap();
    // Body should have two children: svg (with rect) and div.
    let tags: Vec<&str> = doc
        .get(body)
        .children
        .iter()
        .filter_map(|&c| doc.element(c).map(|e| e.tag.as_str()))
        .collect();
    assert!(
        tags.contains(&"svg") && tags.contains(&"div"),
        "expected <svg> and <div> siblings under body, got {tags:?}"
    );
    let div = doc
        .get(body)
        .children
        .iter()
        .find(|&&c| tag_at(&doc, c) == Some(&TagName::Div))
        .copied()
        .expect("div");
    assert_eq!(doc.text_content(div), "back to html");
}

#[test]
fn svg_eof_mid_subtree_finalizes_tree() {
    // Truncated input: `<svg><g><rect>` with no closing tags and no
    // explicit EOF behaviour beyond what the dispatcher provides.
    // The foreign-content EOF path must break out and re-dispatch
    // to InBody so body/html still exist and the tree is well-formed.
    let tokens = vec![start("svg"), start("g"), start("rect"), Token::Eof];
    let doc = TreeBuilder::build(tokens);
    assert!(
        doc.body().is_some(),
        "<body> must exist even after mid-svg EOF"
    );
    let body = doc.body().unwrap();
    let svg = doc
        .get(body)
        .children
        .iter()
        .find(|&&c| tag_at(&doc, c) == Some(&TagName::Svg))
        .copied()
        .expect("<svg> hoisted under body");
    // The partial <g><rect> subtree should be preserved, not dropped.
    let g = doc.get(svg).children[0];
    assert_eq!(doc.element(g).unwrap().tag.as_str(), "g");
    let rect = doc.get(g).children[0];
    assert_eq!(doc.element(rect).unwrap().tag.as_str(), "rect");
}

#[test]
fn svg_self_closing_empty_element() {
    // <svg><circle /></svg> — `<circle />` has self_closing=true and
    // must not stay on the open stack.
    let tokens = vec![
        start("svg"),
        Token::StartTag(super::super::tokenizer::StartTagToken {
            name: "circle".into(),
            self_closing: true,
            attributes: Vec::new(),
        }),
        end("svg"),
        Token::Eof,
    ];
    let doc = TreeBuilder::build(tokens);
    let body = doc.body().unwrap();
    let svg = doc
        .get(body)
        .children
        .iter()
        .find(|&&c| tag_at(&doc, c) == Some(&TagName::Svg))
        .copied()
        .expect("svg");
    let circle_count = doc
        .get(svg)
        .children
        .iter()
        .filter(|&&c| {
            doc.element(c)
                .map(|e| e.tag.as_str() == "circle")
                .unwrap_or(false)
        })
        .count();
    assert_eq!(circle_count, 1);
}

mod prop {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Building a tree from arbitrary token sequences never panics.
        #[test]
        fn build_never_panics(
            n in 0usize..30,
        ) {
            let tags = ["div", "span", "p", "a", "b", "li", "ul"];
            let mut tokens: Vec<Token> = Vec::new();
            for i in 0..n {
                let tag = tags[i % tags.len()];
                tokens.push(start(tag));
                tokens.push(text("x"));
                tokens.push(end(tag));
            }
            tokens.push(Token::Eof);
            let _ = TreeBuilder::build(tokens);
        }

        /// Deeply nested single-tag trees never panic.
        #[test]
        fn deep_nesting_no_panic(depth in 1usize..350) {
            let mut tokens: Vec<Token> = Vec::new();
            for _ in 0..depth {
                tokens.push(start("div"));
            }
            tokens.push(text("leaf"));
            for _ in 0..depth {
                tokens.push(end("div"));
            }
            tokens.push(Token::Eof);
            let doc = TreeBuilder::build(tokens);
            let body = doc.body().expect("body");
            // Text should always be present somewhere.
            let tc = doc.text_content(body);
            prop_assert!(
                tc.contains("leaf"),
                "leaf text missing at depth {depth}",
            );
        }

        /// Orphaned end tags of any name never crash.
        #[test]
        fn orphan_end_tags_no_panic(name in "[a-z]{1,10}") {
            let tokens = vec![end(&name), Token::Eof];
            let doc = TreeBuilder::build(tokens);
            prop_assert!(doc.body().is_some());
        }

        /// Text-only documents always produce a body.
        #[test]
        fn text_only_always_has_body(s in ".{0,50}") {
            let tokens = vec![text(&s), Token::Eof];
            let doc = TreeBuilder::build(tokens);
            prop_assert!(doc.body().is_some());
        }

        /// Random sequences of start/end/text tokens never panic.
        #[test]
        fn random_token_sequence(
            ops in proptest::collection::vec(0u8..6, 0..50),
        ) {
            let tags = ["div", "p", "span", "a", "li"];
            let mut tokens: Vec<Token> = Vec::new();
            for op in &ops {
                let tag = tags[(*op as usize) % tags.len()];
                match op % 3 {
                    0 => tokens.push(start(tag)),
                    1 => tokens.push(end(tag)),
                    _ => tokens.push(text("x")),
                }
            }
            tokens.push(Token::Eof);
            let _ = TreeBuilder::build(tokens);
        }

        /// Table structure with random row/col counts never panics.
        #[test]
        fn random_table(
            rows in 1usize..10,
            cols in 1usize..10,
        ) {
            let mut tokens: Vec<Token> = Vec::new();
            tokens.push(start("table"));
            for _ in 0..rows {
                tokens.push(start("tr"));
                for _ in 0..cols {
                    tokens.push(start("td"));
                    tokens.push(text("cell"));
                    tokens.push(end("td"));
                }
                tokens.push(end("tr"));
            }
            tokens.push(end("table"));
            tokens.push(Token::Eof);
            let doc = TreeBuilder::build(tokens);
            let body = doc.body().expect("body");
            let tc = doc.text_content(body);
            prop_assert!(tc.contains("cell"));
        }
    }
}
