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

use oasis_types::error::Result;

/// Type of a VFS entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    /// Regular file.
    File,
    /// Directory.
    Directory,
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
    /// Owner of the file (default: "user").
    pub owner: String,
    /// Unix-style octal mode (default: 0o644 for files, 0o755 for dirs).
    pub mode: u16,
}

impl FilePermissions {
    /// Default permissions for a regular file (mode 0o644, owner "user").
    pub fn default_file() -> Self {
        Self {
            owner: "user".to_string(),
            mode: 0o644,
        }
    }
    /// Default permissions for a directory (mode 0o755, owner "user").
    pub fn default_dir() -> Self {
        Self {
            owner: "user".to_string(),
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

    /// Check whether a path exists.
    fn exists(&self, path: &str) -> bool;
    /// Get permissions for a path.
    fn get_permissions(&self, path: &str) -> Result<FilePermissions>;
    /// Set permissions for a path.
    fn set_permissions(&mut self, path: &str, perms: FilePermissions) -> Result<()>;
}
