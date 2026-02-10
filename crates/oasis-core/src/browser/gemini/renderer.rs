//! Render text/gemini documents into layout boxes.
//!
//! Converts a parsed [`GeminiDocument`] directly into a layout tree,
//! bypassing the HTML/CSS parsing pipeline. Each Gemini line type maps
//! to a block-level [`LayoutBox`] with inline styling applied via
//! [`ComputedStyle`].

#![allow(clippy::field_reassign_with_default)]

use crate::backend::Color;
use crate::browser::css::values::{
    BorderStyle, ComputedStyle, Display, FontFamily, FontStyle, FontWeight, TextDecoration,
    WhiteSpace,
};
use crate::browser::layout::block::TextMeasurer;
use crate::browser::layout::box_model::{BoxType, EdgeSizes, LayoutBox, ListMarker};

use super::parser::{GeminiDocument, GeminiLine};

/// Default colors for Gemini rendering.
pub struct GeminiTheme {
    /// Color for regular body text.
    pub text_color: Color,
    /// Color for link text.
    pub link_color: Color,
    /// Color for heading text.
    pub heading_color: Color,
    /// Color for blockquote text.
    pub quote_color: Color,
    /// Color for blockquote left border.
    pub quote_border: Color,
    /// Background color for preformatted blocks.
    pub pre_background: Color,
    /// Page background color.
    pub background: Color,
    /// Base font size in pixels.
    pub font_size: f32,
}

impl Default for GeminiTheme {
    fn default() -> Self {
        Self {
            text_color: Color::rgb(33, 33, 33),
            link_color: Color::rgb(0, 102, 204),
            heading_color: Color::rgb(0, 0, 0),
            quote_color: Color::rgb(100, 100, 100),
            quote_border: Color::rgb(180, 180, 180),
            pre_background: Color::rgb(240, 240, 240),
            background: Color::rgb(255, 255, 255),
            font_size: 8.0,
        }
    }
}

/// Render a Gemini document into a layout tree.
///
/// Returns a root [`LayoutBox`] containing one child per Gemini line.
/// The tree is positioned within the given `viewport_width` and uses
/// the `measurer` for text width calculations.
pub fn render_gemini(
    doc: &GeminiDocument,
    viewport_width: f32,
    theme: &GeminiTheme,
    measurer: &dyn TextMeasurer,
) -> LayoutBox {
    let margin = 8.0;
    let content_width = viewport_width - margin * 2.0;

    let root_style = ComputedStyle {
        display: Display::Block,
        background_color: theme.background,
        color: theme.text_color,
        font_size: theme.font_size,
        ..ComputedStyle::default()
    };

    let mut root = LayoutBox::new(BoxType::Block, root_style, None);
    root.dimensions.content.x = margin;
    root.dimensions.content.y = margin;
    root.dimensions.content.width = content_width;
    root.dimensions.padding = EdgeSizes {
        top: margin,
        right: margin,
        bottom: margin,
        left: margin,
    };

    let mut cursor_y = margin;
    let line_height = theme.font_size * 1.5;

    for gemini_line in &doc.lines {
        let child = render_gemini_line(
            gemini_line,
            content_width,
            &mut cursor_y,
            line_height,
            theme,
            measurer,
        );
        if let Some(child) = child {
            root.children.push(child);
        }
    }

    root.dimensions.content.height = cursor_y;
    root
}

fn render_gemini_line(
    line: &GeminiLine,
    content_width: f32,
    cursor_y: &mut f32,
    line_height: f32,
    theme: &GeminiTheme,
    measurer: &dyn TextMeasurer,
) -> Option<LayoutBox> {
    match line {
        GeminiLine::Text(text) => {
            let mut style = ComputedStyle::default();
            style.display = Display::Block;
            style.color = theme.text_color;
            style.font_size = theme.font_size;
            style.line_height = line_height;
            style.margin_bottom = line_height * 0.5;

            let height = calculate_wrapped_height(text, content_width, theme.font_size, measurer);

            let mut b = LayoutBox::new(BoxType::Block, style, None);
            b.dimensions.content.x = 0.0;
            b.dimensions.content.y = *cursor_y;
            b.dimensions.content.width = content_width;
            b.dimensions.content.height = height;

            // Store text as an inline child (anonymous text box).
            let mut text_style = ComputedStyle::default();
            text_style.color = theme.text_color;
            text_style.font_size = theme.font_size;
            let mut text_box = LayoutBox::new(BoxType::Inline, text_style, None);
            text_box.dimensions.content.x = 0.0;
            text_box.dimensions.content.y = *cursor_y;
            text_box.dimensions.content.width = content_width;
            text_box.dimensions.content.height = height;
            b.children.push(text_box);

            *cursor_y += height + line_height * 0.5;
            Some(b)
        },

        GeminiLine::Link { url, display } => {
            let _text = display.as_deref().unwrap_or(url.as_str());
            let mut style = ComputedStyle::default();
            style.display = Display::Block;
            style.color = theme.link_color;
            style.font_size = theme.font_size;
            style.text_decoration = TextDecoration::Underline;
            style.margin_bottom = line_height * 0.3;

            let height = line_height;
            let mut b = LayoutBox::new(BoxType::Block, style, None);
            b.dimensions.content.x = 0.0;
            b.dimensions.content.y = *cursor_y;
            b.dimensions.content.width = content_width;
            b.dimensions.content.height = height;

            *cursor_y += height + line_height * 0.3;
            Some(b)
        },

        GeminiLine::Heading1(text) => {
            render_heading(text, 2.0, content_width, cursor_y, line_height, theme)
        },
        GeminiLine::Heading2(text) => {
            render_heading(text, 1.5, content_width, cursor_y, line_height, theme)
        },
        GeminiLine::Heading3(text) => {
            render_heading(text, 1.17, content_width, cursor_y, line_height, theme)
        },

        GeminiLine::ListItem(_text) => {
            let mut style = ComputedStyle::default();
            style.display = Display::Block;
            style.color = theme.text_color;
            style.font_size = theme.font_size;
            style.margin_bottom = line_height * 0.2;
            style.padding_left = 20.0;

            let mut b = LayoutBox::new(
                BoxType::ListItem {
                    marker: ListMarker::Disc,
                },
                style,
                None,
            );
            b.dimensions.content.x = 20.0;
            b.dimensions.content.y = *cursor_y;
            b.dimensions.content.width = content_width - 20.0;
            b.dimensions.content.height = line_height;

            *cursor_y += line_height + line_height * 0.2;
            Some(b)
        },

        GeminiLine::Quote(text) => {
            let mut style = ComputedStyle::default();
            style.display = Display::Block;
            style.color = theme.quote_color;
            style.font_size = theme.font_size;
            style.font_style = FontStyle::Italic;
            style.border_left_width = 3.0;
            style.border_left_color = theme.quote_border;
            style.border_left_style = BorderStyle::Solid;
            style.padding_left = 10.0;
            style.margin_bottom = line_height * 0.3;

            let height =
                calculate_wrapped_height(text, content_width - 13.0, theme.font_size, measurer);

            let mut b = LayoutBox::new(BoxType::Block, style, None);
            b.dimensions.content.x = 13.0;
            b.dimensions.content.y = *cursor_y;
            b.dimensions.content.width = content_width - 13.0;
            b.dimensions.content.height = height;
            b.dimensions.border.left = 3.0;
            b.dimensions.padding.left = 10.0;

            *cursor_y += height + line_height * 0.3;
            Some(b)
        },

        GeminiLine::Preformatted { lines, .. } => {
            let mut style = ComputedStyle::default();
            style.display = Display::Block;
            style.font_family = FontFamily::Monospace;
            style.font_size = theme.font_size - 1.0;
            style.color = theme.text_color;
            style.background_color = theme.pre_background;
            style.white_space = WhiteSpace::Pre;
            style.padding_top = 4.0;
            style.padding_bottom = 4.0;
            style.padding_left = 8.0;
            style.padding_right = 8.0;
            style.margin_bottom = line_height * 0.5;

            let height = lines.len() as f32 * line_height + 8.0;

            let mut b = LayoutBox::new(BoxType::Block, style, None);
            b.dimensions.content.x = 0.0;
            b.dimensions.content.y = *cursor_y;
            b.dimensions.content.width = content_width;
            b.dimensions.content.height = height;
            b.dimensions.padding = EdgeSizes {
                top: 4.0,
                right: 8.0,
                bottom: 4.0,
                left: 8.0,
            };

            *cursor_y += height + line_height * 0.5;
            Some(b)
        },

        GeminiLine::Empty => {
            *cursor_y += line_height * 0.5;
            None
        },
    }
}

fn render_heading(
    _text: &str,
    scale: f32,
    content_width: f32,
    cursor_y: &mut f32,
    line_height: f32,
    theme: &GeminiTheme,
) -> Option<LayoutBox> {
    let mut style = ComputedStyle::default();
    style.display = Display::Block;
    style.color = theme.heading_color;
    style.font_size = theme.font_size * scale;
    style.font_weight = FontWeight::Bold;
    style.margin_top = line_height * 0.5;
    style.margin_bottom = line_height * 0.3;

    let height = style.font_size * 1.2;
    let mut b = LayoutBox::new(BoxType::Block, style, None);
    b.dimensions.content.x = 0.0;
    b.dimensions.content.y = *cursor_y + line_height * 0.5;
    b.dimensions.content.width = content_width;
    b.dimensions.content.height = height;

    *cursor_y += line_height * 0.5 + height + line_height * 0.3;
    Some(b)
}

/// Calculate height needed for wrapped text.
fn calculate_wrapped_height(
    text: &str,
    width: f32,
    font_size: f32,
    measurer: &dyn TextMeasurer,
) -> f32 {
    if text.is_empty() {
        return font_size * 1.2;
    }
    let text_width = measurer.measure_text(text, font_size as u16) as f32;
    let line_count = (text_width / width).ceil().max(1.0);
    line_count * font_size * 1.5
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::gemini::parser::GeminiDocument;

    /// Stub measurer: 8 pixels per character (matches the 8x8
    /// bitmap font used by OASIS backends).
    struct StubMeasurer;

    impl TextMeasurer for StubMeasurer {
        fn measure_text(&self, text: &str, _font_size: u16) -> u32 {
            text.len() as u32 * 8
        }
    }

    #[test]
    fn render_simple_text_document() {
        let doc = GeminiDocument::parse("Hello world\nMore text");
        let theme = GeminiTheme::default();
        let measurer = StubMeasurer;
        let root = render_gemini(&doc, 480.0, &theme, &measurer);

        // Root should contain two block children (one per text
        // line).
        assert_eq!(root.children.len(), 2);
        assert!(root.dimensions.content.width > 0.0);
        assert!(root.dimensions.content.height > 0.0);
    }

    #[test]
    fn heading_sizes_are_correct() {
        let doc = GeminiDocument::parse("# Big\n## Medium\n### Small");
        let theme = GeminiTheme::default();
        let measurer = StubMeasurer;
        let root = render_gemini(&doc, 480.0, &theme, &measurer);

        assert_eq!(root.children.len(), 3);

        let h1_size = root.children[0].style.font_size;
        let h2_size = root.children[1].style.font_size;
        let h3_size = root.children[2].style.font_size;

        // h1 (2x) > h2 (1.5x) > h3 (1.17x)
        assert!(h1_size > h2_size);
        assert!(h2_size > h3_size);
        assert!((h1_size - theme.font_size * 2.0).abs() < f32::EPSILON);
        assert!((h2_size - theme.font_size * 1.5).abs() < f32::EPSILON);
    }

    #[test]
    fn list_items_have_indentation() {
        let doc = GeminiDocument::parse("* Item one\n* Item two");
        let theme = GeminiTheme::default();
        let measurer = StubMeasurer;
        let root = render_gemini(&doc, 480.0, &theme, &measurer);

        assert_eq!(root.children.len(), 2);

        for child in &root.children {
            // List items should be indented (content.x = 20).
            assert!(
                (child.dimensions.content.x - 20.0).abs() < f32::EPSILON,
                "list item should be indented by 20px"
            );
            assert!(matches!(child.box_type, BoxType::ListItem { .. }));
        }
    }

    #[test]
    fn preformatted_blocks_use_monospace_style() {
        let doc = GeminiDocument::parse("```code\nfn main() {}\n```");
        let theme = GeminiTheme::default();
        let measurer = StubMeasurer;
        let root = render_gemini(&doc, 480.0, &theme, &measurer);

        assert_eq!(root.children.len(), 1);
        let pre_block = &root.children[0];
        assert_eq!(pre_block.style.font_family, FontFamily::Monospace);
        assert_eq!(pre_block.style.white_space, WhiteSpace::Pre);
        assert_eq!(pre_block.style.background_color, theme.pre_background);
    }
}
