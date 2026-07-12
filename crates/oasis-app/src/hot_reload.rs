//! Development-only skin hot reload (`skin-dev` feature).
//!
//! Polls the external directory backing the active skin (~once a second
//! from the frame loop) and reports when any of its files change, so the
//! main loop can re-apply the skin from disk. This is what makes iterating
//! on a skin humane: edit `theme.toml`, save, and the running shell picks
//! it up without a restart.
//!
//! Built-in skins resolve to their compiled-in copy in `resolve_skin`, so
//! the reload path returned by [`SkinWatcher::poll`] is the *directory*
//! (e.g. `skins/classic`) — passing that to `apply_skin_swap` forces the
//! directory branch of `resolve_skin`, which re-reads the TOML and assets
//! from disk.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Frames between directory polls (~1s at 60 FPS).
pub const POLL_INTERVAL_FRAMES: u64 = 60;

/// Watches the external directory backing the active skin for edits.
pub struct SkinWatcher {
    dir: PathBuf,
    stamp: (usize, Option<SystemTime>),
}

impl Default for SkinWatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl SkinWatcher {
    pub fn new() -> Self {
        Self {
            dir: PathBuf::new(),
            stamp: (0, None),
        }
    }

    /// Poll the directory backing `skin_name` for changes.
    ///
    /// Returns the directory to reload from when a file was added, removed,
    /// or modified since the last poll. Returns `None` for skins with no
    /// on-disk directory, on the first poll after a skin swap (the new
    /// directory becomes the baseline), and when nothing changed.
    pub fn poll(&mut self, skin_name: &str) -> Option<PathBuf> {
        let dir = external_skin_dir(skin_name)?;
        let stamp = scan(&dir);
        if dir != self.dir {
            // Different skin than last poll (startup or swap): rebase the
            // baseline without triggering a reload.
            self.dir = dir;
            self.stamp = stamp;
            return None;
        }
        if stamp != self.stamp {
            self.stamp = stamp;
            return Some(self.dir.clone());
        }
        None
    }
}

/// Resolve the on-disk directory for a skin name or path, mirroring the
/// directory branches of `resolve_skin` (an explicit path containing
/// `skin.toml`, then `./skins/{name}/`).
fn external_skin_dir(name_or_path: &str) -> Option<PathBuf> {
    let path = Path::new(name_or_path);
    if path.join("skin.toml").is_file() {
        return Some(path.to_path_buf());
    }
    let skins_dir = Path::new("skins").join(name_or_path);
    if skins_dir.join("skin.toml").is_file() {
        return Some(skins_dir);
    }
    None
}

/// Fingerprint a skin directory: file count + newest mtime across the
/// top-level TOML files and the `assets/` subdirectory.
fn scan(dir: &Path) -> (usize, Option<SystemTime>) {
    let mut count = 0usize;
    let mut newest: Option<SystemTime> = None;
    for d in [dir.to_path_buf(), dir.join("assets")] {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if !meta.is_file() {
                continue;
            }
            count += 1;
            if let Ok(mtime) = meta.modified() {
                newest = Some(newest.map_or(mtime, |n| n.max(mtime)));
            }
        }
    }
    (count, newest)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The repo's skins/ directory (tests run with the crate as CWD).
    fn repo_skin(name: &str) -> String {
        format!("{}/../../skins/{name}", env!("CARGO_MANIFEST_DIR"))
    }

    #[test]
    fn external_dir_resolves_explicit_path() {
        let path = repo_skin("classic");
        assert_eq!(external_skin_dir(&path), Some(PathBuf::from(&path)));
    }

    #[test]
    fn external_dir_none_for_builtin_only() {
        assert_eq!(external_skin_dir("corrupted-nonexistent"), None);
    }

    #[test]
    fn watcher_detects_file_touch() {
        let dir = std::env::temp_dir().join(format!("oasis-skin-dev-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp skin dir");
        std::fs::write(dir.join("skin.toml"), "name = \"t\"").expect("write skin.toml");

        let name = dir.to_string_lossy().into_owned();
        let mut watcher = SkinWatcher::new();
        // First poll establishes the baseline.
        assert_eq!(watcher.poll(&name), None);
        assert_eq!(watcher.poll(&name), None);

        // A new file changes the fingerprint (count), independent of
        // filesystem mtime granularity.
        std::fs::write(dir.join("theme.toml"), "primary = \"#FF8C1E\"").expect("write theme");
        assert_eq!(watcher.poll(&name), Some(dir.clone()));
        // Quiescent again after the reload.
        assert_eq!(watcher.poll(&name), None);

        // Removing it fires too.
        std::fs::remove_file(dir.join("theme.toml")).expect("remove theme");
        assert_eq!(watcher.poll(&name), Some(dir.clone()));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn watcher_rebases_on_skin_change_without_firing() {
        let a = repo_skin("classic");
        let b = repo_skin("balatro");
        let mut watcher = SkinWatcher::new();
        assert_eq!(watcher.poll(&a), None);
        // Switching skins must not fire a spurious reload.
        assert_eq!(watcher.poll(&b), None);
        assert_eq!(watcher.poll(&b), None);
    }
}
