//! `App` trait implementation for the TV Guide.
//!
//! Wraps `TvGuideState` so TV Guide can use the same delegate pattern
//! as all other apps in `AppRunner`.

use oasis_app_core::{App, AppAction};
use oasis_sdi::SdiRegistry;
use oasis_skin::active_theme::ActiveTheme;
use oasis_types::backend::SdiBackend;
use oasis_types::input::Button;
use oasis_vfs::Vfs;

use crate::catalog::ChannelCatalog;
use crate::channel::{ChannelConfig, DEFAULT_CHANNELS_TOML};
use crate::guide::TvGuideState;
use crate::{TV_CHANNELS_PATH, TV_REQUEST_PATH};

/// TV Guide application implementing the `App` trait.
///
/// Wraps `TvGuideState` and manages input handling, VFS IPC for tune
/// requests, and EPG grid rendering via the standard delegate pattern.
#[derive(Debug)]
pub struct TvGuideApp {
    /// The TV guide state (EPG grid, channel list, catalogs, etc.).
    pub guide: TvGuideState,
    /// VFS path for this app.
    path: String,
    /// Cached text lines for the content view.
    lines: Vec<String>,
    /// Pending VFS IPC request (path, data).
    pub pending_request: Option<(String, String)>,
}

impl TvGuideApp {
    /// Create a new TV Guide app from VFS channel config.
    pub fn new(path: &str, vfs: &dyn Vfs, at: &ActiveTheme) -> Self {
        let config = if vfs.exists(TV_CHANNELS_PATH) {
            log::debug!("TV: loading channel config from VFS");
            let data = vfs.read(TV_CHANNELS_PATH).unwrap_or_default();
            let text = String::from_utf8_lossy(&data);
            ChannelConfig::from_toml(&text).unwrap_or_else(|_| {
                ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML)
                    .expect("default channels TOML is valid")
            })
        } else {
            log::debug!("TV: using default channel config");
            ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).expect("default channels TOML is valid")
        };

        log::debug!("TV: TvGuideApp::new with {} channels", config.channel.len());
        let guide = TvGuideState::new(&config, at);
        let lines = guide.text_content();

        Self {
            guide,
            path: path.to_string(),
            lines,
            pending_request: None,
        }
    }

    /// Refresh the cached text lines from the guide state.
    ///
    /// Called after catalog changes, tune/untune, or navigation to keep
    /// the `lines()` output in sync.
    pub fn refresh_text(&mut self) {
        self.lines = self.guide.text_content();
        log::debug!("TV: refresh_text -> {} lines", self.lines.len());
    }

    /// Build a tune request VFS IPC payload from a tune request.
    fn make_tune_ipc(req: &crate::guide::TuneRequest) -> (String, String) {
        let url = ChannelCatalog::download_url(&req.episode);
        let data = format!("tune_url {url} {}", req.seek_secs);
        log::info!("TV: tune CH{} -> {}", req.channel_index, req.episode.title,);
        (TV_REQUEST_PATH.to_string(), data)
    }
}

impl App for TvGuideApp {
    fn title(&self) -> &str {
        "TV Guide"
    }

    fn path(&self) -> &str {
        &self.path
    }

    fn handle_input(&mut self, button: &Button, _vfs: &dyn Vfs) -> AppAction {
        match button {
            Button::Cancel => {
                if self.guide.tuned_channel.is_some() {
                    self.guide.untune();
                    AppAction::None
                } else {
                    AppAction::Exit
                }
            },
            Button::Up => {
                self.guide.select_up();
                self.lines = self.guide.text_content();
                AppAction::None
            },
            Button::Down => {
                self.guide.select_down();
                self.lines = self.guide.text_content();
                AppAction::None
            },
            Button::Left => {
                self.guide.scroll_left();
                AppAction::None
            },
            Button::Right => {
                self.guide.scroll_right();
                AppAction::None
            },
            Button::Confirm => {
                let tuned = if let Some(req) = self.guide.tune() {
                    self.pending_request = Some(Self::make_tune_ipc(&req));
                    true
                } else {
                    false
                };
                self.lines = self.guide.text_content();
                if tuned {
                    AppAction::RequestFullscreen
                } else {
                    AppAction::None
                }
            },
            Button::Select => {
                self.guide.reset_for_retry();
                self.lines = self.guide.text_content();
                AppAction::None
            },
            _ => AppAction::None,
        }
    }

    fn handle_click(&mut self, lx: i32, ly: i32, cw: u32, ch: u32, fullscreen: bool) -> AppAction {
        if let Some(req) = self.guide.handle_click(lx, ly, cw, ch, fullscreen) {
            self.pending_request = Some(Self::make_tune_ipc(&req));
            self.lines = self.guide.text_content();
            return AppAction::RequestFullscreen;
        }
        self.lines = self.guide.text_content();
        AppAction::None
    }

    fn update_sdi(&mut self, sdi: &mut SdiRegistry, at: &ActiveTheme) {
        self.guide.update_sdi(sdi, at);
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
        self.guide.draw_windowed(cx, cy, cw, ch, backend, at)
    }

    fn hide_sdi(&self, sdi: &mut SdiRegistry) {
        TvGuideState::hide_sdi(sdi);
    }

    fn take_pending_request(&mut self) -> Option<(String, String)> {
        self.pending_request.take()
    }

    fn peek_pending_request(&self) -> Option<&(String, String)> {
        self.pending_request.as_ref()
    }

    fn lines(&self) -> &[String] {
        &self.lines
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
    use oasis_vfs::MemoryVfs;

    #[test]
    fn tv_guide_app_implements_app_trait() {
        let vfs = MemoryVfs::new();
        let at = ActiveTheme::default();
        let app = TvGuideApp::new("/apps/TV Guide", &vfs, &at);
        assert_eq!(app.title(), "TV Guide");
        assert_eq!(app.path(), "/apps/TV Guide");
        assert!(!app.lines().is_empty());
    }

    #[test]
    fn tv_guide_app_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<TvGuideApp>();
    }

    #[test]
    fn tv_guide_app_cancel_exits_when_not_tuned() {
        let vfs = MemoryVfs::new();
        let at = ActiveTheme::default();
        let mut app = TvGuideApp::new("/apps/TV Guide", &vfs, &at);
        let action = app.handle_input(&Button::Cancel, &vfs);
        assert_eq!(action, AppAction::Exit);
    }

    #[test]
    fn tv_guide_app_navigation() {
        let vfs = MemoryVfs::new();
        let at = ActiveTheme::default();
        let mut app = TvGuideApp::new("/apps/TV Guide", &vfs, &at);
        app.handle_input(&Button::Down, &vfs);
        assert_eq!(app.guide.selected_channel, 1);
        app.handle_input(&Button::Up, &vfs);
        assert_eq!(app.guide.selected_channel, 0);
    }

    #[test]
    fn tv_guide_app_select_resets_fetch() {
        let vfs = MemoryVfs::new();
        let at = ActiveTheme::default();
        let mut app = TvGuideApp::new("/apps/TV Guide", &vfs, &at);
        app.guide.fetch_attempted = true;
        app.guide.fetch_error = Some("error".into());
        app.handle_input(&Button::Select, &vfs);
        assert!(!app.guide.fetch_attempted);
        assert!(app.guide.fetch_error.is_none());
    }

    #[test]
    fn tv_guide_app_downcast() {
        let vfs = MemoryVfs::new();
        let at = ActiveTheme::default();
        let app = TvGuideApp::new("/apps/TV Guide", &vfs, &at);
        let boxed: Box<dyn App> = Box::new(app);
        assert!(boxed.as_any().downcast_ref::<TvGuideApp>().is_some());
    }
}
