use crate::active_theme::ActiveTheme;
use crate::backend::Color;
use crate::backend::TextureId;
use crate::bottombar::{BottomBar, MediaTab};
use crate::sdi::SdiRegistry;
use oasis_vfs::{EntryKind, Vfs};

/// Maximum lines retained in the scrollback buffer.
pub const MAX_OUTPUT_LINES: usize = 2000;

/// Trim a scrollback buffer to [`MAX_OUTPUT_LINES`], dropping the oldest
/// lines with a single `drain` (one O(n) shift; a `remove(0)` loop was
/// O(n*k) when a command printed k lines into a full buffer).
pub fn trim_scrollback(output_lines: &mut Vec<String>) {
    let excess = output_lines.len().saturating_sub(MAX_OUTPUT_LINES);
    if excess > 0 {
        output_lines.drain(..excess);
    }
}

/// Resolved terminal colors, honoring `[app_themes.terminal]` skin overrides.
///
/// Each slot falls back to the exact theme-derived color the terminal used
/// before per-app overrides existed, so skins without an
/// `[app_themes.terminal]` section render pixel-identically.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalColors {
    /// Terminal background fill (key: `bg`).
    pub bg: Color,
    /// 1px border stroke around the terminal background (key: `border`).
    pub border: Color,
    /// Scrollback output text (key: `output`).
    pub output: Color,
    /// Input bar background (key: `input_bg`).
    pub input_bg: Color,
    /// Prompt line text: cwd, typed input, and cursor (key: `prompt`).
    pub prompt: Color,
    /// Scrollbar track (key: `scrollbar_track`).
    pub scrollbar_track: Color,
    /// Scrollbar thumb (key: `scrollbar_thumb`).
    pub scrollbar_thumb: Color,
}

impl TerminalColors {
    /// Build colors from the active theme, using `[app_themes.terminal]`
    /// overrides where present.
    pub fn from_theme(at: &ActiveTheme) -> Self {
        let c = |key: &str, default: Color| at.app_color("terminal", key).unwrap_or(default);
        Self {
            bg: c("bg", at.app.bg),
            border: c("border", at.bar.separator_color),
            output: c(
                "output",
                oasis_types::color::with_alpha(at.app.terminal_output_color, 255),
            ),
            input_bg: c("input_bg", oasis_types::color::lighten(at.app.bg, 0.03)),
            prompt: c("prompt", at.app.terminal_prompt_color),
            scrollbar_track: c("scrollbar_track", at.scrollbar.track_color),
            scrollbar_thumb: c("scrollbar_thumb", at.scrollbar.thumb_color),
        }
    }
}

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

// -- Media category pages (AUDIO / VIDEO / IMAGE / FILE bottom-bar tabs) --

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

/// Maximum extra colored-run objects per terminal line (`term_line_{i}_r{j}`).
const MAX_LINE_RUNS: usize = 8;

/// Upper bound on `term_line_{i}` objects the visibility pass walks
/// (generous enough for every supported resolution).
const MAX_TERM_LINES: usize = 200;

/// Cached `term_line_{i}` / `term_line_{i}_r{j}` object names.
///
/// `set_terminal_visible(sdi, false)` runs every frame in every non-terminal
/// mode; formatting up to 200 x 9 names per frame was pure churn.
/// `runs[i][j - 1]` is the name of run `j` of line `i`.
struct TermNames {
    lines: Vec<String>,
    runs: Vec<[String; MAX_LINE_RUNS]>,
}

fn term_names() -> &'static TermNames {
    static NAMES: std::sync::OnceLock<TermNames> = std::sync::OnceLock::new();
    NAMES.get_or_init(|| TermNames {
        lines: (0..MAX_TERM_LINES)
            .map(|i| format!("term_line_{i}"))
            .collect(),
        runs: (0..MAX_TERM_LINES)
            .map(|i| std::array::from_fn(|j| format!("term_line_{i}_r{}", j + 1)))
            .collect(),
    })
}

/// Set terminal-mode SDI objects visible/hidden.
///
/// Only objects whose visibility actually changes are touched, so the
/// per-frame hide pass in non-terminal modes leaves the scene clean.
/// Hiding is also short-circuited: every path that shows terminal
/// objects (`setup_terminal_objects`, this function) shows `terminal_bg`,
/// `term_prompt`, and `term_line_0` together, so when all three are
/// already hidden the walk over the line pool is skipped.
pub fn set_terminal_visible(sdi: &mut SdiRegistry, visible: bool) {
    let names = term_names();
    if !visible {
        let hidden = |name: &str| sdi.get(name).ok().is_none_or(|o| !o.visible);
        if hidden("terminal_bg") && hidden("term_prompt") && hidden(&names.lines[0]) {
            return;
        }
    }
    sdi.set_visible("terminal_bg", visible);
    for (line, runs) in names.lines.iter().zip(&names.runs) {
        if !sdi.contains(line) {
            break;
        }
        sdi.set_visible(line, visible);
        // Extra colored-run objects for this line (SGR spans).
        for run in runs {
            if !sdi.contains(run) {
                break;
            }
            sdi.set_visible(run, visible);
        }
    }
    sdi.set_visible("term_input_bg", visible);
    sdi.set_visible("term_prompt", visible);
    if !visible {
        // Shown again (when needed) by the next `setup_terminal_objects*`.
        sdi.set_visible("term_cursor", false);
    }
}

/// Create/update terminal-mode SDI objects with theme-driven colors and layout.
///
/// The text cursor is drawn at the end of `input_buf`; use
/// [`setup_terminal_objects_with_cursor`] for a line editor whose cursor
/// can sit mid-line.
pub fn setup_terminal_objects(
    sdi: &mut SdiRegistry,
    output_lines: &[String],
    cwd: &str,
    input_buf: &str,
    scroll_offset: usize,
    at: &ActiveTheme,
    cursor_visible: bool,
) {
    let cursor_col = input_buf.chars().count();
    setup_terminal_objects_with_cursor(
        sdi,
        output_lines,
        cwd,
        input_buf,
        cursor_col,
        scroll_offset,
        at,
        cursor_visible,
    );
}

/// [`setup_terminal_objects`] with an explicit cursor column.
///
/// `cursor_col` is a *character* index into `input_buf`. At the end of the
/// line the cursor is the classic trailing `_` glyph; mid-line it is a
/// `term_cursor` underline placed under the character at that column
/// (measured with the shared bitmap-font metrics).
#[allow(clippy::too_many_arguments)]
pub fn setup_terminal_objects_with_cursor(
    sdi: &mut SdiRegistry,
    output_lines: &[String],
    cwd: &str,
    input_buf: &str,
    cursor_col: usize,
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
    let colors = TerminalColors::from_theme(at);

    // Terminal background.
    if !sdi.contains("terminal_bg") {
        let obj = sdi.create("terminal_bg");
        obj.x = margin;
        obj.y = top_y;
        obj.w = bg_w;
        obj.h = bg_h;
        obj.color = colors.bg;
        obj.border_radius = Some(at.terminal_border_radius);
        obj.stroke_width = Some(1);
        obj.stroke_color = Some(colors.border);
    }
    if let Ok(obj) = sdi.get_mut("terminal_bg") {
        obj.visible = true;
    }

    // Show visible lines from the scrollback buffer, offset by scroll.
    //
    // Lines containing SGR escape sequences (see [`crate::ansi`]) are
    // split into colored runs: the first run reuses the `term_line_{i}`
    // object; subsequent runs get `term_line_{i}_r{j}` objects offset by
    // the measured width of the preceding text. Plain lines behave
    // exactly as before (single object, theme output color).
    //
    // Run offsets use the shared bitmap-font metrics
    // (`bitmap_measure_text`) since no backend is available here. On
    // backends that render with a different font (skin TTF fonts, PSP
    // system font) colored runs may drift by a few pixels; plain lines
    // are unaffected.
    let end = output_lines.len().saturating_sub(scroll_offset);
    let start = end.saturating_sub(visible_lines);
    let output_color = colors.output;
    // Name buffers reused across the loop: this runs every frame, so a
    // format! per line (plus one per colored run) is real churn.
    use std::fmt::Write as _;
    let mut name = String::with_capacity(16);
    let mut run_name = String::with_capacity(20);
    for i in 0..visible_lines {
        name.clear();
        let _ = write!(name, "term_line_{i}");
        let line_x = margin + 4;
        let line_y = top_y + 2 + (i as i32) * at.terminal_line_height as i32;
        if !sdi.contains(&name) {
            let obj = sdi.create(&name);
            obj.x = line_x;
            obj.y = line_y;
            obj.font_size = at.font_body;
            obj.text_color = output_color;
            obj.w = 0;
            obj.h = 0;
        }

        let raw = output_lines.get(start + i);
        let mut extra_runs = 0usize;
        match raw {
            Some(line) if crate::ansi::has_sgr(line) => {
                let runs = crate::ansi::parse_runs(line);
                let run_color = |c: Option<u8>| match c {
                    Some(slot) => at.ansi.color(slot as usize),
                    None => output_color,
                };
                let mut x = line_x;
                for (j, run) in runs.iter().enumerate() {
                    let width = crate::backend::bitmap_measure_text(run.text, at.font_body) as i32;
                    if j == 0 {
                        if let Ok(obj) = sdi.get_mut(&name) {
                            obj.set_text(run.text);
                            obj.text_color = run_color(run.color);
                            obj.x = x;
                            obj.visible = true;
                        }
                    } else if j <= MAX_LINE_RUNS {
                        run_name.clear();
                        let _ = write!(run_name, "term_line_{i}_r{j}");
                        if !sdi.contains(&run_name) {
                            let obj = sdi.create(&run_name);
                            obj.font_size = at.font_body;
                            obj.w = 0;
                            obj.h = 0;
                        }
                        if let Ok(obj) = sdi.get_mut(&run_name) {
                            obj.x = x;
                            obj.y = line_y;
                            obj.set_text(run.text);
                            obj.text_color = run_color(run.color);
                            obj.visible = true;
                        }
                        extra_runs = j;
                    } else {
                        // Beyond the run budget: append to the last object.
                        run_name.clear();
                        let _ = write!(run_name, "term_line_{i}_r{MAX_LINE_RUNS}");
                        if let Ok(obj) = sdi.get_mut(&run_name)
                            && let Some(ref mut text) = obj.text
                        {
                            text.push_str(run.text);
                        }
                    }
                    x += width;
                }
            },
            _ => {
                if let Ok(obj) = sdi.get_mut(&name) {
                    match raw {
                        Some(line) => obj.set_text(line),
                        None => obj.text = None,
                    }
                    obj.text_color = output_color;
                    obj.x = line_x;
                    obj.visible = true;
                }
            },
        }

        // Hide leftover run objects from a previous, more colorful frame.
        for j in (extra_runs + 1)..=MAX_LINE_RUNS {
            run_name.clear();
            let _ = write!(run_name, "term_line_{i}_r{j}");
            if !sdi.contains(&run_name) {
                break;
            }
            if let Ok(obj) = sdi.get_mut(&run_name) {
                obj.visible = false;
            }
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
        obj.color = colors.input_bg;
        obj.border_radius = Some(at.app.input_border_radius);
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
        obj.text_color = colors.prompt;
        obj.w = 0;
        obj.h = 0;
    }
    // Byte offset of the cursor, or `None` when it sits at end of line.
    let mid_line = input_buf.char_indices().nth(cursor_col);
    if let Ok(obj) = sdi.get_mut("term_prompt") {
        // Rebuild the prompt in the object's own String to reuse its
        // capacity instead of allocating a fresh one every frame.
        let text = obj.text.get_or_insert_default();
        text.clear();
        if mid_line.is_some() {
            let _ = write!(text, "{cwd}> {input_buf}");
        } else {
            let cursor_char = if cursor_visible { '_' } else { ' ' };
            let _ = write!(text, "{cwd}> {input_buf}{cursor_char}");
        }
        obj.visible = true;
    }

    // Mid-line cursor: an underline below the character at the cursor.
    match mid_line {
        Some((byte, ch)) => {
            let fs = at.font_body;
            let prefix_w = crate::backend::bitmap_measure_text(cwd, fs)
                + crate::backend::bitmap_measure_text("> ", fs)
                + crate::backend::bitmap_measure_text(&input_buf[..byte], fs);
            let glyph_w = oasis_types::bitmap_font::glyph_advance_scaled(ch, fs).max(2);
            if !sdi.contains("term_cursor") {
                sdi.create("term_cursor");
            }
            if let Ok(obj) = sdi.get_mut("term_cursor") {
                obj.x = margin + 4 + prefix_w as i32;
                obj.y = input_y + 2 + fs as i32;
                obj.w = glyph_w;
                obj.h = 2;
                obj.color = colors.prompt;
                obj.visible = cursor_visible;
            }
        },
        None => {
            sdi.set_visible("term_cursor", false);
        },
    }
}

fn fmt_color(c: Color) -> String {
    format!("#{:02x}{:02x}{:02x}{:02x}", c.r, c.g, c.b, c.a)
}

/// Render the `sdi list` / `sdi get <name>` terminal output for `sdi`.
///
/// `None` lists every object name (z-ordered, hidden ones marked);
/// `Some(name)` prints all of that object's fields.
pub fn inspect_sdi(sdi: &SdiRegistry, name: Option<&str>) -> String {
    use std::fmt::Write as _;
    let Some(name) = name else {
        let mut objs: Vec<_> = sdi.names().filter_map(|n| sdi.get(n).ok()).collect();
        objs.sort_by(|a, b| a.z.cmp(&b.z).then_with(|| a.name.cmp(&b.name)));
        let mut out = format!("{} SDI objects:", objs.len());
        for o in objs {
            let hidden = if o.visible { "" } else { "  (hidden)" };
            let _ = write!(out, "\n  {:<28} z={}{hidden}", o.name, o.z);
        }
        return out;
    };
    let Ok(o) = sdi.get(name) else {
        return format!("sdi: no object named '{name}'");
    };
    let mut out = format!("{}:", o.name);
    let _ = write!(
        out,
        "\n  pos      {}, {}\n  size     {} x {}\n  z        {}\n  visible  {}\
         \n  overlay  {}\n  alpha    {}\n  color    {}",
        o.x,
        o.y,
        o.w,
        o.h,
        o.z,
        o.visible,
        o.overlay,
        o.alpha,
        fmt_color(o.color)
    );
    if let Some(ref text) = o.text {
        let _ = write!(
            out,
            "\n  text     {text:?}\n  font     {}px\n  text_color {}",
            o.font_size,
            fmt_color(o.text_color)
        );
    }
    if let Some(tex) = o.texture {
        let _ = write!(out, "\n  texture  {}", tex.0);
    }
    if o.nine_patch.is_some() {
        out.push_str("\n  nine_patch yes");
    }
    if let Some(r) = o.border_radius {
        let _ = write!(out, "\n  radius   {r}");
    }
    if let (Some(top), Some(bottom)) = (o.gradient_top, o.gradient_bottom) {
        let _ = write!(
            out,
            "\n  gradient {} -> {}",
            fmt_color(top),
            fmt_color(bottom)
        );
    }
    if let Some(w) = o.stroke_width {
        let color = o.stroke_color.map_or_else(|| "-".to_string(), fmt_color);
        let _ = write!(out, "\n  stroke   {w}px {color}");
    }
    if let Some(level) = o.shadow_level {
        let _ = write!(out, "\n  shadow   level {level}");
    }
    if let Some((dx, dy)) = o.text_shadow_offset {
        let _ = write!(out, "\n  text_shadow {dx}, {dy}");
    }
    if let Some(ref label) = o.aria_label {
        let _ = write!(out, "\n  aria     {label:?}");
    }
    out
}

/// Replace every [`CommandSignal::SdiInspect`] in a command result with
/// the text [`inspect_sdi`] renders, so hosts can print it like any
/// other output.
///
/// [`CommandSignal::SdiInspect`]: crate::terminal::CommandSignal::SdiInspect
pub fn resolve_sdi_inspect(
    output: crate::terminal::CommandOutput,
    sdi: &SdiRegistry,
) -> crate::terminal::CommandOutput {
    use crate::terminal::{CommandOutput, CommandSignal};
    match output {
        CommandOutput::Signal(CommandSignal::SdiInspect { name }) => {
            CommandOutput::Text(inspect_sdi(sdi, name.as_deref()))
        },
        CommandOutput::Multi(outputs) => CommandOutput::Multi(
            outputs
                .into_iter()
                .map(|o| resolve_sdi_inspect(o, sdi))
                .collect(),
        ),
        other => other,
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
    let colors = TerminalColors::from_theme(at);
    let margin = 4i32;
    let top_y = at.statusbar_height as i32 + 2;
    let bot_y = at.screen_h as i32 - at.bottombar_height as i32;
    let bg_w = at.screen_w - (margin * 2) as u32;
    let bg_h = (bot_y - top_y) as u32;

    let sb_w = at.scrollbar.width;
    let track_x: i32 = margin + bg_w as i32 - sb_w as i32 - 1;
    let track_y: i32 = top_y;
    let track_h: u32 = bg_h;

    // Track.
    backend.fill_rect(track_x, track_y, sb_w, track_h, colors.scrollbar_track)?;

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
    backend.fill_rect(track_x, thumb_y, sb_w, thumb_h, colors.scrollbar_thumb)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Media category pages --

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

    #[test]
    fn constants() {
        assert_eq!(VISIBLE_OUTPUT_LINES, 12);
        assert_eq!(MAX_OUTPUT_LINES, 2000);
        const { assert!(VISIBLE_OUTPUT_LINES < MAX_OUTPUT_LINES) };
    }

    #[test]
    fn trim_scrollback_5000_into_full_buffer() {
        let mut lines: Vec<String> = (0..MAX_OUTPUT_LINES).map(|i| format!("old {i}")).collect();
        lines.extend((0..5000).map(|i| format!("new {i}")));
        trim_scrollback(&mut lines);
        assert_eq!(lines.len(), MAX_OUTPUT_LINES);
        assert_eq!(lines[0], format!("new {}", 5000 - MAX_OUTPUT_LINES));
        assert_eq!(lines[MAX_OUTPUT_LINES - 1], "new 4999");
        // Under the cap: untouched.
        let mut short = vec!["a".to_string()];
        trim_scrollback(&mut short);
        assert_eq!(short, ["a"]);
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
    fn set_terminal_visible_hide_pass_leaves_scene_clean() {
        let mut sdi = SdiRegistry::new();
        let lines: Vec<String> = (0..40)
            .map(|i| format!("\u{1b}[31mx\u{1b}[0m {i}"))
            .collect();
        let at = ActiveTheme::default().with_screen_size(800, 600);
        setup_terminal_objects(&mut sdi, &lines, "/", "", 0, &at, true);
        set_terminal_visible(&mut sdi, false);
        assert!(sdi.take_scene_dirty(), "first hide is a real change");
        // Every following frame re-runs the hide pass: nothing changes.
        set_terminal_visible(&mut sdi, false);
        assert!(!sdi.is_scene_dirty());
        assert!(!sdi.get("term_line_5_r1").unwrap().visible);
    }

    #[test]
    fn set_terminal_visible_hides_after_partial_show() {
        // Short-circuit must not skip a pass while the anchor objects
        // are still visible.
        let mut sdi = SdiRegistry::new();
        let at = ActiveTheme::default();
        let lines: Vec<String> = (0..20).map(|i| format!("l{i}")).collect();
        setup_terminal_objects(&mut sdi, &lines, "/", "", 0, &at, true);
        set_terminal_visible(&mut sdi, false);
        set_terminal_visible(&mut sdi, true);
        assert!(sdi.get("term_line_3").unwrap().visible);
        set_terminal_visible(&mut sdi, false);
        assert!(!sdi.get("term_line_3").unwrap().visible);
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

    // -- SGR colored runs --

    #[test]
    fn setup_terminal_objects_sgr_runs() {
        let mut sdi = SdiRegistry::new();
        let at = ActiveTheme::default();
        let lines = vec![format!("\u{1b}[94mdir/\u{1b}[0m file")];
        setup_terminal_objects(&mut sdi, &lines, "/", "", 0, &at, true);

        // First run lives in term_line_0 with the bright-blue slot color.
        let first = sdi.get("term_line_0").unwrap();
        assert_eq!(first.text.as_deref(), Some("dir/"));
        assert_eq!(first.text_color, at.ansi.color(12));

        // Second run gets its own object at an x offset, default color.
        let second = sdi.get("term_line_0_r1").unwrap();
        assert!(second.visible);
        assert_eq!(second.text.as_deref(), Some(" file"));
        assert_eq!(
            second.text_color,
            oasis_types::color::with_alpha(at.app.terminal_output_color, 255)
        );
        assert!(second.x > first.x);
        assert_eq!(second.y, first.y);
    }

    #[test]
    fn setup_terminal_objects_sgr_runs_cleared_on_plain_line() {
        let mut sdi = SdiRegistry::new();
        let at = ActiveTheme::default();
        let colored = vec![format!("\u{1b}[31merr\u{1b}[0m rest")];
        setup_terminal_objects(&mut sdi, &colored, "/", "", 0, &at, true);
        assert!(sdi.get("term_line_0_r1").unwrap().visible);

        // Replacing with a plain line hides the leftover run object and
        // restores the default output color.
        let plain = vec!["plain".to_string()];
        setup_terminal_objects(&mut sdi, &plain, "/", "", 0, &at, true);
        assert!(!sdi.get("term_line_0_r1").unwrap().visible);
        let first = sdi.get("term_line_0").unwrap();
        assert_eq!(first.text.as_deref(), Some("plain"));
        assert_eq!(
            first.text_color,
            oasis_types::color::with_alpha(at.app.terminal_output_color, 255)
        );
    }

    #[test]
    fn set_terminal_visible_hides_run_objects() {
        let mut sdi = SdiRegistry::new();
        let at = ActiveTheme::default();
        let colored = vec![format!("\u{1b}[32mok\u{1b}[0m done")];
        setup_terminal_objects(&mut sdi, &colored, "/", "", 0, &at, true);
        assert!(sdi.get("term_line_0_r1").unwrap().visible);

        set_terminal_visible(&mut sdi, false);
        assert!(!sdi.get("term_line_0_r1").unwrap().visible);
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

    // -- TerminalColors --

    #[test]
    fn terminal_colors_defaults_match_legacy() {
        // Without [app_themes.terminal] every slot must equal the exact
        // theme-derived color used before overrides existed (screenshot
        // regression relies on this).
        let at = ActiveTheme::default();
        let colors = TerminalColors::from_theme(&at);
        assert_eq!(colors.bg, at.app.bg);
        assert_eq!(colors.border, at.bar.separator_color);
        assert_eq!(
            colors.output,
            oasis_types::color::with_alpha(at.app.terminal_output_color, 255)
        );
        assert_eq!(
            colors.input_bg,
            oasis_types::color::lighten(at.app.bg, 0.03)
        );
        assert_eq!(colors.prompt, at.app.terminal_prompt_color);
        assert_eq!(colors.scrollbar_track, at.scrollbar.track_color);
        assert_eq!(colors.scrollbar_thumb, at.scrollbar.thumb_color);
    }

    #[test]
    fn terminal_colors_override_applies() {
        let mut at = ActiveTheme::default();
        let prompt = Color::rgb(255, 0, 255);
        let bg = Color::rgb(1, 2, 3);
        let mut overrides = std::collections::HashMap::new();
        overrides.insert("prompt".to_string(), prompt);
        overrides.insert("bg".to_string(), bg);
        at.app_themes.insert("terminal".to_string(), overrides);

        let colors = TerminalColors::from_theme(&at);
        assert_eq!(colors.prompt, prompt);
        assert_eq!(colors.bg, bg);
        // Slots without an override keep their theme-derived defaults.
        assert_eq!(
            colors.output,
            TerminalColors::from_theme(&ActiveTheme::default()).output
        );
        assert_eq!(colors.border, at.bar.separator_color);
    }

    #[test]
    fn setup_terminal_objects_honors_app_theme_override() {
        let mut at = ActiveTheme::default();
        let prompt = Color::rgb(0, 128, 64);
        let mut overrides = std::collections::HashMap::new();
        overrides.insert("prompt".to_string(), prompt);
        at.app_themes.insert("terminal".to_string(), overrides);

        let mut sdi = SdiRegistry::new();
        setup_terminal_objects(&mut sdi, &[], "/", "", 0, &at, true);
        assert_eq!(sdi.get("term_prompt").unwrap().text_color, prompt);
        // Non-overridden slots keep the default color.
        assert_eq!(sdi.get("terminal_bg").unwrap().color, at.app.bg);
    }

    #[test]
    fn mid_line_cursor_is_underline_at_column() {
        let at = ActiveTheme::default();
        let mut sdi = SdiRegistry::new();
        setup_terminal_objects_with_cursor(&mut sdi, &[], "/", "echo hi", 2, 0, &at, true);
        // No trailing `_` glyph while the cursor is mid-line.
        assert_eq!(
            sdi.get("term_prompt").unwrap().text.as_deref(),
            Some("/> echo hi")
        );
        let cursor = sdi.get("term_cursor").unwrap();
        assert!(cursor.visible);
        let prompt_x = sdi.get("term_prompt").unwrap().x;
        let expected = crate::backend::bitmap_measure_text("/> ec", at.font_body) as i32;
        assert_eq!(cursor.x - prompt_x, expected);
        assert_eq!(
            cursor.w,
            oasis_types::bitmap_font::glyph_advance_scaled('h', at.font_body).max(2)
        );

        // Moving the cursor to the end hides the underline again.
        setup_terminal_objects_with_cursor(&mut sdi, &[], "/", "echo hi", 7, 0, &at, true);
        assert!(!sdi.get("term_cursor").unwrap().visible);
        assert_eq!(
            sdi.get("term_prompt").unwrap().text.as_deref(),
            Some("/> echo hi_")
        );
        // Hiding the terminal hides the cursor object too.
        setup_terminal_objects_with_cursor(&mut sdi, &[], "/", "echo hi", 0, 0, &at, true);
        set_terminal_visible(&mut sdi, false);
        assert!(!sdi.get("term_cursor").unwrap().visible);
    }

    #[test]
    fn inspect_sdi_lists_and_describes_objects() {
        let mut sdi = SdiRegistry::new();
        {
            let obj = sdi.create("label");
            obj.x = 10;
            obj.y = 20;
            obj.w = 30;
            obj.h = 40;
            obj.text = Some("hello".to_string());
            obj.color = Color::rgb(255, 0, 0);
        }
        sdi.create("hidden_box").visible = false;

        let list = inspect_sdi(&sdi, None);
        assert!(list.starts_with("2 SDI objects:"), "{list}");
        assert!(list.contains("hidden_box"));
        assert!(list.contains("(hidden)"));

        let desc = inspect_sdi(&sdi, Some("label"));
        assert!(desc.starts_with("label:"), "{desc}");
        assert!(desc.contains("pos      10, 20"), "{desc}");
        assert!(desc.contains("size     30 x 40"), "{desc}");
        assert!(desc.contains("color    #ff0000ff"), "{desc}");
        assert!(desc.contains("text     \"hello\""), "{desc}");

        assert!(inspect_sdi(&sdi, Some("nope")).contains("no object named 'nope'"));

        use crate::terminal::{CommandOutput, CommandSignal};
        let resolved = resolve_sdi_inspect(
            CommandOutput::Multi(vec![CommandOutput::Signal(CommandSignal::SdiInspect {
                name: Some("label".to_string()),
            })]),
            &sdi,
        );
        let CommandOutput::Multi(inner) = resolved else {
            panic!("expected Multi");
        };
        assert!(matches!(&inner[0], CommandOutput::Text(t) if t.starts_with("label:")));
    }
}
