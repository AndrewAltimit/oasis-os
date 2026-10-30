//! VFS mutation helpers for [`FileOp`]s: recursive copy / delete, unique
//! name generation and the op executor used by `App::apply_vfs_ops`.
//!
//! Everything here works purely through the [`Vfs`] trait, so it behaves
//! the same on `MemoryVfs`, `RealVfs` and `GameAssetVfs`.

use oasis_app_core::file_viewer::{join_path, parent_dir};
use oasis_types::error::{OasisError, VfsError};
use oasis_vfs::{EntryKind, Vfs};

use crate::model::FileOp;

/// Maximum length (in chars) accepted for a typed file / folder name.
pub(crate) const MAX_NAME_LEN: usize = 64;

/// Last path component of `path` (`"/a/b.txt"` -> `"b.txt"`).
pub(crate) fn file_name(path: &str) -> &str {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(path)
}

/// Whether `path` exists and is a directory.
pub(crate) fn is_dir(vfs: &dyn Vfs, path: &str) -> bool {
    vfs.stat(path).is_ok_and(|m| m.kind == EntryKind::Directory)
}

/// Validate a user-typed entry name. Returns the trimmed name.
pub(crate) fn validate_name(name: &str) -> Result<&str, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Name cannot be empty".to_string());
    }
    if name == "." || name == ".." {
        return Err(format!("\"{name}\" is not a valid name"));
    }
    if name.contains('/') || name.contains('\\') || name.chars().any(char::is_control) {
        return Err("Name cannot contain / or \\".to_string());
    }
    if name.chars().count() > MAX_NAME_LEN {
        return Err(format!("Name is longer than {MAX_NAME_LEN} characters"));
    }
    Ok(name)
}

/// Split `name` into stem and extension (including the dot). Dotfiles
/// (`.bashrc`) and extension-less names have an empty extension.
fn split_ext(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    }
}

/// Return a path inside `dir` for `name` that does not exist yet:
/// `name`, then `name (2)`, `name (3)`, ... Files keep their extension
/// (`notes (2).txt`); directories are suffixed as a whole.
pub(crate) fn unique_path(vfs: &dyn Vfs, dir: &str, name: &str, is_directory: bool) -> String {
    let first = join_path(dir, name);
    if !vfs.exists(&first) {
        return first;
    }
    let (stem, ext) = if is_directory {
        (name, "")
    } else {
        split_ext(name)
    };
    let mut n = 2u32;
    loop {
        let candidate = join_path(dir, &format!("{stem} ({n}){ext}"));
        if !vfs.exists(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// Whether `path` is `ancestor` itself or lies somewhere below it.
fn is_within(path: &str, ancestor: &str) -> bool {
    if ancestor == "/" {
        return true;
    }
    path == ancestor || path.starts_with(&format!("{ancestor}/"))
}

/// Recursively copy `from` (file or directory tree) to the new path `to`.
pub(crate) fn copy_recursive(vfs: &mut dyn Vfs, from: &str, to: &str) -> Result<(), String> {
    if is_dir(vfs, from) {
        vfs.mkdir(to).map_err(|e| e.to_string())?;
        let entries = vfs.readdir(from).map_err(|e| e.to_string())?;
        for entry in entries {
            copy_recursive(
                vfs,
                &join_path(from, &entry.name),
                &join_path(to, &entry.name),
            )?;
        }
        Ok(())
    } else {
        let data = vfs.read(from).map_err(|e| e.to_string())?;
        vfs.write(to, &data).map_err(|e| e.to_string())
    }
}

/// Recursively delete `path` (children first, then the entry itself).
pub(crate) fn remove_recursive(vfs: &mut dyn Vfs, path: &str) -> Result<(), String> {
    if is_dir(vfs, path) {
        let entries = vfs.readdir(path).map_err(|e| e.to_string())?;
        for entry in entries {
            remove_recursive(vfs, &join_path(path, &entry.name))?;
        }
    }
    vfs.remove(path).map_err(|e| e.to_string())
}

/// Whether a failed `rename` means "this backend can't move it in place"
/// (so copy + delete is the right fallback) rather than a real error
/// such as a missing source, a bad destination or a permission denial.
///
/// The VFS error types carry no dedicated "unsupported" variant, so this
/// recognises the known cases: a cross-device / unsupported OS rename
/// (`RealVfs`) and an immutable base-layer entry (`GameAssetVfs`).
fn rename_unsupported(err: &OasisError) -> bool {
    match err {
        OasisError::Io(io) => matches!(
            io.kind(),
            std::io::ErrorKind::CrossesDevices | std::io::ErrorKind::Unsupported
        ),
        OasisError::Vfs(VfsError::Other(msg)) => msg.contains("base-layer"),
        _ => false,
    }
}

/// Execute `op` against `vfs`.
///
/// Returns a short human-readable status line on success, or an error
/// message on failure. Never panics on missing paths.
pub(crate) fn apply_file_op(vfs: &mut dyn Vfs, op: &FileOp) -> Result<String, String> {
    match op {
        FileOp::Delete(path) => {
            if path == "/" {
                return Err("Cannot delete /".to_string());
            }
            if !vfs.exists(path) {
                return Err(format!("{} no longer exists", file_name(path)));
            }
            remove_recursive(vfs, path)?;
            Ok(format!("Deleted {}", file_name(path)))
        },
        FileOp::Mkdir(path) => {
            let dir = parent_dir(path);
            let target = unique_path(vfs, &dir, file_name(path), true);
            vfs.mkdir(&target).map_err(|e| e.to_string())?;
            Ok(format!("Created {}", file_name(&target)))
        },
        FileOp::Rename { from, to } => {
            if from == to {
                return Ok(String::new());
            }
            if !vfs.exists(from) {
                return Err(format!("{} no longer exists", file_name(from)));
            }
            if vfs.exists(to) {
                return Err(format!("{} already exists", file_name(to)));
            }
            vfs.rename(from, to).map_err(|e| e.to_string())?;
            Ok(format!("Renamed {} -> {}", file_name(from), file_name(to)))
        },
        FileOp::Copy { from, to_dir } => {
            if !vfs.exists(from) {
                return Err(format!("{} no longer exists", file_name(from)));
            }
            let src_is_dir = is_dir(vfs, from);
            if src_is_dir && is_within(to_dir, from) {
                return Err("Cannot copy a folder into itself".to_string());
            }
            let target = unique_path(vfs, to_dir, file_name(from), src_is_dir);
            copy_recursive(vfs, from, &target)?;
            Ok(format!("Copied {}", file_name(&target)))
        },
        FileOp::Move { from, to_dir } => {
            if !vfs.exists(from) {
                return Err(format!("{} no longer exists", file_name(from)));
            }
            if parent_dir(from) == *to_dir {
                return Ok(String::new());
            }
            let src_is_dir = is_dir(vfs, from);
            if src_is_dir && is_within(to_dir, from) {
                return Err("Cannot move a folder into itself".to_string());
            }
            let target = unique_path(vfs, to_dir, file_name(from), src_is_dir);
            match vfs.rename(from, &target) {
                Ok(()) => {},
                Err(e) if rename_unsupported(&e) => {
                    // The backend can't move this entry in place: copy,
                    // and only remove the source once the copy is whole.
                    if let Err(copy_err) = copy_recursive(vfs, from, &target) {
                        // Best effort: drop the partial copy, keep the source.
                        let _ = remove_recursive(vfs, &target);
                        return Err(copy_err);
                    }
                    remove_recursive(vfs, from).map_err(|rm_err| {
                        format!(
                            "Copied to {} but could not remove the original: {rm_err}",
                            file_name(&target)
                        )
                    })?;
                },
                Err(e) => return Err(e.to_string()),
            }
            Ok(format!("Moved {}", file_name(&target)))
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oasis_vfs::MemoryVfs;

    fn vfs() -> MemoryVfs {
        let mut v = MemoryVfs::new();
        v.mkdir("/a/sub").expect("mkdir");
        v.write("/a/one.txt", b"1").expect("write");
        v.write("/a/sub/two.txt", b"22").expect("write");
        v.mkdir("/b").expect("mkdir");
        v
    }

    #[test]
    fn unique_path_suffixes() {
        let mut v = vfs();
        assert_eq!(unique_path(&v, "/a", "new", true), "/a/new");
        assert_eq!(unique_path(&v, "/a", "sub", true), "/a/sub (2)");
        v.mkdir("/a/sub (2)").expect("mkdir");
        assert_eq!(unique_path(&v, "/a", "sub", true), "/a/sub (3)");
        assert_eq!(unique_path(&v, "/a", "one.txt", false), "/a/one (2).txt");
    }

    #[test]
    fn split_ext_rules() {
        assert_eq!(split_ext("a.txt"), ("a", ".txt"));
        assert_eq!(split_ext(".bashrc"), (".bashrc", ""));
        assert_eq!(split_ext("noext"), ("noext", ""));
    }

    #[test]
    fn validate_name_rules() {
        assert_eq!(validate_name("  ok  "), Ok("ok"));
        assert!(validate_name("").is_err());
        assert!(validate_name("..").is_err());
        assert!(validate_name("a/b").is_err());
    }

    #[test]
    fn delete_non_empty_dir() {
        let mut v = vfs();
        apply_file_op(&mut v, &FileOp::Delete("/a".into())).expect("delete");
        assert!(!v.exists("/a"));
        assert!(!v.exists("/a/sub/two.txt"));
    }

    #[test]
    fn copy_into_self_rejected() {
        let mut v = vfs();
        let op = FileOp::Copy {
            from: "/a".into(),
            to_dir: "/a/sub".into(),
        };
        assert!(apply_file_op(&mut v, &op).is_err());
    }

    #[test]
    fn copy_into_same_dir_duplicates() {
        let mut v = vfs();
        let op = FileOp::Copy {
            from: "/a/one.txt".into(),
            to_dir: "/a".into(),
        };
        apply_file_op(&mut v, &op).expect("copy");
        assert_eq!(v.read("/a/one (2).txt").expect("read"), b"1");
    }

    #[test]
    fn move_dir_tree() {
        let mut v = vfs();
        let op = FileOp::Move {
            from: "/a/sub".into(),
            to_dir: "/b".into(),
        };
        apply_file_op(&mut v, &op).expect("move");
        assert!(!v.exists("/a/sub"));
        assert_eq!(v.read("/b/sub/two.txt").expect("read"), b"22");
    }

    /// A `MemoryVfs` whose `rename` always fails with `rename_err`, and
    /// whose `write` / `remove` can be made to fail for one path.
    struct Flaky {
        inner: MemoryVfs,
        rename_err: fn() -> OasisError,
        fail_write: Option<&'static str>,
        fail_remove: Option<&'static str>,
    }

    impl Flaky {
        fn new(rename_err: fn() -> OasisError) -> Self {
            Self {
                inner: vfs(),
                rename_err,
                fail_write: None,
                fail_remove: None,
            }
        }
    }

    fn denied() -> OasisError {
        VfsError::Other("permission denied".into()).into()
    }

    fn cross_device() -> OasisError {
        std::io::Error::from(std::io::ErrorKind::CrossesDevices).into()
    }

    impl Vfs for Flaky {
        fn readdir(&self, p: &str) -> oasis_types::error::Result<Vec<oasis_vfs::VfsEntry>> {
            self.inner.readdir(p)
        }
        fn read(&self, p: &str) -> oasis_types::error::Result<Vec<u8>> {
            self.inner.read(p)
        }
        fn write(&mut self, p: &str, data: &[u8]) -> oasis_types::error::Result<()> {
            if self.fail_write == Some(p) {
                return Err(denied());
            }
            self.inner.write(p, data)
        }
        fn stat(&self, p: &str) -> oasis_types::error::Result<oasis_vfs::VfsMetadata> {
            self.inner.stat(p)
        }
        fn mkdir(&mut self, p: &str) -> oasis_types::error::Result<()> {
            self.inner.mkdir(p)
        }
        fn remove(&mut self, p: &str) -> oasis_types::error::Result<()> {
            if self.fail_remove == Some(p) {
                return Err(denied());
            }
            self.inner.remove(p)
        }
        fn rename(&mut self, _from: &str, _to: &str) -> oasis_types::error::Result<()> {
            Err((self.rename_err)())
        }
        fn exists(&self, p: &str) -> bool {
            self.inner.exists(p)
        }
        fn get_permissions(
            &self,
            p: &str,
        ) -> oasis_types::error::Result<oasis_vfs::FilePermissions> {
            self.inner.get_permissions(p)
        }
        fn set_permissions(
            &mut self,
            p: &str,
            perms: oasis_vfs::FilePermissions,
        ) -> oasis_types::error::Result<()> {
            self.inner.set_permissions(p, perms)
        }
    }

    fn move_sub() -> FileOp {
        FileOp::Move {
            from: "/a/sub".into(),
            to_dir: "/b".into(),
        }
    }

    #[test]
    fn move_does_not_fall_back_on_a_real_rename_error() {
        let mut v = Flaky::new(denied);
        assert!(apply_file_op(&mut v, &move_sub()).is_err());
        assert!(v.exists("/a/sub/two.txt"), "source untouched");
        assert!(!v.exists("/b/sub"), "no copy made");
    }

    #[test]
    fn move_falls_back_to_copy_when_rename_is_unsupported() {
        let mut v = Flaky::new(cross_device);
        apply_file_op(&mut v, &move_sub()).expect("move via copy");
        assert!(!v.exists("/a/sub"));
        assert_eq!(v.read("/b/sub/two.txt").expect("read"), b"22");
    }

    #[test]
    fn failed_fallback_copy_keeps_source_and_drops_partial_copy() {
        let mut v = Flaky::new(cross_device);
        v.fail_write = Some("/b/sub/two.txt");
        assert!(apply_file_op(&mut v, &move_sub()).is_err());
        assert_eq!(v.read("/a/sub/two.txt").expect("read"), b"22");
        assert!(!v.exists("/b/sub"), "partial copy removed");
    }

    #[test]
    fn failed_source_removal_after_copy_is_reported() {
        let mut v = Flaky::new(cross_device);
        v.fail_remove = Some("/a/sub/two.txt");
        let err = apply_file_op(&mut v, &move_sub()).expect_err("remove fails");
        assert!(err.contains("Copied"), "{err}");
        assert_eq!(v.read("/b/sub/two.txt").expect("read"), b"22");
    }

    #[test]
    fn move_base_layer_entry_falls_back_to_copy() {
        let mut v = oasis_vfs::GameAssetVfs::new();
        v.add_base_dir("/a");
        v.add_base_dir("/b");
        v.add_base_file("/a/f.txt", b"base");
        let op = FileOp::Move {
            from: "/a/f.txt".into(),
            to_dir: "/b".into(),
        };
        apply_file_op(&mut v, &op).expect("move");
        assert!(!v.exists("/a/f.txt"));
        assert_eq!(v.read("/b/f.txt").expect("read"), b"base");
    }
}
