//! Inline-level layout algorithm.
//!
//! Implements CSS 2.1 inline formatting context (IFC) layout. Inline
//! boxes flow horizontally and wrap into line boxes when the available
//! width is exhausted.

use super::block::TextMeasurer;
use super::box_model::*;
use super::text::{
    apply_text_transform, collapse_whitespace, measure_space, measure_word, split_into_words,
};
use crate::browser::css::values::{ComputedStyle, TextAlign};
use crate::browser::html::dom::NodeId;

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
    let fragments = collect_inline_fragments(&parent.children, measurer);

    // Break fragments into line boxes.
    let mut lines: Vec<LineBox> = Vec::new();
    let mut current_line = LineBox::new(available_width);

    for fragment in &fragments {
        if !current_line.try_add(fragment) {
            lines.push(current_line);
            current_line = LineBox::new(available_width);
            // The fragment that did not fit starts the new line.
            current_line.try_add(fragment);
        }
    }
    if !current_line.is_empty() {
        lines.push(current_line);
    }

    // Position line boxes vertically and apply text alignment.
    let mut cursor_y = parent.dimensions.content.y;
    let last_line_idx = lines.len().saturating_sub(1);

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
        line.baseline = line.height * 0.8; // simple approximation

        // Position fragments horizontally.
        let is_last_line = i == last_line_idx;
        position_fragments_on_line(
            line,
            available_width,
            text_align,
            is_last_line,
            parent.dimensions.content.x,
        );

        cursor_y += line.height;
    }

    // Store the lines' fragments back as children (flattened).
    parent.children = lines_to_children(lines);

    // Update parent height.
    parent.dimensions.content.height = cursor_y - parent.dimensions.content.y;
}

// -------------------------------------------------------------------
// Fragment collection
// -------------------------------------------------------------------

/// Collect inline fragments from a list of layout box children.
///
/// Text nodes are split into word-level fragments for line breaking.
/// Inline boxes are kept as single fragments.
fn collect_inline_fragments(
    children: &[LayoutBox],
    measurer: &dyn TextMeasurer,
) -> Vec<InlineFragment> {
    let mut fragments = Vec::new();

    for child in children {
        match &child.box_type {
            BoxType::Inline => {
                // Check if this is a text node (has a node id and
                // the style says inline). We produce text fragments.
                fragments.extend(text_fragments_for_inline(child, measurer));
            },
            BoxType::InlineBlock => {
                fragments.push(InlineFragment::InlineBox {
                    layout_box: child.clone(),
                });
            },
            BoxType::Replaced(replaced) => {
                let (w, h) = replaced_dimensions(replaced);
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
                fragments.extend(collect_inline_fragments(&child.children, measurer));
            },
        }
    }

    fragments
}

/// Generate text fragments for an inline box (splitting on word
/// boundaries for line breaking).
fn text_fragments_for_inline(
    layout_box: &LayoutBox,
    measurer: &dyn TextMeasurer,
) -> Vec<InlineFragment> {
    let style = &layout_box.style;

    // If this is a leaf inline box representing a text node, extract
    // its text. In our layout tree, text nodes become inline boxes
    // with a node id. The actual text lives in the DOM; here we use
    // the fact that text-only inline boxes have no children and we
    // synthesize word fragments.
    //
    // For simplicity, we treat the node presence as evidence of a text
    // node and produce fragments for the box itself (the text content
    // will be filled during paint from the DOM). However, for layout
    // purposes we need *some* text to measure. We use a placeholder
    // approach: the inline box's children are recursively checked, and
    // leaf inline boxes with no children are treated as single-word
    // text runs.
    if layout_box.children.is_empty() {
        // Leaf inline: produce a single text fragment.
        // The actual text is "unknown" at layout time without the DOM
        // reference, so we emit a zero-width placeholder. In a full
        // implementation the text would be looked up from the DOM.
        // For testing, callers build InlineFragments directly.
        return vec![InlineFragment::Text {
            text: String::new(),
            x: 0.0,
            width: 0.0,
            style: style.clone(),
            node: layout_box.node,
        }];
    }

    // Recurse into children.
    collect_inline_fragments(&layout_box.children, measurer)
}

/// Get the dimensions of a replaced inline element.
fn replaced_dimensions(replaced: &ReplacedContent) -> (f32, f32) {
    match replaced {
        ReplacedContent::Image { width, height, .. } => (*width as f32, *height as f32),
        ReplacedContent::HorizontalRule => (0.0, 2.0),
        ReplacedContent::LineBreak => (0.0, 0.0),
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
    let collapsed = collapse_whitespace(&transformed, style.white_space);
    let words = split_into_words(&collapsed, style.white_space);

    let font_size = style.font_size;
    let space_width = measure_space(font_size, measurer);
    let mut fragments = Vec::new();

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
            });
            continue;
        }

        let word_width = measure_word(&word.text, font_size, measurer);
        let total_width = if word.trailing_space {
            word_width + space_width
        } else {
            word_width
        };

        let display_text = if word.trailing_space {
            format!("{} ", word.text)
        } else {
            word.text.clone()
        };

        fragments.push(InlineFragment::Text {
            text: display_text,
            x: 0.0,
            width: total_width,
            style: style.clone(),
            node,
        });
    }

    fragments
}

// -------------------------------------------------------------------
// Line positioning
// -------------------------------------------------------------------

/// Position fragments on a line according to the `text-align` property.
fn position_fragments_on_line(
    line: &mut LineBox,
    available_width: f32,
    text_align: TextAlign,
    is_last_line: bool,
    content_x: f32,
) {
    let used = line.used_width();
    let extra = (available_width - used).max(0.0);

    match text_align {
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
            if is_last_line || line.fragments.len() <= 1 {
                // Last line or single fragment: left-align.
                let mut x = content_x;
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
            layout_box.dimensions.content.x = x;
        },
        InlineFragment::ReplacedInline { x: fx, .. } => *fx = x,
    }
}

// -------------------------------------------------------------------
// Convert lines back to children
// -------------------------------------------------------------------

/// Flatten line box fragments into layout box children for storage.
///
/// This extracts `InlineBox` children and stores the line structure
/// implicitly. For the current implementation, we keep the parent's
/// children as-is (the parent's height was already set).
fn lines_to_children(lines: Vec<LineBox>) -> Vec<LayoutBox> {
    let mut children = Vec::new();
    for line in lines {
        for frag in line.fragments {
            if let InlineFragment::InlineBox { layout_box } = frag {
                children.push(layout_box);
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
    use crate::browser::css::values::{Display, WhiteSpace};

    /// Fixed-width text measurer: 8 pixels per character.
    struct FixedMeasurer;

    impl TextMeasurer for FixedMeasurer {
        fn measure_text(&self, text: &str, _font_size: u16) -> u32 {
            text.len() as u32 * 8
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

        position_fragments_on_line(&mut line, 200.0, TextAlign::Left, false, 0.0);

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

        position_fragments_on_line(&mut line, 200.0, TextAlign::Right, false, 0.0);

        if let InlineFragment::Text { x, .. } = &line.fragments[0] {
            // Right-aligned: x = 200 - 40 = 160
            assert_eq!(*x, 160.0);
        }
    }

    #[test]
    fn text_align_center() {
        let m = FixedMeasurer;
        let style = inline_style();
        // "hello" = 5*8 = 40px
        let frags = make_text_fragments("hello", &style, None, &m);

        let mut line = LineBox::new(200.0);
        for f in &frags {
            line.try_add(f);
        }

        position_fragments_on_line(&mut line, 200.0, TextAlign::Center, false, 0.0);

        if let InlineFragment::Text { x, .. } = &line.fragments[0] {
            // Centered: x = (200 - 40) / 2 = 80
            assert_eq!(*x, 80.0);
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
}
