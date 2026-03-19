//! Inline-level layout algorithm.
//!
//! Implements CSS 2.1 inline formatting context (IFC) layout. Inline
//! boxes flow horizontally and wrap into line boxes when the available
//! width is exhausted.

use super::block::{TextMeasurer, layout_block};
use super::box_model::*;
use super::text::{
    apply_text_transform, collapse_whitespace, detect_direction, measure_space, measure_word,
    replace_unrenderable, split_into_words,
};
use crate::css::values::{
    ComputedStyle, Dimension, OverflowWrap, TextAlign, TextDirection, VerticalAlign, WhiteSpace,
    WordBreak,
};
use crate::html::dom::NodeId;

// -------------------------------------------------------------------
// Public entry point
// -------------------------------------------------------------------

/// Layout inline children of an anonymous or block box into line
/// boxes, then position fragments and update the parent's content
/// height.
pub fn layout_inline(parent: &mut LayoutBox, measurer: &dyn TextMeasurer) {
    let available_width = parent.dimensions.content.width;
    let text_align = parent.style.text_align;

    // Collect all inline fragments from the children.
    let fragments = collect_inline_fragments(&parent.children, available_width, measurer);

    // Resolve `TextDirection::Auto` by detecting the dominant
    // direction from the first text fragment's content.
    let direction = if parent.style.direction == TextDirection::Auto {
        let sample = fragments.iter().find_map(|f| {
            if let InlineFragment::Text { text, .. } = f
                && !text.is_empty()
                && text != "\n"
            {
                return Some(text.as_str());
            }
            None
        });
        sample.map_or(TextDirection::Ltr, detect_direction)
    } else {
        parent.style.direction
    };

    // Break fragments into line boxes.
    // Estimate ~1 line per 80px of content width as a rough heuristic
    // to reduce Vec reallocations.
    let estimated_lines = if available_width > 0.0 {
        let total_frag_width: f32 = fragments.iter().map(InlineFragment::width).sum();
        ((total_frag_width / available_width) as usize + 1).max(1)
    } else {
        1
    };
    let mut lines: Vec<LineBox> = Vec::with_capacity(estimated_lines);
    let text_indent = parent.style.text_indent;
    let first_line_width = (available_width - text_indent).max(0.0);
    let mut current_line = LineBox::new(first_line_width);
    let nowrap = parent.style.white_space == WhiteSpace::NoWrap;
    let break_all = parent.style.word_break == WordBreak::BreakAll;
    let break_word = parent.style.overflow_wrap == OverflowWrap::BreakWord
        || parent.style.overflow_wrap == OverflowWrap::Anywhere;

    for fragment in &fragments {
        // Check for line break fragments (<br> or "\n" in pre mode).
        let is_line_break = match fragment {
            InlineFragment::ReplacedInline {
                replaced: ReplacedContent::LineBreak,
                ..
            } => true,
            InlineFragment::Text { text, .. } if text == "\n" => true,
            _ => false,
        };

        if is_line_break {
            // Push current line (even if empty, to create blank lines
            // for consecutive <br>s).
            lines.push(current_line);
            current_line = LineBox::new(available_width);
            continue;
        }

        // white-space: nowrap -- force-add without breaking.
        if nowrap {
            current_line.fragments.push(fragment.clone());
            continue;
        }

        // word-break: break-all — always break at character boundaries.
        if break_all {
            let pieces = break_word_fragment(fragment, available_width, measurer);
            for piece in &pieces {
                if !current_line.try_add(piece) {
                    lines.push(current_line);
                    current_line = LineBox::new(available_width);
                    current_line.try_add(piece);
                }
            }
            continue;
        }

        if !current_line.try_add(fragment) {
            // If the fragment doesn't fit on an empty line, break it
            // character-by-character (emergency word breaking, or
            // overflow-wrap: break-word).
            if current_line.is_empty() || break_word {
                if current_line.is_empty() {
                    let pieces = break_word_fragment(fragment, available_width, measurer);
                    for piece in &pieces {
                        if !current_line.try_add(piece) {
                            lines.push(current_line);
                            current_line = LineBox::new(available_width);
                            current_line.try_add(piece);
                        }
                    }
                    continue;
                }
                // break-word with non-empty line: wrap to next line first,
                // then break if still doesn't fit.
                lines.push(current_line);
                current_line = LineBox::new(available_width);
                if !current_line.try_add(fragment) && current_line.is_empty() {
                    let pieces = break_word_fragment(fragment, available_width, measurer);
                    for piece in &pieces {
                        if !current_line.try_add(piece) {
                            lines.push(current_line);
                            current_line = LineBox::new(available_width);
                            current_line.try_add(piece);
                        }
                    }
                }
                continue;
            }
            lines.push(current_line);
            current_line = LineBox::new(available_width);
            // The fragment that did not fit starts the new line.
            if !current_line.try_add(fragment) && current_line.is_empty() {
                // Still doesn't fit: emergency break.
                let pieces = break_word_fragment(fragment, available_width, measurer);
                for piece in &pieces {
                    if !current_line.try_add(piece) {
                        lines.push(current_line);
                        current_line = LineBox::new(available_width);
                        current_line.try_add(piece);
                    }
                }
            }
        }
    }
    if !current_line.is_empty() {
        lines.push(current_line);
    }

    // CSS 2.1 §16.6.1: strip spaces at line boundaries.
    for line in &mut lines {
        trim_line_boundary_spaces(line, measurer);
    }

    // Soft-hyphen resolution: for every non-last line whose last
    // fragment has `soft_hyphen == true`, append a visible "-" and
    // widen the fragment accordingly.
    let line_count = lines.len();
    for (i, line) in lines.iter_mut().enumerate() {
        if i >= line_count - 1 {
            break; // last line: no hyphen needed
        }
        if let Some(InlineFragment::Text {
            text,
            width,
            style,
            soft_hyphen,
            ..
        }) = line.fragments.last_mut()
            && *soft_hyphen
        {
            let hyphen_w = measurer.measure_text("-", style.font_size as u16) as f32;
            text.push('-');
            *width += hyphen_w;
            *soft_hyphen = false; // consumed
        }
    }

    // Position line boxes vertically and apply text alignment.
    let mut cursor_y = parent.dimensions.content.y;
    let last_line_idx = lines.len().saturating_sub(1);

    // Track (line, line_y) pairs for child reconstruction.
    let mut line_positions: Vec<f32> = Vec::with_capacity(lines.len());

    for (i, line) in lines.iter_mut().enumerate() {
        // Compute line height (max of fragment heights).
        let line_height = line
            .fragments
            .iter()
            .map(InlineFragment::height)
            .fold(0.0_f32, f32::max);
        line.height = if line_height > 0.0 {
            line_height
        } else {
            parent.style.line_height
        };
        // Baseline: bitmap font ascender is ~75% of em square.
        line.baseline = line.height * 0.75;

        // Position fragments horizontally.
        // First line gets text-indent offset and reduced available width.
        let is_last_line = i == last_line_idx;
        let (line_avail, line_offset) = if i == 0 && text_indent != 0.0 {
            (first_line_width, text_indent)
        } else {
            (available_width, 0.0)
        };
        position_fragments_on_line(
            line,
            line_avail,
            text_align,
            is_last_line,
            parent.dimensions.content.x + line_offset,
            direction,
        );

        line_positions.push(cursor_y);
        cursor_y += line.height;
    }

    // Store the lines' fragments back as children (flattened).
    parent.children = lines_to_children(lines, &line_positions);

    // Update parent height.
    parent.dimensions.content.height = cursor_y - parent.dimensions.content.y;
}

// -------------------------------------------------------------------
// Fragment collection
// -------------------------------------------------------------------

/// Collect inline fragments from a list of layout box children.
///
/// Text nodes are split into word-level fragments for line breaking.
/// Inline boxes are kept as single fragments. `available_width` is the
/// containing block's content width, used for inline-block percentage
/// resolution.
fn collect_inline_fragments(
    children: &[LayoutBox],
    available_width: f32,
    measurer: &dyn TextMeasurer,
) -> Vec<InlineFragment> {
    // Pre-allocate: most inline children produce at least one fragment.
    let mut fragments = Vec::with_capacity(children.len());

    for child in children {
        match &child.box_type {
            BoxType::Inline => {
                // Check if this is a text node (has a node id and
                // the style says inline). We produce text fragments.
                fragments.extend(text_fragments_for_inline(child, available_width, measurer));
            },
            BoxType::InlineBlock => {
                // InlineBlock boxes participate in inline flow but
                // establish their own block formatting context. We
                // must lay them out now so dimensions are known.
                let mut lb = child.clone();
                layout_block(&mut lb, available_width, measurer);
                if matches!(lb.style.width, Dimension::Auto) {
                    // Shrink content width to actual children extent.
                    let max_child_right = lb
                        .children
                        .iter()
                        .map(|c| {
                            let bb = c.dimensions.border_box();
                            bb.x + bb.width - lb.dimensions.content.x
                        })
                        .fold(0.0_f32, f32::max);
                    lb.dimensions.content.width = max_child_right;
                }
                // InlineBlock margins must not be inflated by the
                // block-level over-constrained rule. Reset to declared
                // values (auto margins resolve to zero for inline-block).
                lb.dimensions.margin.left = if lb.style.margin_left_auto {
                    0.0
                } else {
                    lb.style.margin_left
                };
                lb.dimensions.margin.right = if lb.style.margin_right_auto {
                    0.0
                } else {
                    lb.style.margin_right
                };
                fragments.push(InlineFragment::InlineBox { layout_box: lb });
            },
            BoxType::Replaced(replaced) => {
                let (intrinsic_w, intrinsic_h) = replaced_dimensions(replaced);
                // Apply CSS width/height if set, falling back to intrinsic.
                let w = match child.style.width {
                    crate::css::values::Dimension::Px(px) => px,
                    _ => intrinsic_w,
                };
                let h = match child.style.height {
                    crate::css::values::Dimension::Px(px) => px,
                    _ => intrinsic_h,
                };
                // Preserve aspect ratio when only one dimension is set.
                let (w, h) = if child.style.width != crate::css::values::Dimension::Auto
                    && child.style.height == crate::css::values::Dimension::Auto
                    && intrinsic_h > 0.0
                {
                    (w, w * intrinsic_h / intrinsic_w.max(1.0))
                } else if child.style.height != crate::css::values::Dimension::Auto
                    && child.style.width == crate::css::values::Dimension::Auto
                    && intrinsic_w > 0.0
                {
                    (h * intrinsic_w / intrinsic_h.max(1.0), h)
                } else {
                    (w, h)
                };
                fragments.push(InlineFragment::ReplacedInline {
                    replaced: replaced.clone(),
                    x: 0.0,
                    width: w,
                    height: h,
                    style: child.style.clone(),
                    node: child.node,
                });
            },
            _ => {
                // Nested children (shouldn't happen in a well-formed
                // anonymous box, but handle gracefully).
                fragments.extend(collect_inline_fragments(
                    &child.children,
                    available_width,
                    measurer,
                ));
            },
        }
    }

    fragments
}

/// Generate text fragments for an inline box (splitting on word
/// boundaries for line breaking).
fn text_fragments_for_inline(
    layout_box: &LayoutBox,
    available_width: f32,
    measurer: &dyn TextMeasurer,
) -> Vec<InlineFragment> {
    let style = &layout_box.style;

    // If this is a leaf inline box with stored text content, produce
    // properly measured word fragments for line breaking.
    if let Some(ref text) = layout_box.text {
        return make_text_fragments(text, style, layout_box.node, measurer);
    }

    if layout_box.children.is_empty() {
        // Leaf inline with no text: emit a zero-width placeholder.
        return vec![InlineFragment::Text {
            text: String::new(),
            x: 0.0,
            width: 0.0,
            style: style.clone(),
            node: layout_box.node,
            soft_hyphen: false,
        }];
    }

    // Recurse into children.
    let mut frags = collect_inline_fragments(&layout_box.children, available_width, measurer);

    // Propagate this element's node ID to child fragments so that
    // link elements (<a>) are tracked through the paint pass.
    if let Some(node_id) = layout_box.node {
        for frag in &mut frags {
            if let InlineFragment::Text { node, .. } = frag {
                *node = Some(node_id);
            }
        }
    }

    // Apply inline-level horizontal margins: left margin adds space
    // before the first fragment, right margin after the last.
    let ml = style.margin_left;
    let mr = style.margin_right;
    if !frags.is_empty() {
        if ml > 0.0
            && let InlineFragment::Text { width, .. } = &mut frags[0]
        {
            *width += ml;
        }
        let last = frags.len() - 1;
        if mr > 0.0
            && let InlineFragment::Text { width, .. } = &mut frags[last]
        {
            *width += mr;
        }
    }

    frags
}

/// Get the dimensions of a replaced inline element.
fn replaced_dimensions(replaced: &ReplacedContent) -> (f32, f32) {
    match replaced {
        ReplacedContent::Image { width, height, .. } => (*width as f32, *height as f32),
        ReplacedContent::HorizontalRule => (0.0, 2.0),
        ReplacedContent::LineBreak => (0.0, 0.0),
        ReplacedContent::TextInput { size, .. } => {
            // Use bitmap measurement for 'M' width as the per-character size.
            let char_w = oasis_types::backend::bitmap_measure_text("M", 8) as f32;
            (*size as f32 * char_w + 8.0, 18.0)
        },
        ReplacedContent::SubmitButton { label } => {
            // Use bitmap measurement for accurate label width.
            let text_w = oasis_types::backend::bitmap_measure_text(label, 10) as f32;
            (text_w + 16.0, 20.0)
        },
        ReplacedContent::SelectBox { label, .. } => {
            let text_w = oasis_types::backend::bitmap_measure_text(label, 10) as f32;
            (text_w + 20.0, 18.0) // extra space for dropdown arrow
        },
        ReplacedContent::Checkbox { .. } => (13.0, 13.0),
        ReplacedContent::RadioButton { .. } => (13.0, 13.0),
        ReplacedContent::TextArea { rows, cols, .. } => {
            let char_w = oasis_types::backend::bitmap_measure_text("M", 8) as f32;
            let line_height = 14.0;
            (
                *cols as f32 * char_w + 8.0,
                *rows as f32 * line_height + 4.0,
            )
        },
        ReplacedContent::Svg { element } => (element.width, element.height),
        ReplacedContent::Canvas { state } => {
            let s = state.borrow();
            (s.width as f32, s.height as f32)
        },
    }
}

// -------------------------------------------------------------------
// Fragment creation from raw text
// -------------------------------------------------------------------

/// Create inline text fragments from a raw text string, splitting on
/// word boundaries. This is used when the caller has direct access to
/// the text content.
pub fn make_text_fragments(
    text: &str,
    style: &ComputedStyle,
    node: Option<NodeId>,
    measurer: &dyn TextMeasurer,
) -> Vec<InlineFragment> {
    let transformed = apply_text_transform(text, style.text_transform);
    // Expand tabs with custom tab-size before whitespace collapsing.
    let tab_expanded = if style.tab_size != 8
        && matches!(style.white_space, WhiteSpace::Pre | WhiteSpace::PreWrap)
        && transformed.contains('\t')
    {
        let tab = style.tab_size as usize;
        let mut result = String::with_capacity(transformed.len());
        let mut col = 0usize;
        for ch in transformed.chars() {
            if ch == '\t' {
                let spaces = if tab > 0 { tab - (col % tab) } else { 0 };
                for _ in 0..spaces {
                    result.push(' ');
                }
                col += spaces;
            } else if ch == '\n' {
                result.push(ch);
                col = 0;
            } else {
                result.push(ch);
                col += 1;
            }
        }
        std::borrow::Cow::Owned(result)
    } else {
        std::borrow::Cow::Borrowed(transformed.as_str())
    };
    let collapsed = collapse_whitespace(&tab_expanded, style.white_space);
    let renderable = replace_unrenderable(&collapsed);
    let words = split_into_words(&renderable, style.white_space);

    let font_size = style.font_size;
    let letter_spacing = style.letter_spacing;
    let word_spacing = style.word_spacing;
    let space_width = measure_space(font_size, word_spacing, measurer);
    let mut fragments = Vec::with_capacity(words.len());

    for word in &words {
        if word.text == "\n" {
            // Line break: represented as a zero-width fragment that
            // forces a new line.
            fragments.push(InlineFragment::Text {
                text: "\n".to_string(),
                x: 0.0,
                width: 0.0,
                style: style.clone(),
                node,
                soft_hyphen: false,
            });
            continue;
        }

        let word_width = measure_word(&word.text, font_size, letter_spacing, measurer);
        let mut total_width = word_width;
        let mut display_text = word.text.clone();

        // Prepend space when the source text had whitespace before
        // this word (inter-element whitespace preservation).
        if word.leading_space {
            display_text.insert(0, ' ');
            total_width += space_width;
        }

        // Append space when the source text had whitespace after
        // this word.
        if word.trailing_space {
            display_text.push(' ');
            total_width += space_width;
        }

        fragments.push(InlineFragment::Text {
            text: display_text,
            x: 0.0,
            width: total_width,
            style: style.clone(),
            node,
            soft_hyphen: word.soft_hyphen,
        });
    }

    fragments
}

// -------------------------------------------------------------------
// Emergency word breaking
// -------------------------------------------------------------------

/// Break a text fragment at the available width boundary, producing
/// multiple sub-fragments that each fit within the given width.
fn break_word_fragment(
    fragment: &InlineFragment,
    available_width: f32,
    measurer: &dyn TextMeasurer,
) -> Vec<InlineFragment> {
    if let InlineFragment::Text {
        text, style, node, ..
    } = fragment
    {
        let chars: Vec<char> = text.chars().collect();
        let mut pieces = Vec::new();
        let mut start = 0;

        while start < chars.len() {
            let mut end = start + 1;
            // Greedily extend until the piece exceeds available width.
            while end < chars.len() {
                let candidate: String = chars[start..=end].iter().collect();
                let w = measure_word(&candidate, style.font_size, style.letter_spacing, measurer);
                if w > available_width && end > start + 1 {
                    break;
                }
                end += 1;
            }
            // end is now one past the last character that fits.
            let piece_text: String = chars[start..end].iter().collect();
            let piece_width =
                measure_word(&piece_text, style.font_size, style.letter_spacing, measurer);
            pieces.push(InlineFragment::Text {
                text: piece_text,
                x: 0.0,
                width: piece_width,
                style: style.clone(),
                node: *node,
                soft_hyphen: false,
            });
            start = end;
        }

        if pieces.is_empty() {
            // Fallback: return the original fragment.
            pieces.push(fragment.clone());
        }
        pieces
    } else {
        vec![fragment.clone()]
    }
}

// -------------------------------------------------------------------
// Line boundary whitespace trimming (CSS 2.1 §16.6.1)
// -------------------------------------------------------------------

/// Strip leading whitespace from the first fragment and trailing
/// whitespace from the last fragment on a line. This implements the
/// CSS rule that spaces at line boundaries are removed.
fn trim_line_boundary_spaces(line: &mut LineBox, measurer: &dyn TextMeasurer) {
    // Trim leading space from first text fragment.
    if let Some(InlineFragment::Text {
        text, width, style, ..
    }) = line.fragments.first_mut()
        && text.starts_with(' ')
    {
        *text = text[1..].to_string();
        let sw = measure_space(style.font_size, style.word_spacing, measurer);
        *width = (*width - sw).max(0.0);
    }

    // Trim trailing space from last text fragment.
    if let Some(InlineFragment::Text {
        text, width, style, ..
    }) = line.fragments.last_mut()
        && text.ends_with(' ')
    {
        text.pop();
        let sw = measure_space(style.font_size, style.word_spacing, measurer);
        *width = (*width - sw).max(0.0);
    }
}

// -------------------------------------------------------------------
// Line positioning
// -------------------------------------------------------------------

/// Resolve text-align for a given direction context.
///
/// When the direction is RTL and text-align is Left (the CSS initial
/// value), the effective alignment flips to Right (and vice versa).
/// This implements the CSS `start`/`end` mapping without adding new
/// TextAlign variants.
fn resolve_text_align(text_align: TextAlign, direction: TextDirection) -> TextAlign {
    if direction.is_rtl() {
        match text_align {
            TextAlign::Left => TextAlign::Right,
            TextAlign::Right => TextAlign::Left,
            other => other,
        }
    } else {
        text_align
    }
}

/// Position fragments on a line according to the `text-align` property
/// and text `direction`.
fn position_fragments_on_line(
    line: &mut LineBox,
    available_width: f32,
    text_align: TextAlign,
    is_last_line: bool,
    content_x: f32,
    direction: TextDirection,
) {
    let used = line.used_width();
    let extra = (available_width - used).max(0.0);

    // When direction is RTL, reverse the visual order of fragments
    // so they flow right-to-left.
    if direction.is_rtl() {
        line.fragments.reverse();
    }

    let effective_align = resolve_text_align(text_align, direction);

    match effective_align {
        TextAlign::Left => {
            let mut x = content_x;
            for frag in &mut line.fragments {
                set_fragment_x(frag, x);
                x += frag.width();
            }
        },
        TextAlign::Right => {
            let mut x = content_x + extra;
            for frag in &mut line.fragments {
                set_fragment_x(frag, x);
                x += frag.width();
            }
        },
        TextAlign::Center => {
            let mut x = content_x + extra / 2.0;
            for frag in &mut line.fragments {
                set_fragment_x(frag, x);
                x += frag.width();
            }
        },
        TextAlign::Justify => {
            // For RTL justify, the last line is right-aligned.
            let fallback = if direction.is_rtl() {
                TextAlign::Right
            } else {
                TextAlign::Left
            };
            if is_last_line || line.fragments.len() <= 1 {
                let start_x = if fallback == TextAlign::Right {
                    content_x + extra
                } else {
                    content_x
                };
                let mut x = start_x;
                for frag in &mut line.fragments {
                    set_fragment_x(frag, x);
                    x += frag.width();
                }
            } else {
                let gaps = line.fragments.len() - 1;
                let gap_extra = extra / gaps as f32;
                let mut x = content_x;
                for (i, frag) in line.fragments.iter_mut().enumerate() {
                    set_fragment_x(frag, x);
                    x += frag.width();
                    if i < gaps {
                        x += gap_extra;
                    }
                }
            }
        },
    }
}

/// Set the x position on a fragment.
fn set_fragment_x(frag: &mut InlineFragment, x: f32) {
    match frag {
        InlineFragment::Text { x: fx, .. } => *fx = x,
        InlineFragment::InlineBox { layout_box } => {
            // x is the start of the margin box on the line.
            // Content.x must account for margin + border + padding.
            let content_x = x
                + layout_box.dimensions.margin.left
                + layout_box.dimensions.border.left
                + layout_box.dimensions.padding.left;
            let old_x = layout_box.dimensions.content.x;
            let dx = content_x - old_x;
            layout_box.dimensions.content.x = content_x;
            // Offset all descendants by the delta so they stay
            // positioned correctly relative to the InlineBlock.
            if dx.abs() > 0.001 {
                for child in &mut layout_box.children {
                    offset_subtree_x(child, dx);
                }
            }
        },
        InlineFragment::ReplacedInline { x: fx, .. } => *fx = x,
    }
}

/// Recursively offset a layout box and all descendants in the x axis.
fn offset_subtree_x(lb: &mut LayoutBox, dx: f32) {
    lb.dimensions.content.x += dx;
    for child in &mut lb.children {
        offset_subtree_x(child, dx);
    }
}

/// Recursively offset a layout box and all descendants in the y axis.
fn offset_subtree_y(lb: &mut LayoutBox, dy: f32) {
    lb.dimensions.content.y += dy;
    for child in &mut lb.children {
        offset_subtree_y(child, dy);
    }
}

// -------------------------------------------------------------------
// Convert lines back to children
// -------------------------------------------------------------------

/// Flatten line box fragments into layout box children for storage.
///
/// Converts all fragments (text, inline boxes, replaced) into
/// positioned `LayoutBox` children so the paint pass can render
/// text and record link hit regions.
/// Compute the vertical offset for a fragment based on `vertical-align`.
fn align_vertically(va: VerticalAlign, frag_height: f32, line_height: f32) -> f32 {
    match va {
        VerticalAlign::Top | VerticalAlign::TextTop | VerticalAlign::Baseline => 0.0,
        VerticalAlign::Middle => (line_height - frag_height) / 2.0,
        VerticalAlign::Bottom | VerticalAlign::TextBottom => line_height - frag_height,
        VerticalAlign::Sub => line_height * 0.3,
        VerticalAlign::Super => -(line_height * 0.3),
        VerticalAlign::Length(offset) => -offset,
    }
}

fn lines_to_children(lines: Vec<LineBox>, line_positions: &[f32]) -> Vec<LayoutBox> {
    let total_frags: usize = lines.iter().map(|l| l.fragments.len()).sum();
    let mut children = Vec::with_capacity(total_frags);
    for (line, &line_y) in lines.into_iter().zip(line_positions.iter()) {
        let line_height = line.height;
        for frag in line.fragments {
            match frag {
                InlineFragment::Text {
                    text,
                    x,
                    width,
                    style,
                    node,
                    ..
                } => {
                    let mut lb = LayoutBox::new(BoxType::Inline, style.clone(), node);
                    lb.text = Some(text);
                    lb.dimensions.content.x = x;
                    lb.dimensions.content.y = line_y;
                    lb.dimensions.content.width = width;
                    lb.dimensions.content.height = line_height;
                    children.push(lb);
                },
                InlineFragment::InlineBox { mut layout_box } => {
                    let va = layout_box.style.vertical_align;
                    let frag_h = layout_box.dimensions.margin_box().height;
                    let va_offset = align_vertically(va, frag_h, line_height);
                    // Offset the entire InlineBlock subtree to its
                    // final line position. The block was laid out at
                    // y=0; the x was set by position_fragments_on_line.
                    let old_y = layout_box.dimensions.content.y;
                    let dy = line_y + va_offset - old_y;
                    if dy.abs() > 0.001 {
                        offset_subtree_y(&mut layout_box, dy);
                    }
                    children.push(layout_box);
                },
                InlineFragment::ReplacedInline {
                    replaced,
                    x,
                    width,
                    height,
                    style,
                    node,
                } => {
                    let va_offset = align_vertically(style.vertical_align, height, line_height);
                    let mut lb = LayoutBox::new(BoxType::Replaced(replaced), style, node);
                    lb.dimensions.content.x = x;
                    lb.dimensions.content.y = line_y + va_offset;
                    lb.dimensions.content.width = width;
                    lb.dimensions.content.height = height;
                    children.push(lb);
                },
            }
        }
    }
    children
}

// -------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css::values::{Display, WhiteSpace};

    /// Fixed-width text measurer: each character is 8 pixels wide.
    struct FixedMeasurer;

    impl TextMeasurer for FixedMeasurer {
        fn measure_text(&self, text: &str, font_size: u16) -> u32 {
            oasis_types::backend::bitmap_measure_text(text, font_size)
        }
    }

    fn inline_style() -> ComputedStyle {
        let mut s = ComputedStyle::default();
        s.display = Display::Inline;
        s.font_size = 16.0;
        s.line_height = 20.0;
        s
    }

    fn anon_parent(width: f32) -> LayoutBox {
        let mut s = ComputedStyle::default();
        s.display = Display::Block;
        let mut lb = LayoutBox::new(BoxType::Anonymous, s, None);
        lb.dimensions.content.width = width;
        lb.dimensions.content.x = 0.0;
        lb.dimensions.content.y = 0.0;
        lb
    }

    // -- single line fitting in width ---------------------------------

    #[test]
    fn single_line_text_fits() {
        let m = FixedMeasurer;
        let style = inline_style();
        // "hello world" = 11 chars (with space) * 8 = 88px
        let frags = make_text_fragments("hello world", &style, None, &m);

        let mut parent = anon_parent(480.0);
        // Simulate inline layout by creating line boxes manually.
        let mut line = LineBox::new(480.0);
        for f in &frags {
            assert!(line.try_add(f), "all fragments should fit on one line",);
        }
        assert_eq!(line.fragments.len(), 2); // "hello " + "world"

        // Also test through the full layout path.
        parent.children = frags
            .into_iter()
            .map(|f| match f {
                InlineFragment::Text {
                    text: _,
                    style,
                    node,
                    ..
                } => {
                    let mut lb = LayoutBox::new(BoxType::Inline, style, node);
                    lb.children = Vec::new();
                    lb
                },
                _ => unreachable!(),
            })
            .collect();

        // The parent should have some height after inline layout.
        layout_inline(&mut parent, &m);
        assert!(parent.dimensions.content.height > 0.0);
    }

    // -- line break when text exceeds width ----------------------------

    #[test]
    fn line_break_when_exceeds_width() {
        let m = FixedMeasurer;
        let style = inline_style();

        // Create fragments that exceed 100px width.
        // "hello " = 6*8=48, "world" = 5*8=40 => total 88
        // "more" = 4*8=32 => 88+32=120 > 100
        let frags = make_text_fragments("hello world more", &style, None, &m);

        let mut line1 = LineBox::new(100.0);
        let mut line2 = LineBox::new(100.0);
        let mut current = &mut line1;
        let mut lines_used = 1;

        for f in &frags {
            if !current.try_add(f) {
                current = &mut line2;
                lines_used += 1;
                current.try_add(f);
            }
        }

        assert!(
            lines_used >= 2,
            "should need at least 2 lines, got {lines_used}",
        );
    }

    // -- text alignment ------------------------------------------------

    #[test]
    fn text_align_left() {
        let m = FixedMeasurer;
        let style = inline_style();
        let frags = make_text_fragments("hello", &style, None, &m);

        let mut line = LineBox::new(200.0);
        for f in &frags {
            line.try_add(f);
        }

        position_fragments_on_line(
            &mut line,
            200.0,
            TextAlign::Left,
            false,
            0.0,
            TextDirection::Ltr,
        );

        if let InlineFragment::Text { x, .. } = &line.fragments[0] {
            assert_eq!(*x, 0.0);
        }
    }

    #[test]
    fn text_align_right() {
        let m = FixedMeasurer;
        let style = inline_style();
        // "hello" = 5*8 = 40px
        let frags = make_text_fragments("hello", &style, None, &m);

        let mut line = LineBox::new(200.0);
        for f in &frags {
            line.try_add(f);
        }

        position_fragments_on_line(
            &mut line,
            200.0,
            TextAlign::Right,
            false,
            0.0,
            TextDirection::Ltr,
        );

        if let InlineFragment::Text { x, .. } = &line.fragments[0] {
            // Right-aligned: x = 200 - 62 = 138 (proportional "hello"@16 = 31*2 = 62)
            assert_eq!(*x, 138.0);
        }
    }

    #[test]
    fn text_align_center() {
        let m = FixedMeasurer;
        let style = inline_style();
        // Proportional "hello"@16 = 31*2 = 62px
        let frags = make_text_fragments("hello", &style, None, &m);

        let mut line = LineBox::new(200.0);
        for f in &frags {
            line.try_add(f);
        }

        position_fragments_on_line(
            &mut line,
            200.0,
            TextAlign::Center,
            false,
            0.0,
            TextDirection::Ltr,
        );

        if let InlineFragment::Text { x, .. } = &line.fragments[0] {
            // Centered: x = (200 - 62) / 2 = 69
            assert_eq!(*x, 69.0);
        }
    }

    // -- white-space: nowrap prevents breaks --------------------------

    #[test]
    fn nowrap_prevents_breaks() {
        let m = FixedMeasurer;
        let mut style = inline_style();
        style.white_space = WhiteSpace::NoWrap;

        // Create a long text that would normally wrap.
        let frags = make_text_fragments(
            "this is a very long line that should not wrap",
            &style,
            None,
            &m,
        );

        // With nowrap, words still get split but the entire text is
        // measured. The key behavior is that *all* words are split
        // normally by split_into_words (NoWrap still collapses
        // whitespace and splits on spaces), but the layout should be
        // told not to break. In practice, the caller checks
        // white_space == NoWrap and does not break lines.
        //
        // For this test, verify that the collapsed text is one line's
        // worth: all fragments should fit on one LineBox even if they
        // exceed the width (the first fragment always fits, and NoWrap
        // semantics means we keep adding).

        let total_width: f32 = frags.iter().map(|f| f.width()).sum();
        // The full text is 46 chars * 8 = 368 px (with spaces).
        assert!(total_width > 100.0, "text should exceed a narrow container",);

        // Verify words were produced (whitespace still collapses).
        assert!(frags.len() > 1, "should have multiple word fragments",);
    }

    // -- nowrap via layout_inline -------------------------------------

    #[test]
    fn nowrap_layout_produces_single_line() {
        let m = FixedMeasurer;
        let mut style = ComputedStyle::default();
        style.display = Display::Block;
        style.white_space = WhiteSpace::NoWrap;
        style.font_size = 16.0;
        style.line_height = 20.0;

        let mut parent = LayoutBox::new(BoxType::Anonymous, style.clone(), None);
        parent.dimensions.content.width = 50.0; // very narrow
        parent.dimensions.content.x = 0.0;
        parent.dimensions.content.y = 0.0;

        // Create inline children with text that exceeds 50px.
        let mut child = LayoutBox::new(BoxType::Inline, style, None);
        child.text = Some("hello world test".to_string());
        parent.children = vec![child];

        layout_inline(&mut parent, &m);

        // With nowrap, content height should be a single line height.
        assert!(
            parent.dimensions.content.height <= 20.0 + 0.01,
            "nowrap should produce single line, got height {}",
            parent.dimensions.content.height,
        );
    }

    // -- <br> causes line break ---------------------------------------

    #[test]
    fn br_causes_line_break() {
        let m = FixedMeasurer;
        let mut style = inline_style();
        style.display = Display::Block;

        let mut parent = LayoutBox::new(BoxType::Anonymous, style.clone(), None);
        parent.dimensions.content.width = 480.0;
        parent.dimensions.content.x = 0.0;
        parent.dimensions.content.y = 0.0;

        // "hello" inline, then <br>, then "world" inline.
        let mut hello = LayoutBox::new(BoxType::Inline, style.clone(), None);
        hello.text = Some("hello".to_string());

        let br = LayoutBox::new(
            BoxType::Replaced(ReplacedContent::LineBreak),
            style.clone(),
            None,
        );

        let mut world = LayoutBox::new(BoxType::Inline, style, None);
        world.text = Some("world".to_string());

        parent.children = vec![hello, br, world];

        layout_inline(&mut parent, &m);

        // Should produce 2 lines worth of height.
        assert!(
            parent.dimensions.content.height > 20.0,
            "br should cause two lines, got height {}",
            parent.dimensions.content.height,
        );
    }

    // -- text-indent on first line ------------------------------------

    #[test]
    fn text_indent_applied_to_first_line() {
        let m = FixedMeasurer;
        let mut style = inline_style();
        style.display = Display::Block;
        style.text_indent = 20.0;

        let mut parent = LayoutBox::new(BoxType::Anonymous, style.clone(), None);
        parent.dimensions.content.width = 480.0;
        parent.dimensions.content.x = 0.0;
        parent.dimensions.content.y = 0.0;

        let mut child = LayoutBox::new(BoxType::Inline, style, None);
        child.text = Some("hello".to_string());
        parent.children = vec![child];

        layout_inline(&mut parent, &m);

        // The first (and only) text child should be indented by 20px.
        assert!(!parent.children.is_empty());
        let first_x = parent.children[0].dimensions.content.x;
        assert!(
            (first_x - 20.0).abs() < 0.01,
            "first line should be indented by 20, got {first_x}",
        );
    }

    // -- emergency break includes letter_spacing ----------------------

    #[test]
    fn test_emergency_break_includes_letter_spacing() {
        let m = FixedMeasurer;
        let mut style = inline_style();
        style.letter_spacing = 4.0;

        // "ab" without letter_spacing: measure_word gives bitmap width.
        // With letter_spacing=4, measure_word adds 4*(2-1) = 4 extra px.
        let frag = InlineFragment::Text {
            text: "abcd".to_string(),
            x: 0.0,
            width: 999.0, // will be recomputed by break_word_fragment
            style: style.clone(),
            node: None,
            soft_hyphen: false,
        };

        // Break into pieces at a narrow width that forces splitting.
        let pieces = break_word_fragment(&frag, 30.0, &m);
        assert!(pieces.len() > 1, "should have broken word into pieces");

        // Each piece width should include letter_spacing via measure_word.
        for piece in &pieces {
            if let InlineFragment::Text { text, width, .. } = piece {
                let expected = super::super::text::measure_word(
                    text,
                    style.font_size,
                    style.letter_spacing,
                    &m,
                );
                assert!(
                    (*width - expected).abs() < 0.01,
                    "piece '{text}' width {width} should match measure_word {expected}",
                );
            }
        }
    }

    // -- replaced element CSS dimensions override intrinsic -----------

    #[test]
    fn test_image_css_dimensions_override_intrinsic() {
        use crate::css::values::Dimension;

        let m = FixedMeasurer;
        let mut style = inline_style();
        style.width = Dimension::Px(50.0);
        style.height = Dimension::Px(30.0);

        let replaced = ReplacedContent::Image {
            width: 100,
            height: 80,
            texture: None,
            alt: String::new(),
            atlas_region: None,
        };

        let child = LayoutBox::new(BoxType::Replaced(replaced), style, None);
        let frags = collect_inline_fragments(&[child], 480.0, &m);

        assert_eq!(frags.len(), 1);
        assert!(
            matches!(&&frags[0], InlineFragment::ReplacedInline { .. }),
            "expected ReplacedInline fragment"
        );
        let InlineFragment::ReplacedInline { width, height, .. } = &&frags[0] else {
            unreachable!()
        };
        assert!(
            (*width - 50.0).abs() < 0.01,
            "CSS width should override intrinsic, got {width}",
        );
        assert!(
            (*height - 30.0).abs() < 0.01,
            "CSS height should override intrinsic, got {height}",
        );
    }

    #[test]
    fn test_image_css_width_preserves_aspect_ratio() {
        use crate::css::values::Dimension;

        let m = FixedMeasurer;
        let mut style = inline_style();
        style.width = Dimension::Px(50.0);
        // height stays Auto

        let replaced = ReplacedContent::Image {
            width: 100,
            height: 80,
            texture: None,
            alt: String::new(),
            atlas_region: None,
        };

        let child = LayoutBox::new(BoxType::Replaced(replaced), style, None);
        let frags = collect_inline_fragments(&[child], 480.0, &m);

        if let InlineFragment::ReplacedInline { width, height, .. } = &frags[0] {
            assert!((*width - 50.0).abs() < 0.01);
            // 50 * (80/100) = 40
            assert!(
                (*height - 40.0).abs() < 0.01,
                "height should preserve aspect ratio: expected 40, got {height}",
            );
        }
    }

    // ---------------------------------------------------------------
    // word-break: break-all
    // ---------------------------------------------------------------

    #[test]
    fn word_break_break_all_splits_long_word() {
        let m = FixedMeasurer;
        let mut style = inline_style();
        style.display = Display::Block;
        style.word_break = WordBreak::BreakAll;

        let mut parent = LayoutBox::new(BoxType::Anonymous, style.clone(), None);
        parent.dimensions.content.width = 40.0; // very narrow
        parent.dimensions.content.x = 0.0;
        parent.dimensions.content.y = 0.0;

        let mut child = LayoutBox::new(BoxType::Inline, style, None);
        child.text = Some("abcdefghijklmnop".to_string());
        parent.children = vec![child];

        layout_inline(&mut parent, &m);

        // With break-all, the long word should break into multiple lines.
        // At ~8px per char with 40px width, ~5 chars per line.
        // 16 chars -> at least 2 lines.
        assert!(
            parent.dimensions.content.height > 20.0,
            "break-all should produce multiple lines, got height {}",
            parent.dimensions.content.height,
        );
    }

    // ---------------------------------------------------------------
    // overflow-wrap: break-word
    // ---------------------------------------------------------------

    #[test]
    fn overflow_wrap_break_word() {
        let m = FixedMeasurer;
        let mut style = inline_style();
        style.display = Display::Block;
        style.overflow_wrap = OverflowWrap::BreakWord;

        let mut parent = LayoutBox::new(BoxType::Anonymous, style.clone(), None);
        // 80px wide -- enough for "ab " but not "ab " + the long word.
        parent.dimensions.content.width = 80.0;
        parent.dimensions.content.x = 0.0;
        parent.dimensions.content.y = 0.0;

        // "ab superlongwordthatdoesnotfit" -- the first word fits,
        // but the second doesn't and must be broken with break-word.
        let mut child = LayoutBox::new(BoxType::Inline, style, None);
        child.text = Some("ab superlongwordthatdoesnotfit".to_string());
        parent.children = vec![child];

        layout_inline(&mut parent, &m);

        // overflow-wrap: break-word should break the long word across
        // lines, producing more than one line of height.
        assert!(
            parent.dimensions.content.height > 20.0,
            "break-word should produce multiple lines, got height {}",
            parent.dimensions.content.height,
        );
    }

    // ---------------------------------------------------------------
    // white-space: pre (preserve whitespace and newlines)
    // ---------------------------------------------------------------

    #[test]
    fn whitespace_pre_preserves_spaces() {
        let m = FixedMeasurer;
        let mut style = inline_style();
        style.display = Display::Block;
        style.white_space = WhiteSpace::Pre;

        let mut parent = LayoutBox::new(BoxType::Anonymous, style.clone(), None);
        parent.dimensions.content.width = 480.0;
        parent.dimensions.content.x = 0.0;
        parent.dimensions.content.y = 0.0;

        let mut child = LayoutBox::new(BoxType::Inline, style, None);
        // Pre mode should preserve the two spaces and the newline.
        child.text = Some("hello  world\nnext line".to_string());
        parent.children = vec![child];

        layout_inline(&mut parent, &m);

        // The newline in pre mode should create at least 2 lines.
        assert!(
            parent.dimensions.content.height > 20.0,
            "white-space:pre should produce multiple lines for \\n, \
             got height {}",
            parent.dimensions.content.height,
        );
    }

    // ---------------------------------------------------------------
    // white-space: nowrap (no wrapping) -- already tested above,
    // but add a more specific test
    // ---------------------------------------------------------------

    #[test]
    fn whitespace_nowrap_single_line_even_when_overflow() {
        let m = FixedMeasurer;
        let mut style = inline_style();
        style.display = Display::Block;
        style.white_space = WhiteSpace::NoWrap;

        let mut parent = LayoutBox::new(BoxType::Anonymous, style.clone(), None);
        parent.dimensions.content.width = 30.0; // extremely narrow
        parent.dimensions.content.x = 0.0;
        parent.dimensions.content.y = 0.0;

        let mut child = LayoutBox::new(BoxType::Inline, style, None);
        child.text = Some("this text is way too long".to_string());
        parent.children = vec![child];

        layout_inline(&mut parent, &m);

        // With nowrap, should remain a single line height.
        assert!(
            parent.dimensions.content.height <= 20.0 + 0.01,
            "nowrap should produce single line, got height {}",
            parent.dimensions.content.height,
        );
    }

    // ---------------------------------------------------------------
    // Mixed inline elements (bold + normal)
    // ---------------------------------------------------------------

    #[test]
    fn mixed_inline_elements() {
        let m = FixedMeasurer;
        let style = inline_style();

        let mut parent = anon_parent(480.0);

        // Two inline children with different font sizes (simulating
        // bold vs normal -- different line heights).
        let mut normal = LayoutBox::new(BoxType::Inline, style.clone(), None);
        normal.text = Some("hello ".to_string());

        let mut bold_style = style.clone();
        bold_style.font_size = 20.0;
        bold_style.line_height = 24.0;
        let mut bold = LayoutBox::new(BoxType::Inline, bold_style, None);
        bold.text = Some("world".to_string());

        parent.children = vec![normal, bold];

        layout_inline(&mut parent, &m);

        // Both fragments should contribute to the line height.
        assert!(
            parent.dimensions.content.height > 0.0,
            "mixed inline elements should produce positive height"
        );
    }

    // ---------------------------------------------------------------
    // text-align: center and right via layout_inline
    // ---------------------------------------------------------------

    #[test]
    fn text_align_center_via_layout() {
        let m = FixedMeasurer;
        let mut style = inline_style();
        style.display = Display::Block;
        style.text_align = TextAlign::Center;

        let mut parent = LayoutBox::new(BoxType::Anonymous, style.clone(), None);
        parent.dimensions.content.width = 200.0;
        parent.dimensions.content.x = 0.0;
        parent.dimensions.content.y = 0.0;

        let mut child = LayoutBox::new(BoxType::Inline, style, None);
        child.text = Some("hi".to_string());
        parent.children = vec![child];

        layout_inline(&mut parent, &m);

        // The text fragment should be offset toward center.
        if !parent.children.is_empty() {
            let x = parent.children[0].dimensions.content.x;
            // "hi" at font_size 16 ~ 16px wide. center of 200 = 92.
            assert!(x > 0.0, "center-aligned text should have x > 0, got {x}");
        }
    }

    #[test]
    fn text_align_right_via_layout() {
        let m = FixedMeasurer;
        let mut style = inline_style();
        style.display = Display::Block;
        style.text_align = TextAlign::Right;

        let mut parent = LayoutBox::new(BoxType::Anonymous, style.clone(), None);
        parent.dimensions.content.width = 200.0;
        parent.dimensions.content.x = 0.0;
        parent.dimensions.content.y = 0.0;

        let mut child = LayoutBox::new(BoxType::Inline, style, None);
        child.text = Some("hi".to_string());
        parent.children = vec![child];

        layout_inline(&mut parent, &m);

        // The text fragment should be offset to the right.
        if !parent.children.is_empty() {
            let x = parent.children[0].dimensions.content.x;
            assert!(x > 100.0, "right-aligned text should have x > 100, got {x}");
        }
    }
}
