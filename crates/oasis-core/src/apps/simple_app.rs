//! Generic simple app for static-content screens.
//!
//! Used as the delegate for the in-core Terminal app (which syncs its
//! display lines from the desktop terminal pipeline via
//! [`SimpleApp::sync_terminal_lines`]) and as the placeholder `AppRunner`
//! shows for a title with no registered app. Plugin apps bring their own
//! `App` implementation through `PluginAppRegistration`.

use crate::active_theme::ActiveTheme;
use crate::backend::SdiBackend;
use crate::input::Button;
use crate::vfs::Vfs;

use super::App;
use super::AppAction;
use super::ContentState;
use oasis_app_core::render::{WINDOWED_FONT_SIZE, draw_content_windowed, windowed_line_origin};

/// Candidate front-trim offsets [`sync_lines`] tries before falling back
/// to a full rewrite.
const SYNC_MAX_CANDIDATES: usize = 4;

/// Make `dst` equal to `src`, reusing as much of `dst` as possible.
///
/// Built for scrollback-style buffers that change by trimming lines off
/// the front and appending at the back: the overlap `dst[d..]` ==
/// `src[..p]` is found (d = lines trimmed), `dst` is shifted with one
/// `drain` and only the new tail is cloned. Syncing a 2000-line buffer
/// after a one-line append therefore costs a memcmp pass and one `String`
/// clone instead of 2000 allocations. Anything else (edits in the middle,
/// a cleared buffer) degrades to an in-place `clone_from` rewrite that
/// still reuses `dst`'s allocations. The result is always exactly `src`;
/// the alignment search only affects cost.
///
/// Returns the number of lines that had to be cloned.
pub fn sync_lines(dst: &mut Vec<String>, src: &[String]) -> usize {
    let Some(first) = src.first() else {
        dst.clear();
        return 0;
    };
    // Front-trim candidates: positions in dst holding src's first line.
    let mut best: Option<(usize, usize)> = None; // (d, overlap)
    for d in dst
        .iter()
        .enumerate()
        .filter(|(_, l)| *l == first)
        .map(|(d, _)| d)
        .take(SYNC_MAX_CANDIDATES)
    {
        let p = dst[d..].iter().zip(src).take_while(|(a, b)| a == b).count();
        if best.is_none_or(|(_, bp)| p > bp) {
            best = Some((d, p));
        }
        if d + p == dst.len() {
            // Overlap reaches the end of dst: can't do better.
            break;
        }
    }
    match best {
        Some((d, p)) => {
            dst.drain(..d);
            dst.truncate(p);
            dst.extend(src[p..].iter().cloned());
            src.len() - p
        },
        None => {
            // No alignment: rewrite in place, reusing String capacity.
            dst.truncate(src.len());
            for (a, b) in dst.iter_mut().zip(src) {
                a.clone_from(b);
            }
            let n = dst.len();
            dst.extend(src[n..].iter().cloned());
            src.len()
        },
    }
}

/// A simple static-content app that implements the `App` trait.
///
/// Used for apps that display informational text with basic navigation.
/// Content is set at creation time via a builder or direct lines.
#[derive(Debug)]
pub struct SimpleApp {
    pub content: ContentState,
    /// From-the-bottom scroll offset of the last terminal sync, re-applied
    /// when [`App::layout`] changes the row count.
    scroll_offset: usize,
    /// Whether the last line of `content.lines` is the prompt appended by
    /// [`Self::sync_terminal_lines`] (stripped before the next sync).
    trailing_prompt: bool,
    /// Text cursor column (in characters) within the trailing prompt line,
    /// drawn as an underline by `draw_windowed`. `None` = no cursor.
    prompt_cursor: Option<usize>,
}

impl SimpleApp {
    /// Create a simple app with pre-set content lines.
    pub fn new(title: &str, path: &str, lines: Vec<String>) -> Self {
        let mut content = ContentState::new(title, path);
        content.lines = lines;
        Self {
            content,
            scroll_offset: 0,
            trailing_prompt: false,
            prompt_cursor: None,
        }
    }

    /// Set the text cursor column (characters) within the trailing prompt
    /// line synced by [`Self::sync_terminal_lines`]; `None` hides it.
    /// Returns `true` when the cursor actually moved.
    pub fn set_prompt_cursor(&mut self, col: Option<usize>) -> bool {
        let changed = self.prompt_cursor != col;
        self.prompt_cursor = col;
        changed
    }

    /// Create the Terminal app (windowed interactive terminal).
    ///
    /// Text input and command execution are handled by the desktop input
    /// dispatcher, which syncs output lines back into this app via
    /// [`Self::sync_terminal_lines`].
    pub fn terminal(path: &str) -> Self {
        Self::new(
            "Terminal",
            path,
            vec![
                "OASIS_OS Terminal".to_string(),
                String::new(),
                "Type a command and press Enter.".to_string(),
            ],
        )
    }

    /// Update the display lines (used by the desktop input handler to sync
    /// terminal output into this app's display).
    ///
    /// `scroll_offset` scrolls up from the bottom (0 = fully scrolled down).
    pub fn set_lines(&mut self, lines: Vec<String>, scroll_offset: usize) {
        self.content.lines = lines;
        self.trailing_prompt = false;
        self.apply_scroll_offset(scroll_offset);
    }

    /// Sync terminal scrollback plus a trailing prompt line into this app's
    /// display without copying the whole buffer.
    ///
    /// Equivalent to `set_lines(output + [prompt], scroll_offset)`, but
    /// incremental (see [`sync_lines`]): appending one line to a full
    /// 2000-line scrollback clones one `String`, not 2000.
    ///
    /// Returns the number of lines that changed: scrollback lines cloned,
    /// plus one if the prompt line differs from the last sync (0 means the
    /// lines are unchanged, though trimming can still shorten them).
    pub fn sync_terminal_lines(
        &mut self,
        output: &[String],
        prompt: &str,
        scroll_offset: usize,
    ) -> usize {
        let old_prompt = if self.trailing_prompt {
            self.content.lines.pop()
        } else {
            None
        };
        let cloned = sync_lines(&mut self.content.lines, output);
        let prompt_changed = old_prompt.as_deref() != Some(prompt);
        match old_prompt {
            Some(mut line) if !prompt_changed => {
                // Reuse the old allocation (it already holds `prompt`).
                line.clear();
                line.push_str(prompt);
                self.content.lines.push(line);
            },
            _ => self.content.lines.push(prompt.to_string()),
        }
        self.trailing_prompt = true;
        self.apply_scroll_offset(scroll_offset);
        cloned + usize::from(prompt_changed)
    }

    /// Map a from-the-bottom scroll offset onto `content.scroll`.
    fn apply_scroll_offset(&mut self, scroll_offset: usize) {
        self.scroll_offset = scroll_offset;
        let len = self.content.lines.len();
        if len > self.content.cached_max_visible {
            let max_scroll = len - self.content.cached_max_visible;
            self.content.scroll = max_scroll.saturating_sub(scroll_offset);
        } else {
            self.content.scroll = 0;
        }
    }
}

impl App for SimpleApp {
    fn title(&self) -> &str {
        &self.content.title
    }

    fn path(&self) -> &str {
        &self.content.app_path
    }

    fn handle_input(&mut self, button: &Button, _vfs: &dyn Vfs) -> AppAction {
        self.content.handle_nav(button).unwrap_or(AppAction::None)
    }

    fn layout(&mut self, _cw: u32, ch: u32, at: &ActiveTheme) {
        let rows = self.content.cached_max_visible;
        self.content.update_layout_windowed(ch, at);
        // The terminal scrolls from the bottom: keep the newest lines and
        // the prompt in view at the window's real row count.
        if self.trailing_prompt && self.content.cached_max_visible != rows {
            self.apply_scroll_offset(self.scroll_offset);
        }
    }

    fn draw_windowed(
        &self,
        cx: i32,
        cy: i32,
        cw: u32,
        ch: u32,
        backend: &mut dyn SdiBackend,
        at: &ActiveTheme,
    ) -> crate::error::Result<()> {
        draw_content_windowed(&self.content, cx, cy, cw, ch, backend, at)?;
        // Line-editor cursor: an underline below the character at the
        // cursor column of the trailing prompt line.
        if let (true, Some(col)) = (self.trailing_prompt, self.prompt_cursor)
            && let Some(prompt) = self.content.lines.last()
            && let Some((x, y, prefix)) =
                windowed_line_origin(&self.content, cx, cy, ch, at, self.content.lines.len() - 1)
        {
            let byte = prompt
                .char_indices()
                .nth(col)
                .map_or(prompt.len(), |(b, _)| b);
            let fs = WINDOWED_FONT_SIZE;
            let prefix_w =
                backend.measure_text(prefix, fs) + backend.measure_text(&prompt[..byte], fs);
            let glyph = prompt[byte..].chars().next().unwrap_or(' ');
            let glyph_w = backend
                .measure_text(glyph.encode_utf8(&mut [0u8; 4]), fs)
                .max(4);
            backend.fill_rect(
                x + prefix_w as i32,
                y + fs as i32,
                glyph_w,
                2,
                at.app.selected_text,
            )?;
        }
        Ok(())
    }

    fn lines(&self) -> &[String] {
        &self.content.lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::MemoryVfs;

    fn make_vfs() -> MemoryVfs {
        MemoryVfs::new()
    }

    fn numbered(range: std::ops::Range<usize>) -> Vec<String> {
        range.map(|i| format!("line {i}")).collect()
    }

    #[test]
    fn sync_lines_append_reuses_existing_strings() {
        let src = numbered(0..2000);
        let mut dst = src.clone();
        let ptrs: Vec<*const u8> = dst.iter().map(|l| l.as_ptr()).collect();
        let mut src2 = src.clone();
        src2.push("new".into());
        assert_eq!(
            sync_lines(&mut dst, &src2),
            1,
            "only the new line is cloned"
        );
        assert_eq!(dst, src2);
        assert!(dst.iter().zip(&ptrs).all(|(l, &p)| l.as_ptr() == p));
    }

    #[test]
    fn sync_lines_trim_and_append() {
        let mut dst = numbered(0..2000);
        let kept = dst[3].as_ptr();
        // Buffer full: 3 lines appended, 3 trimmed off the front.
        let src = numbered(3..2003);
        assert_eq!(sync_lines(&mut dst, &src), 3);
        assert_eq!(dst, src);
        assert_eq!(
            dst[0].as_ptr(),
            kept,
            "surviving lines are moved, not cloned"
        );
    }

    #[test]
    fn sync_lines_handles_duplicates_and_rewrites() {
        // Repeated lines create several alignment candidates; the result
        // must still be exactly src.
        let mut dst: Vec<String> = ["", "", "a", "", "b"].map(String::from).to_vec();
        let src: Vec<String> = ["", "a", "", "b", "c"].map(String::from).to_vec();
        assert_eq!(sync_lines(&mut dst, &src), 1);
        assert_eq!(dst, src);
        // Middle edit: overlap stops at the edit, the rest is re-cloned.
        let src2: Vec<String> = ["", "X", "", "b", "c"].map(String::from).to_vec();
        sync_lines(&mut dst, &src2);
        assert_eq!(dst, src2);
        // Completely different content.
        let src3 = numbered(0..3);
        assert_eq!(sync_lines(&mut dst, &src3), 3);
        assert_eq!(dst, src3);
        assert_eq!(sync_lines(&mut dst, &[]), 0);
        assert!(dst.is_empty());
    }

    #[test]
    fn sync_terminal_lines_matches_set_lines() {
        let mut inc = SimpleApp::terminal("/apps/terminal");
        let mut full = SimpleApp::terminal("/apps/terminal");
        inc.content.cached_max_visible = 10;
        full.content.cached_max_visible = 10;
        let mut output = numbered(0..50);
        for step in 0..5 {
            output.push(format!("step {step}"));
            let prompt = format!("> cmd{step}");
            inc.sync_terminal_lines(&output, &prompt, step);
            let mut lines = output.clone();
            lines.push(prompt);
            full.set_lines(lines, step);
            assert_eq!(inc.content.lines, full.content.lines);
            assert_eq!(inc.content.scroll, full.content.scroll);
        }
    }

    #[test]
    fn sync_terminal_lines_one_append_clones_one_line() {
        let mut app = SimpleApp::terminal("/apps/terminal");
        let mut output = numbered(0..2000);
        app.sync_terminal_lines(&output, "> ", 0);
        let first = app.content.lines[1].as_ptr();
        output.drain(..1);
        output.push("fresh".into());
        assert_eq!(
            app.sync_terminal_lines(&output, "> ", 0),
            1,
            "an unchanged prompt is not re-cloned"
        );
        assert_eq!(app.sync_terminal_lines(&output, "> ", 0), 0, "no change");
        assert_eq!(app.sync_terminal_lines(&output, "> l", 0), 1, "prompt");
        assert_eq!(app.content.lines.len(), 2001);
        assert_eq!(app.content.lines[0].as_ptr(), first);
        assert_eq!(app.content.lines[1999], "fresh");
        assert_eq!(app.content.lines[2000], "> l");
    }

    /// A placeholder-style app with a few lines.
    fn sample() -> SimpleApp {
        SimpleApp::new(
            "Sample",
            "/apps/sample",
            vec!["first".into(), String::new(), "third".into()],
        )
    }

    #[test]
    fn title_and_path() {
        let app = sample();
        assert_eq!(app.title(), "Sample");
        assert_eq!(app.path(), "/apps/sample");
    }

    #[test]
    fn cancel_exits() {
        let vfs = make_vfs();
        let mut app = sample();
        assert_eq!(app.handle_input(&Button::Cancel, &vfs), AppAction::Exit);
    }

    #[test]
    fn navigate_up_down() {
        let vfs = make_vfs();
        let mut app = sample();
        app.content.cached_max_visible = 20;
        app.handle_input(&Button::Down, &vfs);
        assert_eq!(app.content.cursor, 1);
        app.handle_input(&Button::Up, &vfs);
        assert_eq!(app.content.cursor, 0);
    }

    #[test]
    fn custom_content() {
        let app = SimpleApp::new(
            "Custom",
            "/apps/custom",
            vec!["Line 1".into(), "Line 2".into()],
        );
        assert_eq!(app.title(), "Custom");
        assert_eq!(app.lines().len(), 2);
    }

    #[test]
    fn no_browse_dir_or_viewing_file() {
        let app = sample();
        assert!(app.browse_dir().is_none());
        assert!(app.viewing_file().is_none());
    }

    #[test]
    fn no_pending_request() {
        let mut app = sample();
        let mut out = Vec::new();
        app.drain_requests(&mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn downcast_works() {
        let app: Box<dyn App> = Box::new(sample());
        assert!(app.as_any().downcast_ref::<SimpleApp>().is_some());
    }

    #[test]
    fn confirm_is_noop() {
        let vfs = make_vfs();
        let mut app = sample();
        assert_eq!(app.handle_input(&Button::Confirm, &vfs), AppAction::None);
    }

    /// A terminal window resized to ~6 rows keeps the newest output line
    /// and the prompt on screen, and a from-the-bottom scroll offset maps
    /// onto the window's real row count (F14 item 4, app half).
    #[test]
    fn short_terminal_window_shows_prompt_and_last_line() {
        use oasis_test_backend::ClipAuditBackend;
        let at = ActiveTheme::default();
        let line_h = at.terminal_line_height.max(12);
        let ch = oasis_app_core::render::WINDOWED_TOP_PAD + 16 + 6 * line_h;
        let mut app = SimpleApp::terminal("/apps/terminal");
        let output = numbered(0..50);
        app.sync_terminal_lines(&output, "> ls", 0);

        let drawn = |app: &mut SimpleApp| {
            app.layout(300, ch, &at);
            let mut b = ClipAuditBackend::new(300, ch, (0, 0, 300, ch));
            app.draw_windowed(0, 0, 300, ch, &mut b, &at).expect("draw");
            b.texts().iter().map(|t| t.text.clone()).collect::<Vec<_>>()
        };
        let texts = drawn(&mut app);
        assert!(texts.iter().any(|t| t.ends_with("> ls")), "{texts:?}");
        assert!(texts.iter().any(|t| t.ends_with("line 49")), "{texts:?}");
        assert!(!texts.iter().any(|t| t.ends_with("line 43")), "{texts:?}");

        // Scrolled all the way back: the oldest line is the first row.
        app.sync_terminal_lines(&output, "> ls", 51 - 6);
        let texts = drawn(&mut app);
        assert!(texts.iter().any(|t| t.ends_with("line 0")), "{texts:?}");
        // ...and back at the bottom.
        app.sync_terminal_lines(&output, "> ls", 0);
        let texts = drawn(&mut app);
        assert!(texts.iter().any(|t| t.ends_with("> ls")), "{texts:?}");
    }
}
