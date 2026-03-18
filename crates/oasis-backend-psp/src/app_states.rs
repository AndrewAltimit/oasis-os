//! Per-app mutable state structs.
//!
//! Collects the scattered `let mut` state variables for each app into
//! organized structs so the main loop is cleaner.

use oasis_backend_psp::{FileEntry, TextureId};

// ---------------------------------------------------------------------------
// Terminal
// ---------------------------------------------------------------------------

pub(crate) struct TerminalState {
    pub(crate) lines: Vec<String>,
    pub(crate) input: String,
    /// Scroll offset: 0 means "show latest lines" (auto-scroll).
    /// Positive values scroll back into history.
    pub(crate) scroll: usize,
}

impl TerminalState {
    pub(crate) fn new(initial_lines: Vec<String>) -> Self {
        Self {
            lines: initial_lines,
            input: String::new(),
            scroll: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// File Manager (dual-panel)
// ---------------------------------------------------------------------------

/// State for one panel of the dual-panel file manager.
pub(crate) struct FmPanel {
    pub(crate) path: String,
    pub(crate) entries: Vec<FileEntry>,
    pub(crate) selected: usize,
    pub(crate) scroll: usize,
    pub(crate) loaded: bool,
}

impl FmPanel {
    pub(crate) fn new(path: &str) -> Self {
        Self {
            path: String::from(path),
            entries: Vec::new(),
            selected: 0,
            scroll: 0,
            loaded: false,
        }
    }
}

pub(crate) struct FileManagerState {
    pub(crate) left: FmPanel,
    pub(crate) right: FmPanel,
    /// 0 = left panel, 1 = right panel.
    pub(crate) active_panel: usize,
    /// UMD drive activated flag.
    pub(crate) umd_activated: bool,
}

impl FileManagerState {
    pub(crate) fn new() -> Self {
        Self {
            left: FmPanel::new("ms0:/"),
            right: FmPanel::new("ms0:/"),
            active_panel: 0,
            umd_activated: false,
        }
    }

    /// Get mutable references to the active panel's fields.
    pub(crate) fn active_panel_mut(&mut self) -> &mut FmPanel {
        if self.active_panel == 0 {
            &mut self.left
        } else {
            &mut self.right
        }
    }

    /// Get a reference to the active panel.
    pub(crate) fn active_panel_ref(&self) -> &FmPanel {
        if self.active_panel == 0 {
            &self.left
        } else {
            &self.right
        }
    }
}

// ---------------------------------------------------------------------------
// Photo Viewer
// ---------------------------------------------------------------------------

pub(crate) struct PhotoViewerState {
    pub(crate) path: String,
    pub(crate) entries: Vec<FileEntry>,
    pub(crate) selected: usize,
    pub(crate) scroll: usize,
    pub(crate) loaded: bool,
    pub(crate) viewing: bool,
    pub(crate) loading: bool,
    pub(crate) tex: Option<TextureId>,
    pub(crate) img_w: u32,
    pub(crate) img_h: u32,
}

impl PhotoViewerState {
    pub(crate) fn new() -> Self {
        Self {
            path: String::from("ms0:/"),
            entries: Vec::new(),
            selected: 0,
            scroll: 0,
            loaded: false,
            viewing: false,
            loading: false,
            tex: None,
            img_w: 0,
            img_h: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Music Player
// ---------------------------------------------------------------------------

pub(crate) struct MusicPlayerState {
    pub(crate) path: String,
    pub(crate) entries: Vec<FileEntry>,
    pub(crate) selected: usize,
    pub(crate) scroll: usize,
    pub(crate) loaded: bool,
    pub(crate) file_name: String,
}

impl MusicPlayerState {
    pub(crate) fn new() -> Self {
        Self {
            path: String::from("ms0:/"),
            entries: Vec::new(),
            selected: 0,
            scroll: 0,
            loaded: false,
            file_name: String::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Browser (full oasis-browser engine)
// ---------------------------------------------------------------------------

pub(crate) struct BrowserState {
    /// Full browser engine widget -- lazily initialized to save RAM.
    /// Only allocated when the user first opens the browser.
    pub(crate) widget: Option<oasis_browser::BrowserWidget>,
    /// Minimal VFS for the browser (PSP has no oasis-vfs on disk).
    pub(crate) vfs: oasis_core::vfs::MemoryVfs,
    /// Status message for the SDI fallback display.
    pub(crate) status_msg: String,
    /// Whether the browser is currently loading (for loading indicator).
    pub(crate) loading: bool,
    /// Cached wrapped error lines to avoid per-frame allocation.
    pub(crate) cached_error_lines: Vec<String>,
    /// The raw error message that produced `cached_error_lines`.
    pub(crate) cached_error_msg: String,
}

impl BrowserState {
    pub(crate) fn new() -> Self {
        Self {
            widget: None,
            vfs: oasis_core::vfs::MemoryVfs::new(),
            status_msg: String::from("Press [] to enter URL, X to navigate"),
            loading: false,
            cached_error_lines: Vec::new(),
            cached_error_msg: String::new(),
        }
    }

    /// Ensure the BrowserWidget is initialized. Returns a mutable ref.
    pub(crate) fn ensure_widget(&mut self) -> &mut oasis_browser::BrowserWidget {
        if self.widget.is_none() {
            use oasis_browser::BrowserConfig;
            use oasis_browser::config::BrowserFeatures;

            let config = BrowserConfig {
                default_font_size: 8.0,
                max_image_dimension: 256, // PSP VRAM constraint
                url_bar_height: 14,
                status_bar_height: 10,
                smooth_scroll: false,
                features: BrowserFeatures {
                    enabled: true,
                    native_engine: true,
                    gemini: false,
                    reader_mode: true,
                    sandbox_only: false,
                    home_url: "https://info.cern.ch".to_string(),
                    max_cache_mb: 1, // PSP RAM budget -- keep small
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut widget = oasis_browser::BrowserWidget::new(config);
            widget.set_window(0, 14, 480, 244);
            // Attach TLS provider for HTTPS.
            widget.set_tls_provider(Box::new(oasis_backend_psp::PspTlsProvider::new()));
            self.widget = Some(widget);
        }
        self.widget.as_mut().expect("just initialized above")
    }

    /// Get the current URL for display.
    pub(crate) fn url(&self) -> &str {
        self.widget
            .as_ref()
            .and_then(|w| w.current_url())
            .unwrap_or("https://info.cern.ch")
    }
}

// ---------------------------------------------------------------------------
// Radio
// ---------------------------------------------------------------------------

use crate::types::RadioStatus;

pub(crate) struct RadioState {
    pub(crate) selected: usize,
    pub(crate) scroll: usize,
    pub(crate) status: RadioStatus,
    pub(crate) station_name: String,
    pub(crate) now_playing: String,
    pub(crate) error_msg: String,
}

impl RadioState {
    pub(crate) fn new() -> Self {
        Self {
            selected: 0,
            scroll: 0,
            status: RadioStatus::Stopped,
            station_name: String::new(),
            now_playing: String::new(),
            error_msg: String::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// TV Guide
// ---------------------------------------------------------------------------

pub(crate) struct TvGuideState {
    pub(crate) channels: Vec<oasis_core::apps::tv_guide::Channel>,
    pub(crate) catalogs: Vec<Option<oasis_core::apps::tv_guide::ChannelCatalog>>,
    pub(crate) selected: usize,
    pub(crate) scroll: usize,
    pub(crate) tuned: Option<usize>,
    pub(crate) downloading: bool,
    pub(crate) download_progress: f32,
    pub(crate) preview_tex: Option<TextureId>,
    pub(crate) error_msg: String,
    pub(crate) now_playing: String,
}

impl TvGuideState {
    pub(crate) fn new() -> Self {
        Self {
            channels: Vec::new(),
            catalogs: Vec::new(),
            selected: 0,
            scroll: 0,
            tuned: None,
            downloading: false,
            download_progress: 0.0,
            preview_tex: None,
            error_msg: String::new(),
            now_playing: String::new(),
        }
    }
}
