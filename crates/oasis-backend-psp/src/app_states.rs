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
// Browser
// ---------------------------------------------------------------------------

pub(crate) struct BrowserState {
    pub(crate) url: String,
    pub(crate) content_lines: Vec<String>,
    pub(crate) scroll: usize,
    pub(crate) loading: bool,
    pub(crate) status_msg: String,
}

impl BrowserState {
    pub(crate) fn new() -> Self {
        Self {
            url: String::from("http://info.cern.ch"),
            content_lines: Vec::new(),
            scroll: 0,
            loading: false,
            status_msg: String::from("Press [] to enter URL"),
        }
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
