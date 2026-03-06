//! Real filesystem VFS implementation.
//!
//! Wraps `std::fs` operations behind the `Vfs` trait. A configurable root
//! directory is prepended to all paths, providing sandboxing -- the VFS
//! cannot escape its root.

use std::fs;
use std::path::PathBuf;

use oasis_types::error::{OasisError, Result};

use crate::{EntryKind, FilePermissions, Vfs, VfsEntry, VfsMetadata};

/// A VFS backed by the real filesystem, rooted at a configurable directory.
#[derive(Debug)]
pub struct RealVfs {
    /// Canonical absolute path to the VFS root on the real filesystem.
    root: PathBuf,
}

impl RealVfs {
    /// Create a new RealVfs rooted at the given directory.
    /// The root must exist and be a directory.
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        if !root.is_dir() {
            return Err(OasisError::Vfs(format!(
                "VFS root is not a directory: {}",
                root.display()
            )));
        }
        Ok(Self {
            root: root
                .canonicalize()
                .map_err(|e| OasisError::Vfs(format!("cannot canonicalize root: {e}")))?,
        })
    }

    /// Resolve a VFS path to a real filesystem path, ensuring it stays
    /// within the root directory.
    fn resolve(&self, vfs_path: &str) -> Result<PathBuf> {
        // Strip leading `/` and join with root.
        let relative = vfs_path.strip_prefix('/').unwrap_or(vfs_path);
        let candidate = self.root.join(relative);

        // Canonicalize if path exists, otherwise check that its parent is within root.
        let resolved = if candidate.exists() {
            candidate
                .canonicalize()
                .map_err(|e| OasisError::Vfs(format!("cannot resolve path: {e}")))?
        } else {
            // For non-existent paths (write/mkdir), verify the parent is inside root.
            let parent = candidate
                .parent()
                .ok_or_else(|| OasisError::Vfs("invalid path: no parent".to_string()))?;
            if parent.exists() {
                let canon_parent = parent
                    .canonicalize()
                    .map_err(|e| OasisError::Vfs(format!("cannot resolve parent: {e}")))?;
                if !canon_parent.starts_with(&self.root) {
                    return Err(OasisError::Vfs("path escapes VFS root".to_string()));
                }
            }
            candidate
        };

        if !resolved.starts_with(&self.root) {
            return Err(OasisError::Vfs("path escapes VFS root".to_string()));
        }
        Ok(resolved)
    }
}

impl Vfs for RealVfs {
    fn readdir(&self, path: &str) -> Result<Vec<VfsEntry>> {
        let real_path = self.resolve(path)?;
        if !real_path.is_dir() {
            return Err(OasisError::Vfs(format!("not a directory: {path}")));
        }
        let mut entries = Vec::new();
        for entry in fs::read_dir(&real_path)? {
            let entry = entry?;
            let meta = entry.metadata()?;
            entries.push(VfsEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
                kind: if meta.is_dir() {
                    EntryKind::Directory
                } else {
                    EntryKind::File
                },
                size: meta.len(),
            });
        }
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(entries)
    }

    fn read(&self, path: &str) -> Result<Vec<u8>> {
        let real_path = self.resolve(path)?;
        if real_path.is_dir() {
            return Err(OasisError::Vfs(format!("is a directory: {path}")));
        }
        Ok(fs::read(&real_path)?)
    }

    fn write(&mut self, path: &str, data: &[u8]) -> Result<()> {
        let real_path = self.resolve(path)?;
        fs::write(&real_path, data)?;
        Ok(())
    }

    fn stat(&self, path: &str) -> Result<VfsMetadata> {
        let real_path = self.resolve(path)?;
        let meta = fs::metadata(&real_path)?;
        Ok(VfsMetadata {
            kind: if meta.is_dir() {
                EntryKind::Directory
            } else {
                EntryKind::File
            },
            size: meta.len(),
        })
    }

    fn mkdir(&mut self, path: &str) -> Result<()> {
        let real_path = self.resolve(path)?;
        fs::create_dir_all(&real_path)?;
        Ok(())
    }

    fn remove(&mut self, path: &str) -> Result<()> {
        let real_path = self.resolve(path)?;
        if real_path == self.root {
            return Err(OasisError::Vfs("cannot remove VFS root".to_string()));
        }
        if real_path.is_dir() {
            fs::remove_dir(&real_path)?;
        } else {
            fs::remove_file(&real_path)?;
        }
        Ok(())
    }

    fn exists(&self, path: &str) -> bool {
        self.resolve(path).map(|p| p.exists()).unwrap_or(false)
    }

    fn get_permissions(&self, path: &str) -> Result<FilePermissions> {
        let real_path = self.resolve(path)?;
        if !real_path.exists() {
            return Err(OasisError::Vfs(format!("no such path: {path}")));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = fs::metadata(&real_path)
                .map_err(|e| OasisError::Vfs(format!("cannot read metadata: {e}")))?;
            let mode = meta.permissions().mode() & 0o7777;
            Ok(FilePermissions {
                owner: "user".to_string(),
                mode: mode as u16,
            })
        }
        #[cfg(not(unix))]
        {
            if real_path.is_dir() {
                Ok(FilePermissions::default_dir())
            } else {
                Ok(FilePermissions::default_file())
            }
        }
    }

    fn set_permissions(&mut self, path: &str, perms: FilePermissions) -> Result<()> {
        let real_path = self.resolve(path)?;
        if !real_path.exists() {
            return Err(OasisError::Vfs(format!("no such path: {path}")));
        }
        // RealVfs cannot change file ownership (requires root/libc).
        let current = self.get_permissions(path)?;
        if perms.owner != current.owner {
            return Err(OasisError::Vfs(format!(
                "RealVfs does not support changing file ownership (from '{}' to '{}')",
                current.owner, perms.owner
            )));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let unix_perms = fs::Permissions::from_mode(u32::from(perms.mode));
            fs::set_permissions(&real_path, unix_perms)
                .map_err(|e| OasisError::Vfs(format!("cannot set permissions: {e}")))?;
        }
        #[cfg(not(unix))]
        {
            let _ = perms;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_vfs() -> (tempfile::TempDir, RealVfs) {
        let dir = tempfile::tempdir().unwrap();
        let vfs = RealVfs::new(dir.path()).unwrap();
        (dir, vfs)
    }

    #[test]
    fn root_exists() {
        let (_dir, vfs) = temp_vfs();
        assert!(vfs.exists("/"));
    }

    #[test]
    fn write_and_read() {
        let (_dir, mut vfs) = temp_vfs();
        vfs.write("/hello.txt", b"world").unwrap();
        let data = vfs.read("/hello.txt").unwrap();
        assert_eq!(data, b"world");
    }

    #[test]
    fn mkdir_and_readdir() {
        let (_dir, mut vfs) = temp_vfs();
        vfs.mkdir("/subdir").unwrap();
        vfs.write("/subdir/file.txt", b"content").unwrap();
        let entries = vfs.readdir("/subdir").unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "file.txt");
    }

    #[test]
    fn stat_file_and_dir() {
        let (_dir, mut vfs) = temp_vfs();
        vfs.mkdir("/d").unwrap();
        vfs.write("/d/f", b"abc").unwrap();
        let dm = vfs.stat("/d").unwrap();
        assert_eq!(dm.kind, EntryKind::Directory);
        let fm = vfs.stat("/d/f").unwrap();
        assert_eq!(fm.kind, EntryKind::File);
        assert_eq!(fm.size, 3);
    }

    #[test]
    fn remove_file_and_dir() {
        let (_dir, mut vfs) = temp_vfs();
        vfs.mkdir("/d").unwrap();
        vfs.write("/d/f", b"x").unwrap();
        vfs.remove("/d/f").unwrap();
        assert!(!vfs.exists("/d/f"));
        vfs.remove("/d").unwrap();
        assert!(!vfs.exists("/d"));
    }

    #[test]
    fn path_traversal_blocked() {
        let (_dir, vfs) = temp_vfs();
        assert!(vfs.resolve("/../../../etc/passwd").is_err());
    }

    #[test]
    fn nonexistent_root_fails() {
        let result = RealVfs::new(std::path::Path::new("/nonexistent_oasis_test_dir"));
        assert!(result.is_err());
    }

    #[cfg(unix)]
    #[test]
    fn get_permissions_reads_real_mode() {
        use std::os::unix::fs::PermissionsExt;

        let (_dir, mut vfs) = temp_vfs();
        vfs.write("/test.txt", b"hello").unwrap();

        // Set a known mode via std::fs so we can verify get_permissions reads it.
        let real_path = vfs.resolve("/test.txt").unwrap();
        fs::set_permissions(&real_path, fs::Permissions::from_mode(0o755)).unwrap();

        let perms = vfs.get_permissions("/test.txt").unwrap();
        assert_eq!(perms.mode, 0o755);
    }

    #[cfg(unix)]
    #[test]
    fn set_permissions_applies_mode() {
        use std::os::unix::fs::PermissionsExt;

        let (_dir, mut vfs) = temp_vfs();
        vfs.write("/test.txt", b"hello").unwrap();

        vfs.set_permissions(
            "/test.txt",
            FilePermissions {
                owner: "user".to_string(),
                mode: 0o600,
            },
        )
        .unwrap();

        // Verify via std::fs that the mode was actually applied.
        let real_path = vfs.resolve("/test.txt").unwrap();
        let actual_mode = fs::metadata(&real_path).unwrap().permissions().mode() & 0o7777;
        assert_eq!(actual_mode, 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn set_then_get_permissions_roundtrip() {
        let (_dir, mut vfs) = temp_vfs();
        vfs.write("/roundtrip.txt", b"data").unwrap();

        let target = FilePermissions {
            owner: "user".to_string(),
            mode: 0o744,
        };
        vfs.set_permissions("/roundtrip.txt", target).unwrap();

        let got = vfs.get_permissions("/roundtrip.txt").unwrap();
        assert_eq!(got.mode, 0o744);
    }

    #[cfg(unix)]
    #[test]
    fn get_permissions_directory() {
        use std::os::unix::fs::PermissionsExt;

        let (_dir, mut vfs) = temp_vfs();
        vfs.mkdir("/mydir").unwrap();

        let real_path = vfs.resolve("/mydir").unwrap();
        fs::set_permissions(&real_path, fs::Permissions::from_mode(0o700)).unwrap();

        let perms = vfs.get_permissions("/mydir").unwrap();
        assert_eq!(perms.mode, 0o700);
    }

    #[cfg(unix)]
    #[test]
    fn set_permissions_rejects_owner_change() {
        let (_dir, mut vfs) = temp_vfs();
        vfs.write("/test.txt", b"hello").unwrap();
        let result = vfs.set_permissions(
            "/test.txt",
            FilePermissions {
                owner: "root".to_string(),
                mode: 0o644,
            },
        );
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("ownership"), "unexpected error: {err}");
    }

    #[cfg(unix)]
    #[test]
    fn permissions_nonexistent_path_errors() {
        let (_dir, mut vfs) = temp_vfs();
        assert!(vfs.get_permissions("/nope").is_err());
        assert!(
            vfs.set_permissions(
                "/nope",
                FilePermissions {
                    owner: "user".to_string(),
                    mode: 0o644,
                },
            )
            .is_err()
        );
    }
}
