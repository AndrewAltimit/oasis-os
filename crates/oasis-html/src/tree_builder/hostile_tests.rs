//! Tree-builder and tokenizer regressions for hostile or malformed
//! markup: resource bounds, stack hygiene, character references.

use std::time::{Duration, Instant};

use super::super::dom::{NodeId, NodeKind, TagName};
use super::super::tokenizer::Tokenizer;
use super::*;

fn parse(html: &str) -> Document {
    TreeBuilder::build(Tokenizer::new(html).tokenize())
}

/// Depth of the deepest node in the document.
fn max_depth(doc: &Document) -> usize {
    let mut best = 0;
    let mut stack = vec![(doc.root, 0usize)];
    while let Some((id, d)) = stack.pop() {
        best = best.max(d);
        stack.extend(doc.nodes[id].children.iter().map(|&c| (c, d + 1)));
    }
    best
}

fn find(doc: &Document, tag: &TagName) -> Vec<NodeId> {
    (0..doc.nodes.len())
        .filter(|&id| doc.element(id).is_some_and(|e| &e.tag == tag))
        .collect()
}

fn total_attributes(doc: &Document) -> usize {
    (0..doc.nodes.len())
        .filter_map(|id| doc.element(id))
        .map(|e| e.attributes.len())
        .sum()
}

/// Whether `id` is inside the `<html>` element.
fn under_html(doc: &Document, id: NodeId) -> bool {
    let mut cur = doc.nodes[id].parent;
    while let Some(p) = cur {
        if doc.element(p).is_some_and(|e| e.tag == TagName::Html) {
            return true;
        }
        cur = doc.nodes[p].parent;
    }
    false
}

fn assert_fast(start: Instant, what: &str) {
    assert!(
        start.elapsed() < Duration::from_secs(20),
        "{what}: {:?}",
        start.elapsed()
    );
}

#[test]
fn attributes_are_capped_and_deduplicated() {
    let attrs: String = (0..40_000).map(|i| format!(" a{i}")).collect();
    let start = Instant::now();
    let doc = parse(&format!("<b{attrs}>x").repeat(4));
    assert_fast(start, "duplicate-formatting check");
    for b in find(&doc, &TagName::B) {
        assert!(doc.element(b).unwrap().attributes.len() <= 256);
    }
    // First of a repeated name wins; `getElementById` agrees.
    let doc = parse(r#"<div id=a id=b>A</div><div id=b>B</div>"#);
    let divs = find(&doc, &TagName::Div);
    assert_eq!(doc.element(divs[0]).unwrap().attributes.len(), 1);
    assert_eq!(doc.get_element_by_id("b"), Some(divs[1]));
}

/// Stray `<body>` tags merge their new attributes into the body; distinct
/// names in each tag used to pile up without bound (and each merge
/// rescanned them all).
#[test]
fn merged_body_attributes_are_capped() {
    let mut html = String::from("<body>x");
    for t in 0..300 {
        let attrs: String = (0..200).map(|i| format!(" t{t}a{i}")).collect();
        html.push_str(&format!("<body{attrs}>"));
    }
    let start = Instant::now();
    let doc = parse(&html);
    assert_fast(start, "body attribute merge");
    let body = find(&doc, &TagName::Body)[0];
    assert!(doc.element(body).unwrap().attributes.len() <= 256);
    // A legitimate late attribute still lands.
    let doc = parse("<body>x<body class=late>");
    let body = find(&doc, &TagName::Body)[0];
    assert_eq!(
        doc.element(body).unwrap().get_attribute("class"),
        Some("late")
    );
}

/// Reopening a `<b>` with thousands of attributes after every `<p>`
/// multiplied them: 37 KB of markup became ten million attributes.
#[test]
fn reopened_formatting_clones_share_an_attribute_budget() {
    let attrs: String = (0..250).map(|i| format!(" a{i}")).collect();
    let html = format!("<p><b{attrs}>{}", "x<p>".repeat(5000));
    let doc = parse(&html);
    assert!(
        total_attributes(&doc) <= MAX_CLONED_ATTRIBUTES + 250,
        "{}",
        total_attributes(&doc)
    );
}

/// The adoption agency moved the furthest block's children one detach
/// at a time, each rescanning the list: quadratic.
#[test]
fn adoption_agency_moves_children_in_linear_time() {
    let html = format!("<b><div>{}</b>", "<br>".repeat(90_000));
    let start = Instant::now();
    let doc = parse(&html);
    assert_fast(start, "adoption agency");
    assert!(!find(&doc, &TagName::Br).is_empty());
}

#[test]
fn foster_parenting_respects_the_nesting_cap() {
    let start = Instant::now();
    let doc = parse(&"<table>".repeat(80_000));
    assert_fast(start, "foster parenting");
    assert!(max_depth(&doc) <= MAX_DOM_DEPTH + 1, "{}", max_depth(&doc));
}

/// `<a>` pulled out of the middle of the stack let the DOM grow to twice
/// the depth the recursive passes are sized for.
#[test]
fn dom_depth_stays_within_the_cap() {
    for unit in ["<a><div>", "<a><b><div>", "<div>", "<table><tr><td>"] {
        let doc = parse(&unit.repeat(5000));
        // Elements at the cap plus one text leaf below them.
        assert!(
            max_depth(&doc) <= MAX_DOM_DEPTH + 1,
            "{unit}: {}",
            max_depth(&doc)
        );
    }
}

/// A stray end tag crossed the table cell, and the later `</table>`
/// then emptied the stack: `c` and `<p>` landed outside `<html>`.
#[test]
fn stray_end_tags_do_not_cross_table_cells() {
    let doc = parse("<div><table><tr><td>a</div>b</td></tr></table>c<p>d</p>");
    for p in find(&doc, &TagName::P) {
        assert!(under_html(&doc, p));
    }
    let td = find(&doc, &TagName::Td)[0];
    assert_eq!(doc.text_content(td), "ab");
    // `</span>` inside a cell, with the span outside the table.
    let doc = parse("<span><table><tr><td>a</span>b</td></tr></table>c</span><p>z</p>");
    let td = find(&doc, &TagName::Td)[0];
    assert_eq!(doc.text_content(td), "ab");
    assert!(under_html(&doc, find(&doc, &TagName::P)[0]));
}

/// At the nesting cap the implied `<p>` of a stray `</p>` was not pushed,
/// and closing it popped the whole stack.
#[test]
fn end_tags_at_the_nesting_cap_keep_the_stack() {
    let doc = parse(&format!("{}<p>x</p>y<span>s</span>", "<div>".repeat(260)));
    for span in find(&doc, &TagName::Span) {
        assert!(under_html(&doc, span));
    }
}

/// At the cap `<script>` was not pushed: its source became visible text
/// of the parent, and `</script>` popped the parent.
#[test]
fn raw_text_at_the_nesting_cap_stays_inside_its_element() {
    let doc = parse(&format!(
        "{}<script>var secret=1</script><span>after</span>",
        "<div>".repeat(260)
    ));
    let script = find(&doc, &TagName::Script)[0];
    assert_eq!(doc.text_content(script), "var secret=1");
    let parent = doc.nodes[script].parent.unwrap();
    for &c in &doc.nodes[parent].children {
        if let NodeKind::Text(t) = &doc.nodes[c].kind {
            assert!(!t.contains("secret"), "{t:?}");
        }
    }
    assert!(under_html(&doc, find(&doc, &TagName::Span)[0]));
}

/// `</template>` in a table mode was foster-parented, leaving the parser
/// in table mode.
#[test]
fn template_end_tag_restores_the_mode_from_table_content() {
    let doc = parse("<template><table></template><p>hi</p></table><p>z</p>");
    for p in find(&doc, &TagName::P) {
        assert!(under_html(&doc, p));
    }
}

#[test]
fn input_stream_is_preprocessed() {
    let doc = parse("<pre>a\r\nb\rc</pre><p title=\"x\r\ny\">n\0n</p>");
    let pre = find(&doc, &TagName::Pre)[0];
    assert_eq!(doc.text_content(pre), "a\nb\nc");
    let p = find(&doc, &TagName::P)[0];
    assert_eq!(doc.element(p).unwrap().get_attribute("title"), Some("x\ny"));
    assert_eq!(doc.text_content(p), "n\u{FFFD}n");
}

#[test]
fn character_reference_edge_cases() {
    let text_of = |html: &str| {
        let doc = parse(&format!("<p>{html}</p>"));
        doc.text_content(find(&doc, &TagName::P)[0])
    };
    assert_eq!(text_of("&#;"), "&#;");
    assert_eq!(text_of("&#abc"), "&#abc");
    assert_eq!(text_of("&#0abc"), "\u{FFFD}abc");
    assert_eq!(text_of("&#150;&#128;"), "\u{2013}\u{20AC}");
    assert_eq!(text_of("&#65;&#x42;"), "AB");
    // RCDATA (`<title>`, `<textarea>`) maps C1 references the same way.
    let doc = parse("<title>A &#151; B</title><textarea>&#x96;</textarea>");
    assert_eq!(
        doc.text_content(find(&doc, &TagName::Title)[0]),
        "A \u{2014} B"
    );
    assert_eq!(
        doc.text_content(find(&doc, &TagName::Textarea)[0]),
        "\u{2013}"
    );
    // In attributes, a `;`-less reference followed by `=` stays literal.
    let doc = parse(r#"<a href="/s?q=1&pi=3&copy=2&amp;x&copy;">l</a>"#);
    let a = find(&doc, &TagName::A)[0];
    assert_eq!(
        doc.element(a).unwrap().get_attribute("href"),
        Some("/s?q=1&pi=3&copy=2&x\u{a9}")
    );
}
