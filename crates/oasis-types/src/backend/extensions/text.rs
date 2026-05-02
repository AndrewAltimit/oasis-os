//! `SdiText` extension trait.
//!
//! Extracted from the historical monolithic `extensions.rs`.

use crate::backend::{Color, SdiCore, TextMetrics};
use crate::error::Result;

// ---------------------------------------------------------------------------
// SdiText
// ---------------------------------------------------------------------------

/// Text measurement and drawing helpers.
#[allow(clippy::too_many_arguments)]
pub trait SdiText: SdiCore {
    /// Measure the height of text at the given font size.
    fn measure_text_height(&self, font_size: u16) -> u32 {
        let fs = font_size as u32;
        (fs * 6).div_ceil(5)
    }

    /// Measure the font's ascent.
    fn font_ascent(&self, font_size: u16) -> u32 {
        let fs = font_size as u32;
        (fs * 17).div_ceil(20)
    }

    /// Return full text metrics (width, height, ascent) for a string.
    fn text_metrics(&self, text: &str, font_size: u16) -> TextMetrics {
        TextMetrics {
            width: self.measure_text(text, font_size),
            height: self.measure_text_height(font_size),
            ascent: self.font_ascent(font_size),
        }
    }

    /// Measure both width and height of a text string.
    fn measure_text_extents(&self, text: &str, font_size: u16) -> (u32, u32) {
        let m = self.text_metrics(text, font_size);
        (m.width, m.height)
    }

    /// Draw text truncated with "..." if it exceeds `max_width`.
    ///
    /// Returns the actual drawn width in pixels.
    fn draw_text_ellipsis(
        &mut self,
        text: &str,
        x: i32,
        y: i32,
        font_size: u16,
        color: Color,
        max_width: u32,
    ) -> Result<u32> {
        let text_w = self.measure_text(text, font_size);
        if text_w <= max_width {
            self.draw_text(text, x, y, font_size, color)?;
            return Ok(text_w);
        }
        let ellipsis_w = self.measure_text("...", font_size);
        let target = max_width.saturating_sub(ellipsis_w);
        let mut drawn_w = 0u32;
        let mut end_byte = 0;
        for (i, ch) in text.char_indices() {
            let ch_w = self.measure_text(&text[i..i + ch.len_utf8()], font_size);
            if drawn_w + ch_w > target {
                break;
            }
            drawn_w += ch_w;
            end_byte = i + ch.len_utf8();
        }
        let truncated = format!("{}...", &text[..end_byte]);
        self.draw_text(&truncated, x, y, font_size, color)?;
        Ok(drawn_w + ellipsis_w)
    }

    /// Draw text with bold and italic style hints.
    ///
    /// Faux-bold is implemented via double-strike (drawing at x and x+1).
    /// The default implementation ignores `italic` because a true faux-italic
    /// requires per-scanline skew which cannot be achieved with `draw_text`.
    /// Backends that support italic rendering should override this method.
    fn draw_text_styled(
        &mut self,
        text: &str,
        x: i32,
        y: i32,
        font_size: u16,
        color: Color,
        bold: bool,
        _italic: bool,
    ) -> Result<()> {
        self.draw_text(text, x, y, font_size, color)?;
        if bold {
            self.draw_text(text, x + 1, y, font_size, color)?;
        }
        Ok(())
    }

    /// Draw multiline word-wrapped text within a bounding box.
    ///
    /// Returns the total height used in pixels.
    fn draw_text_wrapped(
        &mut self,
        text: &str,
        x: i32,
        y: i32,
        font_size: u16,
        color: Color,
        max_width: u32,
        line_height: u32,
    ) -> Result<u32> {
        let lh = if line_height > 0 {
            line_height
        } else {
            self.measure_text_height(font_size)
        };
        let mut cy = y;
        for line in text.split('\n') {
            let words: Vec<&str> = line.split_whitespace().collect();
            if words.is_empty() {
                cy += lh as i32;
                continue;
            }
            let mut current_line = String::new();
            for word in words {
                let test = if current_line.is_empty() {
                    word.to_string()
                } else {
                    format!("{current_line} {word}")
                };
                let tw = self.measure_text(&test, font_size);
                if tw > max_width && !current_line.is_empty() {
                    self.draw_text(&current_line, x, cy, font_size, color)?;
                    cy += lh as i32;
                    current_line = word.to_string();
                } else {
                    current_line = test;
                }
            }
            if !current_line.is_empty() {
                self.draw_text(&current_line, x, cy, font_size, color)?;
                cy += lh as i32;
            }
        }
        Ok((cy - y) as u32)
    }
}
