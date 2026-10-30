//! Virtual File System abstraction.
//!
//! The VFS trait provides a uniform interface over fundamentally different
//! storage backends. On PSP, `ls` lists Memory Stick contents. On Pi, it
//! lists real Linux directories. In UE5, it lists game-authored content.
//! In tests, MemoryVfs provides a fully in-memory tree.

mod game_asset;
mod memory;
#[cfg(not(target_arch = "wasm32"))]
mod real;

pub use game_asset::GameAssetVfs;
pub use memory::MemoryVfs;
#[cfg(not(target_arch = "wasm32"))]
pub use real::RealVfs;

use oasis_types::error::{OasisError, Result, VfsError};

/// Type of a VFS entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    /// Regular file.
    File,
    /// Directory.
    Directory,
}

/// Access mode for permission checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessMode {
    /// Read access (file contents or directory listing).
    Read,
    /// Write access (create, modify, or delete).
    Write,
    /// Execute access (run a script or enter a directory).
    Execute,
}

/// Context for VFS permission checks.
///
/// Tracks the current user identity and whether root privileges apply.
/// Root bypasses all permission checks.
#[derive(Debug, Clone)]
pub struct VfsContext {
    /// The current user name (compared against `FilePermissions::owner`).
    pub current_user: String,
    /// When true, all permission checks are bypassed.
    pub is_root: bool,
}

impl VfsContext {
    /// Create a default context for the "oasis" user (non-root).
    pub fn default_user() -> Self {
        Self {
            current_user: "oasis".to_string(),
            is_root: false,
        }
    }

    /// Create a root context that bypasses all permission checks.
    pub fn root() -> Self {
        Self {
            current_user: "root".to_string(),
            is_root: true,
        }
    }
}

/// A single entry returned by `readdir`.
#[derive(Debug, Clone)]
pub struct VfsEntry {
    /// Name of the file or directory (basename, not full path).
    pub name: String,
    /// Whether this entry is a file or directory.
    pub kind: EntryKind,
    /// Size in bytes (0 for directories).
    pub size: u64,
}

/// Metadata about a file or directory.
#[derive(Debug, Clone)]
pub struct VfsMetadata {
    /// Whether this is a file or directory.
    pub kind: EntryKind,
    /// Size in bytes (0 for directories).
    pub size: u64,
}

/// Unix-style file permissions (owner + mode).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilePermissions {
    /// Owner of the file (default: "oasis").
    pub owner: String,
    /// Unix-style octal mode (default: 0o644 for files, 0o755 for dirs).
    pub mode: u16,
}

impl FilePermissions {
    /// Default permissions for a regular file (mode 0o644, owner "oasis").
    pub fn default_file() -> Self {
        Self {
            owner: "oasis".to_string(),
            mode: 0o644,
        }
    }
    /// Default permissions for a directory (mode 0o755, owner "oasis").
    pub fn default_dir() -> Self {
        Self {
            owner: "oasis".to_string(),
            mode: 0o755,
        }
    }
    /// Check if the owner has write permission.
    pub fn owner_can_write(&self) -> bool {
        self.mode & 0o200 != 0
    }
    /// Check if the owner has read permission.
    pub fn owner_can_read(&self) -> bool {
        self.mode & 0o400 != 0
    }
    /// Check if the owner has execute permission.
    pub fn owner_can_execute(&self) -> bool {
        self.mode & 0o100 != 0
    }
    /// Check if the "other" bits grant read permission.
    pub fn other_can_read(&self) -> bool {
        self.mode & 0o004 != 0
    }
    /// Check if the "other" bits grant write permission.
    pub fn other_can_write(&self) -> bool {
        self.mode & 0o002 != 0
    }
    /// Check if the "other" bits grant execute permission.
    pub fn other_can_execute(&self) -> bool {
        self.mode & 0o001 != 0
    }

    /// Check whether `ctx` has the given `mode` of access to this entry.
    ///
    /// Root always passes. Owner bits are used when the context user matches
    /// the file owner; otherwise the "other" bits are checked.
    pub fn allows(&self, ctx: &VfsContext, mode: AccessMode) -> bool {
        if ctx.is_root {
            return true;
        }
        if ctx.current_user == self.owner {
            match mode {
                AccessMode::Read => self.owner_can_read(),
                AccessMode::Write => self.owner_can_write(),
                AccessMode::Execute => self.owner_can_execute(),
            }
        } else {
            match mode {
                AccessMode::Read => self.other_can_read(),
                AccessMode::Write => self.other_can_write(),
                AccessMode::Execute => self.other_can_execute(),
            }
        }
    }
}

/// What [`Vfs::validate_rename`] decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenamePlan {
    /// The rename is valid; perform it.
    Move,
    /// Source and destination are the same path; do nothing.
    NoOp,
}

/// Parent of a normalized absolute path (`/` for top-level entries and root).
pub(crate) fn parent_path(path: &str) -> &str {
    match path.rfind('/') {
        Some(0) | None => "/",
        Some(i) => &path[..i],
    }
}

/// Normalize a VFS path lexically, the one path form every backend uses.
///
/// The result starts with `/`, has no empty or `.` segments and no trailing
/// `/` (except the root itself); each `..` removes the segment before it. A
/// relative path is taken relative to `/`. Only `/` separates segments: a
/// `\` is an ordinary file-name character here (`RealVfs` additionally
/// treats it as a separator on Windows, where the host does).
///
/// Already-normal paths are returned borrowed, without allocating.
///
/// # Errors
///
/// - [`VfsError::PathTraversal`] when a `..` would climb above `/` -- the
///   VFS root is the top of the tree, so `/../x` is rejected rather than
///   silently clamped to `/x`;
/// - [`VfsError::Other`] when the path contains a NUL byte (no backend can
///   store one, and the host filesystem would truncate at it).
pub fn normalize_path(path: &str) -> Result<std::borrow::Cow<'_, str>> {
    use std::borrow::Cow;
    if path.contains('\0') {
        return Err(VfsError::Other(format!(
            "invalid path (contains NUL byte): {}",
            path.escape_debug()
        ))
        .into());
    }
    if is_normalized(path) {
        return Ok(Cow::Borrowed(path));
    }
    let mut parts: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {},
            ".." => {
                if parts.pop().is_none() {
                    return Err(VfsError::PathTraversal { path: path.into() }.into());
                }
            },
            s => parts.push(s),
        }
    }
    Ok(Cow::Owned(format!("/{}", parts.join("/"))))
}

/// Whether `path` is already in the form [`normalize_path`] produces.
fn is_normalized(path: &str) -> bool {
    if path == "/" {
        return true;
    }
    let Some(rest) = path.strip_prefix('/') else {
        return false;
    };
    rest.split('/').all(|seg| !matches!(seg, "" | "." | ".."))
}

/// The virtual file system trait.
///
/// All file operations in the command interpreter go through this trait.
/// Paths are forward-slash separated and absolute (starting with `/`).
///
/// Every implementation runs incoming paths through [`normalize_path`]:
/// `//`, `.` and a trailing `/` are ignored, `..` is resolved lexically, and
/// a path that climbs above `/` or contains a NUL byte is an error. The
/// shared conformance suite (`tests/vfs_conformance.rs`) pins this, along
/// with the type rules: `write` never replaces a directory, `mkdir` never
/// replaces a file, and nothing is created beneath a file.
pub trait Vfs {
    /// List entries in a directory.
    fn readdir(&self, path: &str) -> Result<Vec<VfsEntry>>;

    /// Read entire file contents.
    fn read(&self, path: &str) -> Result<Vec<u8>>;

    /// Write data to a file, creating or overwriting it. The parent must be
    /// an existing directory, and `path` itself must not be a directory.
    fn write(&mut self, path: &str, data: &[u8]) -> Result<()>;

    /// Get metadata for a path.
    fn stat(&self, path: &str) -> Result<VfsMetadata>;

    /// Create a directory (and parents if needed). Succeeds when the
    /// directory already exists; fails when the path, or one of its
    /// ancestors, is a file.
    fn mkdir(&mut self, path: &str) -> Result<()>;

    /// Remove a file or empty directory.
    fn remove(&mut self, path: &str) -> Result<()>;

    /// Rename or move a file/directory within the VFS.
    fn rename(&mut self, from: &str, to: &str) -> Result<()>;

    /// Check whether a path exists.
    fn exists(&self, path: &str) -> bool;
    /// Get permissions for a path.
    fn get_permissions(&self, path: &str) -> Result<FilePermissions>;
    /// Set permissions for a path.
    fn set_permissions(&mut self, path: &str, perms: FilePermissions) -> Result<()>;

    /// Get the current VFS user context.
    ///
    /// Returns the default "oasis" user context. Implementations that
    /// store their own context should override this.
    fn context(&self) -> VfsContext {
        VfsContext::default_user()
    }

    /// Set the VFS user context.
    ///
    /// Default implementation is a no-op. Implementations that store
    /// their own context should override this.
    fn set_context(&mut self, _ctx: VfsContext) {}

    /// A short identifier for this VFS implementation, used by diagnostic
    /// surfaces (e.g. the System Monitor app). Defaults to `"UnknownVfs"`
    /// so a missing override is visible rather than silently mislabelled
    /// as a known implementation. `MemoryVfs`, `RealVfs`, and `GameAssetVfs`
    /// each override it with their own name.
    fn vfs_type(&self) -> &'static str {
        "UnknownVfs"
    }

    /// Check the invariants every `rename` implementation must enforce.
    ///
    /// `from` and `to` must already be in the implementation's normal form
    /// (absolute, no `//`, no trailing `/`, no `.`/`..`). Returns
    /// [`RenamePlan::NoOp`] when `from == to`, [`RenamePlan::Move`] when the
    /// rename may proceed, and an error when:
    ///
    /// - `from` is the root (`/`), or does not exist;
    /// - `to` is `from` itself or lies inside it (`/a` -> `/a/b`), which
    ///   would orphan the moved subtree;
    /// - the parent of `to` is missing or not a directory;
    /// - `to` is an existing directory (empty or not: callers that mean
    ///   "move into" must append the file name themselves);
    /// - `to` is an existing file and `from` is a directory.
    ///
    /// Replacing an existing file with a file is allowed (POSIX `rename`).
    /// Permission checks are left to the implementation: the in-memory
    /// backends enforce their stored modes, `RealVfs` defers to the OS.
    fn validate_rename(&self, from: &str, to: &str) -> Result<RenamePlan> {
        if from == "/" {
            return Err(VfsError::Other("cannot rename root".into()).into());
        }
        if !self.exists(from) {
            return Err(VfsError::NotFound { path: from.into() }.into());
        }
        if from == to {
            return Ok(RenamePlan::NoOp);
        }
        if to == "/"
            || to
                .strip_prefix(from)
                .is_some_and(|rest| rest.starts_with('/'))
        {
            return Err(
                VfsError::Other(format!("cannot move '{from}' into itself ('{to}')")).into(),
            );
        }
        let to_parent = parent_path(to);
        if !self
            .stat(to_parent)
            .is_ok_and(|m| m.kind == EntryKind::Directory)
        {
            return Err(
                VfsError::Other(format!("parent directory does not exist: {to_parent}")).into(),
            );
        }
        if let Ok(dest) = self.stat(to) {
            if dest.kind == EntryKind::Directory {
                return Err(VfsError::Other(format!(
                    "destination exists and is a directory: {to}"
                ))
                .into());
            }
            if self.stat(from)?.kind == EntryKind::Directory {
                return Err(VfsError::NotADirectory { path: to.into() }.into());
            }
        }
        Ok(RenamePlan::Move)
    }

    /// Check whether the current context has the given access mode for
    /// a path. Returns `Ok(())` if allowed, or a permission-denied error.
    ///
    /// Default implementation looks up stored permissions and checks
    /// against the current context. The `RealVfs` backend delegates
    /// permission enforcement to the real OS, so it does not override
    /// this method (the check would be redundant).
    fn check_permission(&self, path: &str, mode: AccessMode) -> Result<()> {
        let ctx = self.context();
        if ctx.is_root {
            return Ok(());
        }
        let perms = self.get_permissions(path)?;
        if perms.allows(&ctx, mode) {
            Ok(())
        } else {
            Err(OasisError::Vfs(format!("permission denied: {path}").into()))
        }
    }
}
