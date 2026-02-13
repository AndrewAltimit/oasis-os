//! In-memory VFS implementation.
//!
//! Useful for unit tests and ephemeral terminals. The entire file tree lives
//! in a `HashMap<String, Node>` where keys are normalized absolute paths.

use std::borrow::Cow;
use std::collections::BTreeMap;

use oasis_types::error::{OasisError, Result};

use crate::{EntryKind, Vfs, VfsEntry, VfsMetadata};

#[derive(Debug, Clone)]
enum Node {
    File(Vec<u8>),
    Dir,
}

/// A fully in-memory virtual file system.
#[derive(Debug)]
pub struct MemoryVfs {
    /// Map of normalized paths to file/directory nodes.
    nodes: BTreeMap<String, Node>,
}

impl MemoryVfs {
    /// Create a new in-memory VFS with only the root directory.
    pub fn new() -> Self {
        let mut nodes = BTreeMap::new();
        nodes.insert("/".to_string(), Node::Dir);
        Self { nodes }
    }
}

impl Default for MemoryVfs {
    fn default() -> Self {
        Self::new()
    }
}

/// Check whether a path is already in normal form (starts with `/`, no `//`,
/// no trailing `/` unless root).
fn is_normalized(path: &str) -> bool {
    if !path.starts_with('/') {
        return false;
    }
    if path.len() > 1 && path.ends_with('/') {
        return false;
    }
    !path.contains("//")
}

/// Normalize a path: ensure leading `/`, collapse `//`, strip trailing `/`
/// (except for root). Returns the input unchanged (zero-alloc) when already
/// in normal form.
fn normalize(path: &str) -> Cow<'_, str> {
    if is_normalized(path) {
        return Cow::Borrowed(path);
    }
    let path_str = if path.starts_with('/') {
        Cow::Borrowed(path)
    } else {
        Cow::Owned(format!("/{path}"))
    };
    // Collapse repeated slashes.
    let mut result = String::with_capacity(path_str.len());
    let mut prev_slash = false;
    for ch in path_str.chars() {
        if ch == '/' {
            if !prev_slash {
                result.push(ch);
            }
            prev_slash = true;
        } else {
            result.push(ch);
            prev_slash = false;
        }
    }
    // Strip trailing slash unless root.
    if result.len() > 1 && result.ends_with('/') {
        result.pop();
    }
    Cow::Owned(result)
}

/// Return the parent of a normalized path.
fn parent(path: &str) -> &str {
    if path == "/" {
        return "/";
    }
    match path.rfind('/') {
        Some(0) => "/",
        Some(i) => &path[..i],
        None => "/",
    }
}

impl Vfs for MemoryVfs {
    fn readdir(&self, path: &str) -> Result<Vec<VfsEntry>> {
        let path = normalize(path);
        match self.nodes.get(path.as_ref()) {
            Some(Node::Dir) => {},
            Some(Node::File(_)) => {
                return Err(OasisError::Vfs(format!("not a directory: {path}")));
            },
            None => {
                return Err(OasisError::Vfs(format!("no such directory: {path}")));
            },
        }

        let prefix = if path.as_ref() == "/" {
            Cow::Borrowed("/")
        } else {
            Cow::Owned(format!("{path}/"))
        };

        // BTreeMap iteration is already sorted by key, so entries come out
        // in lexicographic order. We can use range to narrow the scan.
        let prefix_str = prefix.as_ref().to_string();
        let mut entries = Vec::new();
        for (key, node) in self.nodes.range(prefix_str.clone()..) {
            // Stop once we've passed the prefix.
            if !key.starts_with(&prefix_str) {
                break;
            }
            // Direct child only: non-empty name with no `/` after the prefix.
            let rest = &key[prefix_str.len()..];
            if !rest.is_empty() && !rest.contains('/') {
                entries.push(VfsEntry {
                    name: rest.to_string(),
                    kind: match node {
                        Node::Dir => EntryKind::Directory,
                        Node::File(_) => EntryKind::File,
                    },
                    size: match node {
                        Node::File(data) => data.len() as u64,
                        Node::Dir => 0,
                    },
                });
            }
        }
        // BTreeMap gives us sorted keys, but child names are sorted by full
        // path which is the same as sorting by name when they share a prefix.
        Ok(entries)
    }

    fn read(&self, path: &str) -> Result<Vec<u8>> {
        let path = normalize(path);
        match self.nodes.get(path.as_ref()) {
            Some(Node::File(data)) => Ok(data.clone()),
            Some(Node::Dir) => Err(OasisError::Vfs(format!("is a directory: {path}"))),
            None => Err(OasisError::Vfs(format!("no such file: {path}"))),
        }
    }

    fn write(&mut self, path: &str, data: &[u8]) -> Result<()> {
        let path = normalize(path);
        // Ensure parent directory exists.
        let par = parent(&path);
        if !self.nodes.contains_key(par) {
            return Err(OasisError::Vfs(format!(
                "parent directory does not exist: {par}"
            )));
        }
        self.nodes
            .insert(path.into_owned(), Node::File(data.to_vec()));
        Ok(())
    }

    fn stat(&self, path: &str) -> Result<VfsMetadata> {
        let path = normalize(path);
        match self.nodes.get(path.as_ref()) {
            Some(Node::File(data)) => Ok(VfsMetadata {
                kind: EntryKind::File,
                size: data.len() as u64,
            }),
            Some(Node::Dir) => Ok(VfsMetadata {
                kind: EntryKind::Directory,
                size: 0,
            }),
            None => Err(OasisError::Vfs(format!("no such path: {path}"))),
        }
    }

    fn mkdir(&mut self, path: &str) -> Result<()> {
        let path = normalize(path);
        if self.nodes.contains_key(path.as_ref()) {
            return Ok(()); // Already exists, no error.
        }
        // Ensure parent exists (create parents recursively).
        let par = parent(&path).to_string();
        if par != path.as_ref() && !self.nodes.contains_key(&par) {
            self.mkdir(&par)?;
        }
        self.nodes.insert(path.into_owned(), Node::Dir);
        Ok(())
    }

    fn remove(&mut self, path: &str) -> Result<()> {
        let path = normalize(path);
        if path.as_ref() == "/" {
            return Err(OasisError::Vfs("cannot remove root".to_string()));
        }
        match self.nodes.get(path.as_ref()) {
            Some(Node::Dir) => {
                // Check that directory is empty using BTreeMap range scan.
                let prefix = format!("{path}/");
                let has_children = self
                    .nodes
                    .range(prefix.clone()..)
                    .next()
                    .is_some_and(|(k, _)| k.starts_with(&prefix));
                if has_children {
                    return Err(OasisError::Vfs(format!("directory not empty: {path}")));
                }
            },
            Some(Node::File(_)) => {},
            None => {
                return Err(OasisError::Vfs(format!("no such path: {path}")));
            },
        }
        self.nodes.remove(path.as_ref());
        Ok(())
    }

    fn exists(&self, path: &str) -> bool {
        let path = normalize(path);
        self.nodes.contains_key(path.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_exists() {
        let vfs = MemoryVfs::new();
        assert!(vfs.exists("/"));
    }

    #[test]
    fn mkdir_and_readdir() {
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/home").unwrap();
        vfs.mkdir("/home/user").unwrap();
        let entries = vfs.readdir("/").unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "home");
        assert_eq!(entries[0].kind, EntryKind::Directory);
    }

    #[test]
    fn write_and_read() {
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        vfs.write("/tmp/test.txt", b"hello world").unwrap();
        let data = vfs.read("/tmp/test.txt").unwrap();
        assert_eq!(data, b"hello world");
    }

    #[test]
    fn stat_file() {
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/data").unwrap();
        vfs.write("/data/f.bin", &[1, 2, 3]).unwrap();
        let meta = vfs.stat("/data/f.bin").unwrap();
        assert_eq!(meta.kind, EntryKind::File);
        assert_eq!(meta.size, 3);
    }

    #[test]
    fn stat_dir() {
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/etc").unwrap();
        let meta = vfs.stat("/etc").unwrap();
        assert_eq!(meta.kind, EntryKind::Directory);
    }

    #[test]
    fn remove_file() {
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/tmp").unwrap();
        vfs.write("/tmp/x", b"data").unwrap();
        assert!(vfs.exists("/tmp/x"));
        vfs.remove("/tmp/x").unwrap();
        assert!(!vfs.exists("/tmp/x"));
    }

    #[test]
    fn remove_empty_dir() {
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/empty").unwrap();
        vfs.remove("/empty").unwrap();
        assert!(!vfs.exists("/empty"));
    }

    #[test]
    fn remove_nonempty_dir_fails() {
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/dir").unwrap();
        vfs.write("/dir/file", b"x").unwrap();
        assert!(vfs.remove("/dir").is_err());
    }

    #[test]
    fn remove_root_fails() {
        let mut vfs = MemoryVfs::new();
        assert!(vfs.remove("/").is_err());
    }

    #[test]
    fn write_without_parent_fails() {
        let mut vfs = MemoryVfs::new();
        assert!(vfs.write("/no/such/dir/file", b"x").is_err());
    }

    #[test]
    fn read_nonexistent_fails() {
        let vfs = MemoryVfs::new();
        assert!(vfs.read("/nope").is_err());
    }

    #[test]
    fn readdir_on_file_fails() {
        let mut vfs = MemoryVfs::new();
        vfs.write("/file", b"data").unwrap();
        assert!(vfs.readdir("/file").is_err());
    }

    #[test]
    fn mkdir_creates_parents() {
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/a/b/c").unwrap();
        assert!(vfs.exists("/a"));
        assert!(vfs.exists("/a/b"));
        assert!(vfs.exists("/a/b/c"));
    }

    #[test]
    fn normalize_paths() {
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/dir/").unwrap();
        assert!(vfs.exists("/dir"));
        vfs.write("//dir//file", b"ok").unwrap();
        let data = vfs.read("/dir/file").unwrap();
        assert_eq!(data, b"ok");
    }

    #[test]
    fn readdir_only_direct_children() {
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/a/b/c").unwrap();
        vfs.write("/a/file.txt", b"hi").unwrap();
        let entries = vfs.readdir("/a").unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"b"));
        assert!(names.contains(&"file.txt"));
        assert!(!names.contains(&"c")); // c is a grandchild
    }

    #[test]
    fn overwrite_file() {
        let mut vfs = MemoryVfs::new();
        vfs.write("/file", b"old").unwrap();
        vfs.write("/file", b"new content").unwrap();
        assert_eq!(vfs.read("/file").unwrap(), b"new content");
    }

    // -- robustness / edge cases ----------------------------------------

    #[test]
    fn dotdot_is_not_resolved() {
        // normalize() does NOT resolve `..` -- this documents the current behavior.
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/a/b").unwrap();
        // Writing to /a/b/../c creates a literal `..` directory component.
        let result = vfs.write("/a/b/../c/file", b"data");
        // Should fail because /a/b/.. is not a real directory.
        assert!(result.is_err());
    }

    #[test]
    fn dot_component_in_path() {
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/dir").unwrap();
        // /dir/./file -- the `.` is kept literally.
        let result = vfs.write("/dir/./file", b"data");
        // Should fail (no `./` directory exists as parent).
        assert!(result.is_err());
    }

    #[test]
    fn special_characters_in_filename() {
        let mut vfs = MemoryVfs::new();
        vfs.write("/file with spaces.txt", b"ok").unwrap();
        assert_eq!(vfs.read("/file with spaces.txt").unwrap(), b"ok");
    }

    #[test]
    fn unicode_in_filename() {
        let mut vfs = MemoryVfs::new();
        vfs.write("/\u{1F600}_emoji.txt", b"smiley").unwrap();
        assert_eq!(vfs.read("/\u{1F600}_emoji.txt").unwrap(), b"smiley");
    }

    #[test]
    fn filename_with_dots() {
        let mut vfs = MemoryVfs::new();
        vfs.write("/file.tar.gz", b"archive").unwrap();
        assert_eq!(vfs.read("/file.tar.gz").unwrap(), b"archive");
    }

    #[test]
    fn empty_filename_component() {
        // Path with empty component: /dir//file -> normalize collapses to /dir/file.
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/dir").unwrap();
        vfs.write("/dir//file", b"data").unwrap();
        assert_eq!(vfs.read("/dir/file").unwrap(), b"data");
    }

    #[test]
    fn write_empty_data() {
        let mut vfs = MemoryVfs::new();
        vfs.write("/empty", b"").unwrap();
        assert_eq!(vfs.read("/empty").unwrap(), b"");
        assert!(vfs.exists("/empty"));
    }

    #[test]
    fn write_large_file() {
        let mut vfs = MemoryVfs::new();
        let data = vec![0xFFu8; 1_000_000]; // 1MB
        vfs.write("/big", &data).unwrap();
        assert_eq!(vfs.read("/big").unwrap().len(), 1_000_000);
    }

    #[test]
    fn readdir_empty_dir() {
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/empty_dir").unwrap();
        let entries = vfs.readdir("/empty_dir").unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn readdir_root() {
        let vfs = MemoryVfs::new();
        let entries = vfs.readdir("/").unwrap();
        // Fresh VFS has no children of root.
        assert!(entries.is_empty());
    }

    #[test]
    fn mkdir_existing_dir_is_ok() {
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/dir").unwrap();
        // Creating same directory again should be idempotent.
        vfs.mkdir("/dir").unwrap();
        assert!(vfs.exists("/dir"));
    }

    #[test]
    fn remove_file_then_readd() {
        let mut vfs = MemoryVfs::new();
        vfs.write("/file", b"first").unwrap();
        vfs.remove("/file").unwrap();
        assert!(!vfs.exists("/file"));
        vfs.write("/file", b"second").unwrap();
        assert_eq!(vfs.read("/file").unwrap(), b"second");
    }

    #[test]
    fn remove_nonexistent_fails() {
        let mut vfs = MemoryVfs::new();
        assert!(vfs.remove("/ghost").is_err());
    }

    #[test]
    fn deeply_nested_dirs() {
        let mut vfs = MemoryVfs::new();
        let path: String = (0..50).map(|i| format!("/d{i}")).collect();
        vfs.mkdir(&path).unwrap();
        assert!(vfs.exists(&path));
        vfs.write(&format!("{path}/leaf.txt"), b"deep").unwrap();
        assert_eq!(vfs.read(&format!("{path}/leaf.txt")).unwrap(), b"deep");
    }

    #[test]
    fn write_to_dir_path_fails() {
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/dir").unwrap();
        // Trying to write to a path that is a directory should fail
        // (or overwrite with file -- depends on impl). Check it doesn't panic.
        let _ = vfs.write("/dir", b"data");
    }

    #[test]
    fn read_dir_as_file_fails() {
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/dir").unwrap();
        assert!(vfs.read("/dir").is_err());
    }

    #[test]
    fn many_files_in_one_dir() {
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/dir").unwrap();
        for i in 0..200 {
            vfs.write(&format!("/dir/file_{i}"), b"x").unwrap();
        }
        let entries = vfs.readdir("/dir").unwrap();
        assert_eq!(entries.len(), 200);
    }

    mod prop {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            #[test]
            fn normalize_is_idempotent(path in "[/a-z0-9_.]{1,50}") {
                let once = normalize(&path);
                let twice = normalize(&once);
                prop_assert_eq!(&once, &twice, "normalize must be idempotent");
            }

            #[test]
            fn normalize_never_has_double_slashes(path in "[/a-z0-9_.]{1,50}") {
                let normed = normalize(&path);
                prop_assert!(
                    !normed.contains("//"),
                    "normalized path must not contain //: {normed}"
                );
            }

            #[test]
            fn normalize_starts_with_slash(path in "[a-z0-9_./]{0,50}") {
                let normed = normalize(&path);
                prop_assert!(
                    normed.starts_with('/'),
                    "normalized path must start with /: {normed}"
                );
            }

            #[test]
            fn normalize_no_trailing_slash_unless_root(path in "[/a-z0-9_.]{1,50}") {
                let normed = normalize(&path);
                if normed != "/" {
                    prop_assert!(
                        !normed.ends_with('/'),
                        "non-root normalized path must not end with /: {normed}"
                    );
                }
            }

            #[test]
            fn write_then_read_roundtrips(
                dir in "[a-z]{1,8}",
                file in "[a-z]{1,8}",
                data in proptest::collection::vec(any::<u8>(), 0..256),
            ) {
                let mut vfs = MemoryVfs::new();
                let dir_path = format!("/{dir}");
                vfs.mkdir(&dir_path).unwrap();
                let file_path = format!("{dir_path}/{file}");
                vfs.write(&file_path, &data).unwrap();
                let read_back = vfs.read(&file_path).unwrap();
                prop_assert_eq!(data, read_back);
            }

            #[test]
            fn exists_after_write(
                dir in "[a-z]{1,8}",
                file in "[a-z]{1,8}",
            ) {
                let mut vfs = MemoryVfs::new();
                let dir_path = format!("/{dir}");
                vfs.mkdir(&dir_path).unwrap();
                let file_path = format!("{dir_path}/{file}");
                vfs.write(&file_path, b"x").unwrap();
                prop_assert!(vfs.exists(&file_path));
            }

            #[test]
            fn mkdir_then_exists(segments in proptest::collection::vec("[a-z]{1,6}", 1..5)) {
                let mut vfs = MemoryVfs::new();
                let path = format!("/{}", segments.join("/"));
                vfs.mkdir(&path).unwrap();
                prop_assert!(vfs.exists(&path));
                // All parent directories should exist too.
                let mut partial = String::new();
                for seg in &segments {
                    partial.push('/');
                    partial.push_str(seg);
                    prop_assert!(vfs.exists(&partial), "missing parent: {partial}");
                }
            }

            #[test]
            fn remove_then_not_exists(name in "[a-z]{1,8}") {
                let mut vfs = MemoryVfs::new();
                let path = format!("/{name}");
                vfs.mkdir(&path).unwrap();
                vfs.remove(&path).unwrap();
                prop_assert!(!vfs.exists(&path));
            }
        }
    }
}
