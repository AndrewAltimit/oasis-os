//! Label rendering for dashboard icons.
//!
//! Provides word-wrapping and centered label drawing beneath icons,
//! including optional drop-shadow support.

use crate::active_theme::ActiveTheme;
use crate::backend::Color;
use crate::sdi::SdiRegistry;

use super::IconNames;

/// Word-wrap a label into lines that fit within `max_chars` per line.
pub(crate) fn wrap_label(text: &str, max_chars: usize) -> Vec<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return vec![];
    }
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in words {
        let test_len = if cur.is_empty() {
            word.len()
        } else {
            cur.len() + 1 + word.len()
        };
        if !cur.is_empty() && test_len > max_chars {
            lines.push(cur);
            cur = word.to_string();
        } else {
            if !cur.is_empty() {
                cur.push(' ');
            }
            cur.push_str(word);
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

/// Render word-wrapped label lines under an icon.
///
/// Lines are centered within `[cell_x, cell_x + cell_w]` by default. When
/// `icon_center` is `Some(cx)` (column layout) each line is centered on
/// the icon's horizontal midpoint instead, PSIX-style, clamped so long
/// lines stay on-screen.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_label(
    sdi: &mut SdiRegistry,
    at: &ActiveTheme,
    names: &IconNames,
    cell_x: i32,
    cell_w: u32,
    label_y: i32,
    title: &str,
    icon_center: Option<i32>,
) {
    let fs = at.font_small;
    let glyph_w = (fs.max(8) / 8) as u32 * 8;
    let max_chars = (cell_w / glyph_w).max(1) as usize;
    let lines = wrap_label(title, max_chars);
    let line_h = glyph_w as i32 + 1; // 1px spacing between lines
    // Left edge for a line of pixel width `tw`: centered on the icon
    // midpoint in column layout, otherwise centered within the cell
    // (unchanged legacy arithmetic).
    let line_x = |tw: i32| match icon_center {
        Some(cx) => (cx - tw / 2).max(2),
        None => cell_x + (cell_w as i32 - tw) / 2,
    };

    // Label shadows (1px offset).
    if let Some(shadow_color) = at.icon.label_shadow {
        // Shadow for line 1.
        if let Ok(obj) = sdi.get_mut(&names.shadow) {
            if let Some(line) = lines.first() {
                let tw = line.len() as i32 * glyph_w as i32;
                obj.x = line_x(tw) + 1;
                obj.y = label_y + 1;
                obj.w = 0;
                obj.h = 0;
                obj.font_size = fs;
                obj.text = Some(line.clone());
                obj.text_color = shadow_color;
                obj.visible = true;
                obj.color = Color::rgba(0, 0, 0, 0);
            } else {
                obj.visible = false;
            }
        }
        // Shadow for line 2.
        if let Ok(obj) = sdi.get_mut(&names.shadow2) {
            if lines.len() > 1 {
                let tw = lines[1].len() as i32 * glyph_w as i32;
                obj.x = line_x(tw) + 1;
                obj.y = label_y + line_h + 1;
                obj.w = 0;
                obj.h = 0;
                obj.font_size = fs;
                obj.text = Some(lines[1].clone());
                obj.text_color = shadow_color;
                obj.visible = true;
                obj.color = Color::rgba(0, 0, 0, 0);
            } else {
                obj.visible = false;
            }
        }
    } else {
        if let Ok(obj) = sdi.get_mut(&names.shadow) {
            obj.visible = false;
        }
        if let Ok(obj) = sdi.get_mut(&names.shadow2) {
            obj.visible = false;
        }
    }

    // Line 1.
    if let Ok(obj) = sdi.get_mut(&names.label) {
        if let Some(line) = lines.first() {
            let tw = line.len() as i32 * glyph_w as i32;
            obj.x = line_x(tw);
            obj.y = label_y;
            obj.w = 0;
            obj.h = 0;
            obj.font_size = fs;
            obj.text = Some(line.clone());
            obj.text_color = at.icon.label_color;
            obj.visible = true;
        } else {
            obj.visible = false;
        }
    }
    // Line 2.
    if let Ok(obj) = sdi.get_mut(&names.label2) {
        if lines.len() > 1 {
            let tw = lines[1].len() as i32 * glyph_w as i32;
            obj.x = line_x(tw);
            obj.y = label_y + line_h;
            obj.w = 0;
            obj.h = 0;
            obj.font_size = fs;
            obj.text = Some(lines[1].clone());
            obj.text_color = at.icon.label_color;
            obj.visible = true;
        } else {
            obj.visible = false;
        }
    }
}
