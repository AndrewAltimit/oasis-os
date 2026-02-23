use oasis_core::backend::{Color, TextureId};
use oasis_core::bottombar::BottomBar;
use oasis_core::sdi::SdiRegistry;

/// Maximum lines visible in the terminal output area (display limit).
pub const VISIBLE_OUTPUT_LINES: usize = 12;

/// Maximum lines retained in the scrollback buffer.
pub const MAX_OUTPUT_LINES: usize = 2000;

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
pub fn update_media_page(sdi: &mut SdiRegistry, bottom_bar: &BottomBar) {
    let page_name = "media_page_text";
    if !sdi.contains(page_name) {
        let obj = sdi.create(page_name);
        obj.font_size = 14;
        obj.text_color = Color::rgb(160, 200, 180);
        obj.w = 0;
        obj.h = 0;
    }
    if let Ok(obj) = sdi.get_mut(page_name) {
        obj.x = 160;
        obj.y = 120;
        obj.visible = true;
        obj.text = Some(format!("[ {} Page ]", bottom_bar.active_tab.label()));
    }

    let hint_name = "media_page_hint";
    if !sdi.contains(hint_name) {
        let obj = sdi.create(hint_name);
        obj.font_size = 10;
        obj.text_color = Color::rgb(100, 130, 110);
        obj.w = 0;
        obj.h = 0;
    }
    if let Ok(obj) = sdi.get_mut(hint_name) {
        obj.x = 130;
        obj.y = 145;
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
    for i in 0..VISIBLE_OUTPUT_LINES {
        let name = format!("term_line_{i}");
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

/// Create/update terminal-mode SDI objects.
pub fn setup_terminal_objects(
    sdi: &mut SdiRegistry,
    output_lines: &[String],
    cwd: &str,
    input_buf: &str,
    scroll_offset: usize,
) {
    if !sdi.contains("terminal_bg") {
        let obj = sdi.create("terminal_bg");
        obj.x = 4;
        obj.y = 26;
        obj.w = 472;
        obj.h = 220;
        obj.color = Color::rgb(12, 12, 20);
        obj.border_radius = Some(4);
        obj.stroke_width = Some(1);
        obj.stroke_color = Some(Color::rgba(255, 255, 255, 30));
    }
    if let Ok(obj) = sdi.get_mut("terminal_bg") {
        obj.visible = true;
    }

    // Show VISIBLE_OUTPUT_LINES from the scrollback buffer, offset by scroll.
    let end = output_lines.len().saturating_sub(scroll_offset);
    let start = end.saturating_sub(VISIBLE_OUTPUT_LINES);
    for i in 0..VISIBLE_OUTPUT_LINES {
        let name = format!("term_line_{i}");
        if !sdi.contains(&name) {
            let obj = sdi.create(&name);
            obj.x = 8;
            obj.y = 28 + (i as i32) * 16;
            obj.font_size = 12;
            obj.text_color = Color::rgb(0, 200, 0);
            obj.w = 0;
            obj.h = 0;
        }
        if let Ok(obj) = sdi.get_mut(&name) {
            obj.text = output_lines.get(start + i).cloned();
            obj.visible = true;
        }
    }

    if !sdi.contains("term_input_bg") {
        let obj = sdi.create("term_input_bg");
        obj.x = 4;
        obj.y = 248;
        obj.w = 472;
        obj.h = 20;
        obj.color = Color::rgb(20, 20, 35);
        obj.border_radius = Some(3);
    }
    if let Ok(obj) = sdi.get_mut("term_input_bg") {
        obj.visible = true;
    }

    if !sdi.contains("term_prompt") {
        let obj = sdi.create("term_prompt");
        obj.x = 8;
        obj.y = 250;
        obj.font_size = 12;
        obj.text_color = Color::rgb(100, 200, 255);
        obj.w = 0;
        obj.h = 0;
    }
    if let Ok(obj) = sdi.get_mut("term_prompt") {
        obj.text = Some(format!("{cwd}> {input_buf}_"));
        obj.visible = true;
    }
}

/// Paint a scrollbar on the right edge of the terminal background area.
///
/// Uses `fill_rect` directly on the backend. The terminal background is at
/// (4, 26, 472, 220). The scrollbar sits on the right edge.
pub fn paint_terminal_scrollbar(
    backend: &mut dyn oasis_core::backend::SdiBackend,
    total_lines: usize,
    scroll_offset: usize,
) -> oasis_core::error::Result<()> {
    if total_lines <= VISIBLE_OUTPUT_LINES {
        return Ok(());
    }
    let sb_w: u32 = 6;
    let track_x: i32 = 4 + 472 - sb_w as i32 - 1;
    let track_y: i32 = 26;
    let track_h: u32 = 220;

    // Track.
    backend.fill_rect(
        track_x,
        track_y,
        sb_w,
        track_h,
        Color::rgba(255, 255, 255, 20),
    )?;

    // Thumb: proportional to visible/total ratio.
    let ratio = VISIBLE_OUTPUT_LINES as f32 / total_lines as f32;
    let thumb_h = ((track_h as f32 * ratio) as u32).max(12).min(track_h);
    let scrollable = track_h - thumb_h;
    let max_offset = total_lines - VISIBLE_OUTPUT_LINES;
    let frac = if max_offset > 0 {
        1.0 - (scroll_offset as f32 / max_offset as f32)
    } else {
        1.0
    };
    let thumb_y = track_y + (scrollable as f32 * frac) as i32;
    backend.fill_rect(
        track_x,
        thumb_y,
        sb_w,
        thumb_h,
        Color::rgba(255, 255, 255, 100),
    )?;
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
        update_media_page(&mut sdi, &bb);

        assert!(sdi.contains("media_page_text"));
        assert!(sdi.contains("media_page_hint"));

        let text_obj = sdi.get("media_page_text").unwrap();
        assert_eq!(text_obj.x, 160);
        assert_eq!(text_obj.y, 120);
        assert!(text_obj.visible);
        assert!(text_obj.text.as_ref().unwrap().contains("Page"));

        let hint_obj = sdi.get("media_page_hint").unwrap();
        assert_eq!(hint_obj.x, 130);
        assert_eq!(hint_obj.y, 145);
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
        update_media_page(&mut sdi, &bb);
        update_media_page(&mut sdi, &bb);
        // Should not panic or duplicate objects.
        assert!(sdi.contains("media_page_text"));
        assert!(sdi.contains("media_page_hint"));
    }

    // -- hide_media_page --

    #[test]
    fn hide_media_page_hides_objects() {
        let mut sdi = SdiRegistry::new();
        let bb = BottomBar::new();
        update_media_page(&mut sdi, &bb);

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
        setup_terminal_objects(&mut sdi, &lines, "/home", "ls", 0);

        // All objects should be visible after setup.
        assert!(sdi.get("terminal_bg").unwrap().visible);
        assert!(sdi.get("term_prompt").unwrap().visible);

        set_terminal_visible(&mut sdi, false);
        assert!(!sdi.get("terminal_bg").unwrap().visible);
        assert!(!sdi.get("term_prompt").unwrap().visible);
        assert!(!sdi.get("term_input_bg").unwrap().visible);
        for i in 0..VISIBLE_OUTPUT_LINES {
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
        setup_terminal_objects(&mut sdi, &lines, "/", "", 0);

        assert!(sdi.contains("terminal_bg"));
        assert!(sdi.contains("term_input_bg"));
        assert!(sdi.contains("term_prompt"));
        for i in 0..VISIBLE_OUTPUT_LINES {
            assert!(sdi.contains(&format!("term_line_{i}")));
        }
    }

    #[test]
    fn setup_terminal_objects_prompt_format() {
        let mut sdi = SdiRegistry::new();
        setup_terminal_objects(&mut sdi, &[], "/home/user", "cat foo.txt", 0);

        let prompt = sdi.get("term_prompt").unwrap();
        assert_eq!(prompt.text.as_deref(), Some("/home/user> cat foo.txt_"));
    }

    #[test]
    fn setup_terminal_objects_scrollback_few_lines() {
        let mut sdi = SdiRegistry::new();
        let lines: Vec<String> = (0..3).map(|i| format!("line{i}")).collect();
        setup_terminal_objects(&mut sdi, &lines, "/", "", 0);

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
        setup_terminal_objects(&mut sdi, &lines, "/", "", 0);

        // start = 20 - 12 = 8, so term_line_0 = lines[8]
        assert_eq!(
            sdi.get("term_line_0").unwrap().text.as_deref(),
            Some("line8")
        );
        assert_eq!(
            sdi.get("term_line_11").unwrap().text.as_deref(),
            Some("line19")
        );
    }

    #[test]
    fn setup_terminal_objects_idempotent() {
        let mut sdi = SdiRegistry::new();
        let lines = vec!["first".to_string()];
        setup_terminal_objects(&mut sdi, &lines, "/", "a", 0);

        let lines2 = vec!["second".to_string()];
        setup_terminal_objects(&mut sdi, &lines2, "/tmp", "b", 0);

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
    fn setup_terminal_objects_bg_properties() {
        let mut sdi = SdiRegistry::new();
        setup_terminal_objects(&mut sdi, &[], "/", "", 0);

        let bg = sdi.get("terminal_bg").unwrap();
        assert_eq!(bg.x, 4);
        assert_eq!(bg.y, 26);
        assert_eq!(bg.w, 472);
        assert_eq!(bg.h, 220);
        assert_eq!(bg.border_radius, Some(4));
        assert_eq!(bg.stroke_width, Some(1));
    }

    #[test]
    fn setup_terminal_objects_line_positions() {
        let mut sdi = SdiRegistry::new();
        setup_terminal_objects(&mut sdi, &[], "/", "", 0);

        for i in 0..VISIBLE_OUTPUT_LINES {
            let obj = sdi.get(&format!("term_line_{i}")).unwrap();
            assert_eq!(obj.x, 8);
            assert_eq!(obj.y, 28 + (i as i32) * 16);
            assert_eq!(obj.font_size, 12);
        }
    }

    #[test]
    fn setup_terminal_objects_empty_input() {
        let mut sdi = SdiRegistry::new();
        setup_terminal_objects(&mut sdi, &[], "/", "", 0);

        let prompt = sdi.get("term_prompt").unwrap();
        assert_eq!(prompt.text.as_deref(), Some("/> _"));
    }
}
