//! Integration tests for the browser engine: HTML -> DOM -> CSS -> Layout.
//!
//! These tests exercise the full pipeline from raw HTML source through DOM
//! construction, CSS cascade (including the UA stylesheet), and block layout,
//! without requiring a backend (no rendering).

use std::collections::HashMap;

use oasis_browser::internals::{
    BoxType, CascadeContext, ComputedStyle, Display, LayoutBox, NodeKind, Stylesheet, TagName,
    TextDecorationLine, TextMeasurer, Tokenizer, TreeBuilder, build_layout_tree,
    default_stylesheet, style_tree,
};

// -------------------------------------------------------------------
// Helpers
// -------------------------------------------------------------------

/// A simple text measurer that returns 6px per character (monospace).
struct FixedMeasurer;

impl TextMeasurer for FixedMeasurer {
    fn measure_text(&self, text: &str, _font_size: u16) -> u32 {
        (text.len() as u32) * 6
    }
}

/// Parse an HTML string into a DOM Document.
fn parse(html: &str) -> oasis_browser::internals::Document {
    let tokens = Tokenizer::new(html).tokenize();
    TreeBuilder::build(tokens)
}

/// Run the full pipeline: parse HTML -> style with UA sheet -> layout.
fn pipeline(
    html: &str,
) -> (
    oasis_browser::internals::Document,
    Vec<Option<ComputedStyle>>,
    LayoutBox,
) {
    let doc = parse(html);
    let ua = default_stylesheet();
    let sheets: Vec<&Stylesheet> = vec![&ua];
    let ctx = CascadeContext::default();
    let styles = style_tree(&doc, &sheets, &[], &ctx);
    let layout = build_layout_tree(
        &doc,
        &styles,
        &FixedMeasurer,
        480.0,
        272.0,
        None,
        &HashMap::new(),
    );
    (doc, styles, layout)
}

/// Run parse + style only (no layout).
fn parse_and_style(
    html: &str,
) -> (
    oasis_browser::internals::Document,
    Vec<Option<ComputedStyle>>,
) {
    let doc = parse(html);
    let ua = default_stylesheet();
    let sheets: Vec<&Stylesheet> = vec![&ua];
    let ctx = CascadeContext::default();
    let styles = style_tree(&doc, &sheets, &[], &ctx);
    (doc, styles)
}

/// Run parse + style with an additional author stylesheet.
fn parse_and_style_with_css(
    html: &str,
    css: &str,
) -> (
    oasis_browser::internals::Document,
    Vec<Option<ComputedStyle>>,
) {
    let doc = parse(html);
    let ua = default_stylesheet();
    let author = Stylesheet::parse(css);
    let sheets: Vec<&Stylesheet> = vec![&ua, &author];
    let ctx = CascadeContext::default();
    let styles = style_tree(&doc, &sheets, &[], &ctx);
    (doc, styles)
}

/// Find the first element with the given tag in the document.
fn find_tag(doc: &oasis_browser::internals::Document, tag: TagName) -> Option<usize> {
    doc.nodes.iter().enumerate().find_map(|(id, node)| {
        if let NodeKind::Element(data) = &node.kind {
            if data.tag == tag { Some(id) } else { None }
        } else {
            None
        }
    })
}

/// Find all elements with the given tag in the document.
fn find_all_tags(doc: &oasis_browser::internals::Document, tag: TagName) -> Vec<usize> {
    doc.nodes
        .iter()
        .enumerate()
        .filter_map(|(id, node)| {
            if let NodeKind::Element(data) = &node.kind {
                if data.tag == tag { Some(id) } else { None }
            } else {
                None
            }
        })
        .collect()
}

/// Recursively find all layout boxes matching a predicate.
fn find_layout_boxes<'a, F>(root: &'a LayoutBox, pred: &F) -> Vec<&'a LayoutBox>
where
    F: Fn(&LayoutBox) -> bool,
{
    let mut result = Vec::new();
    if pred(root) {
        result.push(root);
    }
    for child in &root.children {
        result.extend(find_layout_boxes(child, pred));
    }
    result
}

// ===================================================================
// Tests
// ===================================================================

#[test]
fn simple_page_paragraph_block_with_positive_height() {
    let (_doc, _styles, layout) = pipeline("<html><body><p>Hello</p></body></html>");

    // The root layout box should be block-level.
    assert!(
        matches!(layout.box_type, BoxType::Block),
        "root should be Block, got {:?}",
        layout.box_type,
    );

    // The root should have positive height (content was laid out).
    assert!(
        layout.dimensions.content.height > 0.0,
        "root height should be positive, got {}",
        layout.dimensions.content.height,
    );

    // Find the paragraph's layout box -- it should be block-level and
    // have a positive height from the text content.
    let p_boxes = find_layout_boxes(&layout, &|b| {
        b.style.display == Display::Block
            && b.text.is_none()
            && b.children.iter().any(|c| {
                c.text
                    .as_deref()
                    .map(|t| t.contains("Hello"))
                    .unwrap_or(false)
                    || !find_layout_boxes(c, &|inner| {
                        inner
                            .text
                            .as_deref()
                            .map(|t| t.contains("Hello"))
                            .unwrap_or(false)
                    })
                    .is_empty()
            })
    });

    // There should be at least one block box containing "Hello".
    assert!(
        !p_boxes.is_empty(),
        "should find a block box containing 'Hello' text",
    );

    for p in &p_boxes {
        assert!(
            p.dimensions.content.height > 0.0,
            "paragraph box height should be positive, got {}",
            p.dimensions.content.height,
        );
    }
}

#[test]
fn nested_structure_dom_tree_and_style_inheritance() {
    let html = r#"<html><body>
        <div>
            <p>Outer paragraph</p>
            <div>
                <span>Nested span</span>
                <p>Inner paragraph</p>
            </div>
        </div>
    </body></html>"#;

    let (doc, styles) = parse_and_style(html);

    // Verify body exists.
    let body_id = doc.body().expect("should have <body>");

    // Body should have at least one child (the outer div).
    assert!(
        !doc.get(body_id).children.is_empty(),
        "body should have children",
    );

    // Verify the outer div exists and is block-level.
    let divs = find_all_tags(&doc, TagName::Div);
    assert!(
        divs.len() >= 2,
        "should have at least 2 divs, got {}",
        divs.len()
    );

    let outer_div = divs[0];
    let outer_style = styles[outer_div].as_ref().expect("div should have style");
    assert_eq!(outer_style.display, Display::Block, "div should be block");

    // Verify spans are inline.
    let spans = find_all_tags(&doc, TagName::Span);
    assert!(!spans.is_empty(), "should have at least one span");
    let span_style = styles[spans[0]].as_ref().expect("span should have style");
    assert_eq!(span_style.display, Display::Inline, "span should be inline");

    // Verify color inheritance: set color on body via UA stylesheet,
    // children should inherit it (unless overridden).
    let body_style = styles[body_id].as_ref().expect("body should have style");
    let body_color = body_style.color;
    // Nested elements should inherit color from their ancestor.
    let inner_p = find_all_tags(&doc, TagName::P);
    for p_id in &inner_p {
        let p_style = styles[*p_id].as_ref().expect("p should have style");
        assert_eq!(
            p_style.color, body_color,
            "paragraph should inherit color from body",
        );
    }
}

#[test]
fn css_specificity_most_specific_wins() {
    let html = r#"<html><head></head><body>
        <p id="main" class="highlight">Styled text</p>
    </body></html>"#;

    // Class selector (specificity 0,1,0) sets color red.
    // ID selector (specificity 1,0,0) sets color blue.
    // ID should win.
    let css = r#"
        .highlight { color: red; }
        #main { color: blue; }
    "#;

    let (doc, styles) = parse_and_style_with_css(html, css);
    let p_id = find_tag(&doc, TagName::P).expect("should find <p>");
    let p_style = styles[p_id].as_ref().expect("p should have style");

    // Blue = rgb(0, 0, 255).
    assert_eq!(p_style.color.r, 0, "color.r should be 0 (blue wins)");
    assert_eq!(p_style.color.g, 0, "color.g should be 0 (blue wins)");
    assert_eq!(p_style.color.b, 255, "color.b should be 255 (blue wins)");
}

#[test]
fn table_layout_cells_laid_out() {
    let html = r#"<html><body>
        <table>
            <tr>
                <td>Cell 1</td>
                <td>Cell 2</td>
            </tr>
            <tr>
                <td>Cell 3</td>
                <td>Cell 4</td>
            </tr>
        </table>
    </body></html>"#;

    let (_doc, _styles, layout) = pipeline(html);

    // Find table wrapper boxes.
    let table_boxes = find_layout_boxes(&layout, &|b| matches!(b.box_type, BoxType::TableWrapper));
    assert!(
        !table_boxes.is_empty(),
        "should have at least one TableWrapper box",
    );

    // Find table cell boxes.
    let cell_boxes = find_layout_boxes(&layout, &|b| matches!(b.box_type, BoxType::TableCell));
    assert_eq!(
        cell_boxes.len(),
        4,
        "should have 4 table cells, got {}",
        cell_boxes.len()
    );

    // Each cell should have positive dimensions.
    for (i, cell) in cell_boxes.iter().enumerate() {
        assert!(
            cell.dimensions.content.width > 0.0,
            "cell {} width should be positive",
            i,
        );
        assert!(
            cell.dimensions.content.height > 0.0,
            "cell {} height should be positive",
            i,
        );
    }

    // Find table row boxes.
    let row_boxes = find_layout_boxes(&layout, &|b| matches!(b.box_type, BoxType::TableRow));
    assert_eq!(
        row_boxes.len(),
        2,
        "should have 2 table rows, got {}",
        row_boxes.len()
    );
}

#[test]
fn mixed_inline_and_block_content() {
    let html = r#"<html><body>
        <div>
            <p>Block paragraph</p>
            <span>Inline span</span>
            <p>Another block</p>
        </div>
    </body></html>"#;

    let (_doc, _styles, layout) = pipeline(html);

    // The layout should contain both block and inline boxes.
    let block_boxes = find_layout_boxes(&layout, &|b| matches!(b.box_type, BoxType::Block));
    let inline_boxes = find_layout_boxes(&layout, &|b| matches!(b.box_type, BoxType::Inline));

    assert!(
        !block_boxes.is_empty(),
        "should have block-level boxes from <p> elements",
    );
    assert!(
        !inline_boxes.is_empty(),
        "should have inline boxes from <span> element",
    );

    // When inline content is mixed with block content, anonymous block
    // boxes are created to wrap the inline content.
    let anon_boxes = find_layout_boxes(&layout, &|b| matches!(b.box_type, BoxType::Anonymous));
    assert!(
        !anon_boxes.is_empty(),
        "mixed inline/block content should produce anonymous wrapper boxes",
    );
}

#[test]
fn heading_hierarchy_font_sizes() {
    let html = r#"<html><body>
        <h1>Heading 1</h1>
        <h2>Heading 2</h2>
        <h3>Heading 3</h3>
        <h4>Heading 4</h4>
        <h5>Heading 5</h5>
        <h6>Heading 6</h6>
    </body></html>"#;

    let (doc, styles) = parse_and_style(html);

    let heading_tags = [
        TagName::H1,
        TagName::H2,
        TagName::H3,
        TagName::H4,
        TagName::H5,
        TagName::H6,
    ];

    let mut font_sizes = Vec::new();
    for tag in &heading_tags {
        let id =
            find_tag(&doc, tag.clone()).unwrap_or_else(|| panic!("should find <{}>", tag.as_str()));
        let style = styles[id]
            .as_ref()
            .unwrap_or_else(|| panic!("<{}> should have a computed style", tag.as_str()));
        font_sizes.push(style.font_size);
    }

    // h1 should have the largest font, h6 the smallest.
    // Each heading level should be >= the next one.
    for i in 0..font_sizes.len() - 1 {
        assert!(
            font_sizes[i] >= font_sizes[i + 1],
            "h{} font-size ({}) should be >= h{} font-size ({})",
            i + 1,
            font_sizes[i],
            i + 2,
            font_sizes[i + 1],
        );
    }

    // h1 should be strictly larger than h6.
    assert!(
        font_sizes[0] > font_sizes[5],
        "h1 ({}) should be larger than h6 ({})",
        font_sizes[0],
        font_sizes[5],
    );
}

#[test]
fn link_styling_color_and_underline() {
    let html = r#"<html><body>
        <a href="https://example.com">A link</a>
    </body></html>"#;

    let (doc, styles) = parse_and_style(html);
    let a_id = find_tag(&doc, TagName::A).expect("should find <a>");
    let a_style = styles[a_id].as_ref().expect("<a> should have style");

    // UA stylesheet should give links an underline text-decoration.
    assert_eq!(
        a_style.text_decoration.line,
        TextDecorationLine::UNDERLINE,
        "links should have underline decoration",
    );

    // UA stylesheet typically gives links a distinct color (not black).
    // The default body color is black (0,0,0), so link color should differ.
    let body_id = doc.body().expect("should have body");
    let body_style = styles[body_id].as_ref().expect("body should have style");
    assert_ne!(
        a_style.color, body_style.color,
        "link color should differ from body text color",
    );
}

#[test]
fn css_class_selector_applied() {
    let html = r#"<html><head></head><body>
        <div class="red-box">Content</div>
        <div>Plain div</div>
    </body></html>"#;

    let css = r#"
        .red-box {
            background-color: red;
            padding: 10px;
        }
    "#;

    let (doc, styles) = parse_and_style_with_css(html, css);
    let divs = find_all_tags(&doc, TagName::Div);
    assert!(divs.len() >= 2, "should have at least 2 divs");

    // Find which div has class "red-box".
    let red_div = divs
        .iter()
        .find(|&&id| {
            doc.element(id)
                .map(|e| e.has_class("red-box"))
                .unwrap_or(false)
        })
        .expect("should find div with class red-box");

    let plain_div = divs
        .iter()
        .find(|&&id| {
            doc.element(id)
                .map(|e| !e.has_class("red-box"))
                .unwrap_or(false)
        })
        .expect("should find plain div");

    let red_style = styles[*red_div]
        .as_ref()
        .expect("red div should have style");
    let plain_style = styles[*plain_div]
        .as_ref()
        .expect("plain div should have style");

    // The red-box div should have red background (255, 0, 0).
    assert_eq!(red_style.background_color.r, 255, "red-box background red");
    assert_eq!(red_style.background_color.g, 0, "red-box background green");
    assert_eq!(red_style.background_color.b, 0, "red-box background blue");

    // The red-box div should have 10px padding.
    assert_eq!(red_style.padding_top, 10.0, "red-box padding-top");
    assert_eq!(red_style.padding_left, 10.0, "red-box padding-left");

    // The plain div should NOT have the red background.
    assert_ne!(
        plain_style.background_color.r, 255,
        "plain div should not have red background",
    );
}

#[test]
fn nested_lists_display_and_indentation() {
    let html = r#"<html><body>
        <ul>
            <li>Item 1
                <ul>
                    <li>Nested A</li>
                    <li>Nested B</li>
                </ul>
            </li>
            <li>Item 2</li>
        </ul>
    </body></html>"#;

    let (doc, styles) = parse_and_style(html);

    // Verify <ul> elements are block-level.
    let uls = find_all_tags(&doc, TagName::Ul);
    assert!(
        uls.len() >= 2,
        "should have at least 2 <ul> elements (outer + nested)"
    );

    for ul_id in &uls {
        let ul_style = styles[*ul_id].as_ref().expect("ul should have style");
        assert_eq!(ul_style.display, Display::Block, "ul should be block");
    }

    // Verify <li> elements have list-item display.
    let lis = find_all_tags(&doc, TagName::Li);
    assert!(lis.len() >= 4, "should have at least 4 <li> elements");

    for li_id in &lis {
        let li_style = styles[*li_id].as_ref().expect("li should have style");
        assert_eq!(
            li_style.display,
            Display::ListItem,
            "li should have list-item display",
        );
    }

    // Now do layout and verify the nested list is indented.
    let (_doc, _styles, layout) = pipeline(html);

    // Find list-item boxes.
    let list_item_boxes =
        find_layout_boxes(&layout, &|b| matches!(b.box_type, BoxType::ListItem { .. }));
    assert!(
        list_item_boxes.len() >= 4,
        "should have at least 4 list-item layout boxes, got {}",
        list_item_boxes.len(),
    );

    // The nested list items should have a larger x position (indented)
    // than the top-level items due to the nested <ul>'s padding/margin.
    // Collect x positions.
    let xs: Vec<f32> = list_item_boxes
        .iter()
        .map(|b| b.dimensions.content.x)
        .collect();

    // There should be at least two distinct x values (outer vs nested).
    let mut unique_xs: Vec<f32> = xs.clone();
    unique_xs.sort_by(|a, b| a.partial_cmp(b).expect("x positions should be finite"));
    unique_xs.dedup_by(|a, b| (*a - *b).abs() < 0.5);
    assert!(
        unique_xs.len() >= 2,
        "nested lists should produce items at different x positions \
         (indentation), got x values: {:?}",
        xs,
    );
}

// -------------------------------------------------------------------
// Performance regression: complex page with gradients must not freeze
// -------------------------------------------------------------------

#[test]
fn complex_page_with_gradients_completes_within_budget() {
    use std::time::Instant;

    // Simulate a Wikipedia-like page: many elements, CSS gradients,
    // nested structure, tables, and large blocks.
    let html = r#"<html><head><style>
        body { background: linear-gradient(135deg, #f5f7fa 0%, #c3cfe2 100%); }
        .hero { background: radial-gradient(circle, #667eea, #764ba2);
                width: 100%; height: 120px; }
        .card { background: linear-gradient(to bottom, #fff, #eee);
                border: 1px solid #ccc; padding: 8px; margin: 4px; }
        .nav { background: linear-gradient(90deg, #333, #555); color: #fff;
               padding: 4px 8px; }
        table { border-collapse: collapse; width: 100%; }
        td, th { border: 1px solid #aaa; padding: 4px; }
    </style></head><body>
        <div class="nav">Home | About | Contact | Help | Search</div>
        <div class="hero"><h1>Welcome to the Encyclopedia</h1></div>
        <div class="card"><h2>Featured Article</h2>
            <p>Lorem ipsum dolor sit amet, consectetur adipiscing elit.
            Sed do eiusmod tempor incididunt ut labore et dolore magna aliqua.
            Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris.</p>
        </div>
        <div class="card"><h2>Did you know?</h2>
            <ul><li>Fact one about the world</li>
                <li>Fact two about science</li>
                <li>Fact three about history</li>
                <li>Fact four about technology</li></ul>
        </div>
        <div class="card"><h2>Statistics</h2>
            <table>
                <tr><th>Category</th><th>Count</th><th>Updated</th></tr>
                <tr><td>Articles</td><td>6,800,000</td><td>Today</td></tr>
                <tr><td>Editors</td><td>45,000</td><td>This month</td></tr>
                <tr><td>Languages</td><td>300+</td><td>Ongoing</td></tr>
            </table>
        </div>
        <div class="card"><h2>Recent Changes</h2>
            <p>Edit 1: Updated article on quantum physics</p>
            <p>Edit 2: Fixed citation in biology article</p>
            <p>Edit 3: Added new section to mathematics</p>
            <p>Edit 4: Revised geography references</p>
        </div>
    </body></html>"#;

    let start = Instant::now();

    // Phase 1: Parse + cascade.
    let doc = parse(html);
    let ua = default_stylesheet();
    let author_css = doc
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(id, node)| {
            if let NodeKind::Element(data) = &node.kind {
                if data.tag == TagName::Style {
                    Some(doc.text_content(id))
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    let author_sheets: Vec<Stylesheet> = author_css
        .iter()
        .map(|css| Stylesheet::parse(css))
        .collect();
    let mut sheets: Vec<&Stylesheet> = vec![&ua];
    for s in &author_sheets {
        sheets.push(s);
    }
    let ctx = CascadeContext::default();
    let styles = style_tree(&doc, &sheets, &[], &ctx);

    let cascade_elapsed = start.elapsed();

    // Phase 2: Layout.
    let layout = build_layout_tree(
        &doc,
        &styles,
        &FixedMeasurer,
        480.0,
        272.0,
        None,
        &HashMap::new(),
    );

    let total_elapsed = start.elapsed();

    // Budget: parse + cascade + layout must complete in under 500ms.
    // Typical time on modern hardware is ~1-5ms for this page.
    assert!(
        total_elapsed.as_millis() < 500,
        "complex page pipeline took {}ms (cascade: {}ms, total: {}ms) \
         — exceeds 500ms budget; likely a performance regression",
        total_elapsed.as_millis(),
        cascade_elapsed.as_millis(),
        total_elapsed.as_millis(),
    );

    // Verify layout produced meaningful content.
    assert!(
        layout.dimensions.content.height > 0.0,
        "layout should have positive height",
    );
}
