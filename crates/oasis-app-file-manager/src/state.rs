//! [`FileManagerApp`] struct definition, constructors, and simple
//! accessor / mutation helpers.
//!
//! Behavioural splits live in the sibling modules: [`crate::commands`]
//! handles input + menu actions, [`crate::view`] handles all rendering
//! and SDI scene updates.

use std::cell::Cell;

use oasis_app_core::ContentState;
use oasis_ui::menu_bar::MenuBar;
use oasis_vfs::Vfs;

use crate::commands::default_menu_bar;
use crate::model::{Clipboard, Dialog, FileOp, FilePanel, NavTarget, ViewMode};

use oasis_app_core::file_viewer::list_directory;

/// File Manager application with dual-panel browsing.
#[derive(Debug)]
pub struct FileManagerApp {
    /// Shared content state (title, lines, scroll, cursor, etc.).
    pub content: ContentState,
    /// Dual panels.
    pub panels: [FilePanel; 2],
    /// Which panel is active (0 = left, 1 = right).
    pub active_panel: usize,
    /// File operations queued by input handlers, applied in order by
    /// `App::apply_vfs_ops` (the only hook with mutable VFS access).
    pub pending_ops: Vec<FileOp>,
    /// Open modal prompt (delete confirmation / name entry), if any.
    pub dialog: Option<Dialog>,
    /// Copy / Cut clipboard for Paste.
    pub clipboard: Option<Clipboard>,
    /// One-line feedback for the last operation (shown in the hint /
    /// status strip until the next key press).
    pub status: Option<String>,
    /// Active view mode (toggled via Button::Select).
    pub view_mode: ViewMode,
    /// Cached column count for the Explorer icon grid (written by the
    /// renderer each frame so the next input tick can navigate by tile
    /// coordinates). `Cell` so the `&self` windowed renderer can refresh it.
    pub(crate) explorer_cols: Cell<usize>,
    /// Cached visible row count for the Explorer icon grid.
    pub(crate) explorer_visible_rows: Cell<usize>,
    /// Cached `font_hint` value from the active theme. Used by the click
    /// handler (which has no `&ActiveTheme`) to derive tree-pane row
    /// heights consistently with the renderer's
    /// `(font_hint as i32 + 2).max(11)` formula.
    pub(crate) cached_font_hint: Cell<u16>,
    /// Cached `statusbar_height + bottombar_height` from the active theme.
    /// In SDI/fullscreen mode the renderer subtracts these from `body_h`
    /// before computing geometry; the click handler needs the same value
    /// so hit-tests don't extend over the system bars.
    pub(crate) cached_system_bars: Cell<u32>,
    /// Top menu bar (File / Edit / View) shared by both view modes.
    pub menu: MenuBar,
    /// Last-clicked tile index (absolute, into `panels[active].lines`).
    /// Used to fake double-click without timing info: clicking the same
    /// already-selected tile activates it; clicking a different tile
    /// just selects it.
    pub(crate) last_click_tile: Cell<Option<usize>>,
    /// Click target waiting for vfs (applied on the next refresh tick).
    pub pending_navigation: Option<NavTarget>,
}

impl FileManagerApp {
    /// Create a new File Manager app.
    pub fn new(path: &str, vfs: &dyn Vfs) -> Self {
        let mut content = ContentState::new("File Manager", path);
        content.browse_dir = Some("/".to_string());
        content.lines = list_directory(vfs, "/");
        Self {
            content,
            panels: [FilePanel::new("/", vfs), FilePanel::new("/", vfs)],
            active_panel: 0,
            pending_ops: Vec::new(),
            dialog: None,
            clipboard: None,
            status: None,
            view_mode: ViewMode::Explorer,
            explorer_cols: Cell::new(1),
            explorer_visible_rows: Cell::new(1),
            cached_font_hint: Cell::new(11),
            cached_system_bars: Cell::new(0),
            menu: default_menu_bar(),
            last_click_tile: Cell::new(None),
            pending_navigation: None,
        }
    }

    /// Toggle between dual-panel and Explorer view modes.
    pub fn toggle_view_mode(&mut self) {
        self.view_mode = match self.view_mode {
            ViewMode::Dual => ViewMode::Explorer,
            ViewMode::Explorer => ViewMode::Dual,
        };
    }

    /// Take the oldest queued file operation, if any.
    pub fn take_file_op(&mut self) -> Option<FileOp> {
        if self.pending_ops.is_empty() {
            None
        } else {
            Some(self.pending_ops.remove(0))
        }
    }

    /// Currently active panel (the one driving Explorer view too).
    pub(crate) fn active(&self) -> &FilePanel {
        &self.panels[self.active_panel]
    }

    /// Mutable accessor for the currently active panel.
    pub(crate) fn active_mut(&mut self) -> &mut FilePanel {
        &mut self.panels[self.active_panel]
    }
}
