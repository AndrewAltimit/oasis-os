//! Shared rendering helpers for app chrome (title bar, content, selection).

use oasis_sdi::SdiRegistry;
use oasis_skin::ActiveTheme;
use oasis_types::backend::SdiBackend;
use oasis_ui::flex;

use crate::app_trait::ContentState;
use crate::layout::AppLayout;

/// Size of the `app_line_{i}` / `app_lp_line_{i}` / `app_rp_line_{i}` pools
/// the hide pass walks.
const MAX_APP_LINES: usize = 100;

/// Cached pool object names: `hide_app_sdi` runs every frame in every
/// non-app mode, and `render_content_sdi` every frame in app mode, so a
/// `format!` per pool slot per frame was pure allocation churn.
struct AppLineNames {
    line: Vec<String>,
    lp: Vec<String>,
    rp: Vec<String>,
}

fn app_line_names() -> &'static AppLineNames {
    static NAMES: std::sync::OnceLock<AppLineNames> = std::sync::OnceLock::new();
    NAMES.get_or_init(|| {
        let pool = |prefix: &str| {
            (0..MAX_APP_LINES)
                .map(|i| format!("{prefix}{i}"))
                .collect::<Vec<_>>()
        };
        AppLineNames {
            line: pool("app_line_"),
            lp: pool("app_lp_line_"),
            rp: pool("app_rp_line_"),
        }
    })
}

/// Name of the `app_line_{i}` object (cached for `i < 100`).
fn app_line_name(i: usize) -> std::borrow::Cow<'static, str> {
    match app_line_names().line.get(i) {
        Some(name) => std::borrow::Cow::Borrowed(name.as_str()),
        None => std::borrow::Cow::Owned(format!("app_line_{i}")),
    }
}

/// Render the app background and title bar chrome to SDI.
pub fn render_app_chrome(sdi: &mut SdiRegistry, at: &ActiveTheme) {
    if !sdi.contains("app_bg") {
        sdi.create("app_bg");
    }
    if let Ok(obj) = sdi.get_mut("app_bg") {
        obj.x = 0;
        obj.y = 0;
        obj.w = at.screen_w;
        obj.h = at.screen_h;
        obj.color = at.app.bg;
        obj.visible = true;
        obj.z = 100;
    }

    if !sdi.contains("app_title_bg") {
        sdi.create("app_title_bg");
    }
    if let Ok(obj) = sdi.get_mut("app_title_bg") {
        obj.x = 0;
        obj.y = 0;
        obj.w = at.screen_w;
        obj.h = at.app.title_bar_height;
        obj.color = at.app.title_bar_bg;
        obj.gradient_top = at.app.title_bar_gradient_top;
        obj.gradient_bottom = at.app.title_bar_gradient_bottom;
        obj.shadow_level = Some(1);
        obj.visible = true;
        obj.z = 101;
    }
}

/// Render generic content (title, lines, scroll indicator, selection) to SDI.
pub fn render_content_sdi(content: &ContentState, sdi: &mut SdiRegistry, at: &ActiveTheme) {
    // Title text.
    if !sdi.contains("app_title_text") {
        sdi.create("app_title_text");
    }
    if let Ok(obj) = sdi.get_mut("app_title_text") {
        let dir_suffix = if let Some(ref file) = content.viewing_file {
            format!("  [{file}]")
        } else {
            content
                .browse_dir
                .as_deref()
                .map(|d| format!("  [{d}]"))
                .unwrap_or_default()
        };
        obj.text = Some(format!("{}{dir_suffix}", content.title));
        obj.x = 8;
        obj.y = 4;
        obj.font_size = at.font_body;
        obj.text_color = at.app.title_bar_text;
        obj.w = 0;
        obj.h = 0;
        obj.visible = true;
        obj.z = 102;
        if at.app.title_bar_text_shadow {
            obj.text_shadow_offset = Some((1, 1));
            obj.text_shadow_color = Some(at.app.title_bar_text_shadow_color);
        } else {
            obj.text_shadow_offset = None;
            obj.text_shadow_color = None;
        }
    }

    // Content lines.
    let app_layout = AppLayout::compute(at, 14);
    let line_rects = flex::vertical_list(
        app_layout.content_x,
        app_layout.content_y,
        app_layout.content_w,
        app_layout.line_h,
        0,
        app_layout.max_visible,
    );

    // Selection highlight.
    if !sdi.contains("app_sel_bg") {
        sdi.create("app_sel_bg");
    }
    let sel_y = app_layout.content_y + (content.visual_selected * app_layout.line_h as f32) as i32;
    if let Ok(obj) = sdi.get_mut("app_sel_bg") {
        obj.x = app_layout.content_x;
        obj.y = sel_y;
        obj.w = app_layout.content_w;
        obj.h = at.terminal_line_height;
        obj.color = at.app.selected_bg;
        obj.border_radius = Some(at.app.selection_border_radius);
        obj.visible = !content.lines.is_empty();
        obj.z = 101;
    }

    // Selection accent bar.
    if !sdi.contains("app_sel_accent") {
        sdi.create("app_sel_accent");
    }
    if let Ok(obj) = sdi.get_mut("app_sel_accent") {
        obj.x = app_layout.content_x;
        obj.y = sel_y;
        obj.w = 3;
        obj.h = at.terminal_line_height;
        obj.color = at.app.selection_accent_color;
        obj.border_radius = Some(at.app.selection_border_radius);
        obj.visible = !content.lines.is_empty();
        obj.z = 102;
    }

    for (i, rect) in line_rects.iter().enumerate() {
        let name = app_line_name(i);
        if !sdi.contains(&name) {
            sdi.create(name.as_ref());
        }
        if let Ok(obj) = sdi.get_mut(&name) {
            let line_idx = content.scroll + i;
            if let Some(line) = content.lines.get(line_idx) {
                obj.set_text(line);
                obj.visible = true;
            } else {
                obj.text = None;
                obj.visible = false;
            }
            obj.x = rect.x + 6;
            obj.y = rect.y;
            obj.font_size = at.font_body;
            obj.text_color = if i == content.cursor {
                at.app.selected_text
            } else {
                at.app.text
            };
            obj.w = 0;
            obj.h = 0;
            obj.z = 102;
        }
    }

    // Scroll indicator.
    if !sdi.contains("app_scroll") {
        sdi.create("app_scroll");
    }
    if let Ok(obj) = sdi.get_mut("app_scroll") {
        if content.lines.len() > app_layout.max_visible {
            obj.text = Some(format!(
                "[{}/{}]  Cancel=back",
                content.scroll + 1,
                content.lines.len().saturating_sub(app_layout.max_visible) + 1,
            ));
        } else {
            obj.text = Some("Cancel=back".to_string());
        }
        obj.x = 8;
        obj.y = at.screen_h as i32 - 14;
        obj.font_size = at.font_hint;
        obj.text_color = at.app.dim_text;
        obj.w = 0;
        obj.h = 0;
        obj.visible = true;
        obj.z = 102;
    }
}

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
/// `ContentState` scrolls against a row count cached from the fullscreen
/// layout, and a smaller window shows fewer rows. The drawn range is
/// shifted so the selected row is always on screen; otherwise the cursor
/// walks off the bottom of a small window and the last lines can never be
/// reached.
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
    let has_context = content.viewing_file.is_some() || content.browse_dir.is_some();
    let content_top = if has_context {
        at.app.title_bar_height as i32
    } else {
        WINDOWED_TOP_PAD as i32
    };
    let line_h = at.terminal_line_height.max(12) as i32;
    let max_lines = ((ch as i32 - content_top - 16) / line_h).max(0) as usize;
    let (first, cursor_row, visible) = windowed_rows(content, max_lines);
    let i = line_idx.checked_sub(first)?;
    if i >= visible {
        return None;
    }
    let prefix = if i == cursor_row { "> " } else { "  " };
    Some((cx + 4, cy + content_top + i as i32 * line_h, prefix))
}

/// Theme metrics [`draw_content_windowed`] lays lines out with. Apps cache
/// them while drawing (their click handler gets no theme) and pass them to
/// [`windowed_line_at`].
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

/// The content line [`draw_content_windowed`] drew at content-local `ly`
/// in a window whose content area is `ch` tall (the inverse of
/// [`windowed_line_origin`]), or `None` when `ly` is outside the lines.
pub fn windowed_line_at(
    content: &ContentState,
    ch: u32,
    metrics: WindowedMetrics,
    ly: i32,
) -> Option<usize> {
    let has_context = content.viewing_file.is_some() || content.browse_dir.is_some();
    let content_top = if has_context {
        metrics.title_bar_height as i32
    } else {
        WINDOWED_TOP_PAD as i32
    };
    let line_h = metrics.line_h.max(1) as i32;
    let max_lines = ((ch as i32 - content_top - 16) / line_h).max(0) as usize;
    let (first, _, visible) = windowed_rows(content, max_lines);
    let y = ly - content_top;
    if y < 0 {
        return None;
    }
    let row = (y / line_h) as usize;
    (row < visible).then_some(first + row)
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
    let content_top = if let Some(ctx) = context {
        backend.draw_text(&ctx, cx + 4, cy + 2, 12, at.app.dim_text)?;
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
    let line_h = at.terminal_line_height.max(12) as i32;
    let max_lines = ((ch as i32 - content_top - 16) / line_h).max(0) as usize;
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
        backend.draw_text(&text, cx + 4, y, WINDOWED_FONT_SIZE, text_color)?;
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
    backend.draw_text(&scroll_text, cx + 4, scroll_y, 10, at.app.dim_text)?;

    Ok(())
}

/// Hide all generic app-related SDI objects.
///
/// This hides objects created by `render_app_chrome` and `render_content_sdi`.
/// App-specific objects (e.g., TV Guide EPG) should be hidden separately.
///
/// Runs every frame outside app mode, so it only touches objects whose
/// visibility actually changes (see `SdiRegistry::set_visible`): on an
/// already-hidden pool it is lookups only and leaves the scene clean.
pub fn hide_app_sdi(sdi: &mut SdiRegistry) {
    let fixed = [
        "app_bg",
        "app_title_bg",
        "app_title_text",
        "app_scroll",
        "app_divider",
        "app_sel_bg",
        "app_sel_accent",
    ];
    for name in fixed {
        sdi.set_visible(name, false);
    }
    let names = app_line_names();
    for name in &names.line {
        if !sdi.contains(name) {
            break;
        }
        sdi.set_visible(name, false);
    }
    for (lp, rp) in names.lp.iter().zip(&names.rp) {
        if !sdi.contains(lp) {
            break;
        }
        sdi.set_visible(lp, false);
        sdi.set_visible(rp, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oasis_types::backend::{Color, SdiCore, TextureId};
    use oasis_types::error::Result;

    /// Backend that records every `draw_text` call as `(text, x, y)`.
    #[derive(Default)]
    struct TextRecorder(Vec<(String, i32, i32)>);

    impl SdiCore for TextRecorder {
        fn init(&mut self, _w: u32, _h: u32) -> Result<()> {
            Ok(())
        }
        fn clear(&mut self, _color: Color) -> Result<()> {
            Ok(())
        }
        fn blit(&mut self, _t: TextureId, _x: i32, _y: i32, _w: u32, _h: u32) -> Result<()> {
            Ok(())
        }
        fn fill_rect(&mut self, _x: i32, _y: i32, _w: u32, _h: u32, _c: Color) -> Result<()> {
            Ok(())
        }
        fn draw_text(&mut self, t: &str, x: i32, y: i32, _fs: u16, _c: Color) -> Result<()> {
            self.0.push((t.to_string(), x, y));
            Ok(())
        }
        fn swap_buffers(&mut self) -> Result<()> {
            Ok(())
        }
        fn load_texture(&mut self, _w: u32, _h: u32, _d: &[u8]) -> Result<TextureId> {
            Ok(TextureId(0))
        }
        fn destroy_texture(&mut self, _t: TextureId) -> Result<()> {
            Ok(())
        }
        fn set_clip_rect(&mut self, _x: i32, _y: i32, _w: u32, _h: u32) -> Result<()> {
            Ok(())
        }
        fn reset_clip_rect(&mut self) -> Result<()> {
            Ok(())
        }
        fn measure_text(&self, _t: &str, _fs: u16) -> u32 {
            0
        }
        fn read_pixels(&self, _x: i32, _y: i32, _w: u32, _h: u32) -> Result<Vec<u8>> {
            Ok(vec![])
        }
        fn shutdown(&mut self) -> Result<()> {
            Ok(())
        }
    }

    impl oasis_types::backend::SdiShapes for TextRecorder {}
    impl oasis_types::backend::SdiGradients for TextRecorder {}
    impl oasis_types::backend::SdiAlpha for TextRecorder {}
    impl oasis_types::backend::SdiText for TextRecorder {}
    impl oasis_types::backend::SdiTextures for TextRecorder {}
    impl oasis_types::backend::SdiClipTransform for TextRecorder {}
    impl oasis_types::backend::SdiVector for TextRecorder {}
    impl oasis_types::backend::SdiBatch for TextRecorder {}
    impl oasis_types::backend::SdiRenderTarget for TextRecorder {}

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
    fn hide_app_sdi_repeat_leaves_scene_clean() {
        let mut content = ContentState::new("Files", "/apps/files");
        content.lines = (0..50).map(|i| format!("entry {i}")).collect();
        let at = ActiveTheme::default();
        let mut sdi = SdiRegistry::new();
        render_app_chrome(&mut sdi, &at);
        render_content_sdi(&content, &mut sdi, &at);
        hide_app_sdi(&mut sdi);
        assert!(sdi.take_scene_dirty());
        assert!(!sdi.get("app_line_0").unwrap().visible);
        assert!(!sdi.get("app_bg").unwrap().visible);
        // The per-frame repeat must not dirty the scene.
        hide_app_sdi(&mut sdi);
        assert!(!sdi.is_scene_dirty());
    }

    #[test]
    fn render_content_sdi_idle_frame_is_clean() {
        let mut content = ContentState::new("Files", "/apps/files");
        content.lines = (0..50).map(|i| format!("entry {i}")).collect();
        let at = ActiveTheme::default();
        let mut sdi = SdiRegistry::new();
        render_content_sdi(&content, &mut sdi, &at);
        sdi.clear_scene_dirty();
        render_content_sdi(&content, &mut sdi, &at);
        assert!(!sdi.is_scene_dirty());
        assert_eq!(
            sdi.get("app_line_1").unwrap().text.as_deref(),
            Some("entry 1")
        );
        // Scrolling changes line text: dirty.
        content.scroll = 1;
        render_content_sdi(&content, &mut sdi, &at);
        assert!(sdi.is_scene_dirty());
        assert_eq!(
            sdi.get("app_line_1").unwrap().text.as_deref(),
            Some("entry 2")
        );
    }

    #[test]
    fn windowed_draw_omits_app_title() {
        // The WM titlebar already shows the app title; drawing it again in
        // the content area produced a "double title bar" in every windowed
        // app. Content must instead start at WINDOWED_TOP_PAD.
        let mut content = ContentState::new("Paint", "/apps/paint");
        content.lines = vec!["first".into(), "second".into()];

        let mut rec = TextRecorder::default();
        let at = ActiveTheme::default();
        draw_content_windowed(&content, 0, 0, 300, 200, &mut rec, &at).unwrap();

        assert!(
            !rec.0.iter().any(|(t, _, _)| t.contains("Paint")),
            "windowed content must not repeat the app title: {:?}",
            rec.0
        );
        let first = rec.0.first().expect("content lines drawn");
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

        let mut rec = TextRecorder::default();
        let at = ActiveTheme::default();
        draw_content_windowed(&content, 0, 0, 300, 200, &mut rec, &at).unwrap();

        assert!(rec.0.iter().any(|(t, _, _)| t.contains("[/notes.txt]")));
        assert!(!rec.0.iter().any(|(t, _, _)| t.contains("File Manager")));
        // Content starts below the header row.
        let line = rec
            .0
            .iter()
            .find(|(t, _, _)| t.contains("hello"))
            .expect("content line drawn");
        assert_eq!(line.2, at.app.title_bar_height as i32);
    }
}
