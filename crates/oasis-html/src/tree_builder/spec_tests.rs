//! Spec-conformance regressions that need the pull parser or parser
//! options (the table-driven tree dumps live in
//! `tests/fixtures/html5lib/tree_construction_spec.dat`).

use super::super::dom::{NodeId, NodeKind, TagName};
use super::super::tokenizer::Tokenizer;
use super::*;

fn find_all(doc: &Document, name: &str) -> Vec<NodeId> {
    (0..doc.nodes.len())
        .filter(|&id| doc.element(id).is_some_and(|e| e.tag.as_str() == name))
        .collect()
}

fn find(doc: &Document, name: &str) -> NodeId {
    find_all(doc, name)
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("no <{name}>"))
}

fn parent_name(doc: &Document, id: NodeId) -> String {
    doc.nodes[id]
        .parent
        .and_then(|p| doc.element(p))
        .map(|e| e.tag.as_str().to_string())
        .unwrap_or_default()
}

fn text_children(doc: &Document, id: NodeId) -> String {
    doc.nodes[id]
        .children
        .iter()
        .filter_map(|&c| match &doc.nodes[c].kind {
            NodeKind::Text(t) => Some(t.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn noscript_is_raw_text_with_scripting_on() {
    let html = "<body><noscript><img src=x></noscript>";
    for doc in [
        TreeBuilder::parse(html),
        TreeBuilder::build(Tokenizer::new(html).tokenize()),
    ] {
        assert!(find_all(&doc, "img").is_empty());
        assert_eq!(text_children(&doc, find(&doc, "noscript")), "<img src=x>");
    }
}

#[test]
fn noscript_is_markup_with_scripting_off() {
    let html = "<body><noscript><img src=x><p>enable JS</p></noscript><p>after";
    let off = ParseOptions { scripting: false };
    for doc in [
        TreeBuilder::parse_with(html, off),
        TreeBuilder::build_with(Tokenizer::with_scripting(html, false).tokenize(), off),
    ] {
        let img = find(&doc, "img");
        assert_eq!(parent_name(&doc, img), "noscript");
        let ps = find_all(&doc, "p");
        assert_eq!(ps.len(), 2);
        assert_eq!(parent_name(&doc, ps[0]), "noscript");
        assert_eq!(parent_name(&doc, ps[1]), "body");
    }
}

#[test]
fn head_noscript_with_scripting_off_keeps_head_content() {
    let html = "<head><noscript><link rel=stylesheet href=a.css><style>p{}</style>\
                </noscript><title>T</title></head><body>x";
    let doc = TreeBuilder::parse_with(html, ParseOptions { scripting: false });
    let noscript = find(&doc, "noscript");
    assert_eq!(parent_name(&doc, noscript), "head");
    assert_eq!(parent_name(&doc, find(&doc, "link")), "noscript");
    assert_eq!(parent_name(&doc, find(&doc, "style")), "noscript");
    assert_eq!(parent_name(&doc, find(&doc, "title")), "head");
}

#[test]
fn head_noscript_with_other_content_closes_it() {
    // Body content inside a head `<noscript>` pops it and opens the body.
    let html = "<head><noscript><p>hi</p></noscript></head>";
    let doc = TreeBuilder::parse_with(html, ParseOptions { scripting: false });
    let p = find(&doc, "p");
    assert_eq!(parent_name(&doc, p), "body");
}

#[test]
fn escaped_script_keeps_nested_script_text() {
    let html = "<script><!-- document.write('<script>x()</script>'); --></script><p>after";
    for doc in [
        TreeBuilder::parse(html),
        TreeBuilder::build(Tokenizer::new(html).tokenize()),
    ] {
        let scripts = find_all(&doc, "script");
        assert_eq!(scripts.len(), 1, "one script element");
        assert_eq!(
            text_children(&doc, scripts[0]),
            "<!-- document.write('<script>x()</script>'); -->"
        );
        assert_eq!(parent_name(&doc, find(&doc, "p")), "body");
    }
}

#[test]
fn escaped_script_end_tag_still_ends_script() {
    let html = "<script><!-- a </script><p>after";
    let doc = TreeBuilder::parse(html);
    assert_eq!(text_children(&doc, find(&doc, "script")), "<!-- a ");
    assert_eq!(parent_name(&doc, find(&doc, "p")), "body");
}

#[test]
fn comment_close_leaves_escape() {
    // After `-->` a `<script>` no longer double-escapes.
    let html = "<script><!-- a --> <script> </script><p>after";
    let doc = TreeBuilder::parse(html);
    assert_eq!(
        text_children(&doc, find(&doc, "script")),
        "<!-- a --> <script> "
    );
    assert_eq!(parent_name(&doc, find(&doc, "p")), "body");
}

#[test]
fn cdata_section_in_svg_is_text() {
    let html = "<svg><style><![CDATA[ .a { fill: red } ]]></style></svg><p>x";
    let doc = TreeBuilder::parse(html);
    let style = find(&doc, "style");
    assert_eq!(text_children(&doc, style), " .a { fill: red } ");
    assert_eq!(parent_name(&doc, find(&doc, "p")), "body");
}

#[test]
fn cdata_outside_foreign_content_is_a_bogus_comment() {
    let doc = TreeBuilder::parse("<div><![CDATA[x]]></div>");
    let div = find(&doc, "div");
    let kids = &doc.nodes[div].children;
    assert_eq!(kids.len(), 1);
    assert!(matches!(&doc.nodes[kids[0]].kind, NodeKind::Comment(c) if c == "[CDATA[x]]"));
}

#[test]
fn svg_title_is_not_rcdata_in_the_pull_parser() {
    // In foreign content `<title>` is an ordinary element (an HTML
    // integration point): its markup is parsed.
    let doc = TreeBuilder::parse("<svg><title>a <b>b</b></title></svg>");
    let b = find(&doc, "b");
    assert_eq!(parent_name(&doc, b), "title");
}

#[test]
fn plaintext_swallows_the_rest() {
    let doc = TreeBuilder::parse("<plaintext><b>x</b></plaintext>");
    let pt = find(&doc, "plaintext");
    assert_eq!(text_children(&doc, pt), "<b>x</b></plaintext>");
    assert!(find_all(&doc, "b").is_empty());
}

#[test]
fn pre_and_textarea_drop_the_leading_newline() {
    let doc = TreeBuilder::parse("<pre>\nline</pre><textarea>\n\nt</textarea>");
    assert_eq!(text_children(&doc, find(&doc, "pre")), "line");
    assert_eq!(text_children(&doc, find(&doc, "textarea")), "\nt");
}

#[test]
fn late_body_tag_attributes_are_merged() {
    let doc = TreeBuilder::parse("<p>x</p><body class=late id=b>");
    let body = find(&doc, "body");
    let data = doc.element(body).expect("body element");
    assert_eq!(data.get_attribute("class"), Some("late"));
    assert_eq!(doc.get_element_by_id("b"), Some(body));
}

#[test]
fn pull_and_vec_parsers_agree_on_ordinary_markup() {
    let html = "<!doctype html><html><head><title>t &amp; u</title><style>a<b</style>\
                </head><body><p>a &lt; b<table><tr><td>1<td>2</table><textarea>x<y\
                </textarea><select><option>a<option>b</select></body></html>";
    let pull = TreeBuilder::parse(html);
    let vec = TreeBuilder::build(Tokenizer::new(html).tokenize());
    let shape = |doc: &Document| -> Vec<(String, usize)> {
        (0..doc.nodes.len())
            .filter_map(|id| {
                doc.element(id)
                    .map(|e| (e.tag.as_str().to_string(), doc.nodes[id].children.len()))
            })
            .collect()
    };
    assert_eq!(shape(&pull), shape(&vec));
    assert_eq!(pull.title(), Some("t & u".to_string()));
}

#[test]
fn tag_names_used_by_the_tests_exist() {
    // Guards the string-based lookups above against TagName renames.
    assert_eq!(TagName::from_str("noscript").as_str(), "noscript");
}

#[test]
fn doctype_sets_the_document_quirks_mode() {
    use super::super::dom::QuirksMode;
    assert_eq!(TreeBuilder::parse("<p>x").quirks_mode, QuirksMode::Quirks);
    assert_eq!(
        TreeBuilder::parse("<!DOCTYPE html><p>x").quirks_mode,
        QuirksMode::NoQuirks
    );
    let limited = "<!DOCTYPE html PUBLIC \"-//W3C//DTD XHTML 1.0 Transitional//EN\" \
                   \"http://www.w3.org/TR/xhtml1/DTD/xhtml1-transitional.dtd\">";
    assert_eq!(
        TreeBuilder::parse(limited).quirks_mode,
        QuirksMode::LimitedQuirks
    );
    // A reused arena starts over.
    let old = TreeBuilder::parse("<p>x");
    let doc = TreeBuilder::parse_reuse("<!DOCTYPE html>", ParseOptions::default(), old);
    assert_eq!(doc.quirks_mode, QuirksMode::NoQuirks);
}

#[test]
fn unclosed_template_in_table_keeps_its_rows() {
    // EOF inside a template inside a table: the row parses as the
    // template's content, not as dropped tags.
    let doc = TreeBuilder::parse("<!DOCTYPE html><table><template><tr><td>a");
    assert_eq!(parent_name(&doc, find(&doc, "td")), "tr");
    assert_eq!(parent_name(&doc, find(&doc, "tr")), "template");
    assert_eq!(parent_name(&doc, find(&doc, "template")), "table");
}

#[test]
fn template_form_does_not_set_the_form_pointer() {
    // A form inside a template is not the document's open form, so a
    // later form outside it is still created.
    let doc = TreeBuilder::parse("<template><form></form></template><form id=b></form>");
    assert_eq!(find_all(&doc, "form").len(), 2);
}
