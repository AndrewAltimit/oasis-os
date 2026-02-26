use crate::active_theme::ActiveTheme;
use crate::backend::TextureId;
use crate::bottombar::BottomBar;
use crate::sdi::SdiRegistry;

/// Maximum lines retained in the scrollback buffer.
pub const MAX_OUTPUT_LINES: usize = 2000;

/// Compute the number of visible output lines for the given theme.
///
/// Returns a value based on the available terminal area height and
/// the line spacing. Falls back to 12 (PSP default) when called
/// without theme information.
pub fn visible_output_lines(at: &ActiveTheme) -> usize {
    let top_y = at.statusbar_height as i32 + 2;
    let bot_y = at.screen_h as i32 - at.bottombar_height as i32;
    let bg_h = bot_y - top_y;
    // Reserve space for the input bar (20px) and a small gap (4px).
    let output_area = bg_h - 24;
    let line_h = at.terminal_line_height as i32;
    let lines = output_area / line_h.max(1);
    (lines.max(1) as usize).min(200)
}

/// Legacy constant for callers that don't have access to an `ActiveTheme`.
pub const VISIBLE_OUTPUT_LINES: usize = 12;

/// Set up the wallpaper SDI object at z=-1000 (behind everything).
pub fn setup_wallpaper(sdi: &mut SdiRegistry, tex: TextureId, w: u32, h: u32) {
    let obj = sdi.create("wallpaper");
    obj.x = 0;
    obj.y = 0;
    obj.w = w;
    obj.h = h;
    obj.texture = Some(tex);
    obj.z = -1000;
}

/// Update SDI objects for the currently selected media category page.
pub fn update_media_page(sdi: &mut SdiRegistry, bottom_bar: &BottomBar, at: &ActiveTheme) {
    let page_name = "media_page_text";
    if !sdi.contains(page_name) {
        let obj = sdi.create(page_name);
        obj.font_size = at.font_heading;
        obj.text_color = oasis_types::color::with_alpha(at.app_text, 200);
        obj.w = 0;
        obj.h = 0;
    }
    if let Ok(obj) = sdi.get_mut(page_name) {
        obj.x = (at.screen_w as i32) / 3;
        obj.y = (at.screen_h as i32) / 2 - 16;
        obj.visible = true;
        obj.text = Some(format!("[ {} Page ]", bottom_bar.active_tab.label()));
    }

    let hint_name = "media_page_hint";
    if !sdi.contains(hint_name) {
        let obj = sdi.create(hint_name);
        obj.font_size = at.font_hint;
        obj.text_color = at.app_dim_text;
        obj.w = 0;
        obj.h = 0;
    }
    if let Ok(obj) = sdi.get_mut(hint_name) {
        obj.x = (at.screen_w as i32) / 3 - 30;
        obj.y = (at.screen_h as i32) / 2 + 9;
        obj.visible = true;
        obj.text = Some("Press R to cycle categories".to_string());
    }
}

/// Hide media page SDI objects.
pub fn hide_media_page(sdi: &mut SdiRegistry) {
    for name in &["media_page_text", "media_page_hint"] {
        if let Ok(obj) = sdi.get_mut(name) {
            obj.visible = false;
        }
    }
}

/// Set terminal-mode SDI objects visible/hidden.
pub fn set_terminal_visible(sdi: &mut SdiRegistry, visible: bool) {
    if let Ok(obj) = sdi.get_mut("terminal_bg") {
        obj.visible = visible;
    }
    // Hide up to a generous upper bound of term lines (handles all resolutions).
    for i in 0..200 {
        let name = format!("term_line_{i}");
        if !sdi.contains(&name) {
            break;
        }
        if let Ok(obj) = sdi.get_mut(&name) {
            obj.visible = visible;
        }
    }
    if let Ok(obj) = sdi.get_mut("term_input_bg") {
        obj.visible = visible;
    }
    if let Ok(obj) = sdi.get_mut("term_prompt") {
        obj.visible = visible;
    }
}

/// Create/update terminal-mode SDI objects with theme-driven colors and layout.
pub fn setup_terminal_objects(
    sdi: &mut SdiRegistry,
    output_lines: &[String],
    cwd: &str,
    input_buf: &str,
    scroll_offset: usize,
    at: &ActiveTheme,
    cursor_visible: bool,
) {
    let margin = 4i32;
    let top_y = at.statusbar_height as i32 + 2;
    let bot_y = at.screen_h as i32 - at.bottombar_height as i32;
    let bg_w = at.screen_w - (margin * 2) as u32;
    let bg_h = (bot_y - top_y) as u32;
    let visible_lines = visible_output_lines(at);

    // Terminal background.
    if !sdi.contains("terminal_bg") {
        let obj = sdi.create("terminal_bg");
        obj.x = margin;
        obj.y = top_y;
        obj.w = bg_w;
        obj.h = bg_h;
        obj.color = at.app_bg;
        obj.border_radius = Some(at.terminal_border_radius);
        obj.stroke_width = Some(1);
        obj.stroke_color = Some(at.separator_color);
    }
    if let Ok(obj) = sdi.get_mut("terminal_bg") {
        obj.visible = true;
    }

    // Show visible lines from the scrollback buffer, offset by scroll.
    let end = output_lines.len().saturating_sub(scroll_offset);
    let start = end.saturating_sub(visible_lines);
    let output_color = oasis_types::color::with_alpha(at.terminal_output_color, 255);
    for i in 0..visible_lines {
        let name = format!("term_line_{i}");
        if !sdi.contains(&name) {
            let obj = sdi.create(&name);
            obj.x = margin + 4;
            obj.y = top_y + 2 + (i as i32) * at.terminal_line_height as i32;
            obj.font_size = at.font_body;
            obj.text_color = output_color;
            obj.w = 0;
            obj.h = 0;
        }
        if let Ok(obj) = sdi.get_mut(&name) {
            obj.text = output_lines.get(start + i).cloned();
            obj.visible = true;
        }
    }

    // Input bar background.
    let input_y = bot_y - 22;
    if !sdi.contains("term_input_bg") {
        let obj = sdi.create("term_input_bg");
        obj.x = margin;
        obj.y = input_y;
        obj.w = bg_w;
        obj.h = 20;
        obj.color = oasis_types::color::lighten(at.app_bg, 0.03);
        obj.border_radius = Some(at.input_border_radius);
    }
    if let Ok(obj) = sdi.get_mut("term_input_bg") {
        obj.visible = true;
    }

    // Prompt line.
    if !sdi.contains("term_prompt") {
        let obj = sdi.create("term_prompt");
        obj.x = margin + 4;
        obj.y = input_y + 2;
        obj.font_size = at.font_body;
        obj.text_color = at.terminal_prompt_color;
        obj.w = 0;
        obj.h = 0;
    }
    if let Ok(obj) = sdi.get_mut("term_prompt") {
        let cursor_char = if cursor_visible { '_' } else { ' ' };
        obj.text = Some(format!("{cwd}> {input_buf}{cursor_char}"));
        obj.visible = true;
    }
}

/// Paint a scrollbar on the right edge of the terminal background area.
///
/// Uses `fill_rect` directly on the backend. Layout is derived from
/// the active theme's screen dimensions and bar heights.
pub fn paint_terminal_scrollbar(
    backend: &mut dyn crate::backend::SdiBackend,
    total_lines: usize,
    scroll_offset: usize,
    at: &ActiveTheme,
) -> crate::error::Result<()> {
    let visible_lines = visible_output_lines(at);
    if total_lines <= visible_lines {
        return Ok(());
    }
    let margin = 4i32;
    let top_y = at.statusbar_height as i32 + 2;
    let bot_y = at.screen_h as i32 - at.bottombar_height as i32;
    let bg_w = at.screen_w - (margin * 2) as u32;
    let bg_h = (bot_y - top_y) as u32;

    let sb_w = at.scrollbar_width;
    let track_x: i32 = margin + bg_w as i32 - sb_w as i32 - 1;
    let track_y: i32 = top_y;
    let track_h: u32 = bg_h;

    // Track.
    backend.fill_rect(track_x, track_y, sb_w, track_h, at.scrollbar_track_color)?;

    // Thumb: proportional to visible/total ratio.
    let ratio = visible_lines as f32 / total_lines as f32;
    let thumb_h = ((track_h as f32 * ratio) as u32).max(12).min(track_h);
    let scrollable = track_h - thumb_h;
    let max_offset = total_lines - visible_lines;
    let frac = if max_offset > 0 {
        1.0 - (scroll_offset as f32 / max_offset as f32)
    } else {
        1.0
    };
    let thumb_y = track_y + (scrollable as f32 * frac) as i32;
    backend.fill_rect(track_x, thumb_y, sb_w, thumb_h, at.scrollbar_thumb_color)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants() {
        assert_eq!(VISIBLE_OUTPUT_LINES, 12);
        assert_eq!(MAX_OUTPUT_LINES, 2000);
        assert!(VISIBLE_OUTPUT_LINES < MAX_OUTPUT_LINES);
    }

    #[test]
    fn visible_lines_default_theme() {
        let at = ActiveTheme::default();
        let lines = visible_output_lines(&at);
        // Default 480x272: (272 - 24 - 2 - 24) area = 222, minus 24 = 198, /16 = 12
        assert_eq!(lines, 12);
    }

    #[test]
    fn visible_lines_large_screen() {
        let at = ActiveTheme::default().with_screen_size(800, 600);
        let lines = visible_output_lines(&at);
        // Much more space at 800x600.
        assert!(lines > 12);
    }

    // -- setup_wallpaper --

    #[test]
    fn setup_wallpaper_creates_object() {
        let mut sdi = SdiRegistry::new();
        setup_wallpaper(&mut sdi, TextureId(42), 480, 272);
        assert!(sdi.contains("wallpaper"));
        let obj = sdi.get("wallpaper").unwrap();
        assert_eq!(obj.x, 0);
        assert_eq!(obj.y, 0);
        assert_eq!(obj.w, 480);
        assert_eq!(obj.h, 272);
        assert_eq!(obj.texture, Some(TextureId(42)));
        assert_eq!(obj.z, -1000);
    }

    #[test]
    fn setup_wallpaper_custom_dimensions() {
        let mut sdi = SdiRegistry::new();
        setup_wallpaper(&mut sdi, TextureId(1), 1920, 1080);
        let obj = sdi.get("wallpaper").unwrap();
        assert_eq!(obj.w, 1920);
        assert_eq!(obj.h, 1080);
    }

    // -- update_media_page --

    #[test]
    fn update_media_page_creates_objects() {
        let mut sdi = SdiRegistry::new();
        let bb = BottomBar::new();
        let at = ActiveTheme::default();
        update_media_page(&mut sdi, &bb, &at);

        assert!(sdi.contains("media_page_text"));
        assert!(sdi.contains("media_page_hint"));

        let text_obj = sdi.get("media_page_text").unwrap();
        assert!(text_obj.visible);
        assert!(text_obj.text.as_ref().unwrap().contains("Page"));

        let hint_obj = sdi.get("media_page_hint").unwrap();
        assert!(hint_obj.visible);
        assert_eq!(
            hint_obj.text.as_deref(),
            Some("Press R to cycle categories")
        );
    }

    #[test]
    fn update_media_page_idempotent() {
        let mut sdi = SdiRegistry::new();
        let bb = BottomBar::new();
        let at = ActiveTheme::default();
        update_media_page(&mut sdi, &bb, &at);
        update_media_page(&mut sdi, &bb, &at);
        // Should not panic or duplicate objects.
        assert!(sdi.contains("media_page_text"));
        assert!(sdi.contains("media_page_hint"));
    }

    // -- hide_media_page --

    #[test]
    fn hide_media_page_hides_objects() {
        let mut sdi = SdiRegistry::new();
        let bb = BottomBar::new();
        let at = ActiveTheme::default();
        update_media_page(&mut sdi, &bb, &at);

        // Objects should be visible after update.
        assert!(sdi.get("media_page_text").unwrap().visible);
        assert!(sdi.get("media_page_hint").unwrap().visible);

        hide_media_page(&mut sdi);

        assert!(!sdi.get("media_page_text").unwrap().visible);
        assert!(!sdi.get("media_page_hint").unwrap().visible);
    }

    #[test]
    fn hide_media_page_noop_when_missing() {
        let mut sdi = SdiRegistry::new();
        // Should not panic when objects don't exist.
        hide_media_page(&mut sdi);
    }

    // -- set_terminal_visible --

    #[test]
    fn set_terminal_visible_toggles() {
        let mut sdi = SdiRegistry::new();
        let lines: Vec<String> = vec!["hello".to_string()];
        let at = ActiveTheme::default();
        setup_terminal_objects(&mut sdi, &lines, "/home", "ls", 0, &at, true);

        // All objects should be visible after setup.
        assert!(sdi.get("terminal_bg").unwrap().visible);
        assert!(sdi.get("term_prompt").unwrap().visible);

        set_terminal_visible(&mut sdi, false);
        assert!(!sdi.get("terminal_bg").unwrap().visible);
        assert!(!sdi.get("term_prompt").unwrap().visible);
        assert!(!sdi.get("term_input_bg").unwrap().visible);
        for i in 0..visible_output_lines(&at) {
            let name = format!("term_line_{i}");
            assert!(!sdi.get(&name).unwrap().visible);
        }

        set_terminal_visible(&mut sdi, true);
        assert!(sdi.get("terminal_bg").unwrap().visible);
        assert!(sdi.get("term_prompt").unwrap().visible);
    }

    #[test]
    fn set_terminal_visible_noop_when_missing() {
        let mut sdi = SdiRegistry::new();
        // Should not panic when objects don't exist.
        set_terminal_visible(&mut sdi, false);
        set_terminal_visible(&mut sdi, true);
    }

    // -- setup_terminal_objects --

    #[test]
    fn setup_terminal_objects_creates_all() {
        let mut sdi = SdiRegistry::new();
        let lines: Vec<String> = vec![];
        let at = ActiveTheme::default();
        setup_terminal_objects(&mut sdi, &lines, "/", "", 0, &at, true);

        assert!(sdi.contains("terminal_bg"));
        assert!(sdi.contains("term_input_bg"));
        assert!(sdi.contains("term_prompt"));
        for i in 0..visible_output_lines(&at) {
            assert!(sdi.contains(&format!("term_line_{i}")));
        }
    }

    #[test]
    fn setup_terminal_objects_prompt_format() {
        let mut sdi = SdiRegistry::new();
        let at = ActiveTheme::default();
        setup_terminal_objects(&mut sdi, &[], "/home/user", "cat foo.txt", 0, &at, true);

        let prompt = sdi.get("term_prompt").unwrap();
        assert_eq!(prompt.text.as_deref(), Some("/home/user> cat foo.txt_"));
    }

    #[test]
    fn setup_terminal_objects_scrollback_few_lines() {
        let mut sdi = SdiRegistry::new();
        let lines: Vec<String> = (0..3).map(|i| format!("line{i}")).collect();
        let at = ActiveTheme::default();
        setup_terminal_objects(&mut sdi, &lines, "/", "", 0, &at, true);

        // With 3 lines and VISIBLE=12, start=0. Lines 0-2 have text, rest None.
        assert_eq!(
            sdi.get("term_line_0").unwrap().text.as_deref(),
            Some("line0")
        );
        assert_eq!(
            sdi.get("term_line_2").unwrap().text.as_deref(),
            Some("line2")
        );
        assert!(sdi.get("term_line_3").unwrap().text.is_none());
    }

    #[test]
    fn setup_terminal_objects_scrollback_overflow() {
        let mut sdi = SdiRegistry::new();
        // 20 lines -- only last 12 should be visible.
        let lines: Vec<String> = (0..20).map(|i| format!("line{i}")).collect();
        let at = ActiveTheme::default();
        setup_terminal_objects(&mut sdi, &lines, "/", "", 0, &at, true);

        let visible = visible_output_lines(&at);
        // start = 20 - 12 = 8, so term_line_0 = lines[8]
        assert_eq!(
            sdi.get("term_line_0").unwrap().text.as_deref(),
            Some(&*format!("line{}", 20 - visible))
        );
        assert_eq!(
            sdi.get(&format!("term_line_{}", visible - 1))
                .unwrap()
                .text
                .as_deref(),
            Some("line19")
        );
    }

    #[test]
    fn setup_terminal_objects_idempotent() {
        let mut sdi = SdiRegistry::new();
        let lines = vec!["first".to_string()];
        let at = ActiveTheme::default();
        setup_terminal_objects(&mut sdi, &lines, "/", "a", 0, &at, true);

        let lines2 = vec!["second".to_string()];
        setup_terminal_objects(&mut sdi, &lines2, "/tmp", "b", 0, &at, true);

        // Should update text, not create duplicates.
        assert_eq!(
            sdi.get("term_line_0").unwrap().text.as_deref(),
            Some("second")
        );
        assert_eq!(
            sdi.get("term_prompt").unwrap().text.as_deref(),
            Some("/tmp> b_")
        );
    }

    #[test]
    fn setup_terminal_objects_bg_uses_theme() {
        let mut sdi = SdiRegistry::new();
        let at = ActiveTheme::default();
        setup_terminal_objects(&mut sdi, &[], "/", "", 0, &at, true);

        let bg = sdi.get("terminal_bg").unwrap();
        assert_eq!(bg.x, 4);
        assert_eq!(bg.border_radius, Some(at.terminal_border_radius));
        assert_eq!(bg.stroke_width, Some(1));
    }

    #[test]
    fn setup_terminal_objects_empty_input() {
        let mut sdi = SdiRegistry::new();
        let at = ActiveTheme::default();
        setup_terminal_objects(&mut sdi, &[], "/", "", 0, &at, true);

        let prompt = sdi.get("term_prompt").unwrap();
        assert_eq!(prompt.text.as_deref(), Some("/> _"));
    }
}
