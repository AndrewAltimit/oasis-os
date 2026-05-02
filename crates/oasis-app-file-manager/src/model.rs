//! Data types and pure helpers for the File Manager app.
//!
//! This module owns the per-panel state ([`FilePanel`]), the value enums
//! ([`FileOp`], [`NavTarget`], [`ViewMode`]), the folder-tree row type
//! ([`TreeEntry`]), and the side-effect-free helpers used by both the
//! rendering layer and the input/command layer (`parse_entry`,
//! `truncate_label`, `build_tree_entries`).

use oasis_app_core::file_viewer::{join_path, list_directory, parent_dir};
use oasis_vfs::Vfs;

// ---------------------------------------------------------------
// FilePanel: per-panel state for dual-panel browsing
// ---------------------------------------------------------------

/// Per-panel state for dual-panel file browsing.
#[derive(Debug, Clone)]
pub struct FilePanel {
    /// Current directory being browsed.
    pub browse_dir: String,
    /// Display lines for the current directory.
    pub lines: Vec<String>,
    /// Scroll offset.
    pub scroll: usize,
    /// Cursor position (relative to visible area).
    pub cursor: usize,
    /// Cached folder-tree entries for the left-pane (Explorer view).
    /// Rebuilt on every navigation via [`Self::refresh`] so the tree
    /// shows siblings of the current path at each ancestor level.
    pub tree_entries: Vec<TreeEntry>,
}

impl FilePanel {
    /// Create a new panel rooted at the given directory.
    pub fn new(dir: &str, vfs: &dyn Vfs) -> Self {
        let lines = list_directory(vfs, dir);
        let tree_entries = build_tree_entries(dir, vfs);
        Self {
            browse_dir: dir.to_string(),
            lines,
            scroll: 0,
            cursor: 0,
            tree_entries,
        }
    }

    pub(crate) fn visible_count(&self, max_visible: usize) -> usize {
        let remaining = self.lines.len().saturating_sub(self.scroll);
        remaining.min(max_visible)
    }

    pub(crate) fn navigate_up(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        } else if self.scroll > 0 {
            self.scroll -= 1;
        }
    }

    pub(crate) fn navigate_down(&mut self, max_visible: usize) {
        let visible = self.visible_count(max_visible);
        if self.cursor + 1 < visible {
            self.cursor += 1;
        } else if self.scroll + max_visible < self.lines.len() {
            self.scroll += 1;
        }
    }

    pub(crate) fn enter_selected(&mut self, vfs: &dyn Vfs) {
        let abs_idx = self.scroll + self.cursor;
        let Some(line) = self.lines.get(abs_idx) else {
            return;
        };
        let line = line.trim().to_string();

        if line == ".." {
            let parent = parent_dir(&self.browse_dir);
            self.navigate_to(&parent, vfs);
        } else if line.ends_with('/') {
            let name = &line[..line.len() - 1];
            let new_dir = join_path(&self.browse_dir, name);
            self.navigate_to(&new_dir, vfs);
        }
    }

    pub(crate) fn enter_selected_parent(&mut self, vfs: &dyn Vfs) {
        let parent = parent_dir(&self.browse_dir);
        self.navigate_to(&parent, vfs);
    }

    /// Navigate to `dir` and rebuild the cached listing + folder tree.
    pub fn navigate_to(&mut self, dir: &str, vfs: &dyn Vfs) {
        self.browse_dir = dir.to_string();
        self.lines = list_directory(vfs, dir);
        self.tree_entries = build_tree_entries(dir, vfs);
        self.scroll = 0;
        self.cursor = 0;
    }

    /// Return the full path of the currently selected entry (if any).
    pub(crate) fn selected_path(&self) -> Option<String> {
        let abs_idx = self.scroll + self.cursor;
        let line = self.lines.get(abs_idx)?;
        let name = line.trim();
        if name == ".." {
            return None;
        }
        // Strip trailing '/' for directories, and strip size suffix for files.
        let name = name
            .strip_suffix('/')
            .unwrap_or_else(|| name.split("  (").next().unwrap_or(name));
        Some(join_path(&self.browse_dir, name))
    }

    /// Refresh the panel listing from VFS.
    pub fn refresh(&mut self, vfs: &dyn Vfs) {
        self.lines = list_directory(vfs, &self.browse_dir);
        self.tree_entries = build_tree_entries(&self.browse_dir, vfs);
        // Clamp cursor to new list size.
        let max = self.lines.len().saturating_sub(1);
        self.cursor = self.cursor.min(max);
        // Clamp scroll so it never points past the new list end, otherwise
        // a directory shrink leaves the main pane rendering empty until the
        // user manually scrolls back up.
        self.scroll = self.scroll.min(max);
    }
}

// ---------------------------------------------------------------
// Pending operations and view modes
// ---------------------------------------------------------------

/// A pending VFS operation for the file manager.
#[derive(Debug, Clone)]
pub enum FileOp {
    /// Delete the file or directory at this path.
    Delete(String),
    /// Create a directory at this path.
    Mkdir(String),
}

/// A target the user activated by clicking and that needs vfs to apply.
/// `App::handle_click` doesn't get a vfs reference, so the click handler
/// queues the action here and `App::refresh(vfs)` consumes it.
#[derive(Debug, Clone)]
pub enum NavTarget {
    /// Navigate the active panel to this absolute path.
    Folder(String),
    /// Open this file in the embedded viewer.
    File(String),
}

/// Which presentation the file manager is using.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    /// Twin text panels (TUI feel) -- the original layout.
    Dual,
    /// Single-pane Win2K Explorer-style icon grid with folder tree.
    Explorer,
}

// ---------------------------------------------------------------
// Folder-tree row + entry classification
// ---------------------------------------------------------------

/// A single row in the Explorer view's left-hand folder tree.
#[derive(Debug, Clone)]
pub struct TreeEntry {
    pub label: String,
    pub depth: usize,
    pub is_current: bool,
    /// Absolute path the row represents. Used directly by hit-testing.
    pub path: String,
}

/// Kind of entry shown in the Explorer view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EntryKind {
    File,
    Dir,
    ParentDir,
}

pub(crate) fn parse_entry(line: &str) -> (String, EntryKind) {
    let trimmed = line.trim();
    if trimmed == ".." {
        ("..".to_string(), EntryKind::ParentDir)
    } else if let Some(name) = trimmed.strip_suffix('/') {
        (name.to_string(), EntryKind::Dir)
    } else {
        let name = trimmed.split("  (").next().unwrap_or(trimmed).to_string();
        (name, EntryKind::File)
    }
}

pub(crate) fn truncate_label(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars || max_chars < 2 {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('\u{2026}');
    out
}

/// Build the folder-tree-pane entries for `current`. The tree shows the
/// "Desktop" alias, root, and at every ancestor level the full set of
/// child directories — the directory on the path-to-current is expanded
/// inline. This makes the left pane an actual navigable tree instead of
/// a path crumb.
pub(crate) fn build_tree_entries(current: &str, vfs: &dyn Vfs) -> Vec<TreeEntry> {
    let mut out = Vec::new();
    out.push(TreeEntry {
        label: "Desktop".to_string(),
        depth: 0,
        is_current: false,
        path: "/".to_string(),
    });

    let trimmed = current.trim_start_matches('/');
    out.push(TreeEntry {
        label: "/".to_string(),
        depth: 1,
        is_current: trimmed.is_empty(),
        path: "/".to_string(),
    });

    let parts: Vec<&str> = trimmed.split('/').filter(|p| !p.is_empty()).collect();
    expand_dir_into_tree("/", &parts, 2, &mut out, vfs);
    out
}

/// Recursively append child directories of `dir_path` to `out`, expanding
/// the entry that lies on the path-to-current (`remaining`) so the user
/// sees siblings at every ancestor level. Listings are sorted
/// case-insensitively for stable display.
fn expand_dir_into_tree(
    dir_path: &str,
    remaining: &[&str],
    depth: usize,
    out: &mut Vec<TreeEntry>,
    vfs: &dyn Vfs,
) {
    let Ok(entries) = vfs.readdir(dir_path) else {
        return;
    };
    let mut dirs: Vec<&oasis_vfs::VfsEntry> = entries
        .iter()
        .filter(|e| e.kind == oasis_vfs::EntryKind::Directory)
        .collect();
    dirs.sort_by(|a, b| {
        a.name
            .to_ascii_lowercase()
            .cmp(&b.name.to_ascii_lowercase())
    });

    let next_part = remaining.first().copied();
    for d in dirs {
        let child_path = if dir_path == "/" {
            format!("/{}", d.name)
        } else {
            format!("{}/{}", dir_path, d.name)
        };
        let on_path = next_part == Some(d.name.as_str());
        let is_current = on_path && remaining.len() == 1;
        out.push(TreeEntry {
            label: d.name.clone(),
            depth,
            is_current,
            path: child_path.clone(),
        });
        if on_path && remaining.len() > 1 {
            // Ancestor on the path: keep walking down.
            expand_dir_into_tree(&child_path, &remaining[1..], depth + 1, out, vfs);
        } else if is_current {
            // Reached the current directory: list its child folders so the
            // tree can be used to step further down without leaving it.
            expand_dir_into_tree(&child_path, &[], depth + 1, out, vfs);
        }
    }
}
