//! Generic simple app for static-content screens.
//!
//! Used as the delegate for the in-core Terminal app (which syncs its
//! display lines from the desktop terminal pipeline via [`SimpleApp::set_lines`])
//! and as the placeholder type the plugin system constructs for dynamically
//! registered apps. The four other static-content apps (Browser, Network,
//! Package Manager, System Monitor) live in their own `oasis-app-*` crates
//! and no longer go through this type.

use crate::active_theme::ActiveTheme;
use crate::backend::SdiBackend;
use crate::input::Button;
use crate::sdi::SdiRegistry;
use crate::vfs::Vfs;

use super::AppAction;
use super::ContentState;
use super::app_trait::App;
use oasis_app_core::render::{
    draw_content_windowed, hide_app_sdi, render_app_chrome, render_content_sdi,
};

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
    /// Action returned on Confirm press (default: None).
    confirm_action: AppAction,
    /// Whether the last line of `content.lines` is the prompt appended by
    /// [`Self::sync_terminal_lines`] (stripped before the next sync).
    trailing_prompt: bool,
}

impl SimpleApp {
    /// Create a simple app with pre-set content lines.
    pub fn new(title: &str, path: &str, lines: Vec<String>) -> Self {
        let mut content = ContentState::new(title, path);
        content.lines = lines;
        Self {
            content,
            confirm_action: AppAction::None,
            trailing_prompt: false,
        }
    }

    /// Create the Settings app.
    pub fn settings(path: &str, skin_name: &str, width: u32, height: u32) -> Self {
        Self::new(
            "Settings",
            path,
            vec![
                "OASIS_OS Settings".to_string(),
                String::new(),
                format!("  Screen:     {width} x {height}"),
                format!("  Skin:       {skin_name}"),
                "  Audio:      Enabled".to_string(),
                "  Network:    Enabled".to_string(),
                "  Terminal:   Enabled".to_string(),
                "  Plugins:    Enabled".to_string(),
                String::new(),
                "(Settings are read-only in this build)".to_string(),
            ],
        )
    }

    /// Create the Terminal app (windowed interactive terminal).
    ///
    /// Text input and command execution are handled by the desktop input
    /// dispatcher, which syncs output lines back into this app via
    /// `set_terminal_lines()`.
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
    /// Returns the number of lines cloned (the prompt counts as one).
    pub fn sync_terminal_lines(
        &mut self,
        output: &[String],
        prompt: &str,
        scroll_offset: usize,
    ) -> usize {
        if self.trailing_prompt {
            self.content.lines.pop();
        }
        let cloned = sync_lines(&mut self.content.lines, output);
        self.content.lines.push(prompt.to_string());
        self.trailing_prompt = true;
        self.apply_scroll_offset(scroll_offset);
        cloned + 1
    }

    /// Map a from-the-bottom scroll offset onto `content.scroll`.
    fn apply_scroll_offset(&mut self, scroll_offset: usize) {
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
        match button {
            Button::Cancel => AppAction::Exit,
            Button::Up => {
                self.content.navigate_up();
                AppAction::None
            },
            Button::Down => {
                self.content.navigate_down();
                AppAction::None
            },
            Button::Confirm => self.confirm_action.clone(),
            _ => AppAction::None,
        }
    }

    fn update_sdi(&mut self, sdi: &mut SdiRegistry, at: &ActiveTheme) {
        self.content.update_layout(at);
        self.content.animate_selection(0.3);
        render_app_chrome(sdi, at);
        render_content_sdi(&self.content, sdi, at);
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
        draw_content_windowed(&self.content, cx, cy, cw, ch, backend, at)
    }

    fn hide_sdi(&self, sdi: &mut SdiRegistry) {
        hide_app_sdi(sdi);
    }

    fn lines(&self) -> &[String] {
        &self.content.lines
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
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
    fn sync_terminal_lines_one_append_clones_one_line_plus_prompt() {
        let mut app = SimpleApp::terminal("/apps/terminal");
        let mut output = numbered(0..2000);
        app.sync_terminal_lines(&output, "> ", 0);
        let first = app.content.lines[1].as_ptr();
        output.drain(..1);
        output.push("fresh".into());
        assert_eq!(app.sync_terminal_lines(&output, "> ", 0), 2);
        assert_eq!(app.content.lines.len(), 2001);
        assert_eq!(app.content.lines[0].as_ptr(), first);
        assert_eq!(app.content.lines[1999], "fresh");
        assert_eq!(app.content.lines[2000], "> ");
    }

    #[test]
    fn settings_title_and_path() {
        let app = SimpleApp::settings("/apps/settings", "Classic", 480, 272);
        assert_eq!(app.title(), "Settings");
        assert_eq!(app.path(), "/apps/settings");
    }

    #[test]
    fn settings_content_lines() {
        let app = SimpleApp::settings("/apps/settings", "Classic", 480, 272);
        assert!(app.lines().iter().any(|l| l.contains("OASIS_OS Settings")));
        assert!(app.lines().iter().any(|l| l.contains("read-only")));
    }

    #[test]
    fn cancel_exits() {
        let vfs = make_vfs();
        let mut app = SimpleApp::settings("/apps/settings", "Classic", 480, 272);
        assert_eq!(app.handle_input(&Button::Cancel, &vfs), AppAction::Exit);
    }

    #[test]
    fn navigate_up_down() {
        let vfs = make_vfs();
        let mut app = SimpleApp::settings("/apps/settings", "Classic", 480, 272);
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
        let app = SimpleApp::settings("/apps/settings", "Classic", 480, 272);
        assert!(app.browse_dir().is_none());
        assert!(app.viewing_file().is_none());
    }

    #[test]
    fn no_pending_request() {
        let mut app = SimpleApp::settings("/apps/settings", "Classic", 480, 272);
        assert!(app.take_pending_request().is_none());
        assert!(app.peek_pending_request().is_none());
    }

    #[test]
    fn downcast_works() {
        let app = SimpleApp::settings("/apps/settings", "Classic", 480, 272);
        let any = app.as_any();
        assert!(any.downcast_ref::<SimpleApp>().is_some());
    }

    #[test]
    fn confirm_is_noop() {
        let vfs = make_vfs();
        let mut app = SimpleApp::settings("/apps/settings", "Classic", 480, 272);
        assert_eq!(app.handle_input(&Button::Confirm, &vfs), AppAction::None);
    }
}
