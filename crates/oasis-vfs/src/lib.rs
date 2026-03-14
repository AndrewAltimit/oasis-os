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

use oasis_types::error::{OasisError, Result};

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

/// The virtual file system trait.
///
/// All file operations in the command interpreter go through this trait.
/// Paths are always forward-slash separated, absolute (starting with `/`).
pub trait Vfs {
    /// List entries in a directory.
    fn readdir(&self, path: &str) -> Result<Vec<VfsEntry>>;

    /// Read entire file contents.
    fn read(&self, path: &str) -> Result<Vec<u8>>;

    /// Write data to a file, creating or overwriting it.
    fn write(&mut self, path: &str, data: &[u8]) -> Result<()>;

    /// Get metadata for a path.
    fn stat(&self, path: &str) -> Result<VfsMetadata>;

    /// Create a directory (and parents if needed).
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
