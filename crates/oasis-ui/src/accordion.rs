//! Accordion widget: collapsible sections with single or multi-expand modes.

use crate::context::DrawContext;
use crate::layout;
use crate::widget::Widget;
use oasis_types::error::Result;

/// Controls how many sections can be expanded simultaneously.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccordionMode {
    /// Only one section can be open at a time. Expanding a section collapses
    /// all others.
    SingleExpand,
    /// Multiple sections can be open simultaneously.
    MultiExpand,
}

/// A single collapsible section within an accordion.
#[derive(Debug, Clone)]
pub struct Section {
    /// Header text displayed on the section bar.
    pub header: String,
    /// Content lines displayed when the section is expanded.
    pub content_lines: Vec<String>,
    /// Whether this section is currently expanded.
    pub expanded: bool,
    /// Whether this section is disabled (cannot be toggled).
    pub disabled: bool,
}

impl Section {
    /// Create a new section with the given header and content lines.
    pub fn new(header: impl Into<String>, content_lines: Vec<String>) -> Self {
        Self {
            header: header.into(),
            content_lines,
            expanded: false,
            disabled: false,
        }
    }
}

/// An accordion widget with collapsible sections.
///
/// Each section has a clickable header bar and expandable content lines.
/// The accordion supports single-expand mode (only one open at a time) or
/// multi-expand mode (any number can be open).
pub struct Accordion {
    /// The sections in this accordion.
    pub sections: Vec<Section>,
    /// Expansion mode.
    pub mode: AccordionMode,
    /// Index of the currently focused section header.
    pub focused_index: usize,
    /// Height of each section header bar in pixels.
    pub header_height: u16,
    /// Height of each content line in pixels.
    pub line_height: u16,
}

/// Horizontal padding inside section headers and content areas.
const CONTENT_PAD_X: i32 = 8;

/// Width reserved for the expand/collapse indicator.
const INDICATOR_WIDTH: u32 = 16;

impl Accordion {
    /// Create a new accordion with the given expansion mode.
    pub fn new(mode: AccordionMode) -> Self {
        Self {
            sections: Vec::new(),
            mode,
            focused_index: 0,
            header_height: 24,
            line_height: 16,
        }
    }

    /// Add a section with the given header and content lines.
    pub fn add_section(&mut self, header: impl Into<String>, content_lines: Vec<String>) {
        self.sections.push(Section::new(header, content_lines));
    }

    /// Toggle the expanded state of a section at the given index.
    ///
    /// In `SingleExpand` mode, expanding a section collapses all others.
    /// Disabled sections are not toggled.
    pub fn toggle(&mut self, index: usize) {
        if index >= self.sections.len() || self.sections[index].disabled {
            return;
        }
        let new_state = !self.sections[index].expanded;
        if new_state && self.mode == AccordionMode::SingleExpand {
            for s in &mut self.sections {
                s.expanded = false;
            }
        }
        self.sections[index].expanded = new_state;
    }

    /// Expand the section at the given index.
    ///
    /// In `SingleExpand` mode, all other sections are collapsed first.
    /// Disabled sections are not modified.
    pub fn expand(&mut self, index: usize) {
        if index >= self.sections.len() || self.sections[index].disabled {
            return;
        }
        if self.mode == AccordionMode::SingleExpand {
            for s in &mut self.sections {
                s.expanded = false;
            }
        }
        self.sections[index].expanded = true;
    }

    /// Collapse the section at the given index.
    ///
    /// Disabled sections are not modified.
    pub fn collapse(&mut self, index: usize) {
        if index >= self.sections.len() || self.sections[index].disabled {
            return;
        }
        self.sections[index].expanded = false;
    }

    /// Expand all non-disabled sections.
    ///
    /// In `SingleExpand` mode only the last non-disabled section ends up
    /// expanded since each expansion collapses the others.
    pub fn expand_all(&mut self) {
        if self.mode == AccordionMode::SingleExpand {
            // In single-expand mode, only the last non-disabled section stays open.
            let last = self.sections.iter().rposition(|s| !s.disabled);
            for (i, s) in self.sections.iter_mut().enumerate() {
                s.expanded = last == Some(i) && !s.disabled;
            }
        } else {
            for s in &mut self.sections {
                if !s.disabled {
                    s.expanded = true;
                }
            }
        }
    }

    /// Collapse all sections (including disabled ones that happen to be open).
    pub fn collapse_all(&mut self) {
        for s in &mut self.sections {
            s.expanded = false;
        }
    }

    /// Move focus to the previous non-disabled section header.
    pub fn navigate_up(&mut self) {
        if self.sections.is_empty() {
            return;
        }
        let start = self.focused_index;
        loop {
            self.focused_index = if self.focused_index == 0 {
                self.sections.len() - 1
            } else {
                self.focused_index - 1
            };
            if !self.sections[self.focused_index].disabled || self.focused_index == start {
                break;
            }
        }
    }

    /// Move focus to the next non-disabled section header.
    pub fn navigate_down(&mut self) {
        if self.sections.is_empty() {
            return;
        }
        let start = self.focused_index;
        loop {
            self.focused_index = (self.focused_index + 1) % self.sections.len();
            if !self.sections[self.focused_index].disabled || self.focused_index == start {
                break;
            }
        }
    }

    /// Toggle the currently focused section.
    pub fn activate(&mut self) {
        let idx = self.focused_index;
        self.toggle(idx);
    }

    /// Return the index of the currently focused section.
    pub fn focused(&self) -> usize {
        self.focused_index
    }

    /// Check whether the section at the given index is expanded.
    pub fn is_expanded(&self, index: usize) -> bool {
        self.sections.get(index).is_some_and(|s| s.expanded)
    }

    /// Return the number of sections.
    pub fn section_count(&self) -> usize {
        self.sections.len()
    }

    /// Compute the total height needed for a section (header + expanded content).
    fn section_height(&self, section: &Section) -> u32 {
        let mut h = self.header_height as u32;
        if section.expanded {
            h += section.content_lines.len() as u32 * self.line_height as u32;
        }
        h
    }
}

impl Widget for Accordion {
    fn measure(&self, _ctx: &DrawContext<'_>, available_w: u32, _available_h: u32) -> (u32, u32) {
        let total_h: u32 = self.sections.iter().map(|s| self.section_height(s)).sum();
        (available_w, total_h)
    }

    fn draw(&self, ctx: &mut DrawContext<'_>, x: i32, y: i32, w: u32, _h: u32) -> Result<()> {
        let fs = ctx.theme.font_size_md;
        let mut cur_y = y;

        for (i, section) in self.sections.iter().enumerate() {
            let hh = self.header_height as u32;

            // -- Header background --
            let header_bg = if i == self.focused_index && !section.disabled {
                ctx.theme.accent_subtle
            } else {
                ctx.theme.surface
            };
            ctx.backend.fill_rect(x, cur_y, w, hh, header_bg)?;

            // -- Top border between sections --
            if i > 0 {
                ctx.backend
                    .draw_line(x, cur_y, x + w as i32, cur_y, 1, ctx.theme.border_subtle)?;
            }

            // -- Expand/collapse indicator --
            let indicator = if section.expanded { "-" } else { "+" };
            let ind_h = ctx.backend.measure_text_height(fs);
            let ind_y = cur_y + layout::center(hh, ind_h);
            let text_color = if section.disabled {
                ctx.theme.text_disabled
            } else {
                ctx.theme.text_primary
            };
            ctx.backend
                .draw_text(indicator, x + CONTENT_PAD_X, ind_y, fs, text_color)?;

            // -- Header text --
            let text_h = ctx.backend.measure_text_height(fs);
            let text_y = cur_y + layout::center(hh, text_h);
            ctx.backend.draw_text(
                &section.header,
                x + CONTENT_PAD_X + INDICATOR_WIDTH as i32,
                text_y,
                fs,
                text_color,
            )?;

            cur_y += hh as i32;

            // -- Expanded content lines --
            if section.expanded {
                let lh = self.line_height as u32;
                for line in &section.content_lines {
                    let line_text_h = ctx.backend.measure_text_height(fs);
                    let ly = cur_y + layout::center(lh, line_text_h);
                    ctx.backend.draw_text(
                        line,
                        x + CONTENT_PAD_X + INDICATOR_WIDTH as i32,
                        ly,
                        fs,
                        ctx.theme.text_secondary,
                    )?;
                    cur_y += lh as i32;
                }
            }
        }

        // Bottom border after the last section.
        if !self.sections.is_empty() {
            ctx.backend
                .draw_line(x, cur_y, x + w as i32, cur_y, 1, ctx.theme.border_subtle)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Unit tests (no backend needed) --

    #[test]
    fn new_defaults() {
        let acc = Accordion::new(AccordionMode::SingleExpand);
        assert_eq!(acc.sections.len(), 0);
        assert_eq!(acc.mode, AccordionMode::SingleExpand);
        assert_eq!(acc.focused_index, 0);
        assert_eq!(acc.header_height, 24);
        assert_eq!(acc.line_height, 16);
    }

    #[test]
    fn add_sections() {
        let mut acc = Accordion::new(AccordionMode::MultiExpand);
        acc.add_section("First", vec!["line1".into()]);
        acc.add_section("Second", vec!["line2a".into(), "line2b".into()]);
        assert_eq!(acc.section_count(), 2);
        assert_eq!(acc.sections[0].header, "First");
        assert_eq!(acc.sections[1].content_lines.len(), 2);
        assert!(!acc.is_expanded(0));
        assert!(!acc.is_expanded(1));
    }

    #[test]
    fn toggle_single_expand() {
        let mut acc = Accordion::new(AccordionMode::SingleExpand);
        acc.add_section("A", vec!["a1".into()]);
        acc.add_section("B", vec!["b1".into()]);
        acc.add_section("C", vec!["c1".into()]);

        acc.toggle(0);
        assert!(acc.is_expanded(0));
        assert!(!acc.is_expanded(1));

        // Expanding another collapses the first.
        acc.toggle(1);
        assert!(!acc.is_expanded(0));
        assert!(acc.is_expanded(1));

        // Collapsing the open one.
        acc.toggle(1);
        assert!(!acc.is_expanded(1));
    }

    #[test]
    fn toggle_multi_expand() {
        let mut acc = Accordion::new(AccordionMode::MultiExpand);
        acc.add_section("A", vec!["a1".into()]);
        acc.add_section("B", vec!["b1".into()]);

        acc.toggle(0);
        acc.toggle(1);
        assert!(acc.is_expanded(0));
        assert!(acc.is_expanded(1));

        acc.toggle(0);
        assert!(!acc.is_expanded(0));
        assert!(acc.is_expanded(1));
    }

    #[test]
    fn expand_and_collapse() {
        let mut acc = Accordion::new(AccordionMode::MultiExpand);
        acc.add_section("A", vec!["a1".into()]);
        acc.add_section("B", vec!["b1".into()]);

        acc.expand(0);
        assert!(acc.is_expanded(0));

        acc.collapse(0);
        assert!(!acc.is_expanded(0));
    }

    #[test]
    fn expand_all_multi() {
        let mut acc = Accordion::new(AccordionMode::MultiExpand);
        acc.add_section("A", vec!["a1".into()]);
        acc.add_section("B", vec!["b1".into()]);
        acc.add_section("C", vec!["c1".into()]);

        acc.expand_all();
        assert!(acc.is_expanded(0));
        assert!(acc.is_expanded(1));
        assert!(acc.is_expanded(2));
    }

    #[test]
    fn expand_all_single() {
        let mut acc = Accordion::new(AccordionMode::SingleExpand);
        acc.add_section("A", vec!["a1".into()]);
        acc.add_section("B", vec!["b1".into()]);
        acc.add_section("C", vec!["c1".into()]);

        acc.expand_all();
        // Only the last non-disabled section should be expanded.
        assert!(!acc.is_expanded(0));
        assert!(!acc.is_expanded(1));
        assert!(acc.is_expanded(2));
    }

    #[test]
    fn collapse_all() {
        let mut acc = Accordion::new(AccordionMode::MultiExpand);
        acc.add_section("A", vec!["a1".into()]);
        acc.add_section("B", vec!["b1".into()]);
        acc.expand_all();
        assert!(acc.is_expanded(0));

        acc.collapse_all();
        assert!(!acc.is_expanded(0));
        assert!(!acc.is_expanded(1));
    }

    #[test]
    fn navigation_wraps() {
        let mut acc = Accordion::new(AccordionMode::MultiExpand);
        acc.add_section("A", vec![]);
        acc.add_section("B", vec![]);
        acc.add_section("C", vec![]);
        assert_eq!(acc.focused(), 0);

        acc.navigate_down();
        assert_eq!(acc.focused(), 1);
        acc.navigate_down();
        assert_eq!(acc.focused(), 2);
        acc.navigate_down();
        assert_eq!(acc.focused(), 0); // wraps

        acc.navigate_up();
        assert_eq!(acc.focused(), 2); // wraps back
        acc.navigate_up();
        assert_eq!(acc.focused(), 1);
    }

    #[test]
    fn disabled_section_skipping() {
        let mut acc = Accordion::new(AccordionMode::MultiExpand);
        acc.add_section("A", vec![]);
        acc.add_section("B", vec![]);
        acc.add_section("C", vec![]);
        acc.sections[1].disabled = true;

        // Toggle on disabled section is a no-op.
        acc.toggle(1);
        assert!(!acc.is_expanded(1));

        // Expand on disabled section is a no-op.
        acc.expand(1);
        assert!(!acc.is_expanded(1));

        // Navigation skips disabled sections.
        assert_eq!(acc.focused(), 0);
        acc.navigate_down();
        assert_eq!(acc.focused(), 2); // skips B

        acc.navigate_up();
        assert_eq!(acc.focused(), 0); // skips B going up
    }

    #[test]
    fn activate_toggles_focused() {
        let mut acc = Accordion::new(AccordionMode::MultiExpand);
        acc.add_section("A", vec!["content".into()]);
        acc.add_section("B", vec!["content".into()]);
        acc.focused_index = 1;

        acc.activate();
        assert!(!acc.is_expanded(0));
        assert!(acc.is_expanded(1));

        acc.activate();
        assert!(!acc.is_expanded(1));
    }

    #[test]
    fn out_of_bounds_toggle_is_noop() {
        let mut acc = Accordion::new(AccordionMode::MultiExpand);
        acc.add_section("A", vec![]);
        acc.toggle(99);
        assert!(!acc.is_expanded(99));
    }

    #[test]
    fn is_expanded_out_of_bounds() {
        let acc = Accordion::new(AccordionMode::MultiExpand);
        assert!(!acc.is_expanded(0));
        assert!(!acc.is_expanded(100));
    }

    #[test]
    fn navigation_empty_accordion() {
        let mut acc = Accordion::new(AccordionMode::MultiExpand);
        // Should not panic.
        acc.navigate_up();
        acc.navigate_down();
        assert_eq!(acc.focused(), 0);
    }

    #[test]
    fn expand_all_skips_disabled() {
        let mut acc = Accordion::new(AccordionMode::MultiExpand);
        acc.add_section("A", vec![]);
        acc.add_section("B", vec![]);
        acc.sections[1].disabled = true;

        acc.expand_all();
        assert!(acc.is_expanded(0));
        assert!(!acc.is_expanded(1));
    }

    #[test]
    fn all_disabled_navigation_stays() {
        let mut acc = Accordion::new(AccordionMode::MultiExpand);
        acc.add_section("A", vec![]);
        acc.add_section("B", vec![]);
        acc.sections[0].disabled = true;
        acc.sections[1].disabled = true;

        acc.navigate_down();
        // Should stop at start since all are disabled.
        assert_eq!(acc.focused(), 0);

        acc.navigate_up();
        assert_eq!(acc.focused(), 0);
    }

    // -- Draw / measure tests using MockBackend --

    use crate::context::DrawContext;
    use crate::test_utils::MockBackend;
    use crate::theme::Theme;
    use crate::widget::Widget;

    #[test]
    fn measure_collapsed_sections() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        let ctx = DrawContext::new(&mut backend, &theme);

        let mut acc = Accordion::new(AccordionMode::MultiExpand);
        acc.add_section("A", vec!["line1".into(), "line2".into()]);
        acc.add_section("B", vec!["line3".into()]);

        let (w, h) = acc.measure(&ctx, 200, 400);
        assert_eq!(w, 200);
        // Two collapsed headers: 24 + 24 = 48.
        assert_eq!(h, 48);
    }

    #[test]
    fn measure_expanded_sections() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        let ctx = DrawContext::new(&mut backend, &theme);

        let mut acc = Accordion::new(AccordionMode::MultiExpand);
        acc.add_section("A", vec!["line1".into(), "line2".into()]);
        acc.add_section("B", vec!["line3".into()]);
        acc.expand(0);

        let (w, h) = acc.measure(&ctx, 200, 400);
        assert_eq!(w, 200);
        // Section A: 24 header + 2*16 content = 56.
        // Section B: 24 header (collapsed) = 24.
        assert_eq!(h, 80);
    }

    #[test]
    fn draw_shows_headers() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut acc = Accordion::new(AccordionMode::MultiExpand);
            acc.add_section("Alpha", vec!["content_a".into()]);
            acc.add_section("Beta", vec!["content_b".into()]);
            acc.draw(&mut ctx, 0, 0, 200, 100).unwrap();
        }
        assert!(backend.has_text("Alpha"));
        assert!(backend.has_text("Beta"));
        // Collapsed sections should not show content.
        assert!(!backend.has_text("content_a"));
        assert!(!backend.has_text("content_b"));
    }

    #[test]
    fn draw_expanded_shows_content() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut acc = Accordion::new(AccordionMode::MultiExpand);
            acc.add_section("Alpha", vec!["content_a".into()]);
            acc.add_section("Beta", vec!["content_b".into()]);
            acc.expand(0);
            acc.draw(&mut ctx, 0, 0, 200, 200).unwrap();
        }
        assert!(backend.has_text("Alpha"));
        assert!(backend.has_text("content_a"));
        assert!(backend.has_text("Beta"));
        assert!(!backend.has_text("content_b"));
    }

    #[test]
    fn draw_indicators() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let mut acc = Accordion::new(AccordionMode::MultiExpand);
            acc.add_section("A", vec!["a1".into()]);
            acc.add_section("B", vec!["b1".into()]);
            acc.expand(0);
            acc.draw(&mut ctx, 0, 0, 200, 200).unwrap();
        }
        // Expanded section gets "-", collapsed gets "+".
        assert!(backend.has_text("-"));
        assert!(backend.has_text("+"));
    }

    #[test]
    fn draw_empty_accordion_no_panic() {
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            let acc = Accordion::new(AccordionMode::MultiExpand);
            acc.draw(&mut ctx, 0, 0, 200, 100).unwrap();
        }
        assert_eq!(backend.draw_text_count(), 0);
        assert_eq!(backend.fill_rect_count(), 0);
    }

    #[test]
    fn draw_all_themes_no_panic() {
        crate::test_utils::test_draw_all_themes(|ctx| {
            let mut acc = Accordion::new(AccordionMode::SingleExpand);
            acc.add_section("Section 1", vec!["line a".into(), "line b".into()]);
            acc.add_section("Section 2", vec!["line c".into()]);
            acc.expand(0);
            acc.draw(ctx, 0, 0, 300, 200).unwrap();
        });
    }
}
