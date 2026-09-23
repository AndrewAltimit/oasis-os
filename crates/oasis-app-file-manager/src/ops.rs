//! VFS mutation helpers for [`FileOp`]s: recursive copy / delete, unique
//! name generation and the op executor used by `App::apply_vfs_ops`.
//!
//! Everything here works purely through the [`Vfs`] trait, so it behaves
//! the same on `MemoryVfs`, `RealVfs` and `GameAssetVfs`.

use oasis_app_core::file_viewer::{join_path, parent_dir};
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
            if vfs.rename(from, &target).is_err() {
                // Backends without cross-directory rename: copy + delete.
                copy_recursive(vfs, from, &target)?;
                remove_recursive(vfs, from)?;
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
}
