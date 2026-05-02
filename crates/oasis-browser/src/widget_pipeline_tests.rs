//! Tests for `widget_pipeline.rs` -- pure helpers and resource dispatch.
//!
//! The high-level navigation paths (load_html, navigate_vfs, process_response)
//! are exercised end-to-end by `browser_tests.rs`. This module focuses on
//! the pure functions that are easy to break by refactor: the print-only
//! media query filter and the link-map builder.
#![allow(clippy::unwrap_used)]

use crate::BrowserWidget;
use crate::config::BrowserConfig;
use crate::html::dom::{Attribute, Document, ElementData, Node, NodeKind, TagName};
use crate::loader::{ContentType, ResourceResponse};

// -------------------------------------------------------------------
// `is_print_only_media_query` -- filtering @media on <link rel="stylesheet">
// -------------------------------------------------------------------

#[test]
fn print_only_query_bare_print() {
    assert!(BrowserWidget::is_print_only_media_query("print"));
}

#[test]
fn print_only_query_only_print() {
    assert!(BrowserWidget::is_print_only_media_query("only print"));
}

#[test]
fn print_only_query_print_with_features() {
    assert!(BrowserWidget::is_print_only_media_query(
        "print and (color)"
    ));
}

#[test]
fn print_only_query_screen_not_print() {
    assert!(!BrowserWidget::is_print_only_media_query("screen"));
}

#[test]
fn print_only_query_all_not_print() {
    assert!(!BrowserWidget::is_print_only_media_query("all"));
}

#[test]
fn print_only_query_min_width_not_print() {
    assert!(!BrowserWidget::is_print_only_media_query(
        "(min-width: 500px)"
    ));
}

#[test]
fn print_only_query_not_all_filters_to_zero() {
    // `not all` matches no media -- treated as print-only so the sheet
    // gets dropped (it would have matched nothing anyway).
    assert!(BrowserWidget::is_print_only_media_query("not all"));
}

#[test]
fn print_only_query_not_print_is_not_print_only() {
    // `not print` matches every medium *except* print -- the screen
    // engine MUST keep it.
    assert!(!BrowserWidget::is_print_only_media_query("not print"));
}

#[test]
fn print_only_query_not_screen_is_not_print_only() {
    // `not screen` matches print + everything else -- still not
    // print-only.
    assert!(!BrowserWidget::is_print_only_media_query("not screen"));
}

#[test]
fn print_only_query_not_all_with_feature_is_not_print_only() {
    // `not all and (color)` matches monochrome screens -- must not be
    // dropped.
    assert!(!BrowserWidget::is_print_only_media_query(
        "not all and (color)"
    ));
}

#[test]
fn print_only_query_empty_string_is_not_print() {
    assert!(!BrowserWidget::is_print_only_media_query(""));
    assert!(!BrowserWidget::is_print_only_media_query("   "));
}

#[test]
fn print_only_query_whitespace_around_print() {
    assert!(BrowserWidget::is_print_only_media_query("  print  "));
    assert!(BrowserWidget::is_print_only_media_query("  only   print  "));
}

// -------------------------------------------------------------------
// `build_link_map` -- DOM walk over <a href> elements
// -------------------------------------------------------------------

/// Build a tiny DOM containing the given `<a>` elements as direct
/// children of `<body>`. Returns the document plus the NodeIds for
/// each anchor in insertion order.
fn make_anchor_doc(anchors: &[Option<&str>]) -> (Document, Vec<usize>) {
    let mut nodes = vec![
        // 0: Document root
        Node {
            kind: NodeKind::Document,
            parent: None,
            children: vec![1],
        },
        // 1: <html>
        Node {
            kind: NodeKind::Element(ElementData {
                tag: TagName::Html,
                attributes: vec![],
            }),
            parent: Some(0),
            children: vec![2],
        },
        // 2: <body> -- children pushed below.
        Node {
            kind: NodeKind::Element(ElementData {
                tag: TagName::Body,
                attributes: vec![],
            }),
            parent: Some(1),
            children: vec![],
        },
    ];

    let mut anchor_ids = Vec::new();
    for href in anchors {
        let mut attributes = Vec::new();
        if let Some(h) = href {
            attributes.push(Attribute {
                name: "href".to_string(),
                value: (*h).to_string(),
            });
        }
        let id = nodes.len();
        nodes.push(Node {
            kind: NodeKind::Element(ElementData {
                tag: TagName::A,
                attributes,
            }),
            parent: Some(2),
            children: vec![],
        });
        nodes[2].children.push(id);
        anchor_ids.push(id);
    }

    (Document::from_nodes(nodes, 0), anchor_ids)
}

#[test]
fn build_link_map_collects_all_anchors_with_href() {
    let (doc, ids) = make_anchor_doc(&[Some("/one"), Some("/two"), Some("/three")]);
    let map = BrowserWidget::build_link_map(&doc);
    assert_eq!(map.len(), 3);
    assert_eq!(map.get(&ids[0]).map(String::as_str), Some("/one"));
    assert_eq!(map.get(&ids[1]).map(String::as_str), Some("/two"));
    assert_eq!(map.get(&ids[2]).map(String::as_str), Some("/three"));
}

#[test]
fn build_link_map_skips_anchors_without_href() {
    // <a> with no href is not navigable -- e.g. a JS-driven button.
    let (doc, ids) = make_anchor_doc(&[Some("/one"), None, Some("/three")]);
    let map = BrowserWidget::build_link_map(&doc);
    assert_eq!(map.len(), 2);
    assert!(map.contains_key(&ids[0]));
    assert!(!map.contains_key(&ids[1]));
    assert!(map.contains_key(&ids[2]));
}

#[test]
fn build_link_map_empty_when_no_anchors() {
    let (doc, _) = make_anchor_doc(&[]);
    let map = BrowserWidget::build_link_map(&doc);
    assert!(map.is_empty());
}

#[test]
fn build_link_map_ignores_non_anchor_elements() {
    // A <p> with an `href`-shaped attribute should not be collected --
    // build_link_map filters by tag name first.
    let mut nodes = vec![
        Node {
            kind: NodeKind::Document,
            parent: None,
            children: vec![1],
        },
        Node {
            kind: NodeKind::Element(ElementData {
                tag: TagName::Body,
                attributes: vec![],
            }),
            parent: Some(0),
            children: vec![2],
        },
        Node {
            kind: NodeKind::Element(ElementData {
                tag: TagName::P,
                attributes: vec![Attribute {
                    name: "href".to_string(),
                    value: "/should-not-match".to_string(),
                }],
            }),
            parent: Some(1),
            children: vec![],
        },
    ];
    nodes[1].children.push(2);
    let doc = Document::from_nodes(nodes, 0);
    let map = BrowserWidget::build_link_map(&doc);
    assert!(map.is_empty());
}

#[test]
fn build_link_map_collects_nested_anchors() {
    // <body> > <div> > <a href="/inner">. The walk visits every node
    // by index, so depth doesn't matter.
    let nodes = vec![
        Node {
            kind: NodeKind::Document,
            parent: None,
            children: vec![1],
        },
        Node {
            kind: NodeKind::Element(ElementData {
                tag: TagName::Body,
                attributes: vec![],
            }),
            parent: Some(0),
            children: vec![2],
        },
        Node {
            kind: NodeKind::Element(ElementData {
                tag: TagName::Div,
                attributes: vec![],
            }),
            parent: Some(1),
            children: vec![3],
        },
        Node {
            kind: NodeKind::Element(ElementData {
                tag: TagName::A,
                attributes: vec![Attribute {
                    name: "href".to_string(),
                    value: "/inner".to_string(),
                }],
            }),
            parent: Some(2),
            children: vec![],
        },
    ];
    let doc = Document::from_nodes(nodes, 0);
    let map = BrowserWidget::build_link_map(&doc);
    assert_eq!(map.len(), 1);
    assert_eq!(map.get(&3).map(String::as_str), Some("/inner"));
}

// -------------------------------------------------------------------
// `process_response` -- content-type dispatch + cache insertion
// -------------------------------------------------------------------

fn fresh_browser() -> BrowserWidget {
    let mut config = BrowserConfig::default();
    config.features.home_url = "vfs://test/home/index.html".to_string();
    BrowserWidget::new(config)
}

fn html_response(url: &str, body: &str) -> ResourceResponse {
    ResourceResponse {
        url: url.to_string(),
        content_type: ContentType::Html,
        body: body.as_bytes().to_vec(),
        status: 200,
    }
}

#[test]
fn process_response_html_loads_document() {
    let mut browser = fresh_browser();
    let resp = html_response(
        "vfs://test/page.html",
        "<html><body><h1>Hello</h1></body></html>",
    );
    browser.process_response(resp);
    let doc = browser.document.as_ref().expect("document parsed");
    assert!(doc.nodes.len() >= 4);
}

#[test]
fn process_response_inserts_into_cache() {
    let mut browser = fresh_browser();
    let url = "vfs://test/cached.html";
    let resp = html_response(url, "<html><body>Cached</body></html>");
    browser.process_response(resp);
    assert!(
        browser.cache.contains(url),
        "process_response should populate the cache for {url}",
    );
}

#[test]
fn process_response_unknown_falls_through_to_html() {
    let mut browser = fresh_browser();
    // `Unknown` content-type takes the same path as Html.
    let mut resp = html_response("vfs://test/binary", "<html><body>OK</body></html>");
    resp.content_type = ContentType::Unknown;
    browser.process_response(resp);
    assert!(browser.document.is_some());
}

#[test]
fn process_response_unsupported_type_renders_message() {
    let mut browser = fresh_browser();
    // Font is not displayable inline -- the fallback wraps a message.
    let mut resp = html_response("vfs://test/blob", "");
    resp.content_type = ContentType::FontTtf;
    browser.process_response(resp);
    let doc = browser.document.as_ref().expect("placeholder doc parsed");
    assert!(doc.nodes.len() >= 3);
}

// -------------------------------------------------------------------
// `load_html` size guard + state reset across navigations
// -------------------------------------------------------------------

#[test]
fn load_html_truncates_oversized_input() {
    let mut browser = fresh_browser();
    // 10 MB + some -- guard kicks in. The wrapper itself is valid HTML
    // so the parser doesn't choke on what it actually receives.
    let mut huge = String::from("<html><body>");
    huge.push_str(&"<p>x</p>".repeat(2_000_000));
    huge.push_str("</body></html>");
    assert!(huge.len() > 10 * 1024 * 1024);
    browser.load_html(&huge, "vfs://test/big.html");
    // Document should still be present -- truncation produces an
    // error page rather than a panic or null state.
    assert!(browser.document.is_some());
}

#[test]
fn load_html_link_map_rebuilds_per_page() {
    // build_link_map is idempotent over the *current* document.
    // Loading a new page should produce a fresh map.
    let mut browser = fresh_browser();
    browser.load_html(
        "<html><body><a href=\"/a\">A</a><a href=\"/b\">B</a></body></html>",
        "vfs://test/first.html",
    );
    let first = BrowserWidget::build_link_map(browser.document.as_ref().expect("first doc"));
    assert_eq!(first.len(), 2);

    browser.load_html(
        "<html><body><a href=\"/only\">Only</a></body></html>",
        "vfs://test/second.html",
    );
    let second = BrowserWidget::build_link_map(browser.document.as_ref().expect("second doc"));
    assert_eq!(second.len(), 1);
}
