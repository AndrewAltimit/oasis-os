//! One behavioural suite run against every `Vfs` implementation.
//!
//! `MemoryVfs`, `GameAssetVfs` (writes go to the overlay) and `RealVfs`
//! (rooted in a fresh temp dir) must agree on the observable semantics of
//! the trait, in particular the `rename` invariants enforced by
//! `Vfs::validate_rename`. Add a test here, not in one backend's module,
//! whenever the behaviour is part of the trait contract.

// Fixture helpers outside `#[test]` fns unwrap too; clippy.toml's
// allow-in-tests does not cover them.
#![allow(clippy::unwrap_used)]

use oasis_vfs::{EntryKind, GameAssetVfs, MemoryVfs, Vfs};

/// Names of every entry under `path`, recursively, as sorted absolute paths.
fn tree(vfs: &dyn Vfs, path: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![path.to_string()];
    while let Some(dir) = stack.pop() {
        for entry in vfs.readdir(&dir).unwrap() {
            let full = if dir == "/" {
                format!("/{}", entry.name)
            } else {
                format!("{dir}/{}", entry.name)
            };
            if entry.kind == EntryKind::Directory {
                stack.push(full.clone());
                out.push(format!("{full}/"));
            } else {
                out.push(full);
            }
        }
    }
    out.sort();
    out
}

/// `/a/` containing `/a/b/f.txt` and `/a/g.txt`, plus a top-level file.
fn populate(vfs: &mut dyn Vfs) {
    vfs.mkdir("/a/b").unwrap();
    vfs.write("/a/b/f.txt", b"f").unwrap();
    vfs.write("/a/g.txt", b"g").unwrap();
    vfs.write("/top.txt", b"top").unwrap();
}

fn populated_tree() -> Vec<String> {
    ["/a/", "/a/b/", "/a/b/f.txt", "/a/g.txt", "/top.txt"]
        .map(String::from)
        .to_vec()
}

macro_rules! vfs_conformance {
    ($module:ident, $make:expr) => {
        mod $module {
            use super::*;

            /// Build a fresh VFS. The first element keeps any backing
            /// storage (the temp dir) alive for the test's duration.
            fn make() -> (Box<dyn std::any::Any>, Box<dyn Vfs>) {
                $make
            }

            // -- basic operations -----------------------------------------

            #[test]
            fn root_is_a_directory() {
                let (_g, vfs) = make();
                assert!(vfs.exists("/"));
                assert_eq!(vfs.stat("/").unwrap().kind, EntryKind::Directory);
                assert!(vfs.readdir("/").unwrap().is_empty());
            }

            #[test]
            fn write_read_stat() {
                let (_g, mut vfs) = make();
                vfs.write("/f.txt", b"hello").unwrap();
                assert_eq!(vfs.read("/f.txt").unwrap(), b"hello");
                let meta = vfs.stat("/f.txt").unwrap();
                assert_eq!(meta.kind, EntryKind::File);
                assert_eq!(meta.size, 5);
            }

            #[test]
            fn write_needs_existing_parent() {
                let (_g, mut vfs) = make();
                assert!(vfs.write("/missing/f.txt", b"x").is_err());
                assert!(!vfs.exists("/missing"));
            }

            #[test]
            fn mkdir_creates_parents_and_is_idempotent() {
                let (_g, mut vfs) = make();
                vfs.mkdir("/x/y/z").unwrap();
                vfs.mkdir("/x/y/z").unwrap();
                assert_eq!(tree(vfs.as_ref(), "/"), ["/x/", "/x/y/", "/x/y/z/"]);
            }

            #[test]
            fn remove_rules() {
                let (_g, mut vfs) = make();
                populate(vfs.as_mut());
                assert!(vfs.remove("/").is_err(), "root");
                assert!(vfs.remove("/a").is_err(), "non-empty directory");
                assert!(vfs.remove("/ghost").is_err(), "missing path");
                vfs.remove("/a/b/f.txt").unwrap();
                vfs.remove("/a/b").unwrap();
                assert_eq!(tree(vfs.as_ref(), "/"), ["/a/", "/a/g.txt", "/top.txt"]);
            }

            #[test]
            fn readdir_of_file_fails() {
                let (_g, mut vfs) = make();
                vfs.write("/f", b"").unwrap();
                assert!(vfs.readdir("/f").is_err());
                assert!(vfs.readdir("/ghost").is_err());
            }

            // -- path normalization ---------------------------------------

            #[test]
            fn dot_segments_resolve_lexically() {
                let (_g, mut vfs) = make();
                populate(vfs.as_mut());
                assert_eq!(vfs.read("/a/./b/../g.txt").unwrap(), b"g");
                assert_eq!(vfs.read("//a//b//f.txt").unwrap(), b"f");
                assert_eq!(vfs.read("a/g.txt").unwrap(), b"g", "relative to /");
                vfs.write("/a/b/../new.txt", b"n").unwrap();
                assert_eq!(vfs.read("/a/new.txt").unwrap(), b"n");
                vfs.mkdir("/a/./c/../d").unwrap();
                assert!(vfs.exists("/a/d") && !vfs.exists("/a/c"));
                assert_eq!(vfs.stat("/a/..").unwrap().kind, EntryKind::Directory);
            }

            #[test]
            fn climbing_above_root_is_an_error() {
                let (_g, mut vfs) = make();
                populate(vfs.as_mut());
                assert!(vfs.read("/../top.txt").is_err());
                assert!(vfs.read("/a/../../top.txt").is_err());
                assert!(vfs.write("/../x", b"x").is_err());
                assert!(vfs.mkdir("/../y").is_err());
                assert!(vfs.remove("/a/b/../../../top.txt").is_err());
                assert!(vfs.rename("/top.txt", "/../t").is_err());
                assert!(!vfs.exists("/.."));
                assert_eq!(tree(vfs.as_ref(), "/"), populated_tree());
            }

            #[test]
            fn nul_byte_is_rejected() {
                let (_g, mut vfs) = make();
                assert!(vfs.write("/a\0b", b"x").is_err());
                assert!(vfs.mkdir("/d\0").is_err());
                assert!(!vfs.exists("/a\0b"));
                assert!(vfs.readdir("/").unwrap().is_empty());
            }

            // -- entry types ----------------------------------------------

            #[test]
            fn write_onto_directory_fails() {
                let (_g, mut vfs) = make();
                populate(vfs.as_mut());
                assert!(vfs.write("/a", b"x").is_err());
                assert_eq!(vfs.stat("/a").unwrap().kind, EntryKind::Directory);
                assert_eq!(tree(vfs.as_ref(), "/"), populated_tree());
            }

            #[test]
            fn nothing_is_created_beneath_a_file() {
                let (_g, mut vfs) = make();
                populate(vfs.as_mut());
                assert!(vfs.write("/top.txt/x", b"x").is_err());
                assert!(vfs.mkdir("/top.txt/sub").is_err());
                assert!(vfs.mkdir("/top.txt/sub/deeper").is_err());
                assert!(vfs.mkdir("/top.txt").is_err(), "mkdir onto a file");
                assert_eq!(vfs.read("/top.txt").unwrap(), b"top");
                assert_eq!(tree(vfs.as_ref(), "/"), populated_tree());
            }

            // -- rename: valid moves --------------------------------------

            #[test]
            fn rename_file_across_directories() {
                let (_g, mut vfs) = make();
                populate(vfs.as_mut());
                vfs.rename("/top.txt", "/a/b/top.txt").unwrap();
                assert!(!vfs.exists("/top.txt"));
                assert_eq!(vfs.read("/a/b/top.txt").unwrap(), b"top");
            }

            #[test]
            fn rename_directory_moves_subtree() {
                let (_g, mut vfs) = make();
                populate(vfs.as_mut());
                vfs.rename("/a", "/z").unwrap();
                assert_eq!(
                    tree(vfs.as_ref(), "/"),
                    ["/top.txt", "/z/", "/z/b/", "/z/b/f.txt", "/z/g.txt"]
                );
                assert_eq!(vfs.read("/z/b/f.txt").unwrap(), b"f");
            }

            #[test]
            fn rename_does_not_touch_prefix_siblings() {
                let (_g, mut vfs) = make();
                vfs.mkdir("/a").unwrap();
                vfs.write("/a/in.txt", b"1").unwrap();
                vfs.mkdir("/a-b").unwrap();
                vfs.write("/a-b/x", b"2").unwrap();
                vfs.write("/ab", b"3").unwrap();
                vfs.rename("/a", "/q").unwrap();
                assert_eq!(
                    tree(vfs.as_ref(), "/"),
                    ["/a-b/", "/a-b/x", "/ab", "/q/", "/q/in.txt"]
                );
            }

            #[test]
            fn rename_file_over_file_replaces_it() {
                let (_g, mut vfs) = make();
                vfs.write("/old", b"old").unwrap();
                vfs.write("/new", b"new").unwrap();
                vfs.rename("/new", "/old").unwrap();
                assert_eq!(vfs.read("/old").unwrap(), b"new");
                assert!(!vfs.exists("/new"));
            }

            #[test]
            fn rename_to_same_path_is_a_noop() {
                let (_g, mut vfs) = make();
                populate(vfs.as_mut());
                vfs.rename("/a", "/a").unwrap();
                vfs.rename("/top.txt", "/top.txt").unwrap();
                assert_eq!(tree(vfs.as_ref(), "/"), populated_tree());
            }

            // -- rename: invariants ---------------------------------------

            #[test]
            fn rename_into_own_subtree_is_rejected() {
                let (_g, mut vfs) = make();
                populate(vfs.as_mut());
                assert!(vfs.rename("/a", "/a/b/inner").is_err());
                assert!(vfs.rename("/a", "/a/new").is_err());
                assert_eq!(tree(vfs.as_ref(), "/"), populated_tree());
            }

            #[test]
            fn rename_root_is_rejected() {
                let (_g, mut vfs) = make();
                populate(vfs.as_mut());
                assert!(vfs.rename("/", "/x").is_err());
                assert!(vfs.rename("/a", "/").is_err());
                assert_eq!(tree(vfs.as_ref(), "/"), populated_tree());
            }

            #[test]
            fn rename_onto_directory_is_rejected() {
                let (_g, mut vfs) = make();
                populate(vfs.as_mut());
                vfs.mkdir("/empty").unwrap();
                // File onto a non-empty directory.
                assert!(vfs.rename("/top.txt", "/a").is_err());
                // Directory onto a non-empty directory.
                vfs.mkdir("/d").unwrap();
                assert!(vfs.rename("/d", "/a").is_err());
                // Onto an empty directory too: callers must name the target.
                assert!(vfs.rename("/top.txt", "/empty").is_err());
                assert!(vfs.rename("/d", "/empty").is_err());
                assert_eq!(vfs.stat("/a").unwrap().kind, EntryKind::Directory);
                assert_eq!(vfs.read("/a/g.txt").unwrap(), b"g");
                assert_eq!(vfs.read("/top.txt").unwrap(), b"top");
            }

            #[test]
            fn rename_directory_onto_file_is_rejected() {
                let (_g, mut vfs) = make();
                populate(vfs.as_mut());
                assert!(vfs.rename("/a", "/top.txt").is_err());
                assert_eq!(tree(vfs.as_ref(), "/"), populated_tree());
            }

            #[test]
            fn rename_missing_source_or_parent_fails() {
                let (_g, mut vfs) = make();
                populate(vfs.as_mut());
                assert!(vfs.rename("/ghost", "/x").is_err());
                assert!(vfs.rename("/top.txt", "/no/such/dir/x").is_err());
                // Destination parent is a file.
                assert!(vfs.rename("/a/g.txt", "/top.txt/g.txt").is_err());
                assert_eq!(tree(vfs.as_ref(), "/"), populated_tree());
            }
        }
    };
}

vfs_conformance!(memory, (Box::new(()), Box::new(MemoryVfs::new())));
vfs_conformance!(game_asset, (Box::new(()), Box::new(GameAssetVfs::new())));
// A subtree view must behave like a whole VFS rooted at its directory.
vfs_conformance!(subtree, {
    let mut inner = MemoryVfs::new();
    inner.mkdir("/srv/files").unwrap();
    inner.write("/outside.txt", b"x").unwrap();
    let inner: Box<dyn Vfs> = Box::new(inner);
    (
        Box::new(()),
        Box::new(oasis_vfs::SubtreeVfs::new(inner, "/srv/files").unwrap()),
    )
});
#[cfg(not(target_arch = "wasm32"))]
vfs_conformance!(real, {
    let dir = tempfile::tempdir().unwrap();
    let vfs = oasis_vfs::RealVfs::new(dir.path()).unwrap();
    (Box::new(dir), Box::new(vfs))
});

// -- Permission enforcement (backends with stored modes) -----------------

macro_rules! rename_permissions {
    ($module:ident, $make:expr) => {
        mod $module {
            use super::*;
            use oasis_vfs::{FilePermissions, VfsContext};

            #[test]
            fn rename_needs_write_on_both_parents() {
                let mut vfs = $make;
                vfs.mkdir("/ro").unwrap();
                vfs.mkdir("/rw").unwrap();
                vfs.write("/rw/f", b"x").unwrap();
                vfs.write("/ro/g", b"y").unwrap();
                vfs.set_permissions(
                    "/ro",
                    FilePermissions {
                        owner: "oasis".into(),
                        mode: 0o555,
                    },
                )
                .unwrap();
                vfs.set_context(VfsContext::default_user());
                assert!(vfs.rename("/rw/f", "/ro/f").is_err(), "into read-only dir");
                assert!(
                    vfs.rename("/ro/g", "/rw/g").is_err(),
                    "out of read-only dir"
                );
                assert!(vfs.exists("/rw/f") && vfs.exists("/ro/g"));
                // Root bypasses the check.
                vfs.set_context(VfsContext::root());
                vfs.rename("/rw/f", "/ro/f").unwrap();
                assert!(vfs.exists("/ro/f"));
            }
        }
    };
}

rename_permissions!(memory_permissions, MemoryVfs::new());
rename_permissions!(game_asset_permissions, GameAssetVfs::new());

// -- GameAssetVfs layering ------------------------------------------------

#[test]
fn game_asset_rename_hides_shadowed_base_entry() {
    let mut vfs = GameAssetVfs::new();
    vfs.add_base_file("/lore.txt", b"base");
    vfs.write("/lore.txt", b"edited").unwrap(); // overlay shadows base
    vfs.rename("/lore.txt", "/notes.txt").unwrap();
    assert_eq!(vfs.read("/notes.txt").unwrap(), b"edited");
    assert!(
        !vfs.exists("/lore.txt"),
        "base entry must not reappear at the old path"
    );
}

#[test]
fn game_asset_rename_onto_deleted_base_path_is_visible() {
    let mut vfs = GameAssetVfs::new();
    vfs.add_base_file("/target", b"base");
    vfs.remove("/target").unwrap();
    vfs.write("/src", b"new").unwrap();
    vfs.rename("/src", "/target").unwrap();
    assert_eq!(vfs.read("/target").unwrap(), b"new");
}

#[test]
fn game_asset_base_only_entries_cannot_move() {
    let mut vfs = GameAssetVfs::new();
    vfs.add_base_file("/base/file", b"x");
    assert!(vfs.rename("/base/file", "/moved").is_err());
    assert!(vfs.rename("/base", "/moved").is_err());
    assert!(vfs.exists("/base/file"));
}

// -- RealVfs symlink sandboxing (unix only) --------------------------------

#[cfg(unix)]
mod real_symlinks {
    use super::*;
    use oasis_vfs::RealVfs;
    use std::os::unix::fs::symlink;

    #[test]
    fn mkdir_through_escaping_symlink_with_missing_levels_fails() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        symlink(outside.path(), root.path().join("link")).unwrap();
        let mut vfs = RealVfs::new(root.path()).unwrap();

        assert!(vfs.mkdir("/link/new/x").is_err());
        assert!(vfs.mkdir("/link/new").is_err());
        assert!(vfs.write("/link/new/f.txt", b"x").is_err());
        assert!(
            std::fs::read_dir(outside.path()).unwrap().next().is_none(),
            "nothing may be created outside the VFS root"
        );
    }

    #[test]
    fn rename_out_through_escaping_symlink_fails() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        symlink(outside.path(), root.path().join("link")).unwrap();
        let mut vfs = RealVfs::new(root.path()).unwrap();
        vfs.write("/f.txt", b"x").unwrap();

        assert!(vfs.rename("/f.txt", "/link/f.txt").is_err());
        assert!(vfs.rename("/f.txt", "/link/new/f.txt").is_err());
        assert!(vfs.exists("/f.txt"));
        assert!(!outside.path().join("f.txt").exists());
    }

    #[test]
    fn rename_into_self_through_internal_symlink_fails() {
        let root = tempfile::tempdir().unwrap();
        let mut vfs = RealVfs::new(root.path()).unwrap();
        vfs.mkdir("/a/b").unwrap();
        // `/alias` -> `/a/b`: lexically unrelated to `/a`, but on disk
        // `/alias/x` is inside `/a`.
        symlink(root.path().join("a/b"), root.path().join("alias")).unwrap();
        assert!(vfs.rename("/a", "/alias/x").is_err());
        assert!(vfs.stat("/a/b").is_ok());
    }
}
