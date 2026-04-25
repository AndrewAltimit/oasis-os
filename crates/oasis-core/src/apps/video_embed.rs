//! YouTube embed app for the WASM build.
//!
//! Renders a skin-themed search/grid UI and delegates video playback to a
//! `youtube-nocookie.com` iframe overlay. Communication with the WASM
//! backend is via VFS IPC:
//!
//! - The app writes commands (`search:<query>`, `play:<id>`, `stop`) to
//!   [`VIDEO_EMBED_REQUEST_PATH`].
//! - The backend writes a JSON result blob to [`VIDEO_EMBED_RESULTS_PATH`]
//!   that the app polls each frame in [`App::refresh`].
//!
//! Feature-gated behind `wasm-youtube`.

use serde::{Deserialize, Serialize};

use crate::active_theme::ActiveTheme;
use crate::backend::SdiBackend;
use crate::input::Button;
use crate::sdi::SdiRegistry;
use crate::vfs::Vfs;
use oasis_types::backend::TextureId;
use oasis_types::error::Result;

use super::app_trait::App;
use super::{AppAction, ContentState};

/// VFS IPC path the app writes commands to.
pub const VIDEO_EMBED_REQUEST_PATH: &str = "/tmp/video_embed_request";

/// VFS IPC path the backend publishes search results to.
pub const VIDEO_EMBED_RESULTS_PATH: &str = "/tmp/video_embed_results";

/// Build a `youtube-nocookie.com` embed URL for the given video ID.
pub fn embed_url(video_id: &str) -> String {
    format!(
        "https://www.youtube-nocookie.com/embed/{video_id}\
         ?autoplay=1&modestbranding=1&rel=0"
    )
}

/// One search hit published by the backend.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SearchResult {
    pub id: String,
    pub title: String,
    pub author: String,
    pub duration: String,
    /// Numeric texture id for the thumbnail. `0` means "not yet loaded".
    pub thumb_tex: u64,
    pub thumb_w: u32,
    pub thumb_h: u32,
}

/// Lifecycle of an in-flight or completed search.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum SearchStatus {
    #[default]
    #[serde(rename = "idle")]
    Idle,
    #[serde(rename = "loading")]
    Loading,
    #[serde(rename = "ready")]
    Ready,
    #[serde(rename = "error")]
    Error,
}

/// Result blob written to [`VIDEO_EMBED_RESULTS_PATH`] by the backend.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SearchResults {
    pub query: String,
    pub status: SearchStatus,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub results: Vec<SearchResult>,
}

/// What the user is currently looking at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EmbedState {
    /// Empty / typing a query.
    Search,
    /// A grid of search results is being displayed.
    Results,
    /// The iframe overlay is showing a video.
    Playing,
}

/// Layout constants for the thumbnail grid.
const GRID_COLS: usize = 3;
const GRID_ROWS: usize = 2;
const GRID_PAGE: usize = GRID_COLS * GRID_ROWS;
const SEARCH_BAR_H: i32 = 24;
const FOOTER_H: i32 = 14;
const CELL_GAP: i32 = 6;
const TITLE_LINES_H: i32 = 22; // two short lines below the thumbnail.

/// YouTube embed app -- search + thumbnail grid + iframe player.
#[derive(Debug)]
pub struct VideoEmbedApp {
    content: ContentState,
    state: EmbedState,
    /// Search input buffer (or playing video id when in Playing state).
    input_buf: String,
    /// Currently playing video id (if any).
    playing_video_id: Option<String>,
    /// Mirror of the latest result blob from the backend.
    results: SearchResults,
    /// Page index into `results.results` (0-based).
    page: usize,
    /// Selected cell within the visible page.
    selection: usize,
}

impl VideoEmbedApp {
    /// Create a new video embed app.
    pub fn new(path: &str) -> Self {
        let mut content = ContentState::new("Video Embed", path);
        content.lines = Vec::new();
        Self {
            content,
            state: EmbedState::Search,
            input_buf: String::new(),
            playing_video_id: None,
            results: SearchResults::default(),
            page: 0,
            selection: 0,
        }
    }

    /// Try to extract a video id from the input. Mirrors the original parser:
    /// supports plain IDs, `watch?v=...`, `youtu.be/...`, `embed/...`.
    fn extract_video_id(input: &str) -> Option<String> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return None;
        }

        let is_youtube =
            trimmed.contains("youtube.com") || trimmed.contains("youtube-nocookie.com");
        if is_youtube && let Some(pos) = trimmed.find("v=") {
            let after_v = &trimmed[pos + 2..];
            let id = after_v.split(&['&', '#', ' '][..]).next().unwrap_or("");
            if !id.is_empty() {
                return Some(id.to_string());
            }
        }

        if let Some(pos) = trimmed.find("youtu.be/") {
            let after = &trimmed[pos + 9..];
            let id = after.split(&['?', '#', '&', ' '][..]).next().unwrap_or("");
            if !id.is_empty() {
                return Some(id.to_string());
            }
        }

        if let Some(pos) = trimmed.find("/embed/") {
            let after = &trimmed[pos + 7..];
            let id = after.split(&['?', '#', '&', ' '][..]).next().unwrap_or("");
            if !id.is_empty() {
                return Some(id.to_string());
            }
        }

        // Plain video id heuristic: 11-char standard ID, or alnum+`-_` 6..=20.
        if trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            && trimmed.len() >= 6
            && trimmed.len() <= 20
        {
            return Some(trimmed.to_string());
        }

        None
    }

    /// Total number of pages in the current result set (>= 1).
    fn page_count(&self) -> usize {
        if self.results.results.is_empty() {
            1
        } else {
            self.results.results.len().div_ceil(GRID_PAGE)
        }
    }

    /// Range of result indices visible on the current page.
    fn page_range(&self) -> (usize, usize) {
        let start = self.page * GRID_PAGE;
        let end = (start + GRID_PAGE).min(self.results.results.len());
        (start, end)
    }

    /// Number of cells currently filled on the visible page.
    fn visible_count(&self) -> usize {
        let (start, end) = self.page_range();
        end - start
    }

    /// Snap selection back into bounds after mutation.
    fn clamp_selection(&mut self) {
        let v = self.visible_count();
        if v == 0 {
            self.selection = 0;
        } else if self.selection >= v {
            self.selection = v - 1;
        }
    }

    /// Submit either a direct video id/URL play or a search query.
    fn submit(&mut self) {
        if let Some(id) = Self::extract_video_id(&self.input_buf) {
            self.start_play(&id);
        } else if !self.input_buf.trim().is_empty() {
            self.kick_search();
        }
    }

    fn kick_search(&mut self) {
        let query = self.input_buf.trim().to_string();
        self.results = SearchResults {
            query: query.clone(),
            status: SearchStatus::Loading,
            error: None,
            results: Vec::new(),
        };
        self.state = EmbedState::Results;
        self.page = 0;
        self.selection = 0;
        self.content.pending_vfs_request = Some((
            VIDEO_EMBED_REQUEST_PATH.to_string(),
            format!("search:{query}"),
        ));
    }

    fn start_play(&mut self, video_id: &str) {
        self.playing_video_id = Some(video_id.to_string());
        self.state = EmbedState::Playing;
        self.content.pending_vfs_request = Some((
            VIDEO_EMBED_REQUEST_PATH.to_string(),
            format!("play:{video_id}"),
        ));
    }

    fn stop_playback(&mut self) {
        self.playing_video_id = None;
        self.state = if self.results.results.is_empty() {
            EmbedState::Search
        } else {
            EmbedState::Results
        };
        self.content.pending_vfs_request =
            Some((VIDEO_EMBED_REQUEST_PATH.to_string(), "stop".to_string()));
    }

    /// Compute the rect of grid cell `i` (0..GRID_PAGE) within `(cw, ch)`.
    fn cell_rect(cw: u32, ch: u32, i: usize) -> (i32, i32, u32, u32) {
        let title_h = 20i32;
        let avail_w = cw as i32 - CELL_GAP * (GRID_COLS as i32 + 1);
        let avail_h =
            ch as i32 - title_h - SEARCH_BAR_H - FOOTER_H - CELL_GAP * (GRID_ROWS as i32 + 1);
        let cell_w = (avail_w / GRID_COLS as i32).max(40);
        let cell_h = (avail_h / GRID_ROWS as i32).max(40);
        let col = (i % GRID_COLS) as i32;
        let row = (i / GRID_COLS) as i32;
        let x = CELL_GAP + col * (cell_w + CELL_GAP);
        let y = title_h + SEARCH_BAR_H + CELL_GAP + row * (cell_h + CELL_GAP);
        (x, y, cell_w as u32, cell_h as u32)
    }

    /// Hit-test a click at local coords against the grid; returns the page-
    /// local cell index, or `None`.
    fn hit_test_grid(lx: i32, ly: i32, cw: u32, ch: u32) -> Option<usize> {
        for i in 0..GRID_PAGE {
            let (x, y, w, h) = Self::cell_rect(cw, ch, i);
            if lx >= x && ly >= y && lx < x + w as i32 && ly < y + h as i32 {
                return Some(i);
            }
        }
        None
    }

    /// Compute thumbnail rect inside a cell (16:9, top portion of the cell).
    fn thumb_rect(cell_x: i32, cell_y: i32, cell_w: u32, cell_h: u32) -> (i32, i32, u32, u32) {
        let avail_h = cell_h as i32 - TITLE_LINES_H;
        let by_h = avail_h.max(20);
        let by_w_from_h = by_h * 16 / 9;
        let (tw, th) = if by_w_from_h <= cell_w as i32 {
            (by_w_from_h as u32, by_h as u32)
        } else {
            let tw = cell_w;
            let th = cell_w * 9 / 16;
            (tw, th)
        };
        let tx = cell_x + (cell_w as i32 - tw as i32) / 2;
        let ty = cell_y;
        (tx, ty, tw, th)
    }
}

impl App for VideoEmbedApp {
    fn title(&self) -> &str {
        &self.content.title
    }

    fn path(&self) -> &str {
        &self.content.app_path
    }

    fn handle_input(&mut self, button: &Button, _vfs: &dyn Vfs) -> AppAction {
        match self.state {
            EmbedState::Search => match button {
                Button::Cancel => AppAction::Exit,
                Button::Confirm => {
                    self.submit();
                    AppAction::None
                },
                _ => AppAction::None,
            },
            EmbedState::Results => match button {
                Button::Cancel => {
                    // Back to idle search prompt.
                    self.results = SearchResults::default();
                    self.state = EmbedState::Search;
                    self.page = 0;
                    self.selection = 0;
                    AppAction::None
                },
                Button::Confirm => {
                    if self.results.status == SearchStatus::Ready {
                        let (start, _) = self.page_range();
                        let idx = start + self.selection;
                        if let Some(r) = self.results.results.get(idx) {
                            let id = r.id.clone();
                            self.start_play(&id);
                        }
                    }
                    AppAction::None
                },
                Button::Up => {
                    if self.selection >= GRID_COLS {
                        self.selection -= GRID_COLS;
                    } else if self.page > 0 {
                        self.page -= 1;
                        let v = self.visible_count();
                        let last_row_start = (v.saturating_sub(1) / GRID_COLS) * GRID_COLS;
                        let col = self.selection % GRID_COLS;
                        self.selection = (last_row_start + col).min(v.saturating_sub(1));
                    }
                    AppAction::None
                },
                Button::Down => {
                    let v = self.visible_count();
                    if self.selection + GRID_COLS < v {
                        self.selection += GRID_COLS;
                    } else if self.page + 1 < self.page_count() {
                        self.page += 1;
                        let col = self.selection % GRID_COLS;
                        self.selection = col.min(self.visible_count().saturating_sub(1));
                    }
                    AppAction::None
                },
                Button::Left => {
                    if !self.selection.is_multiple_of(GRID_COLS) {
                        self.selection -= 1;
                    } else if self.page > 0 {
                        self.page -= 1;
                        let v = self.visible_count();
                        let row = (self.selection / GRID_COLS).min(GRID_ROWS - 1);
                        self.selection = (row * GRID_COLS + GRID_COLS - 1).min(v.saturating_sub(1));
                    }
                    AppAction::None
                },
                Button::Right => {
                    let v = self.visible_count();
                    if self.selection + 1 < v && !(self.selection + 1).is_multiple_of(GRID_COLS) {
                        self.selection += 1;
                    } else if self.page + 1 < self.page_count() {
                        self.page += 1;
                        let row = (self.selection / GRID_COLS).min(GRID_ROWS - 1);
                        self.selection =
                            (row * GRID_COLS).min(self.visible_count().saturating_sub(1));
                    }
                    AppAction::None
                },
                _ => AppAction::None,
            },
            EmbedState::Playing => match button {
                Button::Cancel => {
                    self.stop_playback();
                    AppAction::None
                },
                _ => AppAction::None,
            },
        }
    }

    fn handle_text_input(&mut self, ch: char) {
        if self.state == EmbedState::Playing {
            return;
        }
        if ch.is_control() {
            return;
        }
        // If typing while looking at results, treat it as a new search query.
        if self.state == EmbedState::Results {
            self.state = EmbedState::Search;
            self.input_buf.clear();
        }
        self.input_buf.push(ch);
    }

    fn handle_backspace(&mut self) {
        if self.state == EmbedState::Playing {
            return;
        }
        if self.state == EmbedState::Results {
            self.state = EmbedState::Search;
        }
        self.input_buf.pop();
    }

    fn handle_click(&mut self, lx: i32, ly: i32, cw: u32, ch: u32, _fullscreen: bool) -> AppAction {
        if self.state != EmbedState::Results {
            return AppAction::None;
        }
        if let Some(cell) = Self::hit_test_grid(lx, ly, cw, ch) {
            let v = self.visible_count();
            if cell < v {
                self.selection = cell;
                let (start, _) = self.page_range();
                if let Some(r) = self.results.results.get(start + cell) {
                    let id = r.id.clone();
                    self.start_play(&id);
                }
            }
        }
        AppAction::None
    }

    fn refresh(&mut self, vfs: &dyn Vfs) {
        if !vfs.exists(VIDEO_EMBED_RESULTS_PATH) {
            return;
        }
        let Ok(bytes) = vfs.read(VIDEO_EMBED_RESULTS_PATH) else {
            return;
        };
        if bytes.is_empty() {
            return;
        }
        let Ok(text) = std::str::from_utf8(&bytes) else {
            return;
        };
        let Ok(parsed) = serde_json::from_str::<SearchResults>(text) else {
            return;
        };

        // Only swap in results that match our latest in-flight query.
        if self.state == EmbedState::Results
            && self.results.query == parsed.query
            && parsed != self.results
        {
            self.results = parsed;
            self.clamp_selection();
        }
    }

    fn update_sdi(&mut self, _sdi: &mut SdiRegistry, at: &ActiveTheme) {
        self.content.update_layout(at);
    }

    fn draw_windowed(
        &self,
        cx: i32,
        cy: i32,
        cw: u32,
        ch: u32,
        backend: &mut dyn SdiBackend,
        at: &ActiveTheme,
    ) -> Result<()> {
        let title_bar_h = at.app.title_bar_height as i32;

        // Title bar text.
        backend.draw_text(
            "Video Embed",
            cx + 6,
            cy + 2,
            at.font_body,
            at.app.title_bar_text,
        )?;

        // Search bar (always visible when not playing).
        if self.state != EmbedState::Playing {
            let bar_x = cx + 4;
            let bar_y = cy + title_bar_h + 2;
            let bar_w = cw.saturating_sub(8);
            let bar_h = (SEARCH_BAR_H - 4) as u32;
            backend.fill_rect(bar_x, bar_y, bar_w, bar_h, at.app.selected_bg)?;
            let prompt = format!("Search: {}_", self.input_buf);
            backend.draw_text(&prompt, bar_x + 4, bar_y + 4, at.font_body, at.app.text)?;
        }

        match self.state {
            EmbedState::Search => {
                let hint_y = cy + title_bar_h + SEARCH_BAR_H + 8;
                backend.draw_text(
                    "Type a search query, video id, or URL.",
                    cx + 8,
                    hint_y,
                    at.font_body,
                    at.app.text,
                )?;
                backend.draw_text(
                    "Confirm = play / search   Cancel = back",
                    cx + 8,
                    hint_y + 14,
                    at.font_hint,
                    at.app.dim_text,
                )?;
            },
            EmbedState::Results => {
                draw_results_grid(self, cx, cy, cw, ch, backend, at)?;
            },
            EmbedState::Playing => {
                // The iframe overlay covers the content; draw a minimal
                // placeholder behind it (most of it is hidden anyway).
                let hint_y = cy + title_bar_h + 4;
                let id = self.playing_video_id.as_deref().unwrap_or("");
                backend.draw_text(
                    &format!("Now playing: {id}"),
                    cx + 6,
                    hint_y,
                    at.font_body,
                    at.app.text,
                )?;
                backend.draw_text(
                    "Cancel = back to results",
                    cx + 6,
                    cy + ch as i32 - 14,
                    at.font_hint,
                    at.app.dim_text,
                )?;
            },
        }

        Ok(())
    }

    fn hide_sdi(&self, _sdi: &mut SdiRegistry) {}

    fn take_pending_request(&mut self) -> Option<(String, String)> {
        self.content.pending_vfs_request.take()
    }

    fn peek_pending_request(&self) -> Option<&(String, String)> {
        self.content.pending_vfs_request.as_ref()
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

fn draw_results_grid(
    app: &VideoEmbedApp,
    cx: i32,
    cy: i32,
    cw: u32,
    ch: u32,
    backend: &mut dyn SdiBackend,
    at: &ActiveTheme,
) -> Result<()> {
    let title_bar_h = at.app.title_bar_height as i32;
    let footer_y = cy + ch as i32 - FOOTER_H;

    match app.results.status {
        SearchStatus::Loading => {
            backend.draw_text(
                "Searching YouTube...",
                cx + 8,
                cy + title_bar_h + SEARCH_BAR_H + 8,
                at.font_body,
                at.app.text,
            )?;
        },
        SearchStatus::Error => {
            let err = app.results.error.as_deref().unwrap_or("Search failed");
            backend.draw_text(
                &format!("Error: {err}"),
                cx + 8,
                cy + title_bar_h + SEARCH_BAR_H + 8,
                at.font_body,
                at.app.text,
            )?;
            backend.draw_text(
                "Press Cancel and try a different query.",
                cx + 8,
                cy + title_bar_h + SEARCH_BAR_H + 22,
                at.font_hint,
                at.app.dim_text,
            )?;
        },
        SearchStatus::Idle => {},
        SearchStatus::Ready => {
            if app.results.results.is_empty() {
                backend.draw_text(
                    "No results.",
                    cx + 8,
                    cy + title_bar_h + SEARCH_BAR_H + 8,
                    at.font_body,
                    at.app.text,
                )?;
            } else {
                let (start, end) = app.page_range();
                let grid_origin_x = cx;
                let grid_origin_y = cy;
                for (i, idx) in (start..end).enumerate() {
                    let r = &app.results.results[idx];
                    let (rx, ry, rw, rh) = VideoEmbedApp::cell_rect(cw, ch, i);
                    let cell_x = grid_origin_x + rx;
                    let cell_y = grid_origin_y + ry;

                    // Selection highlight.
                    if i == app.selection {
                        backend.fill_rect(
                            cell_x - 2,
                            cell_y - 2,
                            rw + 4,
                            rh + 4,
                            at.app.selection_accent_color,
                        )?;
                    }

                    // Cell background.
                    backend.fill_rect(cell_x, cell_y, rw, rh, at.app.selected_bg)?;

                    // Thumbnail.
                    let (tx, ty, tw, th) = VideoEmbedApp::thumb_rect(cell_x, cell_y, rw, rh);
                    backend.fill_rect(tx, ty, tw, th, at.app.bg)?;
                    if r.thumb_tex != 0 {
                        let _ = backend.blit(TextureId(r.thumb_tex), tx, ty, tw, th);
                    }

                    // Title (one short line) + author.
                    let title_y = ty + th as i32 + 2;
                    let max_chars = (rw as usize / 6).max(8);
                    let truncated = truncate(&r.title, max_chars);
                    backend.draw_text(
                        &truncated,
                        cell_x + 4,
                        title_y,
                        at.font_body,
                        if i == app.selection {
                            at.app.selected_text
                        } else {
                            at.app.text
                        },
                    )?;
                    let author = truncate(&r.author, max_chars);
                    backend.draw_text(
                        &author,
                        cell_x + 4,
                        title_y + 12,
                        at.font_hint,
                        at.app.dim_text,
                    )?;
                    if !r.duration.is_empty() {
                        let dx = cell_x + rw as i32 - (r.duration.len() as i32 * 6) - 4;
                        backend.draw_text(
                            &r.duration,
                            dx,
                            ty + th as i32 - 12,
                            at.font_hint,
                            at.app.text,
                        )?;
                    }
                }
            }
        },
    }

    // Footer with paging hint.
    let footer = if app.results.status == SearchStatus::Ready && !app.results.results.is_empty() {
        format!(
            "Page {}/{}  -  Confirm=play  Cancel=back",
            app.page + 1,
            app.page_count(),
        )
    } else {
        "Confirm=play  Cancel=back".to_string()
    };
    backend.draw_text(&footer, cx + 6, footer_y, at.font_hint, at.app.dim_text)?;
    Ok(())
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let take = max_chars.saturating_sub(1);
    let mut out: String = s.chars().take(take).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embed_url_format() {
        let url = embed_url("dQw4w9WgXcQ");
        assert!(url.starts_with("https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ"));
        assert!(url.contains("autoplay=1"));
        assert!(url.contains("modestbranding=1"));
        assert!(url.contains("rel=0"));
    }

    #[test]
    fn extract_plain_id() {
        let id = VideoEmbedApp::extract_video_id("dQw4w9WgXcQ");
        assert_eq!(id.as_deref(), Some("dQw4w9WgXcQ"));
    }

    #[test]
    fn extract_from_watch_url() {
        let id = VideoEmbedApp::extract_video_id("https://www.youtube.com/watch?v=dQw4w9WgXcQ");
        assert_eq!(id.as_deref(), Some("dQw4w9WgXcQ"));
    }

    #[test]
    fn extract_from_watch_url_with_params() {
        let id =
            VideoEmbedApp::extract_video_id("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=42");
        assert_eq!(id.as_deref(), Some("dQw4w9WgXcQ"));
    }

    #[test]
    fn extract_from_short_url() {
        let id = VideoEmbedApp::extract_video_id("https://youtu.be/dQw4w9WgXcQ");
        assert_eq!(id.as_deref(), Some("dQw4w9WgXcQ"));
    }

    #[test]
    fn extract_from_embed_url() {
        let id =
            VideoEmbedApp::extract_video_id("https://www.youtube.com/embed/dQw4w9WgXcQ?autoplay=1");
        assert_eq!(id.as_deref(), Some("dQw4w9WgXcQ"));
    }

    #[test]
    fn extract_empty_returns_none() {
        assert!(VideoEmbedApp::extract_video_id("").is_none());
        assert!(VideoEmbedApp::extract_video_id("   ").is_none());
    }

    #[test]
    fn extract_short_id_rejected() {
        assert!(VideoEmbedApp::extract_video_id("abc").is_none());
    }

    #[test]
    fn new_starts_in_search_state() {
        let app = VideoEmbedApp::new("/apps/video-embed");
        assert_eq!(app.state, EmbedState::Search);
        assert!(app.playing_video_id.is_none());
        assert!(app.input_buf.is_empty());
    }

    #[test]
    fn confirm_with_id_emits_play_request() {
        use oasis_vfs::MemoryVfs;
        let mut app = VideoEmbedApp::new("/apps/video-embed");
        app.input_buf = "dQw4w9WgXcQ".to_string();
        let vfs = MemoryVfs::new();
        app.handle_input(&Button::Confirm, &vfs);
        assert_eq!(app.state, EmbedState::Playing);
        let (path, data) = app.take_pending_request().unwrap();
        assert_eq!(path, VIDEO_EMBED_REQUEST_PATH);
        assert!(data.starts_with("play:"));
        assert!(data.contains("dQw4w9WgXcQ"));
    }

    #[test]
    fn confirm_with_query_emits_search_request() {
        use oasis_vfs::MemoryVfs;
        let mut app = VideoEmbedApp::new("/apps/video-embed");
        app.input_buf = "lofi hip hop".to_string();
        let vfs = MemoryVfs::new();
        app.handle_input(&Button::Confirm, &vfs);
        assert_eq!(app.state, EmbedState::Results);
        assert_eq!(app.results.status, SearchStatus::Loading);
        let (path, data) = app.take_pending_request().unwrap();
        assert_eq!(path, VIDEO_EMBED_REQUEST_PATH);
        assert_eq!(data, "search:lofi hip hop");
    }

    #[test]
    fn stop_emits_stop_request() {
        use oasis_vfs::MemoryVfs;
        let mut app = VideoEmbedApp::new("/apps/video-embed");
        app.input_buf = "dQw4w9WgXcQ".to_string();
        let vfs = MemoryVfs::new();
        app.handle_input(&Button::Confirm, &vfs);
        let _ = app.take_pending_request();
        app.handle_input(&Button::Cancel, &vfs);
        let (_, data) = app.take_pending_request().unwrap();
        assert_eq!(data, "stop");
    }

    #[test]
    fn refresh_loads_results_from_vfs() {
        use oasis_vfs::MemoryVfs;
        use oasis_vfs::Vfs;
        let mut app = VideoEmbedApp::new("/apps/video-embed");
        app.input_buf = "cats".to_string();
        let mut vfs = MemoryVfs::new();
        let _ = vfs.mkdir("/tmp");
        app.handle_input(&Button::Confirm, &vfs);
        let _ = app.take_pending_request();

        let payload = SearchResults {
            query: "cats".to_string(),
            status: SearchStatus::Ready,
            error: None,
            results: vec![SearchResult {
                id: "abc12345678".to_string(),
                title: "Cats compilation".to_string(),
                author: "Cute Cats".to_string(),
                duration: "3:21".to_string(),
                thumb_tex: 7,
                thumb_w: 320,
                thumb_h: 180,
            }],
        };
        let json = serde_json::to_string(&payload).unwrap();
        let _ = vfs.write(VIDEO_EMBED_RESULTS_PATH, json.as_bytes());
        app.refresh(&vfs);
        assert_eq!(app.results.results.len(), 1);
        assert_eq!(app.results.results[0].id, "abc12345678");
        assert_eq!(app.results.status, SearchStatus::Ready);
    }

    #[test]
    fn click_in_grid_starts_playback() {
        let mut app = VideoEmbedApp::new("/apps/video-embed");
        app.state = EmbedState::Results;
        app.results = SearchResults {
            query: "x".to_string(),
            status: SearchStatus::Ready,
            error: None,
            results: vec![SearchResult {
                id: "id_one_aaaa".to_string(),
                ..Default::default()
            }],
        };
        let (rx, ry, rw, rh) = VideoEmbedApp::cell_rect(640, 400, 0);
        let cx = rx + rw as i32 / 2;
        let cy = ry + rh as i32 / 2;
        app.handle_click(cx, cy, 640, 400, false);
        assert_eq!(app.state, EmbedState::Playing);
        let (_, data) = app.take_pending_request().unwrap();
        assert_eq!(data, "play:id_one_aaaa");
    }

    #[test]
    fn cancel_in_search_exits() {
        use oasis_vfs::MemoryVfs;
        let mut app = VideoEmbedApp::new("/apps/video-embed");
        let vfs = MemoryVfs::new();
        let action = app.handle_input(&Button::Cancel, &vfs);
        assert_eq!(action, AppAction::Exit);
    }

    #[test]
    fn arrow_keys_navigate_grid() {
        let mut app = VideoEmbedApp::new("/apps/video-embed");
        app.state = EmbedState::Results;
        let mut results = Vec::new();
        for i in 0..6 {
            results.push(SearchResult {
                id: format!("id_{i:08}"),
                ..Default::default()
            });
        }
        app.results = SearchResults {
            query: "q".to_string(),
            status: SearchStatus::Ready,
            error: None,
            results,
        };
        use oasis_vfs::MemoryVfs;
        let vfs = MemoryVfs::new();
        assert_eq!(app.selection, 0);
        app.handle_input(&Button::Right, &vfs);
        assert_eq!(app.selection, 1);
        app.handle_input(&Button::Down, &vfs);
        assert_eq!(app.selection, 4);
        app.handle_input(&Button::Left, &vfs);
        assert_eq!(app.selection, 3);
        app.handle_input(&Button::Up, &vfs);
        assert_eq!(app.selection, 0);
    }

    #[test]
    fn page_count_with_overflow() {
        let mut app = VideoEmbedApp::new("/apps/video-embed");
        app.state = EmbedState::Results;
        let mut results = Vec::new();
        for i in 0..14 {
            results.push(SearchResult {
                id: format!("id_{i:08}"),
                ..Default::default()
            });
        }
        app.results = SearchResults {
            query: "q".to_string(),
            status: SearchStatus::Ready,
            error: None,
            results,
        };
        // 14 results / 6 per page = 3 pages.
        assert_eq!(app.page_count(), 3);
    }

    #[test]
    fn truncate_long_title() {
        let s = "this is a very long title that should be truncated";
        let t = truncate(s, 10);
        assert!(t.chars().count() <= 10);
        assert!(t.ends_with('…'));
    }
}
