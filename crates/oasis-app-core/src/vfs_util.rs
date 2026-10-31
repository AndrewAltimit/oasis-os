//! Small VFS helpers apps share (from their `apply_vfs_ops` hooks).

use oasis_types::error::Result;
use oasis_vfs::Vfs;

/// Create the absolute directory `dir` and every missing ancestor
/// (`mkdir -p`). Existing directories are left alone; `/` and `""` are
/// no-ops.
pub fn ensure_dir(vfs: &mut dyn Vfs, dir: &str) -> Result<()> {
    let mut cur = String::new();
    for part in dir.split('/').filter(|p| !p.is_empty()) {
        cur.push('/');
        cur.push_str(part);
        if !vfs.exists(&cur) {
            vfs.mkdir(&cur)?;
        }
    }
    Ok(())
}

/// Write `data` to the absolute `path`, creating its parent directories
/// first (see [`ensure_dir`]).
pub fn write_creating_dirs(vfs: &mut dyn Vfs, path: &str, data: &[u8]) -> Result<()> {
    if let Some((dir, _)) = path.rsplit_once('/') {
        ensure_dir(vfs, dir)?;
    }
    vfs.write(path, data)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use oasis_vfs::MemoryVfs;

    #[test]
    fn ensure_dir_creates_missing_ancestors_once() {
        let mut vfs = MemoryVfs::new();
        ensure_dir(&mut vfs, "/a/b/c").unwrap();
        assert!(vfs.exists("/a") && vfs.exists("/a/b") && vfs.exists("/a/b/c"));
        // Idempotent, and the root is a no-op.
        ensure_dir(&mut vfs, "/a/b/c").unwrap();
        ensure_dir(&mut vfs, "/").unwrap();
        ensure_dir(&mut vfs, "").unwrap();
    }

    #[test]
    fn write_creating_dirs_writes_nested_file() {
        let mut vfs = MemoryVfs::new();
        write_creating_dirs(&mut vfs, "/var/x/state.txt", b"ok").unwrap();
        assert_eq!(vfs.read("/var/x/state.txt").unwrap(), b"ok");
        // A file at the root needs no directory.
        write_creating_dirs(&mut vfs, "/top.txt", b"t").unwrap();
        assert_eq!(vfs.read("/top.txt").unwrap(), b"t");
    }

    #[test]
    fn a_file_in_the_way_is_an_error_not_a_silent_overwrite() {
        let mut vfs = MemoryVfs::new();
        vfs.write("/notes", b"keep me").unwrap();
        assert!(write_creating_dirs(&mut vfs, "/notes/today.txt", b"x").is_err());
        assert_eq!(vfs.read("/notes").unwrap(), b"keep me", "file untouched");
        assert!(!vfs.exists("/notes/today.txt"));
    }
}
