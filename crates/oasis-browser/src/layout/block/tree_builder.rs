//! DOM-to-layout-tree construction.
//!
//! Walks the DOM tree and builds a corresponding [`LayoutBox`] tree,
//! handling element box type determination, replaced elements, pseudo-
//! elements (`::before`/`::after`), anonymous box wrapping, and
//! insignificant whitespace elimination.

use super::{TextMeasurer, layout_block_with_height, offset_descendant};
use crate::css::values::{ComputedStyle, Display, Float, ListStyleType};
use crate::html::dom::{Document, ElementData, NodeId, NodeKind, TagName};
use crate::layout::box_model::*;
use crate::layout::positioning::apply_positioning;

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
    viewport_height: f32,
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

    relayout_tree(&mut root, measurer, viewport_width, viewport_height);
    root
}

/// Lay out a tree built by [`build_layout_tree`] again, in place, for a
/// `viewport_width` x `viewport_height` viewport: after its content
/// changed (decoded image sizes) or for a new viewport. The geometry is
/// the same as building the tree afresh; boxes, their canvas state and
/// textures are kept.
pub fn relayout_tree(
    root: &mut LayoutBox,
    measurer: &dyn TextMeasurer,
    viewport_width: f32,
    viewport_height: f32,
) {
    // Layout from the root.
    root.dimensions.content.x = 0.0;
    root.dimensions.content.y = 0.0;
    layout_block_with_height(root, viewport_width, Some(viewport_height), measurer);

    // Shift root content origin to include root's own box-model edges.
    // layout_block_children no longer adds parent padding to children,
    // so the root's edges must be baked into content.x/y.
    let dx =
        root.dimensions.margin.left + root.dimensions.border.left + root.dimensions.padding.left;
    let dy = root.dimensions.margin.top + root.dimensions.border.top + root.dimensions.padding.top;
    if dx != 0.0 || dy != 0.0 {
        offset_descendant(root, dx, dy);
    }

    // Apply CSS positioning (relative/absolute/fixed) as a post-pass.
    let viewport_rect = Rect::new(0.0, 0.0, viewport_width, viewport_height);
    apply_positioning(root, viewport_rect);
    root.clamp_geometry();
}

/// Recursively build child layout boxes for a list of DOM node IDs.
pub(super) fn build_children(
    doc: &Document,
    children: &[NodeId],
    styles: &[Option<ComputedStyle>],
    base_url: Option<&str>,
    image_info: &HashMap<String, (u32, u32)>,
) -> Vec<LayoutBox> {
    // Whitespace-only text between block siblings is dropped; decide
    // that once per sibling list (asking per text node walked every
    // sibling each time: O(n^2) on long inline-only runs).
    let block_siblings = children
        .iter()
        .any(|&c| matches!(&doc.get(c).kind, NodeKind::Element(e) if is_block_tag(&e.tag)));
    let mut boxes = Vec::with_capacity(children.len());
    for &child_id in children {
        build_box_for_node(
            doc,
            child_id,
            styles,
            base_url,
            image_info,
            block_siblings,
            &mut boxes,
        );
    }
    boxes
}

/// Build the layout box for a DOM node and append it to `out`. Nothing
/// is appended for `display: none`, comments, and nodes without styles.
///
/// This is the recursive step of layout-tree construction, so its stack
/// frame (plus `build_children`'s) is multiplied by the DOM depth, up to
/// the tree builder's nesting cap. A `LayoutBox` is ~1.7 KB, so the
/// frame must not hold any by value: all per-element work lives in the
/// out-of-line [`begin_box_for_node`], which hands back heap boxes, and
/// the final move into `out` happens in [`push_box`]. This took a
/// 256-deep page from ~12 KB of stack per level (overflowing a 1 MiB
/// Windows main thread) to a few hundred bytes.
fn build_box_for_node(
    doc: &Document,
    node_id: NodeId,
    styles: &[Option<ComputedStyle>],
    base_url: Option<&str>,
    image_info: &HashMap<String, (u32, u32)>,
    block_siblings: bool,
    out: &mut Vec<LayoutBox>,
) {
    match begin_box_for_node(doc, node_id, styles, base_url, image_info, block_siblings) {
        BoxStart::Skip => {},
        BoxStart::Leaf(lb) => push_box(out, lb),
        BoxStart::Container {
            mut lb,
            before,
            after,
        } => {
            // Collect child IDs to avoid holding a borrow on the node
            // while recursing into `build_children`.
            let child_ids = doc.get(node_id).children.clone();
            let mut child_boxes = build_children(doc, &child_ids, styles, base_url, image_info);

            // Insert ::before and ::after pseudo-element boxes.
            if let Some(before) = before {
                insert_box(&mut child_boxes, 0, before);
            }
            if let Some(after) = after {
                push_box(&mut child_boxes, after);
            }

            lb.children = if matches!(lb.box_type, BoxType::Flex | BoxType::Grid) {
                wrap_flex_items(child_boxes, &lb.style)
            } else {
                wrap_anonymous(child_boxes, &lb.style)
            };
            push_box(out, lb);
        },
    }
}

/// Move a heap layout box into `out` (out of line so the by-value
/// temporary never lands in the recursive frame).
#[inline(never)]
fn push_box(out: &mut Vec<LayoutBox>, lb: Box<LayoutBox>) {
    out.push(*lb);
}

/// [`push_box`] at an index.
#[inline(never)]
fn insert_box(out: &mut Vec<LayoutBox>, index: usize, lb: Box<LayoutBox>) {
    out.insert(index, *lb);
}

/// First, non-recursive half of [`build_box_for_node`].
enum BoxStart {
    /// No box (`display: none`, comments, closed `<details>` content…).
    Skip,
    /// A finished box with no DOM children to recurse into.
    Leaf(Box<LayoutBox>),
    /// A box whose DOM children still have to be built, plus its
    /// `::before` / `::after` boxes.
    Container {
        lb: Box<LayoutBox>,
        before: Option<Box<LayoutBox>>,
        after: Option<Box<LayoutBox>>,
    },
}

#[inline(never)]
fn begin_box_for_node(
    doc: &Document,
    node_id: NodeId,
    styles: &[Option<ComputedStyle>],
    base_url: Option<&str>,
    image_info: &HashMap<String, (u32, u32)>,
    block_siblings: bool,
) -> BoxStart {
    let node = doc.get(node_id);

    match &node.kind {
        NodeKind::Element(elem) => {
            let Some(style) = styles.get(node_id).and_then(Option::clone) else {
                return BoxStart::Skip;
            };
            if style.display == Display::None {
                return BoxStart::Skip;
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
                return BoxStart::Skip;
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
                return BoxStart::Leaf(Box::new(lb));
            }

            // Handle <svg> as a replaced element.
            if elem.tag == TagName::Svg
                && let Some(svg_elem) = crate::svg::parse_svg_with_styles(doc, node_id, styles)
            {
                let replaced = ReplacedContent::Svg {
                    element: std::rc::Rc::new(svg_elem),
                };
                let mut lb = LayoutBox::new(BoxType::Replaced(replaced), style, Some(node_id));
                lb.children = Vec::new();
                return BoxStart::Leaf(Box::new(lb));
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
                return BoxStart::Leaf(Box::new(lb));
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
                return BoxStart::Leaf(Box::new(lb));
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
                return BoxStart::Leaf(Box::new(lb));
            }

            // Handle replaced elements.
            if let Some(replaced) = replaced_content(elem, base_url, image_info) {
                let mut lb = LayoutBox::new(BoxType::Replaced(replaced), style, Some(node_id));
                lb.children = Vec::new();
                return BoxStart::Leaf(Box::new(lb));
            }

            let mut lb = LayoutBox::new(box_type, style.clone(), Some(node_id));

            // Generate ::before pseudo-element content.
            let before_box = make_pseudo_box(&style.before_style);

            // Generate ::after pseudo-element content.
            let after_box = make_pseudo_box(&style.after_style);

            if matches!(lb.box_type, BoxType::TableCell) {
                lb.cell_span = CellSpan::from_attrs(
                    elem.get_attribute("colspan"),
                    elem.get_attribute("rowspan"),
                );
            }

            BoxStart::Container {
                lb: Box::new(lb),
                before: before_box.map(Box::new),
                after: after_box.map(Box::new),
            }
        },
        NodeKind::Text(text) => {
            // Skip whitespace-only text nodes that are between
            // block-level siblings (insignificant whitespace). Keep
            // them when they're between inline siblings so that
            // `<em>hello</em> <strong>world</strong>` preserves
            // the space.
            if text.trim().is_empty() && (block_siblings || in_table_structure(doc, node.parent)) {
                return BoxStart::Skip;
            }
            let style = find_inherited_style(doc, node_id, styles);
            let mut inline_style = style;
            inline_style.display = Display::Inline;
            let mut lb = LayoutBox::new(BoxType::Inline, inline_style, Some(node_id));
            lb.text = Some(text.clone());
            BoxStart::Leaf(Box::new(lb))
        },
        NodeKind::Comment(_) | NodeKind::Document => BoxStart::Skip,
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
    use crate::layout::box_model::ListMarkerStyle;
    match &style.list_style_type {
        ListStyleType::Disc => ListMarker::Disc,
        ListStyleType::Circle => ListMarker::Circle,
        ListStyleType::Square => ListMarker::Square,
        ListStyleType::Decimal => ListMarker::Ordered(ListMarkerStyle::Decimal, 1),
        ListStyleType::DecimalLeadingZero => {
            ListMarker::Ordered(ListMarkerStyle::DecimalLeadingZero, 1)
        },
        ListStyleType::LowerAlpha => ListMarker::Ordered(ListMarkerStyle::LowerAlpha, 1),
        ListStyleType::UpperAlpha => ListMarker::Ordered(ListMarkerStyle::UpperAlpha, 1),
        ListStyleType::LowerRoman => ListMarker::Ordered(ListMarkerStyle::LowerRoman, 1),
        ListStyleType::UpperRoman => ListMarker::Ordered(ListMarkerStyle::UpperRoman, 1),
        ListStyleType::Custom(name) => {
            ListMarker::Ordered(ListMarkerStyle::Custom(name.clone()), 1)
        },
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
            let (width, height) = image_size(elem, base_url, image_info);
            let alt = elem.get_attribute("alt").unwrap_or("").to_string();
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

/// The size an `<img>` lays out at: its `width` / `height` attributes,
/// with the missing ones taken from the decoded image in `image_info`
/// (scaled to keep its aspect ratio when one attribute is given).
pub(crate) fn image_size(
    elem: &ElementData,
    base_url: Option<&str>,
    image_info: &HashMap<String, (u32, u32)>,
) -> (u32, u32) {
    let attr = |name| {
        elem.get_attribute(name)
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(0)
    };
    let (mut width, mut height) = (attr("width"), attr("height"));
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
    (width, height)
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

/// Returns true if the tag is normally block-level.
///
/// Whitespace text among block siblings (`<div>\n<p>text</p>\n</div>`)
/// is insignificant and dropped; among inline siblings
/// (`<em>a</em> <strong>b</strong>`) the space is significant.
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

/// Whether `parent` is a table, row group, row or column group, whose
/// whitespace-only text children are not rendered (CSS 2.1 §17.2.1):
/// the newlines between `<tr>` / `<td>` tags drew as spaces when the
/// table ended up in inline content.
fn in_table_structure(doc: &Document, parent: Option<NodeId>) -> bool {
    parent.is_some_and(|p| {
        matches!(
            &doc.get(p).kind,
            NodeKind::Element(e) if matches!(
                e.tag,
                TagName::Table
                    | TagName::Thead
                    | TagName::Tbody
                    | TagName::Tfoot
                    | TagName::Tr
                    | TagName::Colgroup
            )
        )
    })
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
#[inline(never)]
pub(super) fn wrap_anonymous(
    children: Vec<LayoutBox>,
    parent_style: &ComputedStyle,
) -> Vec<LayoutBox> {
    if children.is_empty() {
        return children;
    }

    // Floats participate in the parent's block formatting context
    // even when their computed `display` is inline (CSS 2.1 §9.7 —
    // float computes-to-block for layout purposes). Treat them as
    // block-level here so they don't get swept into an anonymous
    // IFC wrapper and lose visibility to the parent's float context.
    let is_block_level = |c: &LayoutBox| c.is_block_level() || c.style.float != Float::None;

    let has_block = children.iter().any(is_block_level);
    let has_inline = children.iter().any(|c| !is_block_level(c));

    // If all children are the same level, no wrapping needed.
    if !has_block || !has_inline {
        return children;
    }

    // Mixed: wrap runs of inline children in anonymous block boxes.
    let mut result = Vec::with_capacity(children.len());
    let mut inline_run: Vec<LayoutBox> = Vec::new();

    for child in children {
        if is_block_level(&child) {
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

/// Turn a flex or grid container's children into items (CSS Flexbox
/// §4): each inline element child (`<a>`, `<span>`) is blockified into
/// its own anonymous item carrying its flex properties, consecutive
/// text runs share one, and whitespace-only runs are dropped. Without
/// this an all-inline container (a nav bar of links) handed bare inline
/// boxes to block layout, which sized each one 0 x 0.
#[inline(never)]
pub(super) fn wrap_flex_items(
    children: Vec<LayoutBox>,
    parent_style: &ComputedStyle,
) -> Vec<LayoutBox> {
    if !children
        .iter()
        .any(|c| matches!(c.box_type, BoxType::Inline))
    {
        return children;
    }
    let is_blank = |c: &LayoutBox| c.text.as_deref().is_some_and(|t| t.trim().is_empty());
    let mut out = Vec::with_capacity(children.len());
    let mut text_run: Vec<LayoutBox> = Vec::new();
    let flush = |run: &mut Vec<LayoutBox>, out: &mut Vec<LayoutBox>| {
        if !run.is_empty() && !run.iter().all(is_blank) {
            // A text run's item: initial flex properties, none of the
            // container's box properties.
            out.push(make_anonymous_block(std::mem::take(run), parent_style));
        }
        run.clear();
    };
    for child in children {
        if !matches!(child.box_type, BoxType::Inline) {
            flush(&mut text_run, &mut out);
            out.push(child);
        } else if child.text.is_some() {
            text_run.push(child);
        } else {
            flush(&mut text_run, &mut out);
            out.push(blockified_flex_item(child));
        }
    }
    flush(&mut text_run, &mut out);
    out
}

/// Blockify an inline element child of a flex / grid container (CSS
/// Display §2.7): the item box takes the element's whole style as a
/// block (its `margin-left: auto`, padding, width, background, flex
/// properties, `position`, ...), and the element's inline box inside
/// it keeps only what its text inherits, so nothing is applied twice.
fn blockified_flex_item(mut child: LayoutBox) -> LayoutBox {
    let mut item_style = child.style.clone();
    item_style.display = Display::Block;
    let mut inner = ComputedStyle::inherit(&child.style);
    copy_inline_formatting(&mut inner, &child.style);
    child.style = inner;
    let mut item = make_anonymous_block(vec![child], &item_style);
    item.style = item_style;
    item
}

/// The non-inherited properties an anonymous block needs from the box it
/// stands in for to lay out its inline content the same way.
fn copy_inline_formatting(style: &mut ComputedStyle, from: &ComputedStyle) {
    style.overflow_wrap = from.overflow_wrap;
    style.word_break = from.word_break;
    style.tab_size = from.tab_size;
}

/// Create an anonymous block box wrapping the given inline children.
///
/// Starts from the parent's *inherited* properties so that inline
/// content inside the anonymous block retains the correct font-size,
/// color, line-height, text-align, etc. Non-inherited ones keep their
/// initial values: a clone of the parent would repeat its `position`,
/// `opacity`, `transform`, `filter`, shadows, background image and
/// overflow clip on every anonymous box (a `position: fixed` flex nav
/// drew all its links on top of each other at 0,0).
fn make_anonymous_block(children: Vec<LayoutBox>, parent_style: &ComputedStyle) -> LayoutBox {
    let mut style = ComputedStyle::inherit(parent_style);
    copy_inline_formatting(&mut style, parent_style);
    style.display = Display::Block;
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
        mask_image_data: None,
        cell_span: CellSpan::default(),
        sizing: SizingState::default(),
        inline_source: None,
    }
}
