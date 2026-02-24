//! Block-level layout algorithm.
//!
//! Implements CSS 2.1 block formatting context (BFC) layout. Block
//! boxes are stacked vertically; their widths expand to fill the
//! containing block and heights are determined by content.
//!
//! ## Incremental layout
//!
//! Each [`LayoutBox`] carries a `dirty` flag. When only a subtree
//! changes, callers can mark individual boxes dirty via
//! [`LayoutBox::mark_dirty`] and then call [`layout_block_incremental`]
//! to relayout only the dirty subtree while preserving previously
//! computed dimensions for clean branches. A [`StyleCache`] avoids
//! redundant style computations during incremental passes.

use super::box_model::*;
use super::flex::layout_flex;
use super::float::{ClearSide, FloatContext, FloatSide};
use super::inline::layout_inline;
use super::positioning::apply_positioning;
use super::table::layout_table;
use crate::css::values::{
    BoxSizing, Clear, ComputedStyle, Dimension, Display, Float, ListStyleType, Overflow, Position,
};
use crate::html::dom::{Document, ElementData, NodeId, NodeKind, TagName};
use oasis_types::backend::Color;

use std::collections::HashMap;

// -------------------------------------------------------------------
// TextMeasurer trait
// -------------------------------------------------------------------

/// Trait for measuring text width at a given font size.
///
/// Backends supply concrete implementations. The layout engine calls
/// [`measure_text`](TextMeasurer::measure_text) to determine how much
/// horizontal space a text run occupies so it can compute line breaks
/// and box dimensions.
pub trait TextMeasurer {
    /// Return the width in pixels of `text` rendered at `font_size`.
    fn measure_text(&self, text: &str, font_size: u16) -> u32;
}

// -------------------------------------------------------------------
// Build layout tree from DOM
// -------------------------------------------------------------------

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

// -------------------------------------------------------------------
// Style cache
// -------------------------------------------------------------------

/// Cache of computed edge sizes (padding/border/margin) keyed by DOM
/// node ID. Avoids re-resolving edge sizes for nodes whose styles have
/// not changed between incremental layout passes.
#[derive(Debug, Default)]
pub struct StyleCache {
    edges: HashMap<NodeId, CachedEdges>,
}

/// Cached resolved edge sizes for a single layout box.
#[derive(Debug, Clone)]
struct CachedEdges {
    padding: EdgeSizes,
    border: EdgeSizes,
    margin: EdgeSizes,
}

impl StyleCache {
    /// Create an empty style cache.
    pub fn new() -> Self {
        Self {
            edges: HashMap::new(),
        }
    }

    /// Store resolved edge sizes for a node.
    pub fn insert_edges(
        &mut self,
        node: NodeId,
        padding: EdgeSizes,
        border: EdgeSizes,
        margin: EdgeSizes,
    ) {
        self.edges.insert(
            node,
            CachedEdges {
                padding,
                border,
                margin,
            },
        );
    }

    /// Retrieve cached edges for a node. Returns `None` on cache miss.
    pub fn get_edges(&self, node: NodeId) -> Option<(&EdgeSizes, &EdgeSizes, &EdgeSizes)> {
        self.edges
            .get(&node)
            .map(|c| (&c.padding, &c.border, &c.margin))
    }

    /// Number of cached entries (useful for benchmarking).
    pub fn len(&self) -> usize {
        self.edges.len()
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.edges.is_empty()
    }

    /// Clear all cached entries.
    pub fn clear(&mut self) {
        self.edges.clear();
    }
}

// -------------------------------------------------------------------
// Incremental layout
// -------------------------------------------------------------------

/// Perform incremental layout on a previously-built layout tree.
///
/// Only re-lays-out subtrees where at least one box has its `dirty`
/// flag set. Clean subtrees are skipped entirely. After layout
/// completes, all boxes are marked clean. The optional `cache` stores
/// resolved edge sizes to avoid redundant recomputation.
///
/// This is an additive optimisation -- when the entire tree is dirty
/// (e.g. initial layout), the result is identical to a full
/// [`layout_block`] pass.
pub fn layout_block_incremental(
    layout_box: &mut LayoutBox,
    containing_width: f32,
    measurer: &dyn TextMeasurer,
    cache: &mut StyleCache,
) {
    if !layout_box.dirty && !any_child_dirty(layout_box) {
        return;
    }

    // Resolve edge sizes, using cache when available.
    resolve_edge_sizes_cached(layout_box, containing_width, cache);

    calculate_block_width(layout_box, containing_width);

    if matches!(layout_box.box_type, BoxType::Flex) {
        layout_flex(layout_box, containing_width, measurer);
    } else if matches!(layout_box.box_type, BoxType::TableWrapper) {
        layout_table_children(layout_box, measurer);
    } else {
        layout_children_incremental(layout_box, measurer, cache);
        calculate_block_height(layout_box, None);
    }

    layout_box.dirty = false;
}

/// Check whether any child (or deeper descendant) is dirty.
fn any_child_dirty(layout_box: &LayoutBox) -> bool {
    for child in &layout_box.children {
        if child.dirty || any_child_dirty(child) {
            return true;
        }
    }
    false
}

/// Resolve edge sizes with caching. If the box has a DOM node and a
/// cache hit, the cached values are used directly. Otherwise the
/// values are resolved from the style and stored in the cache.
fn resolve_edge_sizes_cached(
    layout_box: &mut LayoutBox,
    containing_width: f32,
    cache: &mut StyleCache,
) {
    if let Some(node) = layout_box.node
        && !layout_box.dirty
        && let Some((p, b, m)) = cache.get_edges(node)
    {
        layout_box.dimensions.padding = *p;
        layout_box.dimensions.border = *b;
        layout_box.dimensions.margin = *m;
        return;
    }

    resolve_edge_sizes(layout_box, containing_width);

    if let Some(node) = layout_box.node {
        cache.insert_edges(
            node,
            layout_box.dimensions.padding,
            layout_box.dimensions.border,
            layout_box.dimensions.margin,
        );
    }
}

/// Incremental version of [`layout_block_children`]. Skips clean
/// children whose subtrees are also clean.
fn layout_children_incremental(
    parent: &mut LayoutBox,
    measurer: &dyn TextMeasurer,
    cache: &mut StyleCache,
) {
    let all_inline =
        !parent.children.is_empty() && parent.children.iter().all(|c| !c.is_block_level());
    if all_inline {
        // Inline formatting context -- relayout fully when dirty.
        if parent.dirty || any_child_dirty(parent) {
            layout_inline(parent, measurer);
        }
        return;
    }

    let content_x = parent.dimensions.content.x;
    let content_width = parent.dimensions.content.width;
    let mut cursor_y = parent.dimensions.content.y;
    let mut prev_margin_bottom: f32 = 0.0;

    let parent_bfc = establishes_bfc(&parent.style);
    let can_collapse_top =
        !parent_bfc && parent.dimensions.padding.top == 0.0 && parent.dimensions.border.top == 0.0;
    let can_collapse_bottom = !parent_bfc
        && parent.dimensions.padding.bottom == 0.0
        && parent.dimensions.border.bottom == 0.0
        && parent.style.height == Dimension::Auto;

    let mut is_first_in_flow = true;

    for child in &mut parent.children {
        match child.box_type {
            BoxType::Block | BoxType::ListItem { .. } | BoxType::TableWrapper => {
                resolve_edge_sizes_cached(child, content_width, cache);

                let child_margin_top = child.dimensions.margin.top;
                let collapsed = if is_first_in_flow && can_collapse_top {
                    let parent_mt = parent.dimensions.margin.top;
                    parent.dimensions.margin.top = collapse_margins(parent_mt, child_margin_top);
                    child.dimensions.margin.top = 0.0;
                    0.0
                } else {
                    collapse_margins(prev_margin_bottom, child_margin_top)
                };
                is_first_in_flow = false;

                child.dimensions.content.x = content_x
                    + child.dimensions.margin.left
                    + child.dimensions.border.left
                    + child.dimensions.padding.left;
                child.dimensions.content.y = cursor_y
                    + collapsed
                    + child.dimensions.border.top
                    + child.dimensions.padding.top;

                if child.dirty || any_child_dirty(child) {
                    layout_block_incremental(child, content_width, measurer, cache);
                }

                let bb = child.dimensions.border_box();
                cursor_y = bb.y + bb.height;
                prev_margin_bottom = child.dimensions.margin.bottom;
            },
            BoxType::Anonymous => {
                is_first_in_flow = false;
                child.dimensions.content.x = content_x;
                child.dimensions.content.y = cursor_y;
                child.dimensions.content.width = content_width;

                if child.dirty || any_child_dirty(child) {
                    layout_inline(child, measurer);
                }

                cursor_y += child.dimensions.content.height;
                prev_margin_bottom = 0.0;
            },
            BoxType::Replaced(_) => {
                resolve_edge_sizes_cached(child, content_width, cache);
                let pad_h = child.dimensions.padding.horizontal();
                let bdr_h = child.dimensions.border.horizontal();
                let mar_h = child.dimensions.margin.horizontal();

                let child_margin_top = child.dimensions.margin.top;
                let collapsed = if is_first_in_flow && can_collapse_top {
                    let parent_mt = parent.dimensions.margin.top;
                    parent.dimensions.margin.top = collapse_margins(parent_mt, child_margin_top);
                    child.dimensions.margin.top = 0.0;
                    0.0
                } else {
                    collapse_margins(prev_margin_bottom, child_margin_top)
                };
                is_first_in_flow = false;

                child.dimensions.content.x = content_x
                    + child.dimensions.margin.left
                    + child.dimensions.border.left
                    + child.dimensions.padding.left;
                child.dimensions.content.y = cursor_y
                    + collapsed
                    + child.dimensions.border.top
                    + child.dimensions.padding.top;

                let mut w = (content_width - pad_h - bdr_h - mar_h).max(0.0);
                if let Dimension::Px(max) = child.style.max_width
                    && max < 999.0
                {
                    w = w.min(max);
                }
                child.dimensions.content.width = w;

                let h = match child.style.height {
                    Dimension::Px(h) => h,
                    _ => {
                        if let BoxType::Replaced(ref rc) = child.box_type {
                            match rc {
                                ReplacedContent::HorizontalRule => 2.0,
                                ReplacedContent::Image { height, .. } => *height as f32,
                                _ => 0.0,
                            }
                        } else {
                            0.0
                        }
                    },
                };
                child.dimensions.content.height = h;

                let bb = child.dimensions.border_box();
                cursor_y = bb.y + bb.height;
                prev_margin_bottom = child.dimensions.margin.bottom;
            },
            _ => {},
        }
    }

    // Parent-last-child bottom margin collapsing.
    if can_collapse_bottom {
        let last_block = parent
            .children
            .iter()
            .rposition(|c| c.is_block_level() && !matches!(c.box_type, BoxType::Anonymous));
        if let Some(idx) = last_block {
            let child_mb = parent.children[idx].dimensions.margin.bottom;
            if child_mb > 0.0 {
                let parent_mb = parent.dimensions.margin.bottom;
                parent.dimensions.margin.bottom = collapse_margins(parent_mb, child_mb);
                parent.children[idx].dimensions.margin.bottom = 0.0;
            }
        }
    }
}

/// Recursively build child layout boxes for a list of DOM node IDs.
fn build_children(
    doc: &Document,
    children: &[NodeId],
    styles: &[Option<ComputedStyle>],
    base_url: Option<&str>,
    image_info: &HashMap<String, (u32, u32)>,
) -> Vec<LayoutBox> {
    let mut boxes = Vec::new();
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

            // Determine box type.
            let box_type = box_type_for_element(elem, &style);

            // Handle replaced elements.
            if let Some(replaced) = replaced_content(elem, base_url, image_info) {
                let mut lb = LayoutBox::new(BoxType::Replaced(replaced), style, Some(node_id));
                lb.children = Vec::new();
                return Some(lb);
            }

            let mut lb = LayoutBox::new(box_type, style.clone(), Some(node_id));

            // Generate ::before pseudo-element content.
            let before_box = if let Some(ref text) = style.before_content {
                if !text.is_empty() {
                    let mut pseudo_style = style.clone();
                    pseudo_style.display = Display::Inline;
                    let mut pb = LayoutBox::new(BoxType::Inline, pseudo_style, None);
                    pb.text = Some(text.clone());
                    Some(pb)
                } else {
                    None
                }
            } else {
                None
            };

            // Generate ::after pseudo-element content.
            let after_box = if let Some(ref text) = style.after_content {
                if !text.is_empty() {
                    let mut pseudo_style = style.clone();
                    pseudo_style.display = Display::Inline;
                    let mut pb = LayoutBox::new(BoxType::Inline, pseudo_style, None);
                    pb.text = Some(text.clone());
                    Some(pb)
                } else {
                    None
                }
            } else {
                None
            };

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
        Display::Flex => BoxType::Flex,
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
                _ => {
                    // text, password, search, etc.
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
                    })
                },
            }
        },
        TagName::Button => {
            let label = elem.get_attribute("value").unwrap_or("Button").to_string();
            Some(ReplacedContent::SubmitButton { label })
        },
        _ => None,
    }
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
fn wrap_anonymous(children: Vec<LayoutBox>, parent_style: &ComputedStyle) -> Vec<LayoutBox> {
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
    let mut result = Vec::new();
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
    }
}

// -------------------------------------------------------------------
// Block layout algorithm
// -------------------------------------------------------------------

/// Lay out a block-level box and all its children.
///
/// The block's `content.x` and `content.y` must be set by the caller
/// (the parent positions each child). This function calculates width,
/// lays out children, and determines height.
pub fn layout_block(
    layout_box: &mut LayoutBox,
    containing_width: f32,
    measurer: &dyn TextMeasurer,
) {
    layout_block_with_height(layout_box, containing_width, None, measurer);
}

/// Internal layout with optional containing height for percentage
/// height resolution.
fn layout_block_with_height(
    layout_box: &mut LayoutBox,
    containing_width: f32,
    containing_height: Option<f32>,
    measurer: &dyn TextMeasurer,
) {
    // 1. Resolve padding, border, and margin from the computed style.
    resolve_edge_sizes(layout_box, containing_width);

    // 2. Calculate width.
    calculate_block_width(layout_box, containing_width);

    // 3. Layout children.
    if matches!(layout_box.box_type, BoxType::Flex) {
        layout_flex(layout_box, containing_width, measurer);
    } else if matches!(layout_box.box_type, BoxType::TableWrapper) {
        layout_table_children(layout_box, measurer);
    } else {
        layout_block_children(layout_box, measurer);
        // 4. Calculate height.
        calculate_block_height(layout_box, containing_height);
    }
}

/// Resolve padding, border, and margin from the computed style into
/// the layout box's dimensions.
pub fn resolve_edge_sizes(layout_box: &mut LayoutBox, _containing_width: f32) {
    let s = &layout_box.style;

    layout_box.dimensions.padding = EdgeSizes {
        top: s.padding_top,
        right: s.padding_right,
        bottom: s.padding_bottom,
        left: s.padding_left,
    };

    // Per CSS spec, border-style:none/hidden → border-width computes to 0.
    use crate::css::values::BorderStyle;
    layout_box.dimensions.border = EdgeSizes {
        top: if s.border_top_style == BorderStyle::None {
            0.0
        } else {
            s.border_top_width
        },
        right: if s.border_right_style == BorderStyle::None {
            0.0
        } else {
            s.border_right_width
        },
        bottom: if s.border_bottom_style == BorderStyle::None {
            0.0
        } else {
            s.border_bottom_width
        },
        left: if s.border_left_style == BorderStyle::None {
            0.0
        } else {
            s.border_left_width
        },
    };

    layout_box.dimensions.margin = EdgeSizes {
        top: s.margin_top,
        right: s.margin_right,
        bottom: s.margin_bottom,
        left: s.margin_left,
    };
}

/// Calculate the width of a block-level box.
///
/// If width is `auto`, the box fills the available space in the
/// containing block. If explicit, auto margins are used for centering.
/// The constraint equation is:
///
///   margin-left + border-left + padding-left + width
///     + padding-right + border-right + margin-right
///     = containing_width
///
/// If over-constrained, `margin-right` absorbs the overflow.
fn calculate_block_width(layout_box: &mut LayoutBox, containing_width: f32) {
    let pad_h = layout_box.dimensions.padding.horizontal();
    let bdr_h = layout_box.dimensions.border.horizontal();
    let mar_h = layout_box.dimensions.margin.horizontal();
    let total_extra = pad_h + bdr_h + mar_h;

    let ml_auto = layout_box.style.margin_left_auto;
    let mr_auto = layout_box.style.margin_right_auto;

    let is_border_box = layout_box.style.box_sizing == BoxSizing::BorderBox;

    match layout_box.style.width {
        Dimension::Px(w) => {
            let content_w = if is_border_box {
                (w - pad_h - bdr_h).max(0.0)
            } else {
                w
            };
            layout_box.dimensions.content.width = content_w;
            let available_for_margins = containing_width - content_w - pad_h - bdr_h;
            if ml_auto && mr_auto {
                let half = available_for_margins.max(0.0) / 2.0;
                layout_box.dimensions.margin.left = half;
                layout_box.dimensions.margin.right = half;
            } else if ml_auto {
                layout_box.dimensions.margin.left =
                    (available_for_margins - layout_box.dimensions.margin.right).max(0.0);
            } else if mr_auto {
                layout_box.dimensions.margin.right =
                    (available_for_margins - layout_box.dimensions.margin.left).max(0.0);
            } else {
                // Over-constrained: margin-right absorbs overflow.
                layout_box.dimensions.margin.right =
                    available_for_margins - layout_box.dimensions.margin.left;
            }
        },
        Dimension::Percent(pct) => {
            let declared_w = containing_width * (pct / 100.0);
            let content_w = if is_border_box {
                (declared_w - pad_h - bdr_h).max(0.0)
            } else {
                declared_w
            };
            layout_box.dimensions.content.width = content_w;
            let available_for_margins = containing_width - content_w - pad_h - bdr_h;
            if ml_auto && mr_auto {
                let half = available_for_margins.max(0.0) / 2.0;
                layout_box.dimensions.margin.left = half;
                layout_box.dimensions.margin.right = half;
            } else if ml_auto {
                layout_box.dimensions.margin.left =
                    (available_for_margins - layout_box.dimensions.margin.right).max(0.0);
            } else if mr_auto {
                layout_box.dimensions.margin.right =
                    (available_for_margins - layout_box.dimensions.margin.left).max(0.0);
            }
        },
        Dimension::Auto => {
            // Width = containing_width minus all horizontal extras.
            let w = (containing_width - total_extra).max(0.0);
            layout_box.dimensions.content.width = w;
        },
    }

    // Apply min-width / max-width constraints.
    match layout_box.style.min_width {
        Dimension::Px(min) if layout_box.dimensions.content.width < min => {
            layout_box.dimensions.content.width = min;
        },
        Dimension::Percent(pct) => {
            let min = containing_width * (pct / 100.0);
            if layout_box.dimensions.content.width < min {
                layout_box.dimensions.content.width = min;
            }
        },
        _ => {},
    }
    match layout_box.style.max_width {
        Dimension::Px(max) if max < 999.0 && layout_box.dimensions.content.width > max => {
            // Don't clamp table rowspan/colspan encoding values (>= 1000).
            layout_box.dimensions.content.width = max;
        },
        Dimension::Percent(pct) => {
            let max = containing_width * (pct / 100.0);
            if layout_box.dimensions.content.width > max {
                layout_box.dimensions.content.width = max;
            }
        },
        _ => {},
    }
}

/// Lay out a table wrapper's children using the table layout algorithm.
///
/// Delegates to [`layout_table`] which computes column widths, row heights,
/// and positions cells. The results are copied back into the table's layout
/// box and offset into the table's content coordinate space.
fn layout_table_children(layout_box: &mut LayoutBox, measurer: &dyn TextMeasurer) {
    let content_width = layout_box.dimensions.content.width;
    let result = layout_table(
        &layout_box.children,
        &layout_box.style,
        content_width,
        measurer,
    );
    let offset_x = layout_box.dimensions.content.x;
    let offset_y = layout_box.dimensions.content.y;
    layout_box.dimensions.content.height = result.dimensions.content.height;
    layout_box.children = result.children;
    // Offset all children into the table's content coordinate space.
    for child in &mut layout_box.children {
        offset_descendant(child, offset_x, offset_y);
    }
}

/// Recursively offset a layout box and its descendants by `(dx, dy)`.
fn offset_descendant(layout_box: &mut LayoutBox, dx: f32, dy: f32) {
    layout_box.dimensions.content.x += dx;
    layout_box.dimensions.content.y += dy;
    for child in &mut layout_box.children {
        offset_descendant(child, dx, dy);
    }
}

/// Returns true if the parent establishes a new block formatting
/// context (BFC), which inhibits margin collapsing with children.
fn establishes_bfc(style: &ComputedStyle) -> bool {
    style.overflow != Overflow::Visible
        || style.float != Float::None
        || matches!(style.position, Position::Absolute | Position::Fixed)
        || matches!(style.display, Display::InlineBlock | Display::Flex)
}

/// Layout block-level children, stacking them vertically.
///
/// If all children are inline-level (no block-level siblings), the
/// parent establishes an inline formatting context and we delegate
/// to [`layout_inline`] directly. This handles the common case of
/// `<p>text</p>` or `<p><a>link</a></p>` where the block box
/// contains only inline content.
fn layout_block_children(parent: &mut LayoutBox, measurer: &dyn TextMeasurer) {
    // If all children are inline, establish an IFC on the parent.
    let all_inline =
        !parent.children.is_empty() && parent.children.iter().all(|c| !c.is_block_level());
    if all_inline {
        layout_inline(parent, measurer);
        return;
    }

    let content_x = parent.dimensions.content.x;
    let content_width = parent.dimensions.content.width;
    let mut cursor_y = parent.dimensions.content.y;

    let mut prev_margin_bottom: f32 = 0.0;
    let mut float_ctx = FloatContext::new();
    let mut list_counter: usize = 1;

    // Parent-child margin collapsing (CSS 2.1 §8.3.1):
    // Margins collapse when there's no padding, border, or BFC
    // between parent and first/last child.
    let parent_bfc = establishes_bfc(&parent.style);
    let can_collapse_top =
        !parent_bfc && parent.dimensions.padding.top == 0.0 && parent.dimensions.border.top == 0.0;
    let can_collapse_bottom = !parent_bfc
        && parent.dimensions.padding.bottom == 0.0
        && parent.dimensions.border.bottom == 0.0
        && parent.style.height == Dimension::Auto;

    let mut is_first_in_flow = true;

    for child in &mut parent.children {
        // Assign sequential numbers to ordered list items.
        if let BoxType::ListItem { ref mut marker } = child.box_type
            && let ListMarker::Decimal(_) = marker
        {
            *marker = ListMarker::Decimal(list_counter);
            list_counter += 1;
        }
        // Handle clear property: advance cursor below cleared floats.
        let clear_y = match child.style.clear {
            Clear::Left => float_ctx.clear_y(ClearSide::Left),
            Clear::Right => float_ctx.clear_y(ClearSide::Right),
            Clear::Both => float_ctx.clear_y(ClearSide::Both),
            Clear::None => 0.0,
        };
        if child.style.clear != Clear::None && clear_y > cursor_y {
            cursor_y = clear_y;
            prev_margin_bottom = 0.0;
        }

        // Handle floated children: position via float context.
        if child.style.float != Float::None {
            resolve_edge_sizes(child, content_width);
            layout_block(child, content_width, measurer);
            let margin_box = child.dimensions.margin_box();
            let side = match child.style.float {
                Float::Left => FloatSide::Left,
                Float::Right => FloatSide::Right,
                Float::None => unreachable!(),
            };
            let float_box = float_ctx.place_float(
                side,
                margin_box.width,
                margin_box.height,
                cursor_y,
                content_width,
            );
            // Position the child at the float's placed position.
            child.dimensions.content.x = float_box.rect.x
                + child.dimensions.margin.left
                + child.dimensions.border.left
                + child.dimensions.padding.left;
            child.dimensions.content.y = float_box.rect.y
                + child.dimensions.margin.top
                + child.dimensions.border.top
                + child.dimensions.padding.top;
            continue;
        }

        match child.box_type {
            BoxType::Block | BoxType::ListItem { .. } | BoxType::TableWrapper => {
                // Resolve child's edge sizes first so we can read
                // margins for positioning.
                resolve_edge_sizes(child, content_width);

                let child_margin_top = child.dimensions.margin.top;

                // Parent-first-child top margin collapsing:
                // When this is the first in-flow block child and
                // there's no padding/border/BFC separating parent
                // from child, their top margins collapse. The
                // child's margin is absorbed into the parent.
                let did_collapse_top = is_first_in_flow && can_collapse_top;
                let collapsed = if did_collapse_top {
                    let parent_mt = parent.dimensions.margin.top;
                    parent.dimensions.margin.top = collapse_margins(parent_mt, child_margin_top);
                    0.0
                } else {
                    // Sibling margin collapsing: the collapsed
                    // margin replaces both the previous bottom and
                    // the current top margin.
                    collapse_margins(prev_margin_bottom, child_margin_top)
                };
                is_first_in_flow = false;

                // Position child's content area.
                child.dimensions.content.x = content_x
                    + child.dimensions.margin.left
                    + child.dimensions.border.left
                    + child.dimensions.padding.left;
                child.dimensions.content.y = cursor_y
                    + collapsed
                    + child.dimensions.border.top
                    + child.dimensions.padding.top;

                layout_block(child, content_width, measurer);

                // layout_block re-resolves edge sizes from the
                // style, so re-zero the top margin that was
                // absorbed into the parent.
                if did_collapse_top {
                    child.dimensions.margin.top = 0.0;
                }

                // Empty block self-collapsing: if a block has zero
                // content height, no padding, and no border, its own
                // top and bottom margins collapse into one margin.
                let is_empty = child.dimensions.content.height == 0.0
                    && child.dimensions.padding.vertical() == 0.0
                    && child.dimensions.border.vertical() == 0.0
                    && child.children.is_empty();

                if is_empty {
                    let self_collapsed = collapse_margins(
                        child.dimensions.margin.top,
                        child.dimensions.margin.bottom,
                    );
                    prev_margin_bottom = collapse_margins(prev_margin_bottom, self_collapsed);
                    // Empty block takes no vertical space beyond
                    // its collapsed margin.
                } else {
                    // Advance cursor_y to this child's border-box
                    // bottom (not margin-box). Margin collapsing
                    // will be handled when positioning the next
                    // sibling.
                    let bb = child.dimensions.border_box();
                    cursor_y = bb.y + bb.height;
                    prev_margin_bottom = child.dimensions.margin.bottom;
                }
            },
            BoxType::Anonymous => {
                // Anonymous box wrapping inline content -- prevents
                // parent-child margin collapsing with later siblings.
                is_first_in_flow = false;

                child.dimensions.content.x = content_x;
                child.dimensions.content.y = cursor_y;
                child.dimensions.content.width = content_width;

                // Layout as inline formatting context.
                layout_inline(child, measurer);

                cursor_y += child.dimensions.content.height;
                prev_margin_bottom = 0.0;
            },
            BoxType::Replaced(_) => {
                // Block-level replaced elements (e.g. <hr>) need to
                // participate in the block flow. They stretch to the
                // containing width and advance the cursor.
                resolve_edge_sizes(child, content_width);
                let pad_h = child.dimensions.padding.horizontal();
                let bdr_h = child.dimensions.border.horizontal();
                let mar_h = child.dimensions.margin.horizontal();

                let child_margin_top = child.dimensions.margin.top;

                // Parent-first-child collapsing for replaced elements.
                let collapsed = if is_first_in_flow && can_collapse_top {
                    let parent_mt = parent.dimensions.margin.top;
                    parent.dimensions.margin.top = collapse_margins(parent_mt, child_margin_top);
                    child.dimensions.margin.top = 0.0;
                    0.0
                } else {
                    collapse_margins(prev_margin_bottom, child_margin_top)
                };
                is_first_in_flow = false;

                child.dimensions.content.x = content_x
                    + child.dimensions.margin.left
                    + child.dimensions.border.left
                    + child.dimensions.padding.left;
                child.dimensions.content.y = cursor_y
                    + collapsed
                    + child.dimensions.border.top
                    + child.dimensions.padding.top;

                // Width: stretch to container (like a block), minus
                // edges. Respect max-width if set.
                let mut w = (content_width - pad_h - bdr_h - mar_h).max(0.0);
                if let Dimension::Px(max) = child.style.max_width
                    && max < 999.0
                {
                    w = w.min(max);
                }
                child.dimensions.content.width = w;

                // Height: use explicit CSS height or intrinsic height.
                let h = match child.style.height {
                    Dimension::Px(h) => h,
                    _ => {
                        // Intrinsic height for replaced content.
                        if let BoxType::Replaced(ref rc) = child.box_type {
                            match rc {
                                ReplacedContent::HorizontalRule => 2.0,
                                ReplacedContent::Image { height, .. } => *height as f32,
                                _ => 0.0,
                            }
                        } else {
                            0.0
                        }
                    },
                };
                child.dimensions.content.height = h;

                let bb = child.dimensions.border_box();
                cursor_y = bb.y + bb.height;
                prev_margin_bottom = child.dimensions.margin.bottom;
            },
            _ => {
                // Inline-level boxes inside a block context should
                // have been wrapped in anonymous boxes. If we get here,
                // just skip.
            },
        }
    }

    // Parent-last-child bottom margin collapsing (CSS 2.1 §8.3.1):
    // When height is auto and no padding/border separates parent
    // from its last in-flow block child, their bottom margins
    // collapse. The child's bottom margin is absorbed into the
    // parent's.
    if can_collapse_bottom {
        let last_block = parent
            .children
            .iter()
            .rposition(|c| c.is_block_level() && !matches!(c.box_type, BoxType::Anonymous));
        if let Some(idx) = last_block {
            let child_mb = parent.children[idx].dimensions.margin.bottom;
            if child_mb > 0.0 {
                let parent_mb = parent.dimensions.margin.bottom;
                parent.dimensions.margin.bottom = collapse_margins(parent_mb, child_mb);
                parent.children[idx].dimensions.margin.bottom = 0.0;
            }
        }
    }

    // Clearfix: ensure the parent's height includes all floats.
    if !float_ctx.is_empty() {
        let float_bottom = float_ctx.clear_y(ClearSide::Both);
        if float_bottom > cursor_y {
            // Extend content height to encompass floated children.
            parent.dimensions.content.height = parent.dimensions.content.height.max(float_bottom);
        }
    }
}

/// Calculate the height of a block-level box.
///
/// If `height` is explicit, use it. Otherwise, height is the distance
/// from the top of the content area to the bottom of the last child's
/// margin box. Percentage heights resolve against `containing_height`
/// when available.
fn calculate_block_height(layout_box: &mut LayoutBox, containing_height: Option<f32>) {
    let is_border_box = layout_box.style.box_sizing == BoxSizing::BorderBox;
    let pad_v = layout_box.dimensions.padding.vertical();
    let bdr_v = layout_box.dimensions.border.vertical();

    match layout_box.style.height {
        Dimension::Px(h) => {
            let content_h = if is_border_box {
                (h - pad_v - bdr_v).max(0.0)
            } else {
                h
            };
            layout_box.dimensions.content.height = content_h;
        },
        Dimension::Percent(pct) => {
            if let Some(ch) = containing_height {
                let declared_h = ch * (pct / 100.0);
                let content_h = if is_border_box {
                    (declared_h - pad_v - bdr_v).max(0.0)
                } else {
                    declared_h
                };
                layout_box.dimensions.content.height = content_h;
            } else {
                // No definite containing height: treat as auto (CSS spec).
                calculate_auto_height(layout_box);
            }
        },
        Dimension::Auto => {
            calculate_auto_height(layout_box);
        },
    }

    // Apply min-height / max-height constraints.
    match layout_box.style.min_height {
        Dimension::Px(min) => {
            layout_box.dimensions.content.height = layout_box.dimensions.content.height.max(min);
        },
        Dimension::Percent(pct) => {
            if let Some(ch) = containing_height {
                let min = ch * (pct / 100.0);
                layout_box.dimensions.content.height =
                    layout_box.dimensions.content.height.max(min);
            }
        },
        Dimension::Auto => {},
    }
    match layout_box.style.max_height {
        Dimension::Px(max) => {
            layout_box.dimensions.content.height = layout_box.dimensions.content.height.min(max);
        },
        Dimension::Percent(pct) => {
            if let Some(ch) = containing_height {
                let max = ch * (pct / 100.0);
                layout_box.dimensions.content.height =
                    layout_box.dimensions.content.height.min(max);
            }
        },
        Dimension::Auto => {},
    }
}

/// Compute auto height from children's occupied space.
fn calculate_auto_height(layout_box: &mut LayoutBox) {
    let content_top = layout_box.dimensions.content.y;
    let mut bottom = content_top;

    for child in &layout_box.children {
        let child_mb = child.dimensions.margin_box();
        let child_bottom = child_mb.y + child_mb.height;
        if child_bottom > bottom {
            bottom = child_bottom;
        }
    }

    layout_box.dimensions.content.height = (bottom - content_top).max(0.0);
}

// -------------------------------------------------------------------
// Margin collapsing
// -------------------------------------------------------------------

/// Collapse adjacent vertical margins between siblings.
///
/// - If both are positive: use the larger one.
/// - If one is negative: sum them.
/// - If both are negative: use the more negative one (min).
///
/// Returns the effective vertical gap to insert between the previous
/// sibling's bottom and the current sibling's top.
pub fn collapse_margins(prev_bottom: f32, next_top: f32) -> f32 {
    if prev_bottom >= 0.0 && next_top >= 0.0 {
        prev_bottom.max(next_top)
    } else if prev_bottom < 0.0 && next_top < 0.0 {
        prev_bottom.min(next_top)
    } else {
        prev_bottom + next_top
    }
}

// -------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css::values::Dimension;

    /// Fixed-width text measurer: each character is 8 pixels wide.
    struct FixedMeasurer;

    impl TextMeasurer for FixedMeasurer {
        fn measure_text(&self, text: &str, font_size: u16) -> u32 {
            oasis_types::backend::bitmap_measure_text(text, font_size)
        }
    }

    fn block_style() -> ComputedStyle {
        let mut s = ComputedStyle::default();
        s.display = Display::Block;
        s
    }

    // -- text measurer -------------------------------------------------

    #[test]
    fn fixed_measurer_returns_expected_width() {
        let m = FixedMeasurer;
        // Sub-pixel: h(7*12/8)+e(7*12/8)+l(5*12/8)+l(5*12/8)+o(7*12/8)
        //          = 10+10+7+7+10 = 44
        assert_eq!(m.measure_text("hello", 12), 44);
        assert_eq!(m.measure_text("", 12), 0);
    }

    // -- block width calculation --------------------------------------

    #[test]
    fn auto_width_fills_container() {
        let m = FixedMeasurer;
        let mut lb = LayoutBox::new(BoxType::Block, block_style(), None);
        lb.dimensions.content.x = 0.0;
        lb.dimensions.content.y = 0.0;
        layout_block(&mut lb, 480.0, &m);
        assert_eq!(lb.dimensions.content.width, 480.0);
    }

    #[test]
    fn explicit_width_centering() {
        let m = FixedMeasurer;
        let mut style = block_style();
        style.width = Dimension::Px(200.0);
        style.margin_left_auto = true;
        style.margin_right_auto = true;
        let mut lb = LayoutBox::new(BoxType::Block, style, None);
        lb.dimensions.content.x = 0.0;
        lb.dimensions.content.y = 0.0;
        layout_block(&mut lb, 480.0, &m);
        assert_eq!(lb.dimensions.content.width, 200.0);
        // Margins should be equal (centered): (480 - 200) / 2 = 140
        let ml = lb.dimensions.margin.left;
        let mr = lb.dimensions.margin.right;
        assert!(
            (ml - mr).abs() < f32::EPSILON,
            "margins should be equal: left={ml}, right={mr}",
        );
        assert!(
            (ml - 140.0).abs() < f32::EPSILON,
            "margin should be 140, got {ml}",
        );
    }

    #[test]
    fn explicit_width_no_auto_margins_left_aligned() {
        let m = FixedMeasurer;
        let mut style = block_style();
        style.width = Dimension::Px(200.0);
        // No auto margins -- should NOT be centered.
        let mut lb = LayoutBox::new(BoxType::Block, style, None);
        lb.dimensions.content.x = 0.0;
        lb.dimensions.content.y = 0.0;
        layout_block(&mut lb, 480.0, &m);
        assert_eq!(lb.dimensions.content.width, 200.0);
        assert!(
            lb.dimensions.margin.left.abs() < f32::EPSILON,
            "left margin should be 0 (left-aligned), got {}",
            lb.dimensions.margin.left,
        );
        assert!(
            (lb.dimensions.margin.right - 280.0).abs() < f32::EPSILON,
            "right margin should absorb remaining space: got {}",
            lb.dimensions.margin.right,
        );
    }

    #[test]
    fn nested_block_layout() {
        let m = FixedMeasurer;
        let mut parent = LayoutBox::new(BoxType::Block, block_style(), None);
        let child = LayoutBox::new(BoxType::Block, block_style(), None);
        parent.children = vec![child];
        parent.dimensions.content.x = 0.0;
        parent.dimensions.content.y = 0.0;
        layout_block(&mut parent, 480.0, &m);

        assert_eq!(parent.dimensions.content.width, 480.0);
        assert_eq!(parent.children[0].dimensions.content.width, 480.0,);
    }

    #[test]
    fn multiple_children_stacked_vertically() {
        let m = FixedMeasurer;
        let mut parent = LayoutBox::new(BoxType::Block, block_style(), None);
        let mut s1 = block_style();
        s1.height = Dimension::Px(30.0);
        let mut s2 = block_style();
        s2.height = Dimension::Px(50.0);

        parent.children = vec![
            LayoutBox::new(BoxType::Block, s1, None),
            LayoutBox::new(BoxType::Block, s2, None),
        ];
        parent.dimensions.content.x = 0.0;
        parent.dimensions.content.y = 0.0;
        layout_block(&mut parent, 480.0, &m);

        let c0_y = parent.children[0].dimensions.content.y;
        let c1_y = parent.children[1].dimensions.content.y;

        assert!(
            c1_y > c0_y,
            "second child should be below first: c0_y={c0_y}, c1_y={c1_y}",
        );
        assert_eq!(
            parent.dimensions.content.height, 80.0,
            "parent height should be sum of children",
        );
    }

    // -- margin collapsing --------------------------------------------

    #[test]
    fn collapse_both_positive() {
        assert_eq!(collapse_margins(10.0, 20.0), 20.0);
        assert_eq!(collapse_margins(20.0, 10.0), 20.0);
    }

    #[test]
    fn collapse_one_negative() {
        assert_eq!(collapse_margins(10.0, -5.0), 5.0);
        assert_eq!(collapse_margins(-5.0, 10.0), 5.0);
    }

    #[test]
    fn collapse_both_negative() {
        assert_eq!(collapse_margins(-10.0, -5.0), -10.0);
        assert_eq!(collapse_margins(-5.0, -10.0), -10.0);
    }

    #[test]
    fn collapse_zero() {
        assert_eq!(collapse_margins(0.0, 0.0), 0.0);
        assert_eq!(collapse_margins(10.0, 0.0), 10.0);
    }

    #[test]
    fn margin_collapsing_between_siblings() {
        let m = FixedMeasurer;
        let mut parent = LayoutBox::new(BoxType::Block, block_style(), None);

        let mut s1 = block_style();
        s1.height = Dimension::Px(20.0);
        s1.margin_bottom = 15.0;

        let mut s2 = block_style();
        s2.height = Dimension::Px(20.0);
        s2.margin_top = 10.0;

        parent.children = vec![
            LayoutBox::new(BoxType::Block, s1, None),
            LayoutBox::new(BoxType::Block, s2, None),
        ];
        parent.dimensions.content.x = 0.0;
        parent.dimensions.content.y = 0.0;
        layout_block(&mut parent, 480.0, &m);

        // With margin collapsing, the space between the two
        // children's border boxes should be max(15, 10) = 15, not
        // the sum 15 + 10 = 25. We verify by checking the gap
        // between the first child's border-box bottom and the
        // second child's border-box top.
        let c0_bb = parent.children[0].dimensions.border_box();
        let c0_bb_bottom = c0_bb.y + c0_bb.height;

        let c1_bb = parent.children[1].dimensions.border_box();
        let c1_bb_top = c1_bb.y;

        let gap = c1_bb_top - c0_bb_bottom;
        assert!(
            (gap - 15.0).abs() < 0.01,
            "collapsed margin between siblings should be 15, got {gap}",
        );
    }

    // -- anonymous box wrapping ----------------------------------------

    #[test]
    fn wrap_anonymous_mixed_children() {
        let inline_box = LayoutBox::new(BoxType::Inline, ComputedStyle::default(), None);
        let block_box = LayoutBox::new(BoxType::Block, block_style(), None);
        let inline_box2 = LayoutBox::new(BoxType::Inline, ComputedStyle::default(), None);

        let ps = block_style();
        let wrapped = wrap_anonymous(vec![inline_box, block_box, inline_box2], &ps);

        // Should be: anon(inline), block, anon(inline)
        assert_eq!(wrapped.len(), 3);
        assert!(matches!(wrapped[0].box_type, BoxType::Anonymous));
        assert!(matches!(wrapped[1].box_type, BoxType::Block));
        assert!(matches!(wrapped[2].box_type, BoxType::Anonymous));
    }

    #[test]
    fn wrap_anonymous_all_blocks() {
        let b1 = LayoutBox::new(BoxType::Block, block_style(), None);
        let b2 = LayoutBox::new(BoxType::Block, block_style(), None);
        let ps = block_style();
        let wrapped = wrap_anonymous(vec![b1, b2], &ps);
        // No wrapping needed.
        assert_eq!(wrapped.len(), 2);
        assert!(matches!(wrapped[0].box_type, BoxType::Block));
        assert!(matches!(wrapped[1].box_type, BoxType::Block));
    }

    #[test]
    fn wrap_anonymous_all_inline() {
        let i1 = LayoutBox::new(BoxType::Inline, ComputedStyle::default(), None);
        let i2 = LayoutBox::new(BoxType::Inline, ComputedStyle::default(), None);
        let ps = ComputedStyle::default();
        let wrapped = wrap_anonymous(vec![i1, i2], &ps);
        // No wrapping needed (all inline).
        assert_eq!(wrapped.len(), 2);
        assert!(matches!(wrapped[0].box_type, BoxType::Inline));
    }

    // -- incremental layout -------------------------------------------

    #[test]
    fn incremental_layout_matches_full_layout() {
        let m = FixedMeasurer;
        let mut parent = LayoutBox::new(BoxType::Block, block_style(), None);
        let mut s1 = block_style();
        s1.height = Dimension::Px(30.0);
        let mut s2 = block_style();
        s2.height = Dimension::Px(50.0);
        parent.children = vec![
            LayoutBox::new(BoxType::Block, s1, None),
            LayoutBox::new(BoxType::Block, s2, None),
        ];
        parent.dimensions.content.x = 0.0;
        parent.dimensions.content.y = 0.0;

        // Full layout.
        let mut full = parent.clone();
        layout_block(&mut full, 480.0, &m);

        // Incremental layout (all dirty initially).
        let mut cache = StyleCache::new();
        layout_block_incremental(&mut parent, 480.0, &m, &mut cache);

        assert_eq!(
            parent.dimensions.content.height,
            full.dimensions.content.height,
        );
        assert_eq!(
            parent.dimensions.content.width,
            full.dimensions.content.width,
        );
    }

    #[test]
    fn clean_subtree_skipped_in_incremental() {
        let m = FixedMeasurer;
        let mut parent = LayoutBox::new(BoxType::Block, block_style(), None);
        let mut s1 = block_style();
        s1.height = Dimension::Px(30.0);
        parent.children = vec![LayoutBox::new(BoxType::Block, s1, None)];
        parent.dimensions.content.x = 0.0;
        parent.dimensions.content.y = 0.0;

        // First pass: full incremental layout.
        let mut cache = StyleCache::new();
        layout_block_incremental(&mut parent, 480.0, &m, &mut cache);

        // Mark everything clean, then call again -- should be no-op.
        parent.mark_clean();
        let old_height = parent.dimensions.content.height;
        layout_block_incremental(&mut parent, 480.0, &m, &mut cache);
        assert_eq!(parent.dimensions.content.height, old_height);
    }

    #[test]
    fn dirty_child_triggers_relayout() {
        let m = FixedMeasurer;
        let mut parent = LayoutBox::new(BoxType::Block, block_style(), None);
        let mut s1 = block_style();
        s1.height = Dimension::Px(30.0);
        parent.children = vec![LayoutBox::new(BoxType::Block, s1, Some(1))];
        parent.dimensions.content.x = 0.0;
        parent.dimensions.content.y = 0.0;

        let mut cache = StyleCache::new();
        layout_block_incremental(&mut parent, 480.0, &m, &mut cache);
        parent.mark_clean();

        // Dirty the child and change its height.
        parent.children[0].dirty = true;
        parent.dirty = true;
        parent.children[0].style.height = Dimension::Px(60.0);
        layout_block_incremental(&mut parent, 480.0, &m, &mut cache);

        assert_eq!(parent.dimensions.content.height, 60.0);
    }

    // -- style cache --------------------------------------------------

    #[test]
    fn style_cache_insert_and_get() {
        let mut cache = StyleCache::new();
        assert!(cache.is_empty());
        let pad = EdgeSizes::new(1.0, 2.0, 3.0, 4.0);
        let bdr = EdgeSizes::new(5.0, 6.0, 7.0, 8.0);
        let mar = EdgeSizes::new(9.0, 10.0, 11.0, 12.0);
        cache.insert_edges(42, pad, bdr, mar);
        assert_eq!(cache.len(), 1);
        let (p, b, m) = cache.get_edges(42).expect("cache hit");
        assert_eq!(*p, pad);
        assert_eq!(*b, bdr);
        assert_eq!(*m, mar);
    }

    #[test]
    fn style_cache_miss() {
        let cache = StyleCache::new();
        assert!(cache.get_edges(99).is_none());
    }

    #[test]
    fn style_cache_clear() {
        let mut cache = StyleCache::new();
        cache.insert_edges(
            1,
            EdgeSizes::default(),
            EdgeSizes::default(),
            EdgeSizes::default(),
        );
        cache.clear();
        assert!(cache.is_empty());
    }

    #[test]
    fn mark_dirty_propagation() {
        let mut lb = LayoutBox::new(BoxType::Block, block_style(), None);
        lb.mark_clean();
        assert!(!lb.dirty);
        lb.mark_dirty();
        assert!(lb.dirty);
    }

    #[test]
    fn any_child_dirty_detection() {
        let mut parent = LayoutBox::new(BoxType::Block, block_style(), None);
        let mut child = LayoutBox::new(BoxType::Block, block_style(), None);
        child.dirty = false;
        parent.children = vec![child];
        parent.dirty = false;
        assert!(!any_child_dirty(&parent));

        parent.children[0].dirty = true;
        assert!(any_child_dirty(&parent));
    }

    // -- BUG 1: nested padding not double-counted ---------------------

    #[test]
    fn nested_padding_not_double_counted() {
        let m = FixedMeasurer;

        // Grandparent with padding 10.
        let mut gp_style = block_style();
        gp_style.padding_left = 10.0;
        gp_style.padding_top = 10.0;
        let mut grandparent = LayoutBox::new(BoxType::Block, gp_style, None);

        // Parent with padding 5.
        let mut p_style = block_style();
        p_style.padding_left = 5.0;
        p_style.padding_top = 5.0;
        p_style.height = Dimension::Px(20.0);
        let mut parent = LayoutBox::new(BoxType::Block, p_style, None);

        // Leaf child with no padding.
        let mut c_style = block_style();
        c_style.height = Dimension::Px(10.0);
        let child = LayoutBox::new(BoxType::Block, c_style, None);

        parent.children = vec![child];
        grandparent.children = vec![parent];
        grandparent.dimensions.content.x = 0.0;
        grandparent.dimensions.content.y = 0.0;
        layout_block(&mut grandparent, 480.0, &m);

        // Apply root offset (same as build_layout_tree does).
        let dx = grandparent.dimensions.margin.left
            + grandparent.dimensions.border.left
            + grandparent.dimensions.padding.left;
        let dy = grandparent.dimensions.margin.top
            + grandparent.dimensions.border.top
            + grandparent.dimensions.padding.top;
        offset_descendant(&mut grandparent, dx, dy);

        // Leaf should be at grandparent_padding(10) + parent_padding(5)
        // = 15, NOT grandparent_padding + 2*parent_padding = 20.
        let leaf = &grandparent.children[0].children[0];
        assert!(
            (leaf.dimensions.content.x - 15.0).abs() < 0.01,
            "leaf x should be 15 (10+5), got {}",
            leaf.dimensions.content.x,
        );
        assert!(
            (leaf.dimensions.content.y - 15.0).abs() < 0.01,
            "leaf y should be 15 (10+5), got {}",
            leaf.dimensions.content.y,
        );
    }

    // -- BUG 7: parent-child margin collapsing ------------------------

    #[test]
    fn parent_child_margin_collapsing() {
        let m = FixedMeasurer;

        // Parent with margin-top 10, no padding/border.
        let mut p_style = block_style();
        p_style.margin_top = 10.0;
        let mut parent = LayoutBox::new(BoxType::Block, p_style, None);

        // Child with margin-top 20.
        let mut c_style = block_style();
        c_style.margin_top = 20.0;
        c_style.height = Dimension::Px(30.0);
        let child = LayoutBox::new(BoxType::Block, c_style, None);

        parent.children = vec![child];
        parent.dimensions.content.x = 0.0;
        parent.dimensions.content.y = 0.0;
        layout_block(&mut parent, 480.0, &m);

        // Parent margin-top should collapse with child: max(10, 20) = 20.
        assert!(
            (parent.dimensions.margin.top - 20.0).abs() < 0.01,
            "collapsed parent margin-top should be 20, got {}",
            parent.dimensions.margin.top,
        );

        // Child's margin was absorbed into parent: child sits at
        // parent's content.y without extra gap.
        assert!(
            parent.children[0].dimensions.margin.top.abs() < 0.01,
            "child margin-top should be 0 (absorbed), got {}",
            parent.children[0].dimensions.margin.top,
        );

        // Parent height should be just the child's content (30px),
        // not child content + child margin (50px).
        assert!(
            (parent.dimensions.content.height - 30.0).abs() < 0.01,
            "parent height should be 30 (child only), got {}",
            parent.dimensions.content.height,
        );
    }

    #[test]
    fn parent_child_margin_no_collapse_with_padding() {
        let m = FixedMeasurer;

        // Parent with padding-top (separates margins).
        let mut p_style = block_style();
        p_style.margin_top = 10.0;
        p_style.padding_top = 1.0;
        let mut parent = LayoutBox::new(BoxType::Block, p_style, None);

        let mut c_style = block_style();
        c_style.margin_top = 20.0;
        c_style.height = Dimension::Px(30.0);
        let child = LayoutBox::new(BoxType::Block, c_style, None);

        parent.children = vec![child];
        parent.dimensions.content.x = 0.0;
        parent.dimensions.content.y = 0.0;
        layout_block(&mut parent, 480.0, &m);

        // With padding, no collapsing: parent margin stays 10.
        assert!(
            (parent.dimensions.margin.top - 10.0).abs() < 0.01,
            "parent margin-top should stay 10 with padding, got {}",
            parent.dimensions.margin.top,
        );
    }

    // -- percentage height resolution ---------------------------------

    #[test]
    fn test_percentage_height_resolves() {
        let m = FixedMeasurer;
        let mut style = block_style();
        style.height = Dimension::Percent(50.0);

        let mut lb = LayoutBox::new(BoxType::Block, style, None);
        lb.dimensions.content.x = 0.0;
        lb.dimensions.content.y = 0.0;

        // Use layout_block_with_height to pass containing height.
        layout_block_with_height(&mut lb, 480.0, Some(200.0), &m);

        assert!(
            (lb.dimensions.content.height - 100.0).abs() < 0.01,
            "50% of 200px containing height should be 100, got {}",
            lb.dimensions.content.height,
        );
    }

    // -- min/max height clamping --------------------------------------

    #[test]
    fn test_min_max_height_clamps() {
        let m = FixedMeasurer;

        // min-height enforced when content is smaller.
        let mut style = block_style();
        style.min_height = Dimension::Px(50.0);
        let mut lb = LayoutBox::new(BoxType::Block, style, None);
        lb.dimensions.content.x = 0.0;
        lb.dimensions.content.y = 0.0;
        layout_block(&mut lb, 480.0, &m);
        assert!(
            lb.dimensions.content.height >= 50.0,
            "min-height 50 should enforce minimum, got {}",
            lb.dimensions.content.height,
        );

        // max-height caps growth.
        let mut style2 = block_style();
        style2.height = Dimension::Px(200.0);
        style2.max_height = Dimension::Px(80.0);
        let mut lb2 = LayoutBox::new(BoxType::Block, style2, None);
        lb2.dimensions.content.x = 0.0;
        lb2.dimensions.content.y = 0.0;
        layout_block(&mut lb2, 480.0, &m);
        assert!(
            (lb2.dimensions.content.height - 80.0).abs() < 0.01,
            "max-height 80 should cap height 200, got {}",
            lb2.dimensions.content.height,
        );
    }

    // -- min/max width applied ----------------------------------------

    #[test]
    fn test_min_max_width_applied() {
        let m = FixedMeasurer;

        // min-width enforced.
        let mut style = block_style();
        style.width = Dimension::Auto;
        style.min_width = Dimension::Px(200.0);
        let mut lb = LayoutBox::new(BoxType::Block, style, None);
        lb.dimensions.content.x = 0.0;
        lb.dimensions.content.y = 0.0;
        layout_block(&mut lb, 100.0, &m);
        assert!(
            lb.dimensions.content.width >= 200.0,
            "min-width 200 should override auto width in 100px container, got {}",
            lb.dimensions.content.width,
        );

        // max-width caps.
        let mut style2 = block_style();
        style2.width = Dimension::Auto;
        style2.max_width = Dimension::Px(150.0);
        let mut lb2 = LayoutBox::new(BoxType::Block, style2, None);
        lb2.dimensions.content.x = 0.0;
        lb2.dimensions.content.y = 0.0;
        layout_block(&mut lb2, 400.0, &m);
        assert!(
            (lb2.dimensions.content.width - 150.0).abs() < 0.01,
            "max-width 150 should cap auto width in 400px container, got {}",
            lb2.dimensions.content.width,
        );
    }

    // -- block-level replaced elements --------------------------------

    #[test]
    fn hr_gets_container_width() {
        let m = FixedMeasurer;
        let mut parent = LayoutBox::new(BoxType::Block, block_style(), None);
        let hr_style = block_style();
        let hr = LayoutBox::new(
            BoxType::Replaced(ReplacedContent::HorizontalRule),
            hr_style,
            None,
        );
        parent.children = vec![hr];
        parent.dimensions.content.x = 0.0;
        parent.dimensions.content.y = 0.0;
        layout_block(&mut parent, 200.0, &m);

        let hr_box = &parent.children[0];
        assert!(
            hr_box.dimensions.content.width > 100.0,
            "HR should stretch to near container width, got {}",
            hr_box.dimensions.content.width,
        );
        assert!(
            hr_box.dimensions.content.height > 0.0,
            "HR should have positive height, got {}",
            hr_box.dimensions.content.height,
        );
    }

    #[test]
    fn hr_is_block_level() {
        let hr = LayoutBox::new(
            BoxType::Replaced(ReplacedContent::HorizontalRule),
            block_style(),
            None,
        );
        assert!(
            hr.is_block_level(),
            "HorizontalRule replaced box should be block-level"
        );
    }

    // -- empty block self-collapsing -----------------------------------

    #[test]
    fn empty_block_collapses_own_margins() {
        let m = FixedMeasurer;
        let mut parent = LayoutBox::new(BoxType::Block, block_style(), None);

        // First child: 20px tall.
        let mut s1 = block_style();
        s1.height = Dimension::Px(20.0);
        s1.margin_bottom = 15.0;

        // Empty block between siblings: margins 10 top, 10 bottom.
        // Should self-collapse to 10, then collapse with siblings.
        let mut s_empty = block_style();
        s_empty.margin_top = 10.0;
        s_empty.margin_bottom = 10.0;

        // Third child: 20px tall.
        let mut s3 = block_style();
        s3.height = Dimension::Px(20.0);
        s3.margin_top = 5.0;

        parent.children = vec![
            LayoutBox::new(BoxType::Block, s1, None),
            LayoutBox::new(BoxType::Block, s_empty, None),
            LayoutBox::new(BoxType::Block, s3, None),
        ];
        parent.dimensions.content.x = 0.0;
        parent.dimensions.content.y = 0.0;
        layout_block(&mut parent, 480.0, &m);

        // The empty block self-collapses: max(10, 10) = 10.
        // Collapsing chain: prev_margin_bottom(15) vs empty(10) vs
        // next_top(5) → the whole chain collapses to max(15, 10, 5) = 15.
        let c0_bb = parent.children[0].dimensions.border_box();
        let c0_bottom = c0_bb.y + c0_bb.height;
        let c2_bb = parent.children[2].dimensions.border_box();
        let c2_top = c2_bb.y;
        let gap = c2_top - c0_bottom;

        assert!(
            (gap - 15.0).abs() < 0.01,
            "gap through empty block should be 15 (collapsed chain), got {gap}",
        );
    }

    // -- BFC inhibits parent-child collapsing -------------------------

    #[test]
    fn overflow_hidden_inhibits_parent_child_collapsing() {
        let m = FixedMeasurer;

        // Parent with overflow:hidden creates a new BFC.
        let mut p_style = block_style();
        p_style.margin_top = 10.0;
        p_style.overflow = Overflow::Hidden;
        let mut parent = LayoutBox::new(BoxType::Block, p_style, None);

        // Child with margin-top 20.
        let mut c_style = block_style();
        c_style.margin_top = 20.0;
        c_style.height = Dimension::Px(30.0);
        let child = LayoutBox::new(BoxType::Block, c_style, None);

        parent.children = vec![child];
        parent.dimensions.content.x = 0.0;
        parent.dimensions.content.y = 0.0;
        layout_block(&mut parent, 480.0, &m);

        // With overflow:hidden, margins should NOT collapse.
        assert!(
            (parent.dimensions.margin.top - 10.0).abs() < 0.01,
            "parent margin-top should stay 10 with overflow:hidden, got {}",
            parent.dimensions.margin.top,
        );

        // Child keeps its own margin inside the parent.
        assert!(
            (parent.children[0].dimensions.margin.top - 20.0).abs() < 0.01,
            "child margin-top should stay 20 (no collapsing), got {}",
            parent.children[0].dimensions.margin.top,
        );
    }

    // -- parent-last-child bottom margin collapsing --------------------

    #[test]
    fn parent_child_bottom_margin_collapsing() {
        let m = FixedMeasurer;

        // Parent with margin-bottom 10, no padding/border, height auto.
        let mut p_style = block_style();
        p_style.margin_bottom = 10.0;
        let mut parent = LayoutBox::new(BoxType::Block, p_style, None);

        // Child with margin-bottom 25.
        let mut c_style = block_style();
        c_style.margin_bottom = 25.0;
        c_style.height = Dimension::Px(30.0);
        let child = LayoutBox::new(BoxType::Block, c_style, None);

        parent.children = vec![child];
        parent.dimensions.content.x = 0.0;
        parent.dimensions.content.y = 0.0;
        layout_block(&mut parent, 480.0, &m);

        // Parent bottom margin should collapse with child: max(10, 25) = 25.
        assert!(
            (parent.dimensions.margin.bottom - 25.0).abs() < 0.01,
            "collapsed parent margin-bottom should be 25, got {}",
            parent.dimensions.margin.bottom,
        );

        // Child's bottom margin was absorbed into parent.
        assert!(
            parent.children[0].dimensions.margin.bottom.abs() < 0.01,
            "child margin-bottom should be 0 (absorbed), got {}",
            parent.children[0].dimensions.margin.bottom,
        );
    }
}
