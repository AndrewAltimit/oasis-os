//! Game asset virtual file system.
//!
//! Two-layer VFS with an immutable base (game-authored content) and a writable
//! overlay (player modifications). Used in UE5 where each in-game terminal
//! has its own pre-populated content defined by level designers.

use std::collections::{HashMap, HashSet};

use oasis_types::error::{OasisError, Result, VfsError};

use crate::{
    AccessMode, EntryKind, FilePermissions, RenamePlan, Vfs, VfsContext, VfsEntry, VfsMetadata,
    normalize_path, parent_path as parent,
};

/// An entry in the game asset VFS.
#[derive(Debug, Clone)]
enum Node {
    File(Vec<u8>),
    Dir,
}

/// Two-layer VFS: immutable base + writable overlay.
///
/// The base layer represents game-authored content that the player can read
/// but not directly modify. The overlay layer captures any writes the player
/// makes. A deleted set tracks base entries that have been "removed".
///
/// This enables gameplay patterns like:
/// - Pre-populated lore files that the player can `cat` but not accidentally destroy
/// - Writable directories where the player can create files to solve puzzles
/// - "Hacking" scenarios where the player edits config files (overlay captures edits)
pub struct GameAssetVfs {
    /// Immutable layer containing game-authored content.
    base: HashMap<String, Node>,
    /// Writable layer capturing player modifications.
    overlay: HashMap<String, Node>,
    /// Paths marked as deleted (hides base entries).
    deleted: HashSet<String>,
    permissions: HashMap<String, FilePermissions>,
    /// Current user context for permission enforcement.
    ctx: VfsContext,
}

impl GameAssetVfs {
    /// Create a new empty GameAssetVfs with only the root directory.
    pub fn new() -> Self {
        let mut base = HashMap::new();
        base.insert("/".to_string(), Node::Dir);
        Self {
            base,
            overlay: HashMap::new(),
            deleted: HashSet::new(),
            permissions: HashMap::new(),
            ctx: VfsContext::default_user(),
        }
    }

    /// Add a file to the immutable base layer.
    ///
    /// Parent directories are created automatically. A path that
    /// [`normalize_path`] rejects (climbs above `/`, contains NUL), or that
    /// would place the file beneath an existing base file, is ignored.
    pub fn add_base_file(&mut self, path: &str, data: &[u8]) {
        self.add_base(path, Node::File(data.to_vec()));
    }

    /// Add a directory to the immutable base layer (same rules as
    /// [`add_base_file`](Self::add_base_file)).
    pub fn add_base_dir(&mut self, path: &str) {
        self.add_base(path, Node::Dir);
    }

    fn add_base(&mut self, path: &str, node: Node) {
        let Ok(path) = normalize_path(path) else {
            return;
        };
        if path == "/" || !ensure_parents(&mut self.base, &path) {
            return;
        }
        self.base.insert(path.into_owned(), node);
    }

    /// Fail unless `path` is an existing directory (the parent check of
    /// `write` / `mkdir`: nothing may be created beneath a file).
    fn require_dir(&self, path: &str) -> Result<()> {
        match self.effective_entry(path) {
            Some(Node::Dir) => Ok(()),
            Some(Node::File(_)) => Err(VfsError::NotADirectory { path: path.into() }.into()),
            None => Err(OasisError::Vfs(
                format!("parent directory does not exist: {path}").into(),
            )),
        }
    }

    /// Look up the effective entry at a path (overlay wins, deleted hides).
    fn effective_entry(&self, path: &str) -> Option<&Node> {
        if self.deleted.contains(path) {
            return None;
        }
        self.overlay.get(path).or_else(|| self.base.get(path))
    }
}

impl Default for GameAssetVfs {
    fn default() -> Self {
        Self::new()
    }
}

impl Vfs for GameAssetVfs {
    fn vfs_type(&self) -> &'static str {
        "GameAssetVfs"
    }

    fn readdir(&self, path: &str) -> Result<Vec<VfsEntry>> {
        let path = normalize_path(path)?;
        match self.effective_entry(&path) {
            Some(Node::Dir) => {
                self.check_permission(&path, AccessMode::Read)?;
            },
            Some(Node::File(_)) => {
                return Err(OasisError::Vfs(format!("not a directory: {path}").into()));
            },
            None => {
                return Err(OasisError::Vfs(format!("no such directory: {path}").into()));
            },
        }

        let prefix = if path == "/" {
            "/".to_string()
        } else {
            format!("{path}/")
        };

        // Merge entries from both layers using a HashMap to deduplicate.
        let mut entries: HashMap<String, (EntryKind, u64)> = HashMap::new();

        // Base layer (skip deleted entries).
        for (p, node) in &self.base {
            if self.deleted.contains(p) {
                continue;
            }
            if let Some(name) = direct_child(p, &prefix) {
                let (kind, size) = match node {
                    Node::File(data) => (EntryKind::File, data.len() as u64),
                    Node::Dir => (EntryKind::Directory, 0),
                };
                entries.insert(name, (kind, size));
            }
        }

        // Overlay overrides base.
        for (p, node) in &self.overlay {
            if let Some(name) = direct_child(p, &prefix) {
                let (kind, size) = match node {
                    Node::File(data) => (EntryKind::File, data.len() as u64),
                    Node::Dir => (EntryKind::Directory, 0),
                };
                entries.insert(name, (kind, size));
            }
        }

        let mut result: Vec<VfsEntry> = entries
            .into_iter()
            .map(|(name, (kind, size))| VfsEntry { name, kind, size })
            .collect();
        result.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(result)
    }

    fn read(&self, path: &str) -> Result<Vec<u8>> {
        let path = normalize_path(path)?;
        match self.effective_entry(&path) {
            Some(Node::File(data)) => {
                self.check_permission(&path, AccessMode::Read)?;
                Ok(data.clone())
            },
            Some(Node::Dir) => Err(OasisError::Vfs(format!("is a directory: {path}").into())),
            None => Err(OasisError::Vfs(format!("no such file: {path}").into())),
        }
    }

    fn write(&mut self, path: &str, data: &[u8]) -> Result<()> {
        let path = normalize_path(path)?;
        let par = parent(&path);
        self.require_dir(par)?;
        // Check parent directory write permission.
        self.check_permission(par, AccessMode::Write)?;
        // Check target file write permission (if it already exists).
        match self.effective_entry(&path) {
            Some(Node::Dir) => {
                return Err(VfsError::IsDirectory {
                    path: path.into_owned(),
                }
                .into());
            },
            Some(Node::File(_)) => self.check_permission(&path, AccessMode::Write)?,
            None => {},
        }
        let path = path.into_owned();
        self.deleted.remove(&path);
        self.permissions
            .entry(path.clone())
            .or_insert_with(FilePermissions::default_file);
        self.overlay.insert(path, Node::File(data.to_vec()));
        Ok(())
    }

    fn stat(&self, path: &str) -> Result<VfsMetadata> {
        let path = normalize_path(path)?;
        match self.effective_entry(&path) {
            Some(Node::File(data)) => Ok(VfsMetadata {
                kind: EntryKind::File,
                size: data.len() as u64,
            }),
            Some(Node::Dir) => Ok(VfsMetadata {
                kind: EntryKind::Directory,
                size: 0,
            }),
            None => Err(OasisError::Vfs(format!("no such path: {path}").into())),
        }
    }

    fn mkdir(&mut self, path: &str) -> Result<()> {
        let path = normalize_path(path)?;
        match self.effective_entry(&path) {
            Some(Node::Dir) => return Ok(()),
            Some(Node::File(_)) => {
                return Err(OasisError::Vfs(format!("file exists: {path}").into()));
            },
            None => {},
        }
        // Ensure parent exists.
        let par = parent(&path).to_string();
        if self.effective_entry(&par).is_none() {
            self.mkdir(&par)?;
        }
        self.require_dir(&par)?;
        // Check parent directory write permission.
        self.check_permission(&par, AccessMode::Write)?;
        let path = path.into_owned();
        self.deleted.remove(&path);
        self.permissions
            .entry(path.clone())
            .or_insert_with(FilePermissions::default_dir);
        self.overlay.insert(path, Node::Dir);
        Ok(())
    }

    fn remove(&mut self, path: &str) -> Result<()> {
        let path = normalize_path(path)?;
        if path == "/" {
            return Err(OasisError::Vfs("cannot remove root".into()));
        }
        // Check parent directory write permission.
        let par = parent(&path);
        self.check_permission(par, AccessMode::Write)?;
        // Check target's own permission.
        if self.effective_entry(&path).is_some() {
            self.check_permission(&path, AccessMode::Write)?;
        }
        match self.effective_entry(&path) {
            Some(Node::Dir) => {
                let prefix = format!("{path}/");
                // Check both layers for children (non-deleted).
                let has_children = self
                    .base
                    .keys()
                    .filter(|k| !self.deleted.contains(*k))
                    .chain(self.overlay.keys())
                    .any(|k| k.starts_with(&prefix));
                if has_children {
                    return Err(OasisError::Vfs(
                        format!("directory not empty: {path}").into(),
                    ));
                }
            },
            Some(Node::File(_)) => {},
            None => {
                return Err(OasisError::Vfs(format!("no such path: {path}").into()));
            },
        }
        let path = path.into_owned();
        self.overlay.remove(&path);
        self.permissions.remove(&path);
        self.deleted.insert(path);
        Ok(())
    }

    fn rename(&mut self, from: &str, to: &str) -> Result<()> {
        let from = normalize_path(from)?;
        let to = normalize_path(to)?;
        if self.validate_rename(&from, &to)? == RenamePlan::NoOp {
            return Ok(());
        }
        // Only allow rename if the source exists in the overlay layer.
        // Base-layer-only entries cannot be renamed (they are immutable).
        if !self.overlay.contains_key(from.as_ref()) {
            return Err(OasisError::Vfs(VfsError::Unsupported {
                path: from.into_owned(),
                reason: "cannot rename base-layer entry not in overlay",
            }));
        }
        let prefix = format!("{from}/");
        // A visible base-layer descendant cannot move with its directory.
        if self
            .base
            .keys()
            .any(|k| k.starts_with(&prefix) && !self.deleted.contains(k))
        {
            return Err(OasisError::Vfs(VfsError::Unsupported {
                path: from.into_owned(),
                reason: "cannot rename directory containing base-layer entries",
            }));
        }
        // Renaming rewrites an entry in both parent directories.
        self.check_permission(parent(&from), AccessMode::Write)?;
        self.check_permission(parent(&to), AccessMode::Write)?;
        // Collect all overlay entries to move (source + children if directory).
        let keys_to_move: Vec<String> = self
            .overlay
            .keys()
            .filter(|k| *k == &from || k.starts_with(&prefix))
            .cloned()
            .collect();
        for old_key in keys_to_move {
            let new_key = if old_key == from {
                to.to_string()
            } else {
                format!("{to}{}", &old_key[from.len()..])
            };
            // A base entry the overlay was shadowing must stay hidden, or
            // it would reappear at the old path once the overlay moves.
            if self.base.contains_key(&old_key) {
                self.deleted.insert(old_key.clone());
            }
            // The destination may have been deleted earlier; un-hide it.
            self.deleted.remove(&new_key);
            if let Some(node) = self.overlay.remove(&old_key) {
                self.overlay.insert(new_key.clone(), node);
            }
            if let Some(perms) = self.permissions.remove(&old_key) {
                self.permissions.insert(new_key, perms);
            }
        }
        Ok(())
    }

    fn exists(&self, path: &str) -> bool {
        normalize_path(path).is_ok_and(|path| self.effective_entry(&path).is_some())
    }

    fn get_permissions(&self, path: &str) -> Result<FilePermissions> {
        let path = normalize_path(path)?;
        if self.effective_entry(&path).is_none() {
            return Err(OasisError::Vfs(format!("no such path: {path}").into()));
        }
        Ok(self
            .permissions
            .get(path.as_ref())
            .cloned()
            .unwrap_or_else(|| match self.effective_entry(&path) {
                Some(Node::Dir) => FilePermissions::default_dir(),
                _ => FilePermissions::default_file(),
            }))
    }

    fn set_permissions(&mut self, path: &str, perms: FilePermissions) -> Result<()> {
        let path = normalize_path(path)?;
        if self.effective_entry(&path).is_none() {
            return Err(OasisError::Vfs(format!("no such path: {path}").into()));
        }
        self.permissions.insert(path.into_owned(), perms);
        Ok(())
    }

    fn context(&self) -> VfsContext {
        self.ctx.clone()
    }

    fn set_context(&mut self, ctx: VfsContext) {
        self.ctx = ctx;
    }
}

/// Extract the direct child name from a full path given a parent prefix.
fn direct_child(full_path: &str, prefix: &str) -> Option<String> {
    let rest = full_path.strip_prefix(prefix)?;
    if rest.is_empty() || rest.contains('/') {
        return None;
    }
    Some(rest.to_string())
}

/// Ensure all ancestor directories of `path` exist in the given map.
/// Returns `false` (leaving the map untouched) when an ancestor is a file.
fn ensure_parents(map: &mut HashMap<String, Node>, path: &str) -> bool {
    let ancestors: Vec<&str> = path
        .match_indices('/')
        .skip(1)
        .map(|(i, _)| &path[..i])
        .collect();
    if ancestors
        .iter()
        .any(|a| matches!(map.get(*a), Some(Node::File(_))))
    {
        return false;
    }
    for ancestor in ancestors {
        map.entry(ancestor.to_string()).or_insert(Node::Dir);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_exists() {
        let vfs = GameAssetVfs::new();
        assert!(vfs.exists("/"));
    }

    #[test]
    fn base_files_readable() {
        let mut vfs = GameAssetVfs::new();
        vfs.add_base_dir("/home");
        vfs.add_base_file("/home/readme.txt", b"Hello from the game!");
        assert_eq!(
            vfs.read("/home/readme.txt").unwrap(),
            b"Hello from the game!"
        );
    }

    #[test]
    fn base_dirs_listable() {
        let mut vfs = GameAssetVfs::new();
        vfs.add_base_dir("/home");
        vfs.add_base_dir("/etc");
        let entries = vfs.readdir("/").unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"home"));
        assert!(names.contains(&"etc"));
    }

    #[test]
    fn overlay_write_creates_file() {
        let mut vfs = GameAssetVfs::new();
        vfs.add_base_dir("/tmp");
        vfs.write("/tmp/player.txt", b"player data").unwrap();
        assert_eq!(vfs.read("/tmp/player.txt").unwrap(), b"player data");
    }

    #[test]
    fn overlay_overrides_base() {
        let mut vfs = GameAssetVfs::new();
        vfs.add_base_dir("/etc");
        vfs.add_base_file("/etc/config", b"original");
        // Player edits the config.
        vfs.write("/etc/config", b"modified by player").unwrap();
        assert_eq!(vfs.read("/etc/config").unwrap(), b"modified by player");
    }

    #[test]
    fn readdir_merges_layers() {
        let mut vfs = GameAssetVfs::new();
        vfs.add_base_dir("/data");
        vfs.add_base_file("/data/base.txt", b"from game");
        vfs.write("/data/player.txt", b"from player").unwrap();
        let entries = vfs.readdir("/data").unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"base.txt"));
        assert!(names.contains(&"player.txt"));
    }

    #[test]
    fn delete_base_file() {
        let mut vfs = GameAssetVfs::new();
        vfs.add_base_dir("/docs");
        vfs.add_base_file("/docs/secret.txt", b"hidden");
        assert!(vfs.exists("/docs/secret.txt"));
        vfs.remove("/docs/secret.txt").unwrap();
        assert!(!vfs.exists("/docs/secret.txt"));
    }

    #[test]
    fn delete_overlay_file() {
        let mut vfs = GameAssetVfs::new();
        vfs.add_base_dir("/tmp");
        vfs.write("/tmp/scratch", b"temp").unwrap();
        vfs.remove("/tmp/scratch").unwrap();
        assert!(!vfs.exists("/tmp/scratch"));
    }

    #[test]
    fn undelete_via_write() {
        let mut vfs = GameAssetVfs::new();
        vfs.add_base_dir("/etc");
        vfs.add_base_file("/etc/conf", b"old");
        vfs.remove("/etc/conf").unwrap();
        assert!(!vfs.exists("/etc/conf"));
        // Writing to a deleted path resurrects it in the overlay.
        vfs.write("/etc/conf", b"new").unwrap();
        assert!(vfs.exists("/etc/conf"));
        assert_eq!(vfs.read("/etc/conf").unwrap(), b"new");
    }

    #[test]
    fn remove_nonempty_dir_fails() {
        let mut vfs = GameAssetVfs::new();
        vfs.add_base_dir("/dir");
        vfs.add_base_file("/dir/file", b"x");
        assert!(vfs.remove("/dir").is_err());
    }

    #[test]
    fn remove_root_fails() {
        let mut vfs = GameAssetVfs::new();
        assert!(vfs.remove("/").is_err());
    }

    #[test]
    fn write_without_parent_fails() {
        let mut vfs = GameAssetVfs::new();
        assert!(vfs.write("/no/such/dir/file", b"x").is_err());
    }

    #[test]
    fn mkdir_creates_parents_in_overlay() {
        let mut vfs = GameAssetVfs::new();
        vfs.mkdir("/a/b/c").unwrap();
        assert!(vfs.exists("/a"));
        assert!(vfs.exists("/a/b"));
        assert!(vfs.exists("/a/b/c"));
    }

    #[test]
    fn stat_base_file() {
        let mut vfs = GameAssetVfs::new();
        vfs.add_base_file("/file", b"abc");
        let meta = vfs.stat("/file").unwrap();
        assert_eq!(meta.kind, EntryKind::File);
        assert_eq!(meta.size, 3);
    }

    #[test]
    fn stat_overlay_dir() {
        let mut vfs = GameAssetVfs::new();
        vfs.mkdir("/mydir").unwrap();
        let meta = vfs.stat("/mydir").unwrap();
        assert_eq!(meta.kind, EntryKind::Directory);
    }

    #[test]
    fn deleted_base_not_in_readdir() {
        let mut vfs = GameAssetVfs::new();
        vfs.add_base_dir("/data");
        vfs.add_base_file("/data/a.txt", b"a");
        vfs.add_base_file("/data/b.txt", b"b");
        vfs.remove("/data/a.txt").unwrap();
        let entries = vfs.readdir("/data").unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(!names.contains(&"a.txt"));
        assert!(names.contains(&"b.txt"));
    }

    #[test]
    fn default_constructor() {
        let vfs = GameAssetVfs::default();
        assert!(vfs.exists("/"));
    }

    #[test]
    fn permissions_enforced() {
        let mut vfs = GameAssetVfs::new();
        vfs.add_base_dir("/etc");
        vfs.add_base_file("/etc/config", b"original");
        vfs.set_permissions(
            "/etc/config",
            FilePermissions {
                owner: "oasis".to_string(),
                mode: 0o444,
            },
        )
        .unwrap();
        assert!(vfs.write("/etc/config", b"hacked").is_err());
        assert_eq!(vfs.read("/etc/config").unwrap(), b"original");
    }

    #[test]
    fn rename_overlay_file() {
        let mut vfs = GameAssetVfs::new();
        vfs.add_base_dir("/src");
        vfs.add_base_dir("/dst");
        vfs.write("/src/file.txt", b"player data").unwrap();
        vfs.rename("/src/file.txt", "/dst/moved.txt").unwrap();
        assert!(!vfs.exists("/src/file.txt"));
        assert_eq!(vfs.read("/dst/moved.txt").unwrap(), b"player data");
    }

    #[test]
    fn rename_base_only_file_fails() {
        let mut vfs = GameAssetVfs::new();
        vfs.add_base_dir("/src");
        vfs.add_base_dir("/dst");
        vfs.add_base_file("/src/base.txt", b"immutable");
        assert!(vfs.rename("/src/base.txt", "/dst/moved.txt").is_err());
    }

    #[test]
    fn rename_nonexistent_fails() {
        let mut vfs = GameAssetVfs::new();
        assert!(vfs.rename("/ghost", "/dest").is_err());
    }

    #[test]
    fn rename_overlay_dir_moves_children() {
        let mut vfs = GameAssetVfs::new();
        vfs.mkdir("/old/sub").unwrap();
        vfs.write("/old/sub/file.txt", b"data").unwrap();
        vfs.rename("/old", "/new").unwrap();
        assert!(!vfs.exists("/old"));
        assert!(vfs.exists("/new"));
        assert!(vfs.exists("/new/sub"));
        assert_eq!(vfs.read("/new/sub/file.txt").unwrap(), b"data");
    }

    #[test]
    fn rename_missing_dest_parent_fails() {
        let mut vfs = GameAssetVfs::new();
        vfs.mkdir("/src").unwrap();
        vfs.write("/src/file.txt", b"data").unwrap();
        assert!(vfs.rename("/src/file.txt", "/no/such/dir/file").is_err());
    }

    #[test]
    fn add_base_ignores_invalid_paths() {
        let mut vfs = GameAssetVfs::new();
        vfs.add_base_file("/f", b"x");
        vfs.add_base_file("/f/under_file", b"y");
        vfs.add_base_file("/../escape", b"z");
        vfs.add_base_dir("/");
        assert!(!vfs.exists("/f/under_file"));
        assert!(!vfs.exists("/escape"));
        assert_eq!(vfs.stat("/").unwrap().kind, EntryKind::Directory);
        assert_eq!(vfs.read("/f").unwrap(), b"x");
    }

    #[test]
    fn write_onto_base_directory_fails() {
        let mut vfs = GameAssetVfs::new();
        vfs.add_base_file("/dir/child", b"x");
        assert!(vfs.write("/dir", b"shadow").is_err());
        assert_eq!(vfs.readdir("/dir").unwrap().len(), 1);
    }

    #[test]
    fn add_base_file_creates_parents() {
        let mut vfs = GameAssetVfs::new();
        vfs.add_base_file("/a/b/c/file.txt", b"deep");
        assert!(vfs.exists("/a"));
        assert!(vfs.exists("/a/b"));
        assert!(vfs.exists("/a/b/c"));
        assert_eq!(vfs.read("/a/b/c/file.txt").unwrap(), b"deep");
    }
}
