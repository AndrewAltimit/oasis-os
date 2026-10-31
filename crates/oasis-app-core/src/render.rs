//! Shared windowed renderer for line-list apps ([`ContentState`]): the
//! row layout [`draw_content_windowed`] draws and the inverse mappings
//! click handlers use ([`windowed_line_at`], [`windowed_line_origin`]).

use oasis_skin::ActiveTheme;
use oasis_types::backend::SdiBackend;

use crate::app_trait::ContentState;

/// Top inset for windowed app content when no context header row is drawn.
///
/// The WM titlebar already shows the app title, so windowed content must not
/// repeat it in an inner title bar (it reads as a double title bar). Click
/// handlers that map a local Y back to a content line must use this same
/// inset.
pub const WINDOWED_TOP_PAD: u32 = 4;

/// Font size [`draw_content_windowed`] draws content lines with.
pub const WINDOWED_FONT_SIZE: u16 = 12;

/// Rows [`draw_content_windowed`] shows in a window with room for
/// `max_lines`: `(first line index, selected row, row count)`.
///
/// `ContentState` scrolls against the row count cached by its last
/// [`ContentState::update_layout_windowed`], which can disagree with this
/// window (apps that never call it, a resize since the last layout). The drawn
/// range is shifted so the selected row is always on screen; otherwise the
/// cursor could walk off the bottom of a small window.
fn windowed_rows(content: &ContentState, max_lines: usize) -> (usize, usize, usize) {
    let shift = if max_lines > 0 {
        (content.cursor + 1).saturating_sub(max_lines)
    } else {
        0
    };
    let first = content.scroll + shift;
    let visible = content.lines.len().saturating_sub(first).min(max_lines);
    (first, content.cursor - shift, visible)
}

/// Where [`draw_content_windowed`] draws line `line_idx` of `content` in a
/// window whose content area starts at `(cx, cy)` and is `ch` tall.
///
/// Returns `(x, y, prefix)`: the text origin of that line and the 2-char
/// selection prefix (`"> "` or `"  "`) drawn in front of the line text, or
/// `None` when the line is scrolled out of view. Used to overlay a text
/// cursor on a line.
pub fn windowed_line_origin(
    content: &ContentState,
    cx: i32,
    cy: i32,
    ch: u32,
    at: &ActiveTheme,
    line_idx: usize,
) -> Option<(i32, i32, &'static str)> {
    let metrics = WindowedMetrics::from_theme(at);
    let content_top = windowed_content_top(content, metrics);
    let line_h = metrics.line_h as i32;
    let max_lines = windowed_max_lines(content, ch, metrics);
    let (first, cursor_row, visible) = windowed_rows(content, max_lines);
    let i = line_idx.checked_sub(first)?;
    if i >= visible {
        return None;
    }
    let prefix = if i == cursor_row { "> " } else { "  " };
    Some((cx + 4, cy + content_top + i as i32 * line_h, prefix))
}

/// Theme metrics [`draw_content_windowed`] lays lines out with. Apps record
/// them in [`App::layout`](crate::App::layout) (their click handler gets
/// no theme) and pass them to [`windowed_line_at`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowedMetrics {
    /// Height of the context header row (`at.app.title_bar_height`).
    pub title_bar_height: u32,
    /// Content line height (`at.terminal_line_height`, at least 12).
    pub line_h: u32,
}

impl WindowedMetrics {
    /// Metrics of `at`.
    pub fn from_theme(at: &ActiveTheme) -> Self {
        Self {
            title_bar_height: at.app.title_bar_height,
            line_h: at.terminal_line_height.max(12),
        }
    }
}

impl Default for WindowedMetrics {
    fn default() -> Self {
        Self::from_theme(&ActiveTheme::default())
    }
}

/// Top inset of the first content row [`draw_content_windowed`] draws:
/// below the context header row when there is one.
fn windowed_content_top(content: &ContentState, metrics: WindowedMetrics) -> i32 {
    let has_context = content.viewing_file.is_some() || content.browse_dir.is_some();
    if has_context {
        metrics.title_bar_height as i32
    } else {
        WINDOWED_TOP_PAD as i32
    }
}

/// How many content rows [`draw_content_windowed`] fits in a content area
/// `ch` tall (may be 0 for a degenerate window).
pub fn windowed_max_lines(content: &ContentState, ch: u32, metrics: WindowedMetrics) -> usize {
    let content_top = windowed_content_top(content, metrics);
    let line_h = metrics.line_h.max(1) as i32;
    ((ch as i32 - content_top - 16) / line_h).max(0) as usize
}

/// The content line [`draw_content_windowed`] drew at content-local `ly`
/// in a window whose content area is `ch` tall (the inverse of
/// [`windowed_line_origin`]), or `None` when `ly` is outside the lines.
pub fn windowed_line_at(
    content: &ContentState,
    ch: u32,
    metrics: WindowedMetrics,
    ly: i32,
) -> Option<usize> {
    let content_top = windowed_content_top(content, metrics);
    let line_h = metrics.line_h.max(1) as i32;
    let max_lines = windowed_max_lines(content, ch, metrics);
    let (first, _, visible) = windowed_rows(content, max_lines);
    let y = ly - content_top;
    if y < 0 {
        return None;
    }
    let row = (y / line_h) as usize;
    (row < visible).then_some(first + row)
}

/// `text` cut to fit `max_w` pixels at `font_size`, ending in `...` when
/// shortened (the same cut `SdiText::draw_text_ellipsis` draws), or empty
/// when not even the ellipsis fits. For labels an app lays out or batches
/// itself; to just draw one, call `draw_text_ellipsis`.
pub fn ellipsize<'a>(
    backend: &dyn SdiBackend,
    text: &'a str,
    font_size: u16,
    max_w: u32,
) -> std::borrow::Cow<'a, str> {
    if backend.measure_text(text, font_size) <= max_w {
        return text.into();
    }
    let ellipsis_w = backend.measure_text("...", font_size);
    if ellipsis_w > max_w {
        return "".into();
    }
    let target = max_w - ellipsis_w;
    let mut drawn = 0u32;
    let mut end = 0;
    for (i, c) in text.char_indices() {
        let w = backend.measure_text(&text[i..i + c.len_utf8()], font_size);
        if drawn + w > target {
            break;
        }
        drawn += w;
        end = i + c.len_utf8();
    }
    format!("{}...", &text[..end]).into()
}

/// Largest font size, at most `font_size`, whose line box fits in `h`
/// pixels (never below 8, the smallest bitmap size).
pub fn font_size_fitting(backend: &dyn SdiBackend, font_size: u16, h: u32) -> u16 {
    let mut size = font_size.max(8);
    while size > 8 && backend.measure_text_height(size) > h {
        size -= 1;
    }
    size
}

/// Draw a one-line status strip into `rect = (x, y, w, h)`: `left` from
/// the left edge, `right` (a key hint, may be empty) right-aligned.
///
/// The shared form of every app's footer / status bar. The font shrinks
/// (from `font_size`) until a line fits the strip's height and the text is
/// centred vertically; `left` is ellipsized to the width it has, and
/// `right` is dropped when it would collide with `left` (the status
/// message matters more than the hint). `colors` are `(left, right)`;
/// the caller fills the background.
pub fn draw_status_strip(
    backend: &mut dyn SdiBackend,
    rect: (i32, i32, u32, u32),
    left: &str,
    right: &str,
    font_size: u16,
    colors: (oasis_types::backend::Color, oasis_types::backend::Color),
) -> oasis_types::error::Result<()> {
    const PAD: i32 = 4;
    const GAP: u32 = 12;
    let (x, y, w, h) = rect;
    let size = font_size_fitting(backend, font_size, h);
    let ty = y + (h as i32 - backend.measure_text_height(size) as i32).max(0) / 2;
    let inner_w = w.saturating_sub(2 * PAD as u32);
    let left_w = backend.measure_text(left, size);
    let right_w = if right.is_empty() {
        0
    } else {
        backend.measure_text(right, size)
    };
    let mut left_max = inner_w;
    if right_w > 0 && left_w + GAP + right_w <= inner_w {
        let rx = x + w as i32 - PAD - right_w as i32;
        backend.draw_text(right, rx, ty, size, colors.1)?;
        left_max = inner_w - GAP - right_w;
    }
    if !left.is_empty() {
        backend.draw_text_ellipsis(left, x + PAD, ty, size, colors.0, left_max)?;
    }
    Ok(())
}

/// Draw generic content to a windowed region.
///
/// The app title is NOT drawn here — the WM titlebar already shows it. A
/// browse-directory / viewed-file context path still gets a dim header row
/// (occupying `title_bar_height`, same as the old inner title bar); without
/// one, content starts at [`WINDOWED_TOP_PAD`].
pub fn draw_content_windowed(
    content: &ContentState,
    cx: i32,
    cy: i32,
    cw: u32,
    ch: u32,
    backend: &mut dyn SdiBackend,
    at: &ActiveTheme,
) -> oasis_types::error::Result<()> {
    // Context header row (browse dir / viewed file), if any.
    let context = if let Some(ref file) = content.viewing_file {
        Some(format!("[{file}]"))
    } else {
        content.browse_dir.as_deref().map(|d| format!("[{d}]"))
    };
    // Every row is ellipsized to the content width: long lines, paths and
    // the footer hint would otherwise be cut mid-glyph by the window clip.
    let text_w = cw.saturating_sub(8);
    let content_top = if let Some(ctx) = context {
        backend.draw_text_ellipsis(&ctx, cx + 4, cy + 2, 12, at.app.dim_text, text_w)?;
        backend.fill_rect(
            cx,
            cy + at.app.title_bar_height as i32 - 4,
            cw,
            1,
            at.app.divider,
        )?;
        at.app.title_bar_height as i32
    } else {
        WINDOWED_TOP_PAD as i32
    };

    // Content lines.
    let metrics = WindowedMetrics::from_theme(at);
    let line_h = metrics.line_h as i32;
    let max_lines = windowed_max_lines(content, ch, metrics);
    let (first, cursor_row, visible) = windowed_rows(content, max_lines);
    // One buffer reused for every "{prefix}{line}" string (a single
    // draw_text call keeps proportional-font glyph placement identical).
    let mut text = String::new();
    for i in 0..visible {
        let line_idx = first + i;
        let line = &content.lines[line_idx];
        let prefix = if i == cursor_row { "> " } else { "  " };
        text.clear();
        text.push_str(prefix);
        text.push_str(line);
        let text_color = if i == cursor_row {
            at.app.selected_text
        } else {
            at.app.text
        };
        let y = cy + content_top + i as i32 * line_h;
        backend.draw_text_ellipsis(&text, cx + 4, y, WINDOWED_FONT_SIZE, text_color, text_w)?;
    }

    // Scroll indicator.
    let scroll_text = if content.lines.len() > max_lines {
        format!(
            "[{}/{}]  Cancel=back",
            first + 1,
            content.lines.len().saturating_sub(max_lines) + 1,
        )
    } else {
        "Cancel=back".to_string()
    };
    let scroll_y = cy + ch as i32 - 14;
    backend.draw_text_ellipsis(&scroll_text, cx + 4, scroll_y, 10, at.app.dim_text, text_w)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use oasis_test_backend::RecordingBackend;
    use oasis_types::backend::{SdiCore, SdiText};

    #[test]
    fn windowed_line_at_inverts_windowed_line_origin() {
        let at = ActiveTheme::default();
        let m = WindowedMetrics::from_theme(&at);
        for browse in [false, true] {
            let mut content = ContentState::new("Files", "/apps/files");
            content.lines = (0..40).map(|i| format!("entry {i}")).collect();
            if browse {
                content.browse_dir = Some("/home".into());
            }
            // Cursor far enough down that the windowed view shifts.
            content.cursor = 20;
            let ch = 200;
            let mut seen = 0;
            for idx in 0..content.lines.len() {
                if let Some((_, y, _)) = windowed_line_origin(&content, 0, 0, ch, &at, idx) {
                    for dy in [0, m.line_h as i32 - 1] {
                        assert_eq!(windowed_line_at(&content, ch, m, y + dy), Some(idx));
                    }
                    seen += 1;
                }
            }
            assert!(seen > 3, "some lines visible");
            assert_eq!(windowed_line_at(&content, ch, m, -1), None);
            assert_eq!(windowed_line_at(&content, ch, m, ch as i32), None);
        }
    }

    #[test]
    fn ellipsize_keeps_fitting_text_and_cuts_the_rest_to_width() {
        let rec = RecordingBackend::default();
        let text = "The quick brown fox jumps over the lazy dog";
        let full = rec.measure_text(text, 12);
        // Fits: returned unchanged, without allocating.
        let same = ellipsize(&rec, text, 12, full);
        assert!(matches!(same, std::borrow::Cow::Borrowed(t) if t == text));

        let dots = rec.measure_text("...", 12);
        for max_w in dots..full {
            let cut = ellipsize(&rec, text, 12, max_w);
            assert!(cut.ends_with("..."), "{max_w}: {cut}");
            let kept = cut.trim_end_matches("...");
            assert!(text.starts_with(kept), "{max_w}: a prefix of the text");
            assert!(rec.measure_text(&cut, 12) <= max_w, "{max_w}: {cut} fits");
            // The longest prefix that fits: one more char would not.
            let next = &text[..kept.len() + 1];
            assert!(
                rec.measure_text(next, 12) + dots > max_w,
                "{max_w}: '{kept}' could keep one more char"
            );
        }
        // Not even the ellipsis fits.
        assert_eq!(ellipsize(&rec, text, 12, dots - 1), "");
        assert_eq!(ellipsize(&rec, text, 12, 0), "");
    }

    #[test]
    fn ellipsize_cuts_multibyte_text_on_char_boundaries() {
        let rec = RecordingBackend::default();
        let text = "Grüße aus Köln, schöne Straße";
        let full = rec.measure_text(text, 10);
        for max_w in 0..full {
            // Slicing inside a multi-byte char would panic here.
            let cut = ellipsize(&rec, text, 10, max_w);
            assert!(rec.measure_text(&cut, 10) <= max_w);
        }
    }

    #[test]
    fn font_size_fitting_shrinks_to_the_height_but_not_below_8() {
        let rec = RecordingBackend::default();
        let h16 = rec.measure_text_height(16);
        assert_eq!(font_size_fitting(&rec, 16, h16), 16, "already fits");
        assert_eq!(font_size_fitting(&rec, 16, 10_000), 16, "never grows");
        let room = h16 - 1;
        let fitted = font_size_fitting(&rec, 16, room);
        assert!(fitted < 16);
        assert!(rec.measure_text_height(fitted) <= room || fitted == 8);
        assert!(
            rec.measure_text_height(fitted + 1) > room,
            "the largest size that fits"
        );
        assert_eq!(font_size_fitting(&rec, 16, 0), 8, "floor of 8");
        assert_eq!(font_size_fitting(&rec, 4, 100), 8, "sizes below 8 round up");
    }

    #[test]
    fn status_strip_right_aligns_the_hint_and_drops_it_when_crowded() {
        let white = oasis_types::backend::Color::WHITE;
        let colors = (white, white);
        let (x, y, w, h) = (10, 50, 300, 20);

        // Room for both: the hint ends PAD (4) px from the right edge.
        let mut rec = RecordingBackend::default();
        draw_status_strip(&mut rec, (x, y, w, h), "3 items", "Esc=back", 12, colors).unwrap();
        let drawn = rec.text_positions();
        let hint = drawn.iter().find(|t| t.0 == "Esc=back").expect("hint");
        let hint_w = rec.measure_text("Esc=back", hint.3) as i32;
        assert_eq!(hint.1 + hint_w, x + w as i32 - 4);
        let left = drawn.iter().find(|t| t.0 == "3 items").expect("status");
        assert_eq!(left.1, x + 4);
        // Vertically inside the strip.
        for t in &drawn {
            assert!(t.2 >= y && t.2 + rec.measure_text_height(t.3) as i32 <= y + h as i32);
        }

        // Crowded: the status keeps the room (ellipsized), the hint goes.
        let long = "a very long status message that cannot share the strip";
        let mut rec = RecordingBackend::default();
        draw_status_strip(&mut rec, (x, y, 120, h), long, "Esc=back", 12, colors).unwrap();
        assert!(!rec.has_text("Esc=back"), "{:?}", rec.texts());
        let shown = rec.texts();
        assert_eq!(shown.len(), 1);
        assert!(shown[0].ends_with("..."));
        assert!(rec.measure_text(shown[0], 12) <= 120 - 8);

        // Empty texts draw nothing; a too-short strip shrinks the font.
        let mut rec = RecordingBackend::default();
        draw_status_strip(&mut rec, (x, y, w, 0), "", "", 12, colors).unwrap();
        assert_eq!(rec.draw_text_count(), 0);
        let mut rec = RecordingBackend::default();
        draw_status_strip(&mut rec, (x, y, w, 9), "tiny", "", 16, colors).unwrap();
        assert!(rec.text_positions()[0].3 < 16, "font shrank to the strip");
    }

    #[test]
    fn windowed_max_lines_is_zero_for_degenerate_windows() {
        let content = ContentState::new("Files", "/apps/files");
        let m = WindowedMetrics::default();
        assert_eq!(windowed_max_lines(&content, 0, m), 0);
        assert_eq!(windowed_max_lines(&content, 10, m), 0);
        let zero_line = WindowedMetrics { line_h: 0, ..m };
        // A zero line height must not divide by zero.
        assert!(windowed_max_lines(&content, 200, zero_line) > 0);
        assert_eq!(windowed_line_at(&content, 200, zero_line, 50), None);
    }

    #[test]
    fn windowed_draw_keeps_the_cursor_row_on_screen_in_a_small_window() {
        // The cached layout says 40 rows fit, the window shows far fewer:
        // the drawn range shifts so the selected row is still drawn.
        let mut content = ContentState::new("Files", "/apps/files");
        content.lines = (0..40).map(|i| format!("entry {i}")).collect();
        content.cursor = 30;
        let mut rec = RecordingBackend::default();
        let at = ActiveTheme::default();
        draw_content_windowed(&content, 0, 0, 300, 120, &mut rec, &at).unwrap();
        assert!(rec.has_text("> entry 30"), "{:?}", rec.texts());
        // And every row stays above the scroll indicator.
        let footer_y = rec
            .text_positions()
            .into_iter()
            .find(|t| t.0.contains("Cancel=back"))
            .expect("footer")
            .2;
        for t in rec.text_positions() {
            if t.0.contains("entry") {
                assert!(t.2 < footer_y, "{} overlaps the footer", t.0);
            }
        }
    }

    #[test]
    fn windowed_draw_omits_app_title() {
        // The WM titlebar already shows the app title; drawing it again in
        // the content area produced a "double title bar" in every windowed
        // app. Content must instead start at WINDOWED_TOP_PAD.
        let mut content = ContentState::new("Paint", "/apps/paint");
        content.lines = vec!["first".into(), "second".into()];

        let mut rec = RecordingBackend::default();
        let at = ActiveTheme::default();
        draw_content_windowed(&content, 0, 0, 300, 200, &mut rec, &at).unwrap();

        let texts = rec.text_positions();
        assert!(
            !rec.has_text("Paint"),
            "windowed content must not repeat the app title: {texts:?}"
        );
        let first = texts.first().expect("content lines drawn");
        assert!(first.0.contains("first"));
        assert_eq!(first.2, WINDOWED_TOP_PAD as i32);
    }

    #[test]
    fn windowed_draw_keeps_context_header() {
        // A viewed-file (or browse-dir) path still gets a header row so the
        // context isn't lost — but without the app title.
        let mut content = ContentState::new("File Manager", "/apps/files");
        content.viewing_file = Some("/notes.txt".into());
        content.lines = vec!["hello".into()];

        let mut rec = RecordingBackend::default();
        let at = ActiveTheme::default();
        draw_content_windowed(&content, 0, 0, 300, 200, &mut rec, &at).unwrap();

        assert!(rec.has_text("[/notes.txt]"));
        assert!(!rec.has_text("File Manager"));
        // Content starts below the header row.
        let line = rec
            .text_positions()
            .into_iter()
            .find(|t| t.0.contains("hello"))
            .expect("content line drawn");
        assert_eq!(line.2, at.app.title_bar_height as i32);
    }
}
