use crate::active_theme::ActiveTheme;
use crate::backend::Color;
use crate::backend::TextureId;
use crate::bottombar::BottomBar;
use crate::sdi::SdiRegistry;

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

/// Update SDI objects for the currently selected media category page.
pub fn update_media_page(sdi: &mut SdiRegistry, bottom_bar: &BottomBar, at: &ActiveTheme) {
    let page_name = "media_page_text";
    if !sdi.contains(page_name) {
        let obj = sdi.create(page_name);
        obj.font_size = at.font_heading;
        obj.text_color = at.app.text;
        obj.w = 0;
        obj.h = 0;
    }
    let page_str = format!("[ {} Page ]", bottom_bar.active_tab.label());
    if let Ok(obj) = sdi.get_mut(page_name) {
        obj.x = (at.screen_w as i32) / 2 - (page_str.len() as i32 * at.font_heading as i32 / 2);
        obj.y = (at.screen_h as i32) / 2 - 16;
        obj.visible = true;
        obj.set_text(&page_str);
    }

    let hint_name = "media_page_hint";
    let hint_str = "Press R to cycle categories";
    if !sdi.contains(hint_name) {
        let obj = sdi.create(hint_name);
        obj.font_size = at.font_hint;
        obj.text_color = at.app.dim_text;
        obj.w = 0;
        obj.h = 0;
    }
    if let Ok(obj) = sdi.get_mut(hint_name) {
        obj.x = (at.screen_w as i32) / 2 - (hint_str.len() as i32 * at.font_hint as i32 / 2);
        obj.y = (at.screen_h as i32) / 2 + 9;
        obj.visible = true;
        obj.set_text(hint_str);
    }
}

/// Hide media page SDI objects.
pub fn hide_media_page(sdi: &mut SdiRegistry) {
    for name in ["media_page_text", "media_page_hint"] {
        sdi.set_visible(name, false);
    }
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

    #[test]
    fn constants() {
        assert_eq!(VISIBLE_OUTPUT_LINES, 12);
        assert_eq!(MAX_OUTPUT_LINES, 2000);
        assert!(VISIBLE_OUTPUT_LINES < MAX_OUTPUT_LINES);
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
