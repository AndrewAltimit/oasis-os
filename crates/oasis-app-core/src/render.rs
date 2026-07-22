//! Shared rendering helpers for app chrome (title bar, content, selection).

use oasis_sdi::SdiRegistry;
use oasis_skin::ActiveTheme;
use oasis_types::backend::SdiBackend;
use oasis_ui::flex;

use crate::app_trait::ContentState;
use crate::layout::AppLayout;

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
        let name = format!("app_line_{i}");
        if !sdi.contains(&name) {
            sdi.create(&name);
        }
        if let Ok(obj) = sdi.get_mut(&name) {
            let line_idx = content.scroll + i;
            if line_idx < content.lines.len() {
                obj.text = Some(content.lines[line_idx].clone());
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
    let visible = content
        .lines
        .len()
        .saturating_sub(content.scroll)
        .min(max_lines);
    for i in 0..visible {
        let line_idx = content.scroll + i;
        let line = &content.lines[line_idx];
        let prefix = if i == content.cursor { "> " } else { "  " };
        let text = format!("{prefix}{line}");
        let text_color = if i == content.cursor {
            at.app.selected_text
        } else {
            at.app.text
        };
        let y = cy + content_top + i as i32 * line_h;
        backend.draw_text(&text, cx + 4, y, 12, text_color)?;
    }

    // Scroll indicator.
    let scroll_text = if content.lines.len() > max_lines {
        format!(
            "[{}/{}]  Cancel=back",
            content.scroll + 1,
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
    for name in &fixed {
        if let Ok(obj) = sdi.get_mut(name) {
            obj.visible = false;
        }
    }
    for i in 0..100 {
        let name = format!("app_line_{i}");
        if !sdi.contains(&name) {
            break;
        }
        if let Ok(obj) = sdi.get_mut(&name) {
            obj.visible = false;
        }
    }
    for i in 0..100 {
        let lp = format!("app_lp_line_{i}");
        if !sdi.contains(&lp) {
            break;
        }
        let rp = format!("app_rp_line_{i}");
        if let Ok(obj) = sdi.get_mut(&lp) {
            obj.visible = false;
        }
        if let Ok(obj) = sdi.get_mut(&rp) {
            obj.visible = false;
        }
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
