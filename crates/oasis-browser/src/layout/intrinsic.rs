//! Intrinsic (max-content) widths, measured without laying boxes out.
//!
//! A content-sized flex item's base size is its max-content width
//! clamped to the space available (`fit-content`). Measuring that by
//! laying the item out and reading back its extent made every nesting
//! level lay its subtree out once more (and read block children as full
//! width). Here it is computed from the box tree directly and cached per
//! box for the duration of one layout pass, so nested containers query
//! each subtree once: layout cost stays linear in the tree size.
//!
//! A *layout pass* is one outermost layout call (see [`PassGuard`]);
//! cached values from an earlier pass are ignored, so a relayout after
//! a style or content change never sees stale widths.

use std::cell::Cell;

use super::block::{TextMeasurer, fit_content_width, layout_block};
use super::box_model::{BoxType, LayoutBox, ReplacedContent};
use super::inline::{make_text_fragments, replaced_dimensions};
use super::text::{measure_space, web_font_of};
use crate::css::values::{
    BorderStyle, BoxSizing, ComputedStyle, Dimension, FlexDirection, Position, WhiteSpace,
};

thread_local! {
    /// Id of the current layout pass (bumped by each outermost entry).
    static PASS: Cell<u32> = const { Cell::new(0) };
    /// Nesting depth of layout entry points on this thread.
    static DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// Marks a layout entry point. The outermost guard on the thread starts
/// a new layout pass, which invalidates every cached intrinsic width.
pub(crate) struct PassGuard;

impl PassGuard {
    pub(crate) fn enter() -> Self {
        DEPTH.with(|d| {
            if d.get() == 0 {
                PASS.with(|p| p.set(p.get().wrapping_add(1)));
            }
            d.set(d.get() + 1);
        });
        PassGuard
    }
}

impl Drop for PassGuard {
    fn drop(&mut self) {
        DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
    }
}

fn current_pass() -> u32 {
    PASS.with(Cell::get)
}

/// Width tables and grids are laid out at to read back their max-content
/// width (their track algorithms are not duplicated here). Large enough
/// that real content does not wrap; callers clamp the result to the
/// space they have.
const UNCONSTRAINED_WIDTH: f32 = 100_000.0;

/// The max-content width of `b`'s content box: the width it takes when
/// nothing wraps (forced breaks still end lines). Percentages resolve
/// to `0` / `auto`, as they do in intrinsic sizing.
pub(crate) fn max_content_width(b: &mut LayoutBox, measurer: &dyn TextMeasurer) -> f32 {
    let _pass = PassGuard::enter();
    let pass = current_pass();
    if let Some((p, w)) = b.sizing.max_content
        && p == pass
    {
        return w;
    }
    let w = compute_max_content(b, measurer);
    b.sizing.max_content = Some((pass, w));
    w
}

/// `b`'s max-content contribution to its parent: its max-content width
/// plus its own padding, border and (non-auto) margins.
pub(crate) fn max_content_contribution(b: &mut LayoutBox, measurer: &dyn TextMeasurer) -> f32 {
    let content = max_content_width(b, measurer);
    (content + horizontal_edges(&b.style, true)).max(0.0)
}

fn compute_max_content(b: &mut LayoutBox, measurer: &dyn TextMeasurer) -> f32 {
    let declared = match b.style.width {
        Dimension::Px(w) if b.style.box_sizing == BoxSizing::BorderBox => {
            Some((w - horizontal_edges(&b.style, false)).max(0.0))
        },
        Dimension::Px(w) => Some(w.max(0.0)),
        _ => None,
    };
    let content = match declared {
        Some(w) => w,
        None => match &b.box_type {
            BoxType::Replaced(r) => replaced_width(r, &b.style),
            BoxType::Grid | BoxType::TableWrapper => laid_out_fit_width(b, measurer),
            BoxType::Flex => {
                let row = matches!(
                    b.style.flex_direction,
                    FlexDirection::Row | FlexDirection::RowReverse
                );
                let gap = b.style.gap;
                let mut total = 0.0f32;
                let mut widest = 0.0f32;
                let mut n = 0usize;
                for c in in_flow(&mut b.children) {
                    let w = max_content_contribution(c, measurer);
                    total += w;
                    widest = widest.max(w);
                    n += 1;
                }
                if row {
                    total + gap * n.saturating_sub(1) as f32
                } else {
                    widest
                }
            },
            _ => {
                if let Some(text) = &b.text {
                    let mut lines = LineWidths::default();
                    add_text(&mut lines, text, &b.style, measurer);
                    lines.finish()
                } else if b.children.iter().any(LayoutBox::is_block_level) {
                    let mut widest = 0.0f32;
                    for c in in_flow(&mut b.children) {
                        widest = widest.max(max_content_contribution(c, measurer));
                    }
                    widest
                } else {
                    // Measure the original inline content, not the line
                    // fragments an earlier layout left in `children`.
                    let content = match b.inline_source.as_deref_mut() {
                        Some(source) => source,
                        None => &mut b.children,
                    };
                    let mut lines = LineWidths::default();
                    add_inline_children(&mut lines, content, measurer);
                    lines.finish()
                }
            },
        },
    };
    clamp_min_max(content, &b.style)
}

/// In-flow children (absolutely positioned boxes take no space).
fn in_flow(children: &mut [LayoutBox]) -> impl Iterator<Item = &mut LayoutBox> {
    children
        .iter_mut()
        .filter(|c| !matches!(c.style.position, Position::Absolute | Position::Fixed))
}

/// Max-content of a table or grid: lay a copy out unconstrained and read
/// back how wide its content came out.
fn laid_out_fit_width(b: &LayoutBox, measurer: &dyn TextMeasurer) -> f32 {
    // Boxed: a `LayoutBox` is ~1.7 KB, and this frame sits in a walk as
    // deep as the DOM.
    let mut probe = Box::new(b.clone());
    probe.sizing.forced_width = None;
    probe.sizing.forced_height = None;
    probe.dimensions.content.x = 0.0;
    probe.dimensions.content.y = 0.0;
    layout_block(&mut probe, UNCONSTRAINED_WIDTH, measurer);
    // The rightmost row / item edge: how wide the tracks came out.
    fit_content_width(&probe)
}

/// Widest line of an inline formatting context, built up run by run.
#[derive(Default)]
struct LineWidths {
    widest: f32,
    line: f32,
    /// Width of a collapsible space ending the current line (trimmed if
    /// the line ends there).
    trailing_space: f32,
}

impl LineWidths {
    fn push(&mut self, w: f32, trailing_space: f32) {
        self.line += w;
        self.trailing_space = trailing_space;
    }

    fn end_line(&mut self) {
        self.widest = self.widest.max(self.line - self.trailing_space);
        self.line = 0.0;
        self.trailing_space = 0.0;
    }

    fn finish(mut self) -> f32 {
        self.end_line();
        self.widest.max(0.0)
    }
}

fn add_text(
    lines: &mut LineWidths,
    text: &str,
    style: &ComputedStyle,
    measurer: &dyn TextMeasurer,
) {
    let collapsible = !matches!(style.white_space, WhiteSpace::Pre | WhiteSpace::PreWrap);
    let space = measure_space(
        style.font_size,
        style.word_spacing,
        web_font_of(style),
        measurer,
    );
    for frag in make_text_fragments(text, style, None, measurer) {
        let super::box_model::InlineFragment::Text { text, width, .. } = frag else {
            continue;
        };
        if text == "\n" {
            lines.end_line();
            continue;
        }
        // A collapsible space opening a line is removed by layout.
        let lead = if collapsible && lines.line == 0.0 && text.starts_with(' ') {
            space
        } else {
            0.0
        };
        if lead > 0.0 && text.trim().is_empty() {
            continue;
        }
        let trail = if collapsible && text.ends_with(' ') {
            space
        } else {
            0.0
        };
        lines.push((width - lead).max(0.0), trail);
    }
}

fn add_inline_children(
    lines: &mut LineWidths,
    children: &mut [LayoutBox],
    measurer: &dyn TextMeasurer,
) {
    for c in children {
        match &c.box_type {
            BoxType::Inline => {
                if let Some(text) = &c.text {
                    add_text(lines, text, &c.style, measurer);
                } else {
                    // Inline layout adds an inline box's positive
                    // horizontal margins around its content (not its
                    // padding); negative ones are not subtracted.
                    let ml = if c.style.margin_left_auto {
                        0.0
                    } else {
                        c.style.margin_left.max(0.0)
                    };
                    let mr = if c.style.margin_right_auto {
                        0.0
                    } else {
                        c.style.margin_right.max(0.0)
                    };
                    lines.push(ml, 0.0);
                    add_inline_children(lines, &mut c.children, measurer);
                    lines.push(mr, if mr > 0.0 { 0.0 } else { lines.trailing_space });
                }
            },
            BoxType::Replaced(ReplacedContent::LineBreak) => lines.end_line(),
            BoxType::Replaced(r) => {
                let w = replaced_width(r, &c.style);
                lines.push(w, 0.0);
            },
            BoxType::InlineBlock => {
                let w = max_content_contribution(c, measurer);
                lines.push(w, 0.0);
            },
            _ => {
                // A block inside inline content sits on its own line.
                lines.end_line();
                let w = max_content_contribution(c, measurer);
                lines.push(w, 0.0);
                lines.end_line();
            },
        }
    }
}

/// Used width of a replaced element (as inline layout sizes it).
fn replaced_width(r: &ReplacedContent, style: &ComputedStyle) -> f32 {
    let (iw, ih) = replaced_dimensions(r, style);
    let ratio = match style.aspect_ratio {
        Some(r) if r > 0.0 => r,
        _ if iw > 0.0 && ih > 0.0 => iw / ih,
        _ => 0.0,
    };
    match (style.width, style.height) {
        (Dimension::Px(w), _) => w,
        (Dimension::Auto, Dimension::Px(h)) if ratio > 0.0 => h * ratio,
        _ => iw,
    }
}

/// Padding + border (+ non-auto margins when `margins`) on the inline
/// axis, from the style; percentages count as `0`.
fn horizontal_edges(s: &ComputedStyle, margins: bool) -> f32 {
    let pct = |pct: Option<f32>, v: f32| if pct.is_some() { 0.0 } else { v };
    let border = |style: BorderStyle, w: f32| if style == BorderStyle::None { 0.0 } else { w };
    let mut e = pct(s.padding_left_pct, s.padding_left)
        + pct(s.padding_right_pct, s.padding_right)
        + border(s.border_left_style, s.border_left_width)
        + border(s.border_right_style, s.border_right_width);
    if margins {
        if !s.margin_left_auto {
            e += pct(s.margin_left_pct, s.margin_left);
        }
        if !s.margin_right_auto {
            e += pct(s.margin_right_pct, s.margin_right);
        }
    }
    e
}

fn clamp_min_max(w: f32, s: &ComputedStyle) -> f32 {
    let mut w = w;
    if let Dimension::Px(max) = s.max_width {
        w = w.min(max);
    }
    if let Dimension::Px(min) = s.min_width {
        w = w.max(min);
    }
    w.max(0.0)
}
