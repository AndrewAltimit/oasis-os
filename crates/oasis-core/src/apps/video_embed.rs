//! YouTube embed app for the WASM build.
//!
//! Renders a skin-themed search/browse UI on Canvas 2D and delegates video
//! playback to a `youtube-nocookie.com` iframe overlay. Communication with
//! the iframe is via VFS IPC: the app writes an embed URL to
//! [`VIDEO_EMBED_REQUEST_PATH`], and the WASM backend intercepts it to
//! show/navigate the iframe.
//!
//! Feature-gated behind `wasm-youtube`.

use crate::active_theme::ActiveTheme;
use crate::backend::SdiBackend;
use crate::input::Button;
use crate::sdi::SdiRegistry;
use crate::vfs::Vfs;

use super::app_trait::App;
use super::{AppAction, ContentState};

/// VFS IPC path for video embed requests.
pub const VIDEO_EMBED_REQUEST_PATH: &str = "/tmp/video_embed_request";

/// Build a `youtube-nocookie.com` embed URL for the given video ID.
pub fn embed_url(video_id: &str) -> String {
    format!(
        "https://www.youtube-nocookie.com/embed/{video_id}\
         ?autoplay=1&modestbranding=1&rel=0"
    )
}

/// State of the embed app UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EmbedState {
    /// User is typing a search query or video ID.
    Search,
    /// A video is playing via the iframe overlay.
    Playing,
}

/// YouTube embed app -- search + iframe player for the WASM build.
#[derive(Debug)]
pub struct VideoEmbedApp {
    content: ContentState,
    state: EmbedState,
    /// Current search/video-ID input buffer.
    input_buf: String,
    /// Currently playing video ID (if any).
    playing_video_id: Option<String>,
    /// History of played video IDs for back navigation.
    history: Vec<String>,
}

impl VideoEmbedApp {
    /// Create a new video embed app.
    pub fn new(path: &str) -> Self {
        let mut content = ContentState::new("Video Embed", path);
        content.lines = Self::search_lines("");
        Self {
            content,
            state: EmbedState::Search,
            input_buf: String::new(),
            playing_video_id: None,
            history: Vec::new(),
        }
    }

    /// Build the display lines for search mode.
    fn search_lines(input: &str) -> Vec<String> {
        vec![
            "Video Embed".to_string(),
            String::new(),
            "Enter a YouTube video ID or search URL:".to_string(),
            String::new(),
            format!("  > {input}_"),
            String::new(),
            "Examples:".to_string(),
            "  dQw4w9WgXcQ".to_string(),
            "  jNQXAC9IVRw".to_string(),
            String::new(),
            "Press CONFIRM to play, CANCEL to go back.".to_string(),
        ]
    }

    /// Build the display lines for playing mode.
    fn playing_lines(video_id: &str) -> Vec<String> {
        vec![
            "Video Embed".to_string(),
            String::new(),
            format!("Now playing: {video_id}"),
            String::new(),
            "(Video is displayed in the iframe overlay)".to_string(),
            String::new(),
            "Press CANCEL to stop and return to search.".to_string(),
        ]
    }

    /// Extract a video ID from user input.
    ///
    /// Supports plain IDs, full URLs, and short URLs:
    /// - `dQw4w9WgXcQ`
    /// - `https://www.youtube.com/watch?v=dQw4w9WgXcQ`
    /// - `https://youtu.be/dQw4w9WgXcQ`
    fn extract_video_id(input: &str) -> Option<String> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return None;
        }

        // Try to extract from youtube.com/watch?v=ID (require YouTube domain)
        let is_youtube =
            trimmed.contains("youtube.com") || trimmed.contains("youtube-nocookie.com");
        if is_youtube && let Some(pos) = trimmed.find("v=") {
            let after_v = &trimmed[pos + 2..];
            let id = after_v.split(&['&', '#', ' '][..]).next().unwrap_or("");
            if !id.is_empty() {
                return Some(id.to_string());
            }
        }

        // Try to extract from youtu.be/ID
        if let Some(pos) = trimmed.find("youtu.be/") {
            let after = &trimmed[pos + 9..];
            let id = after.split(&['?', '#', '&', ' '][..]).next().unwrap_or("");
            if !id.is_empty() {
                return Some(id.to_string());
            }
        }

        // Try to extract from youtube.com/embed/ID
        if let Some(pos) = trimmed.find("/embed/") {
            let after = &trimmed[pos + 7..];
            let id = after.split(&['?', '#', '&', ' '][..]).next().unwrap_or("");
            if !id.is_empty() {
                return Some(id.to_string());
            }
        }

        // Assume plain video ID (alphanumeric + - + _)
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

    /// Start playing a video ID.
    fn play(&mut self, video_id: &str) {
        if let Some(ref old) = self.playing_video_id {
            self.history.push(old.clone());
        }
        self.playing_video_id = Some(video_id.to_string());
        self.state = EmbedState::Playing;
        self.content.lines = Self::playing_lines(video_id);
        self.content.scroll = 0;
        self.content.cursor = 0;

        // Emit VFS IPC request for the WASM backend to show the iframe.
        let url = embed_url(video_id);
        self.content.pending_vfs_request = Some((VIDEO_EMBED_REQUEST_PATH.to_string(), url));
    }

    /// Stop playing and return to search.
    fn stop(&mut self) {
        self.playing_video_id = None;
        self.state = EmbedState::Search;
        self.content.lines = Self::search_lines(&self.input_buf);
        self.content.scroll = 0;
        self.content.cursor = 0;

        // Emit stop request (empty URL = hide iframe).
        self.content.pending_vfs_request =
            Some((VIDEO_EMBED_REQUEST_PATH.to_string(), String::new()));
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
                    if let Some(id) = Self::extract_video_id(&self.input_buf) {
                        self.play(&id);
                    }
                    AppAction::None
                },
                Button::Up => {
                    self.content.navigate_up();
                    AppAction::None
                },
                Button::Down => {
                    self.content.navigate_down();
                    AppAction::None
                },
                _ => AppAction::None,
            },
            EmbedState::Playing => match button {
                Button::Cancel => {
                    self.stop();
                    AppAction::None
                },
                _ => AppAction::None,
            },
        }
    }

    fn handle_text_input(&mut self, ch: char) {
        if self.state == EmbedState::Search {
            self.input_buf.push(ch);
            self.content.lines = Self::search_lines(&self.input_buf);
        }
    }

    fn handle_backspace(&mut self) {
        if self.state == EmbedState::Search {
            self.input_buf.pop();
            self.content.lines = Self::search_lines(&self.input_buf);
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
    ) -> oasis_types::error::Result<()> {
        super::file_manager::draw_content_windowed(&self.content, cx, cy, cw, ch, backend, at)
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
        // Too short to be a valid video ID.
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
    fn play_emits_vfs_request() {
        let mut app = VideoEmbedApp::new("/apps/video-embed");
        app.play("dQw4w9WgXcQ");
        assert_eq!(app.state, EmbedState::Playing);
        assert_eq!(app.playing_video_id.as_deref(), Some("dQw4w9WgXcQ"));

        let req = app.take_pending_request();
        assert!(req.is_some());
        let (path, data) = req.unwrap();
        assert_eq!(path, VIDEO_EMBED_REQUEST_PATH);
        assert!(data.contains("youtube-nocookie.com"));
        assert!(data.contains("dQw4w9WgXcQ"));
    }

    #[test]
    fn stop_emits_empty_request() {
        let mut app = VideoEmbedApp::new("/apps/video-embed");
        app.play("test123456");
        let _ = app.take_pending_request();

        app.stop();
        assert_eq!(app.state, EmbedState::Search);
        assert!(app.playing_video_id.is_none());

        let req = app.take_pending_request();
        assert!(req.is_some());
        let (path, data) = req.unwrap();
        assert_eq!(path, VIDEO_EMBED_REQUEST_PATH);
        assert!(data.is_empty());
    }

    #[test]
    fn text_input_updates_search() {
        let mut app = VideoEmbedApp::new("/apps/video-embed");
        app.handle_text_input('a');
        app.handle_text_input('b');
        assert_eq!(app.input_buf, "ab");
        assert!(app.lines().iter().any(|l| l.contains("ab_")));
    }

    #[test]
    fn backspace_removes_char() {
        let mut app = VideoEmbedApp::new("/apps/video-embed");
        app.handle_text_input('x');
        app.handle_text_input('y');
        app.handle_backspace();
        assert_eq!(app.input_buf, "x");
    }

    #[test]
    fn confirm_with_valid_id_plays() {
        use oasis_vfs::MemoryVfs;
        let mut app = VideoEmbedApp::new("/apps/video-embed");
        app.input_buf = "dQw4w9WgXcQ".to_string();
        let vfs = MemoryVfs::new();
        app.handle_input(&Button::Confirm, &vfs);
        assert_eq!(app.state, EmbedState::Playing);
    }

    #[test]
    fn confirm_with_invalid_id_stays_in_search() {
        use oasis_vfs::MemoryVfs;
        let mut app = VideoEmbedApp::new("/apps/video-embed");
        app.input_buf = "abc".to_string();
        let vfs = MemoryVfs::new();
        app.handle_input(&Button::Confirm, &vfs);
        assert_eq!(app.state, EmbedState::Search);
    }

    #[test]
    fn cancel_in_playing_returns_to_search() {
        use oasis_vfs::MemoryVfs;
        let mut app = VideoEmbedApp::new("/apps/video-embed");
        app.play("test123456");
        let _ = app.take_pending_request();
        let vfs = MemoryVfs::new();
        let action = app.handle_input(&Button::Cancel, &vfs);
        assert_eq!(action, AppAction::None);
        assert_eq!(app.state, EmbedState::Search);
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
    fn history_tracks_played_videos() {
        let mut app = VideoEmbedApp::new("/apps/video-embed");
        app.play("video1video1");
        app.play("video2video2");
        assert_eq!(app.history.len(), 1);
        assert_eq!(app.history[0], "video1video1");
    }
}
