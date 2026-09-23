//! Fit short UI strings (icon labels, bar captions) into a pixel budget.
//!
//! Measurement uses the proportional bitmap font metrics
//! ([`bitmap_measure_text`]) — the same widths every backend falls back to
//! and the metric the rest of the shell chrome (bars, start menu, taskbar)
//! lays out with — so wrapping and ellipsizing agree with what is drawn
//! instead of assuming a fixed 8px cell per character.

use oasis_types::backend::bitmap_measure_text;

/// Ellipsis appended to truncated text (a single glyph in the bitmap font).
pub const ELLIPSIS: &str = "\u{2026}";

/// Pixel width of `text` at `font_size`.
pub fn text_width(text: &str, font_size: u16) -> u32 {
    bitmap_measure_text(text, font_size)
}

/// Truncate `text` so it fits within `max_w` pixels, appending
/// [`ELLIPSIS`] when anything was cut. Text that already fits is returned
/// unchanged. When not even the ellipsis fits, returns an empty string.
pub fn ellipsize(text: &str, max_w: u32, font_size: u16) -> String {
    if text_width(text, font_size) <= max_w {
        return text.to_string();
    }
    let ell_w = text_width(ELLIPSIS, font_size);
    if ell_w > max_w {
        return String::new();
    }
    let budget = max_w - ell_w;
    let mut used = 0u32;
    let mut end = 0usize;
    for (i, ch) in text.char_indices() {
        let w = oasis_types::bitmap_font::glyph_advance_scaled(ch, font_size);
        if used + w > budget {
            break;
        }
        used += w;
        end = i + ch.len_utf8();
    }
    let mut out = text[..end].trim_end().to_string();
    out.push_str(ELLIPSIS);
    out
}

/// Word-wrap `text` into at most `max_lines` lines, each no wider than
/// `max_w` pixels.
///
/// - Words are packed greedily by measured width.
/// - A single word wider than a whole line is ellipsized on its own line.
/// - Text that needs more than `max_lines` lines is folded into the last
///   line, which is then ellipsized — nothing is silently dropped without
///   an ellipsis marking the cut.
pub fn wrap_lines(text: &str, max_w: u32, font_size: u16, max_lines: usize) -> Vec<String> {
    let max_lines = max_lines.max(1);
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return Vec::new();
    }
    let space_w = text_width(" ", font_size);
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut cur_w = 0u32;
    let mut rest_start = None;
    for (wi, word) in words.iter().enumerate() {
        let ww = text_width(word, font_size);
        if cur.is_empty() {
            cur.push_str(word);
            cur_w = ww;
        } else if cur_w + space_w + ww <= max_w {
            cur.push(' ');
            cur.push_str(word);
            cur_w += space_w + ww;
        } else {
            if lines.len() + 1 == max_lines {
                // Out of lines: fold this word and everything after it into
                // the last line (ellipsized below).
                rest_start = Some(wi);
                break;
            }
            lines.push(std::mem::take(&mut cur));
            cur.push_str(word);
            cur_w = ww;
        }
    }
    if let Some(start) = rest_start {
        for word in &words[start..] {
            cur.push(' ');
            cur.push_str(word);
        }
        // The fold only happens when the next word overflowed, so the
        // folded line never fits: `ellipsize` always marks the cut.
        cur = ellipsize(&cur, max_w, font_size);
    }
    lines.push(cur);
    lines
        .into_iter()
        .map(|l| ellipsize(&l, max_w, font_size))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn width_is_proportional() {
        // 'i' is narrower than 'W' in the proportional bitmap font.
        assert!(text_width("iii", 8) < text_width("WWW", 8));
        assert_eq!(text_width("", 8), 0);
    }

    #[test]
    fn ellipsize_keeps_fitting_text() {
        assert_eq!(ellipsize("Paint", 200, 8), "Paint");
    }

    #[test]
    fn ellipsize_truncates_to_width() {
        let out = ellipsize("Supercalifragilistic", 40, 8);
        assert!(out.ends_with(ELLIPSIS));
        assert!(text_width(&out, 8) <= 40, "{out}");
        assert!(out.len() > ELLIPSIS.len());
    }

    #[test]
    fn ellipsize_too_narrow_for_ellipsis_is_empty() {
        assert_eq!(ellipsize("Hello", 1, 8), "");
    }

    #[test]
    fn wrap_packs_words_by_width() {
        let lines = wrap_lines("Music Player", 200, 8, 2);
        assert_eq!(lines, vec!["Music Player".to_string()]);
        let w = text_width("Music", 8).max(text_width("Player", 8));
        let lines = wrap_lines("Music Player", w, 8, 2);
        assert_eq!(lines, vec!["Music".to_string(), "Player".to_string()]);
    }

    #[test]
    fn wrap_ellipsizes_overlong_single_word() {
        let lines = wrap_lines("Configuration", 30, 8, 2);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].ends_with(ELLIPSIS));
        assert!(text_width(&lines[0], 8) <= 30);
    }

    #[test]
    fn wrap_folds_extra_lines_with_ellipsis() {
        let lines = wrap_lines("one two three four five", 30, 8, 2);
        assert_eq!(lines.len(), 2);
        assert!(lines[1].ends_with(ELLIPSIS), "{lines:?}");
        for l in &lines {
            assert!(text_width(l, 8) <= 30, "{l}");
        }
    }

    #[test]
    fn wrap_marks_fold_of_short_tail() {
        // A tiny third word that overflowed line 2 is still marked.
        let w = text_width("aaaa", 8);
        let lines = wrap_lines("aaaa bbbb c", w, 8, 2);
        assert_eq!(lines.len(), 2);
        assert!(lines[1].ends_with(ELLIPSIS), "{lines:?}");
        assert!(text_width(&lines[1], 8) <= w);
    }

    #[test]
    fn wrap_empty_is_empty() {
        assert!(wrap_lines("   ", 50, 8, 2).is_empty());
    }
}
