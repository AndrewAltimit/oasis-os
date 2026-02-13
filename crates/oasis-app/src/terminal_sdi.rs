use oasis_core::backend::{Color, TextureId};
use oasis_core::bottombar::BottomBar;
use oasis_core::sdi::SdiRegistry;

/// Maximum lines visible in the terminal output area.
pub const MAX_OUTPUT_LINES: usize = 12;

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
    for i in 0..MAX_OUTPUT_LINES {
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

    for i in 0..MAX_OUTPUT_LINES {
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
            obj.text = output_lines.get(i).cloned();
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
