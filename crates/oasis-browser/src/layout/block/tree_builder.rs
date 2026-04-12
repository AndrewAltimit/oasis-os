//! DOM-to-layout-tree construction.
//!
//! Walks the DOM tree and builds a corresponding [`LayoutBox`] tree,
//! handling element box type determination, replaced elements, pseudo-
//! elements (`::before`/`::after`), anonymous box wrapping, and
//! insignificant whitespace elimination.

use super::{TextMeasurer, layout_block_with_height, offset_descendant};
use crate::css::values::{ComputedStyle, Dimension, Display, ListStyleType};
use crate::html::dom::{Document, ElementData, NodeId, NodeKind, TagName};
use crate::layout::box_model::*;
use crate::layout::positioning::apply_positioning;
use oasis_types::backend::Color;

use std::collections::HashMap;

/// Build a layout tree from a styled DOM tree.
///
/// Starts from the `<body>` element (or the document root if no body
/// is found). The returned `LayoutBox` is the root block box with its
/// dimensions laid out to fit the given viewport.
///
/// `base_url` and `image_info` are used to resolve `<img>` elements to
/// their intrinsic dimensions from decoded images. Pass `None` and an
/// empty map when image data is not available.
pub fn build_layout_tree(
    doc: &Document,
    styles: &[Option<ComputedStyle>],
    measurer: &dyn TextMeasurer,
    viewport_width: f32,
    _viewport_height: f32,
    base_url: Option<&str>,
    image_info: &HashMap<String, (u32, u32)>,
) -> LayoutBox {
    let start_node = doc.body().unwrap_or(doc.root);
    let style = styles
        .get(start_node)
        .and_then(|s| s.clone())
        .unwrap_or_else(|| ComputedStyle {
            display: Display::Block,
            ..ComputedStyle::default()
        });

    let mut root = LayoutBox::new(BoxType::Block, style, Some(start_node));

    // Collect child IDs to avoid holding a borrow on the node while
    // recursing into `build_children` (which also borrows `doc`).
    let children = doc.get(start_node).children.clone();
    let child_boxes = build_children(doc, &children, styles, base_url, image_info);
    root.children = wrap_anonymous(child_boxes, &root.style);

    // Layout from the root.
    root.dimensions.content.x = 0.0;
    root.dimensions.content.y = 0.0;
    layout_block_with_height(&mut root, viewport_width, Some(_viewport_height), measurer);

    // Shift root content origin to include root's own box-model edges.
    // layout_block_children no longer adds parent padding to children,
    // so the root's edges must be baked into content.x/y.
    let dx =
        root.dimensions.margin.left + root.dimensions.border.left + root.dimensions.padding.left;
    let dy = root.dimensions.margin.top + root.dimensions.border.top + root.dimensions.padding.top;
    if dx != 0.0 || dy != 0.0 {
        offset_descendant(&mut root, dx, dy);
    }

    // Apply CSS positioning (relative/absolute/fixed) as a post-pass.
    let viewport_rect = Rect::new(0.0, 0.0, viewport_width, _viewport_height);
    apply_positioning(&mut root, viewport_rect);

    root
}

/// Recursively build child layout boxes for a list of DOM node IDs.
pub(super) fn build_children(
    doc: &Document,
    children: &[NodeId],
    styles: &[Option<ComputedStyle>],
    base_url: Option<&str>,
    image_info: &HashMap<String, (u32, u32)>,
) -> Vec<LayoutBox> {
    let mut boxes = Vec::with_capacity(children.len());
    for &child_id in children {
        if let Some(lb) = build_box_for_node(doc, child_id, styles, base_url, image_info) {
            boxes.push(lb);
        }
    }
    boxes
}

/// Build a single layout box for a DOM node. Returns `None` for
/// `display: none`, comments, and nodes without styles.
fn build_box_for_node(
    doc: &Document,
    node_id: NodeId,
    styles: &[Option<ComputedStyle>],
    base_url: Option<&str>,
    image_info: &HashMap<String, (u32, u32)>,
) -> Option<LayoutBox> {
    let node = doc.get(node_id);

    match &node.kind {
        NodeKind::Element(elem) => {
            let style = styles.get(node_id)?.clone()?;
            if style.display == Display::None {
                return None;
            }

            // Hide children of closed <details> (except <summary>).
            // A <details> element without the `open` attribute hides all
            // non-<summary> children.
            if elem.tag != TagName::Summary
                && let Some(parent_id) = node.parent
                && let NodeKind::Element(parent_elem) = &doc.get(parent_id).kind
                && parent_elem.tag == TagName::Details
                && parent_elem.get_attribute("open").is_none()
            {
                return None;
            }

            // Determine box type.
            let box_type = box_type_for_element(elem, &style);

            // Handle <canvas> as a replaced element.
            if elem.tag == TagName::Canvas {
                let w = elem
                    .get_attribute("width")
                    .and_then(|v| v.parse::<u32>().ok())
                    .unwrap_or(300);
                let h = elem
                    .get_attribute("height")
                    .and_then(|v| v.parse::<u32>().ok())
                    .unwrap_or(150);
                let state = std::rc::Rc::new(std::cell::RefCell::new(
                    crate::canvas::CanvasState::new(w, h),
                ));
                let replaced = ReplacedContent::Canvas { state };
                let mut lb = LayoutBox::new(BoxType::Replaced(replaced), style, Some(node_id));
                lb.children = Vec::new();
                return Some(lb);
            }

            // Handle <svg> as a replaced element.
            if elem.tag == TagName::Svg
                && let Some(svg_elem) = crate::svg::parse_svg(doc, node_id)
            {
                let replaced = ReplacedContent::Svg {
                    element: Box::new(svg_elem),
                };
                let mut lb = LayoutBox::new(BoxType::Replaced(replaced), style, Some(node_id));
                lb.children = Vec::new();
                return Some(lb);
            }

            // Handle <textarea> as a replaced element.
            if elem.tag == TagName::Textarea {
                let value = collect_text_content(doc, node_id);
                let placeholder = elem.get_attribute("placeholder").unwrap_or("").to_string();
                let rows = elem
                    .get_attribute("rows")
                    .and_then(|v| v.parse::<u32>().ok())
                    .unwrap_or(2);
                let cols = elem
                    .get_attribute("cols")
                    .and_then(|v| v.parse::<u32>().ok())
                    .unwrap_or(20);
                let replaced = ReplacedContent::TextArea {
                    value,
                    placeholder,
                    rows,
                    cols,
                };
                let mut lb = LayoutBox::new(BoxType::Replaced(replaced), style, Some(node_id));
                lb.children = Vec::new();
                return Some(lb);
            }

            // Handle <select> specially: find selected/first <option> text.
            if elem.tag == TagName::Select {
                let (label, options, selected_index) = find_select_info(doc, node_id);
                let replaced = ReplacedContent::SelectBox {
                    label,
                    open: false,
                    options,
                    selected_index,
                };
                let mut lb = LayoutBox::new(BoxType::Replaced(replaced), style, Some(node_id));
                lb.children = Vec::new();
                return Some(lb);
            }

            // Handle <button> specially: collect child text for label.
            if elem.tag == TagName::Button {
                let text = collect_text_content(doc, node_id);
                let label = if !text.trim().is_empty() {
                    text.trim().to_string()
                } else {
                    elem.get_attribute("value").unwrap_or("Button").to_string()
                };
                let replaced = ReplacedContent::SubmitButton { label };
                let mut lb = LayoutBox::new(BoxType::Replaced(replaced), style, Some(node_id));
                lb.children = Vec::new();
                return Some(lb);
            }

            // Handle replaced elements.
            if let Some(replaced) = replaced_content(elem, base_url, image_info) {
                let mut lb = LayoutBox::new(BoxType::Replaced(replaced), style, Some(node_id));
                lb.children = Vec::new();
                return Some(lb);
            }

            let mut lb = LayoutBox::new(box_type, style.clone(), Some(node_id));

            // Generate ::before pseudo-element content.
            let before_box = make_pseudo_box(&style.before_style);

            // Generate ::after pseudo-element content.
            let after_box = make_pseudo_box(&style.after_style);

            // For table cells, encode colspan/rowspan in the style
            // using the convention expected by the table layout engine.
            if matches!(lb.box_type, BoxType::TableCell) {
                if let Some(cs) = elem
                    .get_attribute("colspan")
                    .and_then(|v| v.parse::<usize>().ok())
                    .filter(|&cs| cs > 1)
                {
                    lb.style.min_width = Dimension::Px(cs as f32 * 1000.0);
                }
                if let Some(rs) = elem
                    .get_attribute("rowspan")
                    .and_then(|v| v.parse::<usize>().ok())
                    .filter(|&rs| rs > 1)
                {
                    lb.style.max_width = Dimension::Px(rs as f32 * 1000.0);
                }
            }

            // Collect child IDs to avoid holding a borrow on `node`
            // while recursing into `build_children`.
            let child_ids = node.children.clone();
            let mut child_boxes = build_children(doc, &child_ids, styles, base_url, image_info);

            // Insert ::before and ::after pseudo-element boxes.
            if let Some(before) = before_box {
                child_boxes.insert(0, before);
            }
            if let Some(after) = after_box {
                child_boxes.push(after);
            }

            lb.children = wrap_anonymous(child_boxes, &lb.style);

            Some(lb)
        },
        NodeKind::Text(text) => {
            // Skip whitespace-only text nodes that are between
            // block-level siblings (insignificant whitespace). Keep
            // them when they're between inline siblings so that
            // `<em>hello</em> <strong>world</strong>` preserves
            // the space.
            if text.trim().is_empty() {
                let dominated_by_blocks = has_block_sibling(doc, node_id);
                if dominated_by_blocks {
                    return None;
                }
            }
            let style = find_inherited_style(doc, node_id, styles);
            let mut inline_style = style;
            inline_style.display = Display::Inline;
            let mut lb = LayoutBox::new(BoxType::Inline, inline_style, Some(node_id));
            lb.text = Some(text.clone());
            Some(lb)
        },
        NodeKind::Comment(_) | NodeKind::Document => None,
    }
}

/// Determine the box type for an element node based on its tag and
/// computed style.
fn box_type_for_element(_elem: &ElementData, style: &ComputedStyle) -> BoxType {
    match style.display {
        Display::Block => BoxType::Block,
        Display::Inline => BoxType::Inline,
        Display::InlineBlock => BoxType::InlineBlock,
        Display::Flex | Display::InlineFlex => BoxType::Flex,
        Display::Grid | Display::InlineGrid => BoxType::Grid,
        Display::ListItem => {
            let marker = resolve_list_marker(style);
            BoxType::ListItem { marker }
        },
        Display::Table => BoxType::TableWrapper,
        Display::TableRow => BoxType::TableRow,
        Display::TableCell => BoxType::TableCell,
        Display::None => BoxType::Block, // unreachable in practice
    }
}

/// Resolve the list marker type from the computed style.
fn resolve_list_marker(style: &ComputedStyle) -> ListMarker {
    match style.list_style_type {
        ListStyleType::Disc => ListMarker::Disc,
        ListStyleType::Circle => ListMarker::Circle,
        ListStyleType::Square => ListMarker::Square,
        ListStyleType::Decimal => ListMarker::Decimal(1),
        ListStyleType::None => ListMarker::None,
    }
}

/// Create a layout box for a `::before` or `::after` pseudo-element.
fn make_pseudo_box(pseudo_style: &Option<Box<ComputedStyle>>) -> Option<LayoutBox> {
    let ps = pseudo_style.as_ref()?;
    let content_text = ps.content.as_ref()?;
    let box_type = match ps.display {
        Display::Block => BoxType::Block,
        Display::None => return None,
        _ => BoxType::Inline,
    };
    let mut pseudo = (**ps).clone();
    pseudo.before_content = None;
    pseudo.after_content = None;
    pseudo.before_style = None;
    pseudo.after_style = None;
    pseudo.content = Some(content_text.clone());
    let mut pb = LayoutBox::new(box_type, pseudo, None);
    if !content_text.is_empty() {
        pb.text = Some(content_text.clone());
    }
    Some(pb)
}

/// Check if an element is a replaced element and return its content.
///
/// For `<img>` elements, uses decoded image intrinsic dimensions when
/// HTML width/height attributes are missing.
fn replaced_content(
    elem: &ElementData,
    base_url: Option<&str>,
    image_info: &HashMap<String, (u32, u32)>,
) -> Option<ReplacedContent> {
    match elem.tag {
        TagName::Img => {
            let mut width = elem
                .get_attribute("width")
                .and_then(|v| v.parse::<u32>().ok())
                .unwrap_or(0);
            let mut height = elem
                .get_attribute("height")
                .and_then(|v| v.parse::<u32>().ok())
                .unwrap_or(0);
            let alt = elem.get_attribute("alt").unwrap_or("").to_string();

            // Use intrinsic dimensions from decoded image if HTML attrs
            // are missing.
            if let Some(src) = elem.src() {
                let resolved = resolve_img_src(base_url, src);
                if let Some(&(iw, ih)) = image_info.get(&resolved) {
                    if width == 0 && height == 0 {
                        width = iw;
                        height = ih;
                    } else if width == 0 && ih > 0 {
                        width = (iw as f32 * height as f32 / ih as f32) as u32;
                    } else if height == 0 && iw > 0 {
                        height = (ih as f32 * width as f32 / iw as f32) as u32;
                    }
                }
            }

            Some(ReplacedContent::Image {
                width,
                height,
                texture: None,
                alt,
                atlas_region: None,
            })
        },
        TagName::Hr => Some(ReplacedContent::HorizontalRule),
        TagName::Br => Some(ReplacedContent::LineBreak),
        TagName::Input => {
            let input_type = elem
                .get_attribute("type")
                .unwrap_or("text")
                .to_ascii_lowercase();
            match input_type.as_str() {
                "hidden" => None,
                "submit" | "button" | "reset" => {
                    let label = elem
                        .get_attribute("value")
                        .unwrap_or(if input_type == "submit" {
                            "Submit"
                        } else if input_type == "reset" {
                            "Reset"
                        } else {
                            "Button"
                        })
                        .to_string();
                    Some(ReplacedContent::SubmitButton { label })
                },
                "checkbox" => {
                    let checked = elem.get_attribute("checked").is_some();
                    Some(ReplacedContent::Checkbox { checked })
                },
                "radio" => {
                    let checked = elem.get_attribute("checked").is_some();
                    Some(ReplacedContent::RadioButton { checked })
                },
                _ => {
                    // text, password, search, etc.
                    let is_password = input_type == "password";
                    let value = elem.get_attribute("value").unwrap_or("").to_string();
                    let placeholder = elem.get_attribute("placeholder").unwrap_or("").to_string();
                    let size = elem
                        .get_attribute("size")
                        .and_then(|v| v.parse::<u32>().ok())
                        .unwrap_or(20);
                    Some(ReplacedContent::TextInput {
                        value,
                        placeholder,
                        size,
                        is_password,
                    })
                },
            }
        },
        // TagName::Button is handled in build_box_for_node before this
        // function is called, so child text content is used for the label.
        _ => None,
    }
}

/// Find the display label for a `<select>` element.
///
/// Searches for the first `<option>` with a `selected` attribute, or
/// falls back to the first `<option>`. Returns the option's text content.
fn find_select_info(doc: &Document, select_id: NodeId) -> (String, Vec<String>, Option<usize>) {
    let mut option_labels = Vec::new();
    let mut selected_idx: Option<usize> = None;
    let mut first_option_idx: Option<usize> = None;
    for &child_id in &doc.get(select_id).children {
        if let NodeKind::Element(ref elem) = doc.get(child_id).kind
            && elem.tag == TagName::Option
        {
            let idx = option_labels.len();
            let text = collect_text_content(doc, child_id);
            let trimmed = text.trim();
            let lbl = if trimmed.is_empty() {
                "Option".to_string()
            } else {
                trimmed.to_string()
            };
            option_labels.push(lbl);
            if first_option_idx.is_none() {
                first_option_idx = Some(idx);
            }
            if elem.get_attribute("selected").is_some() {
                selected_idx = Some(idx);
            }
        }
    }
    let display_idx = selected_idx.or(first_option_idx);
    let label = match display_idx {
        Some(i) => option_labels
            .get(i)
            .cloned()
            .unwrap_or_else(|| "Select".to_string()),
        None => "Select".to_string(),
    };
    (label, option_labels, selected_idx)
}

/// Recursively collect text content from a DOM node and its descendants.
fn collect_text_content(doc: &Document, node_id: NodeId) -> String {
    let mut text = String::new();
    for &child_id in &doc.get(node_id).children {
        match &doc.get(child_id).kind {
            NodeKind::Text(t) => text.push_str(t),
            NodeKind::Element(_) => text.push_str(&collect_text_content(doc, child_id)),
            _ => {},
        }
    }
    text
}

/// Resolve an `<img src>` attribute against a base URL.
fn resolve_img_src(base_url: Option<&str>, src: &str) -> String {
    match base_url {
        Some(base) => {
            if let Some(base_parsed) = crate::loader::Url::parse(base) {
                base_parsed
                    .resolve(src)
                    .map(|u| u.to_string())
                    .unwrap_or_else(|| src.to_string())
            } else {
                src.to_string()
            }
        },
        None => src.to_string(),
    }
}

/// Check if a text node's siblings include block-level elements.
///
/// When whitespace text sits between block elements (e.g.
/// `<div>\n<p>text</p>\n</div>`), the whitespace is insignificant
/// and should be dropped. When it sits between inline elements
/// (e.g. `<em>a</em> <strong>b</strong>`), the space is significant.
fn has_block_sibling(doc: &Document, node_id: NodeId) -> bool {
    let parent_id = match doc.get(node_id).parent {
        Some(p) => p,
        None => return true,
    };
    let siblings = &doc.get(parent_id).children;
    for &sib in siblings {
        if sib == node_id {
            continue;
        }
        if let NodeKind::Element(elem) = &doc.get(sib).kind
            && is_block_tag(&elem.tag)
        {
            return true;
        }
    }
    false
}

/// Returns true if the tag is normally block-level.
fn is_block_tag(tag: &TagName) -> bool {
    matches!(
        tag,
        TagName::Div
            | TagName::P
            | TagName::H1
            | TagName::H2
            | TagName::H3
            | TagName::H4
            | TagName::H5
            | TagName::H6
            | TagName::Ul
            | TagName::Ol
            | TagName::Li
            | TagName::Table
            | TagName::Blockquote
            | TagName::Pre
            | TagName::Hr
            | TagName::Section
            | TagName::Article
            | TagName::Nav
            | TagName::Aside
            | TagName::Header
            | TagName::Footer
            | TagName::Main
            | TagName::Form
            | TagName::Dl
            | TagName::Dt
            | TagName::Dd
            | TagName::Figure
            | TagName::Figcaption
            | TagName::Details
            | TagName::Summary
    )
}

/// Walk up the DOM to find an inherited style for a text node.
fn find_inherited_style(
    doc: &Document,
    node_id: NodeId,
    styles: &[Option<ComputedStyle>],
) -> ComputedStyle {
    if let Some(parent_id) = doc.get(node_id).parent
        && let Some(Some(style)) = styles.get(parent_id)
    {
        return style.clone();
    }
    ComputedStyle::default()
}

// -------------------------------------------------------------------
// Anonymous box wrapping
// -------------------------------------------------------------------

/// When a block box has a mix of block-level and inline-level children,
/// wrap consecutive runs of inline children in anonymous block boxes.
///
/// This ensures the block formatting context only contains block-level
/// boxes, as required by CSS 2.1.
pub(super) fn wrap_anonymous(
    children: Vec<LayoutBox>,
    parent_style: &ComputedStyle,
) -> Vec<LayoutBox> {
    if children.is_empty() {
        return children;
    }

    let has_block = children.iter().any(|c| c.is_block_level());
    let has_inline = children.iter().any(|c| !c.is_block_level());

    // If all children are the same level, no wrapping needed.
    if !has_block || !has_inline {
        return children;
    }

    // Mixed: wrap runs of inline children in anonymous block boxes.
    let mut result = Vec::with_capacity(children.len());
    let mut inline_run: Vec<LayoutBox> = Vec::new();

    for child in children {
        if child.is_block_level() {
            if !inline_run.is_empty() {
                let anon = make_anonymous_block(std::mem::take(&mut inline_run), parent_style);
                result.push(anon);
            }
            result.push(child);
        } else {
            inline_run.push(child);
        }
    }

    // Flush any trailing inline run.
    if !inline_run.is_empty() {
        result.push(make_anonymous_block(inline_run, parent_style));
    }

    result
}

/// Create an anonymous block box wrapping the given inline children.
///
/// Inherits text-related properties from the parent so that inline
/// content inside the anonymous block retains the correct font-size,
/// color, line-height, text-align, etc.
fn make_anonymous_block(children: Vec<LayoutBox>, parent_style: &ComputedStyle) -> LayoutBox {
    let mut style = parent_style.clone();
    // Override display to Block and reset non-inherited box properties.
    style.display = Display::Block;
    style.margin_top = 0.0;
    style.margin_right = 0.0;
    style.margin_bottom = 0.0;
    style.margin_left = 0.0;
    style.margin_left_auto = false;
    style.margin_right_auto = false;
    style.padding_top = 0.0;
    style.padding_right = 0.0;
    style.padding_bottom = 0.0;
    style.padding_left = 0.0;
    style.border_top_width = 0.0;
    style.border_right_width = 0.0;
    style.border_bottom_width = 0.0;
    style.border_left_width = 0.0;
    style.background_color = Color::rgba(0, 0, 0, 0);
    style.width = Dimension::Auto;
    style.height = Dimension::Auto;
    LayoutBox {
        box_type: BoxType::Anonymous,
        dimensions: Dimensions::default(),
        children,
        node: None,
        style,
        text: None,
        dirty: true,
        background_texture: None,
        background_texture_size: None,
    }
}
