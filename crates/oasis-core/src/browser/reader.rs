//! Reader mode: extract article text from complex HTML pages.
//!
//! Uses a heuristic scoring algorithm inspired by Mozilla's
//! Readability.js.

use crate::browser::html::dom::{Document, NodeId, NodeKind, TagName};

/// Extracted article content.
#[derive(Debug, Clone)]
pub struct Article {
    /// Article title (from `<title>` or first `<h1>`).
    pub title: String,
    /// The [`NodeId`] of the identified article container.
    pub content_node: NodeId,
    /// Simplified HTML for re-rendering.
    pub html: String,
}

/// Extract the main article content from a DOM tree.
pub fn extract_article(doc: &Document) -> Option<Article> {
    let title = extract_title(doc);

    // Score every element.
    let scores = score_elements(doc);

    // Find the highest-scoring element.
    let best_node = scores
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(id, _)| id)?;

    // Check minimum score threshold.
    if scores[best_node] < 10.0 {
        return None;
    }

    // Extract content from the best node.
    let html = extract_content_html(doc, best_node);

    Some(Article {
        title,
        content_node: best_node,
        html,
    })
}

fn extract_title(doc: &Document) -> String {
    // Try <title> first.
    if let Some(title) = doc.title() {
        return title;
    }
    // Try first <h1>.
    find_first_heading(doc, doc.root).unwrap_or_default()
}

fn find_first_heading(doc: &Document, node_id: NodeId) -> Option<String> {
    let node = doc.get(node_id);
    if let NodeKind::Element(elem) = &node.kind
        && matches!(elem.tag, TagName::H1 | TagName::H2)
    {
        return Some(doc.text_content(node_id));
    }
    for &child in &node.children {
        if let Some(text) = find_first_heading(doc, child) {
            return Some(text);
        }
    }
    None
}

/// Score each element for article-ness.
fn score_elements(doc: &Document) -> Vec<f32> {
    let mut scores = vec![0.0f32; doc.nodes.len()];

    for (id, node) in doc.nodes.iter().enumerate() {
        let NodeKind::Element(elem) = &node.kind else {
            continue;
        };

        // Tag-based scoring.
        match &elem.tag {
            TagName::Article | TagName::Main => {
                scores[id] += 10.0;
            },
            TagName::Section => scores[id] += 3.0,
            TagName::Div => scores[id] += 1.0,
            TagName::P => {
                // Paragraphs boost their parent.
                if let Some(parent) = node.parent {
                    let text = doc.text_content(id);
                    let text_len = text.trim().len();
                    if text_len >= 25 {
                        scores[parent] += 1.0 + (text_len as f32 / 100.0).min(3.0);
                    }
                }
            },
            TagName::Nav | TagName::Aside | TagName::Footer | TagName::Header => {
                scores[id] -= 10.0;
            },
            TagName::Form => scores[id] -= 5.0,
            _ => {},
        }

        // Class/ID-based scoring.
        let class_str = elem.get_attribute("class").unwrap_or("");
        let id_str = elem.get_attribute("id").unwrap_or("");
        let combined = format!("{} {}", class_str, id_str).to_lowercase();

        // Positive signals.
        for keyword in &[
            "content",
            "article",
            "post",
            "entry",
            "story",
            "text",
            "body-content",
            "main",
        ] {
            if combined.contains(keyword) {
                scores[id] += 5.0;
            }
        }

        // Negative signals.
        for keyword in &[
            "sidebar", "comment", "menu", "nav", "ad", "banner", "footer", "header", "widget",
            "social", "related", "popup", "modal",
        ] {
            if combined.contains(keyword) {
                scores[id] -= 5.0;
            }
        }

        // Text density: high text-to-markup ratio = article-like.
        let text = doc.text_content(id);
        let child_count = node.children.len();
        if child_count > 0 {
            let density = text.len() as f32 / child_count as f32;
            if density > 50.0 {
                scores[id] += 2.0;
            }
        }
    }

    scores
}

/// Extract simplified HTML from the article container.
fn extract_content_html(doc: &Document, node_id: NodeId) -> String {
    let mut html = String::new();
    build_reader_html(doc, node_id, &mut html);

    // Wrap in a reader-mode template.
    format!(
        "<html><head><title>Reader Mode</title></head>\
         <body style=\"margin: 16px; font-size: 14px; \
         line-height: 1.6; max-width: 440px;\">\
         {}</body></html>",
        html
    )
}

fn build_reader_html(doc: &Document, node_id: NodeId, html: &mut String) {
    let node = doc.get(node_id);

    match &node.kind {
        NodeKind::Text(text) => {
            // Escape HTML entities.
            for ch in text.chars() {
                match ch {
                    '<' => html.push_str("&lt;"),
                    '>' => html.push_str("&gt;"),
                    '&' => html.push_str("&amp;"),
                    '"' => html.push_str("&quot;"),
                    _ => html.push(ch),
                }
            }
        },
        NodeKind::Element(elem) => {
            // Only keep certain tags in reader mode.
            let keep_tag = matches!(
                elem.tag,
                TagName::P
                    | TagName::H1
                    | TagName::H2
                    | TagName::H3
                    | TagName::H4
                    | TagName::H5
                    | TagName::H6
                    | TagName::Ul
                    | TagName::Ol
                    | TagName::Li
                    | TagName::Blockquote
                    | TagName::Pre
                    | TagName::Code
                    | TagName::Em
                    | TagName::Strong
                    | TagName::B
                    | TagName::I
                    | TagName::A
                    | TagName::Img
                    | TagName::Br
                    | TagName::Hr
                    | TagName::Div
                    | TagName::Span
                    | TagName::Figure
                    | TagName::Figcaption
                    | TagName::Table
                    | TagName::Tr
                    | TagName::Td
                    | TagName::Th
            );

            if keep_tag {
                let tag = elem.tag.as_str();
                html.push('<');
                html.push_str(tag);

                // Keep href, src, alt attributes.
                if let Some(href) = elem.get_attribute("href") {
                    html.push_str(&format!(" href=\"{}\"", href));
                }
                if let Some(src) = elem.get_attribute("src") {
                    html.push_str(&format!(" src=\"{}\"", src));
                }
                if let Some(alt) = elem.get_attribute("alt") {
                    html.push_str(&format!(" alt=\"{}\"", alt));
                }
                html.push('>');

                for &child in &node.children {
                    build_reader_html(doc, child, html);
                }

                // Close non-void tags.
                if !elem.tag.is_void() {
                    html.push_str("</");
                    html.push_str(tag);
                    html.push('>');
                }
            } else {
                // Skip the tag but process children.
                for &child in &node.children {
                    build_reader_html(doc, child, html);
                }
            }
        },
        _ => {
            for &child in &node.children {
                build_reader_html(doc, child, html);
            }
        },
    }
}

/// Generate a reader mode stylesheet override.
pub fn reader_stylesheet() -> &'static str {
    "body { \
       margin: 16px; \
       font-size: 14px; \
       line-height: 1.6; \
       max-width: 440px; \
       color: #222; \
       background-color: #fafafa; \
     } \
     img { max-width: 100%; height: auto; } \
     h1, h2, h3 { margin-top: 1em; margin-bottom: 0.5em; } \
     p { margin: 0.8em 0; } \
     a { color: #0066cc; } \
     blockquote { \
       border-left: 3px solid #ccc; \
       padding-left: 10px; \
       color: #555; \
       font-style: italic; \
     } \
     pre { \
       background-color: #f0f0f0; \
       padding: 8px; \
       overflow: hidden; \
       font-size: 11px; \
     }"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::html::dom::{Attribute, Document, ElementData, NodeKind, TagName};

    /// Build a minimal document with an `<article>` containing
    /// paragraphs, so `extract_article` can find it.
    fn make_article_doc() -> Document {
        let mut doc = Document::new();
        // html
        let html = doc.add_node(NodeKind::Element(ElementData::new(TagName::Html)));
        doc.append_child(doc.root, html);

        // head > title
        let head = doc.add_node(NodeKind::Element(ElementData::new(TagName::Head)));
        doc.append_child(html, head);
        let title_el = doc.add_node(NodeKind::Element(ElementData::new(TagName::Title)));
        doc.append_child(head, title_el);
        let title_text = doc.add_node(NodeKind::Text("Test Article Title".to_string()));
        doc.append_child(title_el, title_text);

        // body
        let body = doc.add_node(NodeKind::Element(ElementData::new(TagName::Body)));
        doc.append_child(html, body);

        // article
        let article = doc.add_node(NodeKind::Element(ElementData::new(TagName::Article)));
        doc.append_child(body, article);

        // Two substantial paragraphs inside the article.
        for text in &[
            "This is the first paragraph with enough text \
             to pass the minimum threshold for scoring.",
            "And here is a second paragraph that also has \
             plenty of text content for the reader mode.",
        ] {
            let p = doc.add_node(NodeKind::Element(ElementData::new(TagName::P)));
            doc.append_child(article, p);
            let t = doc.add_node(NodeKind::Text(text.to_string()));
            doc.append_child(p, t);
        }

        doc
    }

    #[test]
    fn extract_article_from_simple_html() {
        let doc = make_article_doc();
        let article = extract_article(&doc).expect("should extract article");
        assert_eq!(article.title, "Test Article Title");
        assert!(article.html.contains("<p>"));
        assert!(article.html.contains("first paragraph"));
    }

    #[test]
    fn score_article_tag_higher_than_nav() {
        let mut doc = Document::new();
        let html = doc.add_node(NodeKind::Element(ElementData::new(TagName::Html)));
        doc.append_child(doc.root, html);

        let nav = doc.add_node(NodeKind::Element(ElementData::new(TagName::Nav)));
        doc.append_child(html, nav);

        let article = doc.add_node(NodeKind::Element(ElementData::new(TagName::Article)));
        doc.append_child(html, article);

        let scores = score_elements(&doc);
        assert!(
            scores[article] > scores[nav],
            "article score ({}) should exceed nav score ({})",
            scores[article],
            scores[nav],
        );
    }

    #[test]
    fn negative_scoring_for_sidebar_classes() {
        let mut doc = Document::new();
        let html = doc.add_node(NodeKind::Element(ElementData::new(TagName::Html)));
        doc.append_child(doc.root, html);

        let mut sidebar_data = ElementData::new(TagName::Div);
        sidebar_data.attributes.push(Attribute {
            name: "class".to_string(),
            value: "sidebar widget-area".to_string(),
        });
        let sidebar = doc.add_node(NodeKind::Element(sidebar_data));
        doc.append_child(html, sidebar);

        let scores = score_elements(&doc);
        // div base +1, "sidebar" -5, "widget" -5 => -9
        assert!(
            scores[sidebar] < 0.0,
            "sidebar score ({}) should be negative",
            scores[sidebar],
        );
    }

    #[test]
    fn extract_content_html_preserves_safe_tags() {
        let doc = make_article_doc();
        let article = extract_article(&doc).expect("should extract article");
        // <p> tags should be kept.
        assert!(article.html.contains("<p>"));
        assert!(article.html.contains("</p>"));
    }

    #[test]
    fn strip_unsafe_nav_tags_from_output() {
        let mut doc = Document::new();
        let html = doc.add_node(NodeKind::Element(ElementData::new(TagName::Html)));
        doc.append_child(doc.root, html);
        let body = doc.add_node(NodeKind::Element(ElementData::new(TagName::Body)));
        doc.append_child(html, body);

        // article with a <nav> child that should be stripped.
        let article = doc.add_node(NodeKind::Element(ElementData::new(TagName::Article)));
        doc.append_child(body, article);

        let nav = doc.add_node(NodeKind::Element(ElementData::new(TagName::Nav)));
        doc.append_child(article, nav);
        let nav_text = doc.add_node(NodeKind::Text("Navigation links".to_string()));
        doc.append_child(nav, nav_text);

        // Substantial paragraphs to ensure article scores high.
        for text in &[
            "First paragraph of content that is long enough \
             to meet the scoring threshold requirement here.",
            "Second paragraph of content that is also long \
             enough to boost the article element score too.",
        ] {
            let p = doc.add_node(NodeKind::Element(ElementData::new(TagName::P)));
            doc.append_child(article, p);
            let t = doc.add_node(NodeKind::Text(text.to_string()));
            doc.append_child(p, t);
        }

        let result = extract_article(&doc).expect("should extract article");
        // Nav tag itself should not appear in reader HTML.
        assert!(
            !result.html.contains("<nav>"),
            "reader HTML should not contain <nav>"
        );
        // But the text inside the nav still shows (children
        // are traversed even when the tag is stripped).
        assert!(result.html.contains("Navigation links"));
    }

    #[test]
    fn title_extraction_from_h1() {
        let mut doc = Document::new();
        let html = doc.add_node(NodeKind::Element(ElementData::new(TagName::Html)));
        doc.append_child(doc.root, html);
        let body = doc.add_node(NodeKind::Element(ElementData::new(TagName::Body)));
        doc.append_child(html, body);
        let h1 = doc.add_node(NodeKind::Element(ElementData::new(TagName::H1)));
        doc.append_child(body, h1);
        let text = doc.add_node(NodeKind::Text("Heading Title".to_string()));
        doc.append_child(h1, text);

        // No <title> element, so extract_title falls back to h1.
        let title = extract_title(&doc);
        assert_eq!(title, "Heading Title");
    }
}
