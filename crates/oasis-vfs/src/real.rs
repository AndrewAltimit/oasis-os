//! Real filesystem VFS implementation.
//!
//! Wraps `std::fs` operations behind the `Vfs` trait. A configurable root
//! directory is prepended to all paths, providing sandboxing -- the VFS
//! cannot escape its root.

use std::borrow::Cow;
use std::fs;
use std::path::{Path, PathBuf};

use oasis_types::error::{OasisError, Result, VfsError};

use crate::{EntryKind, FilePermissions, RenamePlan, Vfs, VfsEntry, VfsMetadata, normalize_path};

/// The owner `RealVfs` reports for every entry (it cannot map host users).
const REAL_OWNER: &str = "user";

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
            return Err(OasisError::Vfs(
                format!("VFS root is not a directory: {}", root.display()).into(),
            ));
        }
        Ok(Self {
            root: root
                .canonicalize()
                .map_err(|e| OasisError::Vfs(format!("cannot canonicalize root: {e}").into()))?,
        })
    }

    /// Map a VFS path onto the host filesystem without touching it: the
    /// path is normalized ([`normalize_path`]; on Windows `\\` also counts
    /// as a separator) and each segment is checked by [`check_host_segment`]
    /// before being appended to the root, so a segment can never be read by
    /// the host as a drive, UNC prefix or device.
    fn host_path(&self, vfs_path: &str) -> Result<PathBuf> {
        let vfs_path: Cow<'_, str> = if cfg!(windows) && vfs_path.contains('\\') {
            Cow::Owned(vfs_path.replace('\\', "/"))
        } else {
            Cow::Borrowed(vfs_path)
        };
        let normalized = normalize_path(&vfs_path)?;
        let mut out = self.root.clone();
        for seg in normalized.split('/').filter(|s| !s.is_empty()) {
            check_host_segment(seg)?;
            out.push(seg);
        }
        // Defence in depth: the checks above already guarantee this.
        if !out.starts_with(&self.root) {
            return Err(OasisError::Vfs("path escapes VFS root".into()));
        }
        Ok(out)
    }

    /// Resolve a VFS path to a real filesystem path, following symlinks,
    /// and ensure it stays within the root directory.
    ///
    /// The deepest ancestor that exists on disk -- the path itself, if it
    /// exists -- is canonicalized, which follows every symlink in the
    /// existing part of the path, and must still lie inside the root. The
    /// components that do not exist yet are appended to that canonical
    /// prefix, so `create_dir_all` / `write` cannot be steered through a
    /// symlink: a link pointing outside the root fails here even when
    /// several levels below it are still missing (`/link/new/x`).
    fn resolve(&self, vfs_path: &str) -> Result<PathBuf> {
        self.confine(&self.host_path(vfs_path)?)
    }

    /// Like [`resolve`](Self::resolve), but the final component is not
    /// followed: the result names the directory entry itself. `remove` and
    /// `rename` use this so they act on a symlink, not on its target (a
    /// link pointing outside the root can still be deleted or renamed; its
    /// target is never touched).
    fn resolve_entry(&self, vfs_path: &str) -> Result<PathBuf> {
        let host = self.host_path(vfs_path)?;
        if host == self.root {
            return Ok(host);
        }
        match (host.parent(), host.file_name()) {
            (Some(parent), Some(name)) => Ok(self.confine(parent)?.join(name)),
            _ => Err(OasisError::Vfs("invalid path".into())),
        }
    }

    /// Canonicalize the existing prefix of `candidate` (see
    /// [`resolve`](Self::resolve)) and check it is inside the root.
    fn confine(&self, candidate: &Path) -> Result<PathBuf> {
        // Split into the deepest existing ancestor and the missing tail.
        // `symlink_metadata` does not follow the final link, so a dangling
        // symlink counts as existing and then fails to canonicalize.
        let mut existing = candidate;
        let mut tail = Vec::new();
        while fs::symlink_metadata(existing).is_err() {
            let Some(name) = existing.file_name() else {
                return Err(OasisError::Vfs("invalid path".into()));
            };
            tail.push(name.to_owned());
            existing = existing
                .parent()
                .ok_or_else(|| OasisError::Vfs("invalid path: no parent".into()))?;
        }
        let mut resolved = existing
            .canonicalize()
            .map_err(|e| OasisError::Vfs(format!("cannot resolve path: {e}").into()))?;
        if !resolved.starts_with(&self.root) {
            return Err(OasisError::Vfs("path escapes VFS root".into()));
        }
        resolved.extend(tail.iter().rev());
        Ok(resolved)
    }
}

/// Reject a path segment the host would not treat as a plain file name.
///
/// On Windows that is any segment with a `:` (a drive prefix such as `C:`
/// replaces the whole path when joined, and `name:stream` addresses an
/// NTFS alternate data stream), a segment ending in `.` or a space (Win32
/// strips those, so `/secret.` would alias `/secret`), and the reserved
/// device names (`CON`, `NUL`, `COM1`, `LPT¹`, ... with or without an
/// extension), which open a device instead of a file. Elsewhere every
/// segment [`normalize_path`] accepts is fine.
fn check_host_segment(seg: &str) -> Result<()> {
    if cfg!(windows) {
        if seg.contains(':') || seg.ends_with(['.', ' ']) {
            return Err(VfsError::Other(format!("invalid path segment: {seg}")).into());
        }
        if is_reserved_device_name(seg) {
            return Err(VfsError::Other(format!("reserved device name: {seg}")).into());
        }
    }
    Ok(())
}

/// Whether `seg` names a Windows device (`NUL`, `com1.txt`, `LPT²`, ...):
/// the part before the first `.`, trailing spaces ignored, compared
/// case-insensitively.
fn is_reserved_device_name(seg: &str) -> bool {
    let stem = seg.split('.').next().unwrap_or(seg).trim_end_matches(' ');
    let upper = stem.to_uppercase();
    if matches!(
        upper.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) {
        return true;
    }
    let Some(rest) = upper
        .strip_prefix("COM")
        .or_else(|| upper.strip_prefix("LPT"))
    else {
        return false;
    };
    let mut chars = rest.chars();
    matches!(
        (chars.next(), chars.next()),
        (Some('0'..='9' | '¹' | '²' | '³'), None)
    )
}

impl RealVfs {
    /// The metadata `readdir` reports for `entry`: its symlinks followed
    /// (as `stat` does), so a linked directory lists as a directory. A
    /// dangling link, or one whose target is outside the root, is listed by
    /// the link's own metadata (as a file) and is only good for `remove` /
    /// `rename`.
    fn entry_metadata(&self, entry: &fs::DirEntry) -> Result<fs::Metadata> {
        let own = entry.metadata()?;
        if !own.file_type().is_symlink() {
            return Ok(own);
        }
        match self.confine(&entry.path()) {
            Ok(target) => Ok(fs::metadata(target).unwrap_or(own)),
            Err(_) => Ok(own),
        }
    }
}

impl Vfs for RealVfs {
    fn vfs_type(&self) -> &'static str {
        "RealVfs"
    }

    fn readdir(&self, path: &str) -> Result<Vec<VfsEntry>> {
        let real_path = self.resolve(path)?;
        if !real_path.is_dir() {
            if !real_path.exists() {
                return Err(VfsError::NotFound { path: path.into() }.into());
            }
            return Err(VfsError::NotADirectory { path: path.into() }.into());
        }
        let mut entries = Vec::new();
        for entry in fs::read_dir(&real_path)? {
            let entry = entry?;
            let meta = self.entry_metadata(&entry)?;
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
            return Err(OasisError::Vfs(format!("is a directory: {path}").into()));
        }
        Ok(fs::read(&real_path)?)
    }

    fn write(&mut self, path: &str, data: &[u8]) -> Result<()> {
        let real_path = self.resolve(path)?;
        if real_path.is_dir() {
            return Err(VfsError::IsDirectory { path: path.into() }.into());
        }
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
        let real_path = self.resolve_entry(path)?;
        if real_path == self.root {
            return Err(OasisError::Vfs("cannot remove VFS root".into()));
        }
        // `symlink_metadata`: a symlink is removed itself, never its target.
        let meta = fs::symlink_metadata(&real_path)?;
        if meta.is_dir() {
            fs::remove_dir(&real_path)?;
        } else if meta.file_type().is_symlink() && cfg!(windows) {
            // Windows directory symlinks / junctions are removed as dirs.
            fs::remove_file(&real_path).or_else(|_| fs::remove_dir(&real_path))?;
        } else {
            fs::remove_file(&real_path)?;
        }
        Ok(())
    }

    fn rename(&mut self, from: &str, to: &str) -> Result<()> {
        let real_from = self.resolve_entry(from)?;
        let real_to = self.resolve_entry(to)?;
        let from_norm = normalize_path(from)?;
        let to_norm = normalize_path(to)?;
        // On case-insensitive filesystems `/Dir` -> `/dir` names the same
        // entry twice: that is a case change, not a move onto an existing
        // directory.
        if from_norm != to_norm
            && from_norm.eq_ignore_ascii_case(&to_norm)
            && real_from != self.root
            && self.resolve(from)? == self.resolve(to)?
        {
            fs::rename(&real_from, &real_to)?;
            return Ok(());
        }
        // A symlink is renamed as itself (like `remove`), even when it
        // dangles or points outside the root, where `exists` -- which
        // follows it -- is false.
        let from_is_link = real_from != self.root
            && fs::symlink_metadata(&real_from).is_ok_and(|m| m.file_type().is_symlink());
        if from_is_link {
            if from_norm == to_norm {
                return Ok(());
            }
            if self.stat(&to_norm).is_ok_and(|m| m.kind == EntryKind::Directory) {
                return Err(VfsError::Other(format!(
                    "destination exists and is a directory: {to}"
                ))
                .into());
            }
            fs::rename(&real_from, &real_to)?;
            return Ok(());
        }
        if self.validate_rename(&from_norm, &to_norm)? == RenamePlan::NoOp {
            return Ok(());
        }
        // Symlinks inside the root can make a lexically unrelated `to` land
        // inside `from` on disk, and on a case-insensitive filesystem `/A`
        // and `/a/sub` are the same tree: compare the resolved source with
        // the resolved parent of the destination.
        let from_resolved = self.resolve(&from_norm)?;
        let to_parent_resolved = self.resolve(crate::parent_path(&to_norm))?;
        if real_to.starts_with(&real_from) || to_parent_resolved.starts_with(&from_resolved) {
            return Err(OasisError::Vfs(
                format!("cannot move '{from}' into itself ('{to}')").into(),
            ));
        }
        fs::rename(&real_from, &real_to)?;
        Ok(())
    }

    fn exists(&self, path: &str) -> bool {
        self.resolve(path).map(|p| p.exists()).unwrap_or(false)
    }

    fn get_permissions(&self, path: &str) -> Result<FilePermissions> {
        let real_path = self.resolve(path)?;
        if !real_path.exists() {
            return Err(OasisError::Vfs(format!("no such path: {path}").into()));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = fs::metadata(&real_path)
                .map_err(|e| OasisError::Vfs(format!("cannot read metadata: {e}").into()))?;
            let mode = meta.permissions().mode() & 0o7777;
            Ok(FilePermissions {
                owner: REAL_OWNER.to_string(),
                mode: mode as u16,
            })
        }
        #[cfg(not(unix))]
        {
            // The only bit Windows has is read-only (owner write).
            let mut mode: u16 = if real_path.is_dir() { 0o755 } else { 0o644 };
            if fs::metadata(&real_path).is_ok_and(|m| m.permissions().readonly()) {
                mode &= !0o222;
            }
            Ok(FilePermissions {
                owner: REAL_OWNER.to_string(),
                mode,
            })
        }
    }

    fn set_permissions(&mut self, path: &str, perms: FilePermissions) -> Result<()> {
        let real_path = self.resolve(path)?;
        if !real_path.exists() {
            return Err(OasisError::Vfs(format!("no such path: {path}").into()));
        }
        // RealVfs cannot change file ownership (requires root/libc).
        // Check against the owner `get_permissions` reports.
        if perms.owner != REAL_OWNER {
            return Err(OasisError::Vfs(
                format!(
                    "RealVfs does not support changing file ownership (from 'user' to '{}')",
                    perms.owner
                )
                .into(),
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let unix_perms = fs::Permissions::from_mode(u32::from(perms.mode));
            fs::set_permissions(&real_path, unix_perms)
                .map_err(|e| OasisError::Vfs(format!("cannot set permissions: {e}").into()))?;
        }
        #[cfg(not(unix))]
        {
            // Map write permission to the platform's readonly flag.
            let mut fs_perms = fs::metadata(&real_path)
                .map_err(|e| OasisError::Vfs(format!("cannot read metadata: {e}").into()))?
                .permissions();
            // Mode 0o200 is owner-write; if absent, mark readonly.
            fs_perms.set_readonly(perms.mode & 0o200 == 0);
            fs::set_permissions(&real_path, fs_perms)
                .map_err(|e| OasisError::Vfs(format!("cannot set permissions: {e}").into()))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_device_names_are_recognised() {
        for name in [
            "CON", "nul", "Aux.txt", "com1", "COM9.log", "lpt3", "LPT¹", "com².x", "NUL .txt",
            "conin$",
        ] {
            assert!(is_reserved_device_name(name), "{name}");
        }
        for name in ["CONSOLE", "com", "com10", "lpt0x", "nul_", "file", "com¹¹"] {
            assert!(!is_reserved_device_name(name), "{name}");
        }
    }

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

    #[test]
    fn rename_file() {
        let (_dir, mut vfs) = temp_vfs();
        vfs.mkdir("/src").unwrap();
        vfs.mkdir("/dst").unwrap();
        vfs.write("/src/file.txt", b"hello").unwrap();
        vfs.rename("/src/file.txt", "/dst/moved.txt").unwrap();
        assert!(!vfs.exists("/src/file.txt"));
        assert_eq!(vfs.read("/dst/moved.txt").unwrap(), b"hello");
    }

    #[test]
    fn rename_directory() {
        let (_dir, mut vfs) = temp_vfs();
        vfs.mkdir("/old").unwrap();
        vfs.write("/old/file.txt", b"data").unwrap();
        vfs.rename("/old", "/new").unwrap();
        assert!(!vfs.exists("/old"));
        assert!(vfs.exists("/new"));
        assert_eq!(vfs.read("/new/file.txt").unwrap(), b"data");
    }

    #[test]
    fn rename_nonexistent_fails() {
        let (_dir, mut vfs) = temp_vfs();
        assert!(vfs.rename("/ghost", "/dest").is_err());
    }

    #[test]
    fn rename_path_traversal_blocked() {
        let (_dir, mut vfs) = temp_vfs();
        vfs.write("/file.txt", b"data").unwrap();
        assert!(vfs.rename("/file.txt", "/../../escaped").is_err());
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

    #[cfg(unix)]
    #[test]
    fn symlink_outside_root_read_blocked() {
        use std::os::unix::fs::symlink;

        let outside = tempfile::tempdir().unwrap();
        let outside_file = outside.path().join("secret.txt");
        fs::write(&outside_file, b"sensitive data").unwrap();

        let (_dir, vfs) = temp_vfs();
        // Create a symlink inside the VFS root pointing outside.
        let link_path = vfs.root.join("escape_link");
        symlink(&outside_file, &link_path).unwrap();

        let result = vfs.read("/escape_link");
        assert!(
            result.is_err(),
            "read through symlink outside root should fail"
        );
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("escapes VFS root"),
            "expected 'escapes VFS root', got: {err}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlink_outside_root_write_blocked() {
        use std::os::unix::fs::symlink;

        let outside = tempfile::tempdir().unwrap();
        let outside_file = outside.path().join("target.txt");
        fs::write(&outside_file, b"original").unwrap();

        let (_dir, mut vfs) = temp_vfs();
        let link_path = vfs.root.join("write_link");
        symlink(&outside_file, &link_path).unwrap();

        let result = vfs.write("/write_link", b"overwritten");
        assert!(
            result.is_err(),
            "write through symlink outside root should fail"
        );

        // Verify the outside file was not modified.
        let content = fs::read(&outside_file).unwrap();
        assert_eq!(content, b"original", "outside file should not be modified");
    }

    #[cfg(unix)]
    #[test]
    fn symlink_chain_escape_blocked() {
        use std::os::unix::fs::symlink;

        let outside = tempfile::tempdir().unwrap();
        let outside_file = outside.path().join("chain_target.txt");
        fs::write(&outside_file, b"chained secret").unwrap();

        let (_dir, vfs) = temp_vfs();
        // link_a -> link_b -> outside_file
        let link_b = vfs.root.join("link_b");
        symlink(&outside_file, &link_b).unwrap();
        let link_a = vfs.root.join("link_a");
        symlink(&link_b, &link_a).unwrap();

        let result = vfs.read("/link_a");
        assert!(
            result.is_err(),
            "read through symlink chain escaping root should fail"
        );
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("escapes VFS root"),
            "expected 'escapes VFS root', got: {err}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn broken_symlink_errors_gracefully() {
        use std::os::unix::fs::symlink;

        let (_dir, vfs) = temp_vfs();
        // Point to a non-existent target inside the root.
        let link_path = vfs.root.join("broken_link");
        let nonexistent = vfs.root.join("does_not_exist.txt");
        symlink(&nonexistent, &link_path).unwrap();

        let result = vfs.read("/broken_link");
        assert!(result.is_err(), "reading a broken symlink should fail");

        let stat_result = vfs.stat("/broken_link");
        assert!(stat_result.is_err(), "stat on a broken symlink should fail");

        // exists() should return false for broken symlinks.
        assert!(
            !vfs.exists("/broken_link"),
            "broken symlink should not 'exist'"
        );
    }

    #[cfg(unix)]
    #[test]
    fn directory_symlink_escape_readdir_blocked() {
        use std::os::unix::fs::symlink;

        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("private.txt"), b"private").unwrap();

        let (_dir, vfs) = temp_vfs();
        let link_path = vfs.root.join("dir_escape");
        symlink(outside.path(), &link_path).unwrap();

        let result = vfs.readdir("/dir_escape");
        assert!(
            result.is_err(),
            "readdir through directory symlink outside root should fail"
        );
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("escapes VFS root"),
            "expected 'escapes VFS root', got: {err}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn directory_symlink_escape_stat_blocked() {
        use std::os::unix::fs::symlink;

        let outside = tempfile::tempdir().unwrap();

        let (_dir, vfs) = temp_vfs();
        let link_path = vfs.root.join("dir_stat_escape");
        symlink(outside.path(), &link_path).unwrap();

        let result = vfs.stat("/dir_stat_escape");
        assert!(
            result.is_err(),
            "stat on directory symlink outside root should fail"
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlink_inside_root_still_resolves() {
        use std::os::unix::fs::symlink;

        let (_dir, mut vfs) = temp_vfs();
        vfs.mkdir("/real").unwrap();
        symlink(vfs.root.join("real"), vfs.root.join("alias")).unwrap();
        vfs.mkdir("/alias/a/b").unwrap();
        assert!(vfs.root.join("real/a/b").is_dir());
    }

    #[test]
    fn dot_dot_is_resolved_lexically() {
        let (_dir, mut vfs) = temp_vfs();
        vfs.mkdir("/a/b").unwrap();
        vfs.write("/a/b/../f.txt", b"x").unwrap();
        assert_eq!(vfs.read("/a/f.txt").unwrap(), b"x");
        assert!(vfs.resolve("/a/../../x").is_err());
    }

    #[test]
    fn climbing_back_into_root_by_name_is_rejected() {
        // `/../<root name>/x` used to re-enter the root lexically.
        let (dir, mut vfs) = temp_vfs();
        let name = dir
            .path()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert!(vfs.write(&format!("/../{name}/x"), b"x").is_err());
        assert!(!vfs.exists("/x"));
    }

    #[test]
    fn nul_byte_is_rejected() {
        let (_dir, mut vfs) = temp_vfs();
        assert!(vfs.write("/a\0b", b"x").is_err());
        assert!(vfs.readdir("/").unwrap().is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn windows_prefixes_and_devices_are_rejected() {
        let (_dir, mut vfs) = temp_vfs();
        vfs.mkdir("/sub").unwrap();
        for path in [
            "/C:/Windows/x.txt",
            "C:\\x.txt",
            "/\\\\server\\share\\x",
            "/a\\..\\..\\x",
            "/file.txt:stream",
            "/NUL",
            "/con.txt",
            "/sub/COM1",
            "/lpt9.log",
        ] {
            assert!(vfs.write(path, b"x").is_err(), "{path} must be rejected");
        }
        // Backslash is a separator on Windows, within the root.
        vfs.mkdir("/d").unwrap();
        vfs.write("/d\\f.txt", b"ok").unwrap();
        assert_eq!(vfs.read("/d/f.txt").unwrap(), b"ok");
        // Names that merely start like a device are fine.
        vfs.write("/console.txt", b"ok").unwrap();
        vfs.write("/com10", b"ok").unwrap();
        // A 4-byte stem whose third byte is not a char boundary.
        vfs.write("/ab\u{e9}.txt", b"ok").unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn remove_symlink_removes_the_link_not_the_target() {
        use std::os::unix::fs::symlink;

        let outside = tempfile::tempdir().unwrap();
        let outside_file = outside.path().join("keep.txt");
        fs::write(&outside_file, b"keep").unwrap();
        let (_dir, mut vfs) = temp_vfs();
        vfs.write("/target.txt", b"inside").unwrap();
        symlink(vfs.root.join("target.txt"), vfs.root.join("inner_link")).unwrap();
        symlink(&outside_file, vfs.root.join("outer_link")).unwrap();

        vfs.remove("/inner_link").unwrap();
        assert_eq!(vfs.read("/target.txt").unwrap(), b"inside");
        // A link escaping the root can be deleted; its target survives.
        vfs.remove("/outer_link").unwrap();
        assert!(fs::symlink_metadata(vfs.root.join("outer_link")).is_err());
        assert_eq!(fs::read(&outside_file).unwrap(), b"keep");
    }

    #[cfg(unix)]
    #[test]
    fn rename_symlink_moves_the_link() {
        use std::os::unix::fs::symlink;

        let (_dir, mut vfs) = temp_vfs();
        vfs.mkdir("/real").unwrap();
        vfs.write("/real/f", b"x").unwrap();
        symlink(vfs.root.join("real"), vfs.root.join("alias")).unwrap();
        vfs.rename("/alias", "/alias2").unwrap();
        assert!(vfs.root.join("real/f").is_file(), "target untouched");
        assert!(
            fs::symlink_metadata(vfs.root.join("alias2"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }
}
