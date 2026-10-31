//! The dashboard's media category pages (the AUDIO / VIDEO / IMAGE / FILE
//! bottom-bar tabs): the files of the tab's directory listed as SDI rows
//! with file-type icons, or a centered empty-state message.

use crate::active_theme::ActiveTheme;
use crate::backend::Color;
use crate::bottombar::{BottomBar, MediaTab};
use crate::sdi::SdiRegistry;
use oasis_vfs::{EntryKind, Vfs};

/// VFS directory listed by each media category tab.
pub fn media_tab_dir(tab: MediaTab) -> Option<&'static str> {
    match tab {
        MediaTab::None => None,
        MediaTab::Audio => Some("/home/user/music"),
        MediaTab::Video => Some("/home/user/videos"),
        MediaTab::Image => Some("/home/user/photos"),
        MediaTab::File => Some("/home/user"),
    }
}

/// Kind of a listed media entry (drives the file-type icon).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    /// Sub-directory.
    Folder,
    /// Audio file (mp3, ogg, wav, ...).
    Audio,
    /// Video file (mp4, mkv, webm, ...).
    Video,
    /// Image file (png, jpg, gif, ...).
    Image,
    /// Anything else.
    Document,
}

impl MediaKind {
    /// Classify a file name by extension.
    pub fn from_name(name: &str) -> Self {
        let ext = name
            .rsplit_once('.')
            .map(|(_, e)| e.to_ascii_lowercase())
            .unwrap_or_default();
        match ext.as_str() {
            "mp3" | "ogg" | "wav" | "flac" | "m4a" | "aac" | "opus" => Self::Audio,
            "mp4" | "m4v" | "mkv" | "webm" | "avi" | "mov" => Self::Video,
            "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" => Self::Image,
            _ => Self::Document,
        }
    }
}

/// One row of a media category page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaEntry {
    /// File / directory name (basename).
    pub name: String,
    /// Classified type.
    pub kind: MediaKind,
    /// Size in bytes (0 for directories).
    pub size: u64,
}

/// List the entries a media tab shows: the matching files of its
/// directory (audio files for AUDIO, ...), or every entry for FILE.
/// Directories sort first, then names case-insensitively. A missing
/// directory yields an empty list.
pub fn list_media_entries(vfs: &dyn Vfs, tab: MediaTab) -> Vec<MediaEntry> {
    let Some(dir) = media_tab_dir(tab) else {
        return Vec::new();
    };
    let Ok(entries) = vfs.readdir(dir) else {
        return Vec::new();
    };
    let wanted = match tab {
        MediaTab::Audio => Some(MediaKind::Audio),
        MediaTab::Video => Some(MediaKind::Video),
        MediaTab::Image => Some(MediaKind::Image),
        MediaTab::File | MediaTab::None => None,
    };
    let mut out: Vec<MediaEntry> = entries
        .into_iter()
        .filter_map(|e| {
            let kind = if e.kind == EntryKind::Directory {
                MediaKind::Folder
            } else {
                MediaKind::from_name(&e.name)
            };
            let keep = match wanted {
                Some(w) => kind == w,
                None => true,
            };
            keep.then_some(MediaEntry {
                name: e.name,
                kind,
                size: e.size,
            })
        })
        .collect();
    out.sort_by(|a, b| {
        (a.kind != MediaKind::Folder)
            .cmp(&(b.kind != MediaKind::Folder))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    out
}

/// Empty-state message for a media tab with nothing to list.
pub fn media_empty_message(tab: MediaTab) -> String {
    let what = match tab {
        MediaTab::Audio => "audio files",
        MediaTab::Video => "videos",
        MediaTab::Image => "images",
        MediaTab::File | MediaTab::None => "files",
    };
    match media_tab_dir(tab) {
        Some(dir) => format!("No {what} in {dir}"),
        None => format!("No {what}"),
    }
}

/// Maximum rows a media page lays out (the rest collapse into "+N more").
const MAX_MEDIA_ROWS: usize = 32;
/// Hint shown at the bottom of every media page.
const MEDIA_HINT: &str = "Press R to cycle categories";
/// Every fixed media-page object name (row objects are generated).
const MEDIA_FIXED_OBJECTS: [&str; 6] = [
    "media_page_bg",
    "media_page_text",
    "media_page_path",
    "media_page_rule",
    "media_page_empty",
    "media_page_hint",
];

/// Rendered pixel width of `s` in the shared bitmap font.
fn media_text_px(s: &str, font_size: u16) -> i32 {
    oasis_types::backend::bitmap_measure_text(s, font_size) as i32
}

/// X that horizontally centers `s` on a `screen_w`-wide screen.
fn centered_x(s: &str, font_size: u16, screen_w: u32) -> i32 {
    (screen_w as i32 - media_text_px(s, font_size)) / 2
}

/// Create-or-update a text-only SDI object.
fn media_text(
    sdi: &mut SdiRegistry,
    name: &str,
    text: &str,
    (x, y): (i32, i32),
    font_size: u16,
    color: Color,
) {
    if !sdi.contains(name) {
        let obj = sdi.create(name);
        obj.w = 0;
        obj.h = 0;
    }
    if let Ok(obj) = sdi.get_mut(name) {
        obj.x = x;
        obj.y = y;
        obj.font_size = font_size;
        obj.text_color = color;
        obj.visible = true;
        obj.set_text(text);
    }
}

/// Create-or-update a filled rectangle SDI object.
fn media_rect(sdi: &mut SdiRegistry, name: &str, rect: (i32, i32, u32, u32), color: Color) {
    if !sdi.contains(name) {
        sdi.create(name);
    }
    if let Ok(obj) = sdi.get_mut(name) {
        (obj.x, obj.y, obj.w, obj.h) = rect;
        obj.color = color;
        obj.border_radius = Some(2);
        obj.visible = true;
    }
}

/// Icon tile color for a media kind, from the skin's semantic colors.
fn media_kind_color(kind: MediaKind, at: &ActiveTheme) -> Color {
    let ui = &at.ui_theme;
    match kind {
        MediaKind::Folder => ui.warning,
        MediaKind::Audio => ui.accent,
        MediaKind::Video => ui.error,
        MediaKind::Image => ui.success,
        MediaKind::Document => ui.info,
    }
}

/// Short type badge drawn on the icon tile.
fn media_kind_badge(entry: &MediaEntry) -> String {
    if entry.kind == MediaKind::Folder {
        return "DIR".to_string();
    }
    match entry.name.rsplit_once('.') {
        Some((_, ext)) if !ext.is_empty() => ext.chars().take(3).collect::<String>().to_uppercase(),
        _ => "---".to_string(),
    }
}

/// Black or white, whichever reads better on `bg`.
fn readable_on(bg: Color) -> Color {
    if oasis_types::color::relative_luminance(bg) > 0.4 {
        Color::rgb(0, 0, 0)
    } else {
        Color::rgb(255, 255, 255)
    }
}

/// Human-readable size readout.
fn media_size_label(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{} KB", bytes / 1024)
    } else {
        format!("{bytes} B")
    }
}

/// Update SDI objects for the currently selected media category page.
///
/// Without VFS access only the category header and hint are shown; hosts
/// with a VFS should call [`update_media_page_with_vfs`] to list files.
pub fn update_media_page(sdi: &mut SdiRegistry, bottom_bar: &BottomBar, at: &ActiveTheme) {
    render_media_page(sdi, bottom_bar.active_tab, at, None);
}

/// Update the media category page, listing the tab's matching files
/// from `vfs` with file-type icons (or a centered empty-state message).
pub fn update_media_page_with_vfs(
    sdi: &mut SdiRegistry,
    bottom_bar: &BottomBar,
    at: &ActiveTheme,
    vfs: &dyn Vfs,
) {
    let entries = list_media_entries(vfs, bottom_bar.active_tab);
    render_media_page(sdi, bottom_bar.active_tab, at, Some(&entries));
}

fn render_media_page(
    sdi: &mut SdiRegistry,
    tab: MediaTab,
    at: &ActiveTheme,
    entries: Option<&[MediaEntry]>,
) {
    let pad = 12i32;
    let top = (at.statusbar_height + at.tab_row_height) as i32 + 6;
    let bottom = at.screen_h as i32 - at.bottombar_height as i32 - 4;
    let (font_heading, font_body, font_hint) = (at.font_heading, at.font_body, at.font_hint);

    // Panel in the app-screen background so the app text colors below
    // sit on the surface they were designed for (not the wallpaper).
    media_rect(
        sdi,
        "media_page_bg",
        (
            pad / 2,
            top - 4,
            at.screen_w.saturating_sub(pad as u32),
            (bottom - top + 6).max(0) as u32,
        ),
        at.app.bg,
    );
    if let Ok(obj) = sdi.get_mut("media_page_bg") {
        obj.z = -1;
        obj.border_radius = Some(at.ui_theme.border_radius_md);
    }

    // Header: category name + source directory, then a divider rule.
    media_text(
        sdi,
        "media_page_text",
        tab.label(),
        (pad, top),
        font_heading,
        at.app.text,
    );
    let mut y = top + font_heading as i32 + 4;
    let dir = media_tab_dir(tab).unwrap_or("");
    let path_line = match entries {
        Some(list) => format!("{dir}  ({} items)", list.len()),
        None => dir.to_string(),
    };
    media_text(
        sdi,
        "media_page_path",
        &path_line,
        (pad, y),
        font_hint,
        at.app.dim_text,
    );
    y += font_hint as i32 + 4;
    let rule_w = at.screen_w.saturating_sub(pad as u32 * 2);
    media_rect(sdi, "media_page_rule", (pad, y, rule_w, 1), at.app.divider);
    y += 6;

    // Footer hint, centered by measured width.
    let hint_y = bottom - font_hint as i32 - 2;
    media_text(
        sdi,
        "media_page_hint",
        MEDIA_HINT,
        (centered_x(MEDIA_HINT, font_hint, at.screen_w), hint_y),
        font_hint,
        at.app.dim_text,
    );

    let list = entries.unwrap_or(&[]);
    if entries.is_some() && list.is_empty() {
        let msg = media_empty_message(tab);
        let msg_y = y + (hint_y - y - font_body as i32) / 2;
        media_text(
            sdi,
            "media_page_empty",
            &msg,
            (centered_x(&msg, font_body, at.screen_w), msg_y),
            font_body,
            at.app.dim_text,
        );
    } else {
        sdi.set_visible("media_page_empty", false);
    }

    // Rows: icon tile + badge, name, size.
    let icon_h = (font_body as u32 + 6).max(12);
    let badge_fs = (font_body.saturating_sub(2)).max(6);
    // Wide enough for any 3-letter badge plus a 2px margin each side.
    let icon_w = (icon_h + icon_h / 2).max(media_text_px("MMM", badge_fs) as u32 + 4);
    let row_h = icon_h as i32 + 4;
    let capacity = ((hint_y - 4 - y) / row_h).max(0) as usize;
    let capacity = capacity.min(MAX_MEDIA_ROWS);
    let overflow = list.len() > capacity;
    let shown = if overflow {
        capacity.saturating_sub(1)
    } else {
        list.len()
    };
    for (i, entry) in list.iter().take(shown).enumerate() {
        let ry = y + i as i32 * row_h;
        let tile = media_kind_color(entry.kind, at);
        media_rect(
            sdi,
            &format!("media_row_{i}_icon"),
            (pad, ry, icon_w, icon_h),
            tile,
        );
        let badge = media_kind_badge(entry);
        let bx = pad + (icon_w as i32 - media_text_px(&badge, badge_fs)) / 2;
        let by = ry + (icon_h as i32 - badge_fs as i32) / 2;
        media_text(
            sdi,
            &format!("media_row_{i}_badge"),
            &badge,
            (bx, by),
            badge_fs,
            readable_on(tile),
        );
        let ty = ry + (icon_h as i32 - font_body as i32) / 2;
        media_text(
            sdi,
            &format!("media_row_{i}_name"),
            &entry.name,
            (pad + icon_w as i32 + 8, ty),
            font_body,
            at.app.text,
        );
        let meta = if entry.kind == MediaKind::Folder {
            String::new()
        } else {
            media_size_label(entry.size)
        };
        let mx = at.screen_w as i32 - pad - media_text_px(&meta, font_hint);
        media_text(
            sdi,
            &format!("media_row_{i}_meta"),
            &meta,
            (mx, ty),
            font_hint,
            at.app.dim_text,
        );
    }
    if overflow {
        let more = format!("+{} more", list.len() - shown);
        let ry = y + shown as i32 * row_h + (row_h - font_hint as i32) / 2;
        media_text(
            sdi,
            "media_page_more",
            &more,
            (pad, ry),
            font_hint,
            at.app.dim_text,
        );
    } else {
        sdi.set_visible("media_page_more", false);
    }
    hide_media_rows(sdi, shown);
}

/// Hide row objects from index `from` onward.
fn hide_media_rows(sdi: &mut SdiRegistry, from: usize) {
    for i in from..MAX_MEDIA_ROWS {
        let icon = format!("media_row_{i}_icon");
        if !sdi.contains(&icon) {
            // Rows are created in order, so nothing beyond this exists.
            break;
        }
        sdi.set_visible(&icon, false);
        for part in ["badge", "name", "meta"] {
            sdi.set_visible(&format!("media_row_{i}_{part}"), false);
        }
    }
}

/// Hide media page SDI objects.
pub fn hide_media_page(sdi: &mut SdiRegistry) {
    for name in MEDIA_FIXED_OBJECTS {
        sdi.set_visible(name, false);
    }
    sdi.set_visible("media_page_more", false);
    hide_media_rows(sdi, 0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use oasis_vfs::MemoryVfs;

    fn media_vfs() -> MemoryVfs {
        let mut vfs = MemoryVfs::new();
        for d in [
            "/home",
            "/home/user",
            "/home/user/music",
            "/home/user/photos",
        ] {
            vfs.mkdir(d).expect("mkdir");
        }
        vfs.write("/home/user/music/b_song.mp3", &[0u8; 2048])
            .expect("write");
        vfs.write("/home/user/music/A_tune.ogg", b"ogg")
            .expect("write");
        vfs.write("/home/user/music/notes.txt", b"not audio")
            .expect("write");
        vfs.write("/home/user/readme.txt", b"hi").expect("write");
        vfs
    }

    fn page(tab: MediaTab, vfs: &MemoryVfs) -> SdiRegistry {
        let mut sdi = SdiRegistry::new();
        let mut bar = BottomBar::new();
        bar.active_tab = tab;
        let at = ActiveTheme::default();
        update_media_page_with_vfs(&mut sdi, &bar, &at, vfs);
        sdi
    }

    fn text_of(sdi: &SdiRegistry, name: &str) -> Option<String> {
        sdi.get(name)
            .ok()
            .filter(|o| o.visible)
            .and_then(|o| o.text.clone())
    }

    #[test]
    fn media_tab_lists_matching_files_only() {
        let vfs = media_vfs();
        let entries = list_media_entries(&vfs, MediaTab::Audio);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["A_tune.ogg", "b_song.mp3"], "sorted, txt filtered");
        assert!(entries.iter().all(|e| e.kind == MediaKind::Audio));

        // FILE lists everything, folders first.
        let files = list_media_entries(&vfs, MediaTab::File);
        assert_eq!(files[0].kind, MediaKind::Folder);
        assert!(files.iter().any(|e| e.name == "readme.txt"));
    }

    #[test]
    fn media_page_renders_rows_with_type_icons() {
        let vfs = media_vfs();
        let sdi = page(MediaTab::Audio, &vfs);
        assert_eq!(text_of(&sdi, "media_page_text").as_deref(), Some("AUDIO"));
        assert_eq!(
            text_of(&sdi, "media_row_0_name").as_deref(),
            Some("A_tune.ogg")
        );
        assert_eq!(
            text_of(&sdi, "media_row_1_name").as_deref(),
            Some("b_song.mp3")
        );
        assert_eq!(text_of(&sdi, "media_row_1_badge").as_deref(), Some("MP3"));
        assert_eq!(text_of(&sdi, "media_row_1_meta").as_deref(), Some("2 KB"));
        let icon = sdi.get("media_row_0_icon").expect("icon");
        assert!(icon.visible && icon.w > 0 && icon.h > 0);
        assert!(text_of(&sdi, "media_row_2_name").is_none());
        assert!(text_of(&sdi, "media_page_empty").is_none());
    }

    #[test]
    fn empty_media_tab_shows_centered_empty_state() {
        let vfs = media_vfs();
        // No /home/user/videos directory at all.
        let sdi = page(MediaTab::Video, &vfs);
        let msg = text_of(&sdi, "media_page_empty").expect("empty-state text");
        assert_eq!(msg, media_empty_message(MediaTab::Video));
        let at = ActiveTheme::default();
        let obj = sdi.get("media_page_empty").expect("obj");
        let w = oasis_types::backend::bitmap_measure_text(&msg, obj.font_size) as i32;
        assert!(
            (obj.x - (at.screen_w as i32 - w) / 2).abs() <= 1,
            "empty-state must be centered by measured width"
        );
        assert!(sdi.get("media_row_0_icon").is_err(), "no rows");
    }

    #[test]
    fn switching_tabs_hides_stale_rows() {
        let vfs = media_vfs();
        let mut sdi = page(MediaTab::Audio, &vfs);
        let mut bar = BottomBar::new();
        bar.active_tab = MediaTab::Image; // empty photos dir
        update_media_page_with_vfs(&mut sdi, &bar, &ActiveTheme::default(), &vfs);
        assert!(text_of(&sdi, "media_row_0_name").is_none());
        assert!(text_of(&sdi, "media_page_empty").is_some());
        hide_media_page(&mut sdi);
        assert!(text_of(&sdi, "media_page_empty").is_none());
        assert!(text_of(&sdi, "media_page_text").is_none());
    }

    // -- update_media_page --

    #[test]
    fn update_media_page_creates_objects() {
        let mut sdi = SdiRegistry::new();
        let mut bb = BottomBar::new();
        bb.active_tab = MediaTab::Audio;
        let at = ActiveTheme::default();
        update_media_page(&mut sdi, &bb, &at);

        assert!(sdi.contains("media_page_text"));
        assert!(sdi.contains("media_page_hint"));

        // The header names the category (no "[ AUDIO Page ]" placeholder).
        let text_obj = sdi.get("media_page_text").unwrap();
        assert!(text_obj.visible);
        assert_eq!(text_obj.text.as_deref(), Some("AUDIO"));
        // Without a VFS no (possibly wrong) empty-state is claimed.
        assert!(!sdi.contains("media_page_empty"));

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
}
