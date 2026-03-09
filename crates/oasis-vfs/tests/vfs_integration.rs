//! Integration tests for the VFS subsystem.
//!
//! Tests end-to-end workflows that span multiple VFS operations,
//! exercising MemoryVfs, GameAssetVfs, and (on non-WASM) RealVfs.

use oasis_vfs::{EntryKind, FilePermissions, GameAssetVfs, MemoryVfs, Vfs, VfsEntry};

// -----------------------------------------------------------------------
// MemoryVfs: file lifecycle (create, read, overwrite, delete)
// -----------------------------------------------------------------------

#[test]
fn memory_vfs_file_lifecycle() {
    let mut vfs = MemoryVfs::new();

    // Create directory structure
    vfs.mkdir("/projects").unwrap();
    vfs.mkdir("/projects/alpha").unwrap();

    // Write files
    vfs.write("/projects/alpha/readme.txt", b"hello world")
        .unwrap();
    vfs.write("/projects/alpha/data.bin", &[0xDE, 0xAD, 0xBE, 0xEF])
        .unwrap();

    // Read back
    assert_eq!(
        vfs.read("/projects/alpha/readme.txt").unwrap(),
        b"hello world"
    );
    assert_eq!(
        vfs.read("/projects/alpha/data.bin").unwrap(),
        &[0xDE, 0xAD, 0xBE, 0xEF]
    );

    // Overwrite
    vfs.write("/projects/alpha/readme.txt", b"updated content")
        .unwrap();
    assert_eq!(
        vfs.read("/projects/alpha/readme.txt").unwrap(),
        b"updated content"
    );

    // List directory
    let entries = vfs.readdir("/projects/alpha").unwrap();
    assert_eq!(entries.len(), 2);
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"readme.txt"));
    assert!(names.contains(&"data.bin"));

    // Remove file
    vfs.remove("/projects/alpha/data.bin").unwrap();
    assert!(!vfs.exists("/projects/alpha/data.bin"));
    assert_eq!(vfs.readdir("/projects/alpha").unwrap().len(), 1);

    // Remove remaining file and directory
    vfs.remove("/projects/alpha/readme.txt").unwrap();
    vfs.remove("/projects/alpha").unwrap();
    assert!(!vfs.exists("/projects/alpha"));
}

// -----------------------------------------------------------------------
// MemoryVfs: deep nested directory operations
// -----------------------------------------------------------------------

#[test]
fn memory_vfs_deep_nested_dirs() {
    let mut vfs = MemoryVfs::new();

    // Create deep path
    vfs.mkdir("/a").unwrap();
    vfs.mkdir("/a/b").unwrap();
    vfs.mkdir("/a/b/c").unwrap();
    vfs.mkdir("/a/b/c/d").unwrap();

    // Write file at depth
    vfs.write("/a/b/c/d/leaf.txt", b"deep").unwrap();
    assert_eq!(vfs.read("/a/b/c/d/leaf.txt").unwrap(), b"deep");

    // Stat checks
    let meta = vfs.stat("/a/b/c").unwrap();
    assert_eq!(meta.kind, EntryKind::Directory);

    let meta = vfs.stat("/a/b/c/d/leaf.txt").unwrap();
    assert_eq!(meta.kind, EntryKind::File);
    assert_eq!(meta.size, 4);
}

// -----------------------------------------------------------------------
// MemoryVfs: error cases
// -----------------------------------------------------------------------

#[test]
fn memory_vfs_error_cases() {
    let mut vfs = MemoryVfs::new();

    // Read non-existent file
    assert!(vfs.read("/nonexistent").is_err());

    // Readdir on non-existent path
    assert!(vfs.readdir("/nonexistent").is_err());

    // Remove non-existent file
    assert!(vfs.remove("/nonexistent").is_err());

    // Remove non-empty directory
    vfs.mkdir("/stuff").unwrap();
    vfs.write("/stuff/file.txt", b"x").unwrap();
    assert!(vfs.remove("/stuff").is_err());

    // Cannot remove root
    assert!(vfs.remove("/").is_err());

    // Stat non-existent
    assert!(vfs.stat("/ghost").is_err());

    // Readdir on a file (not a directory)
    vfs.write("/plain_file", b"data").unwrap();
    assert!(vfs.readdir("/plain_file").is_err());
}

// -----------------------------------------------------------------------
// MemoryVfs: rename operations
// -----------------------------------------------------------------------

#[test]
fn memory_vfs_rename() {
    let mut vfs = MemoryVfs::new();

    vfs.mkdir("/src").unwrap();
    vfs.mkdir("/dst").unwrap();
    vfs.write("/src/file.txt", b"content").unwrap();

    // Rename file
    vfs.rename("/src/file.txt", "/dst/moved.txt").unwrap();
    assert!(!vfs.exists("/src/file.txt"));
    assert!(vfs.exists("/dst/moved.txt"));
    assert_eq!(vfs.read("/dst/moved.txt").unwrap(), b"content");
}

// -----------------------------------------------------------------------
// MemoryVfs: permissions
// -----------------------------------------------------------------------

#[test]
fn memory_vfs_permissions() {
    let mut vfs = MemoryVfs::new();

    vfs.write("/secret.txt", b"classified").unwrap();

    // Default file permissions
    let perms = vfs.get_permissions("/secret.txt").unwrap();
    assert_eq!(perms.mode, 0o644);
    assert!(perms.owner_can_read());
    assert!(perms.owner_can_write());
    assert!(!perms.owner_can_execute());

    // Change permissions
    vfs.set_permissions(
        "/secret.txt",
        FilePermissions {
            owner: "root".to_string(),
            mode: 0o400,
        },
    )
    .unwrap();

    let perms = vfs.get_permissions("/secret.txt").unwrap();
    assert_eq!(perms.owner, "root");
    assert_eq!(perms.mode, 0o400);
    assert!(perms.owner_can_read());
    assert!(!perms.owner_can_write());

    // Directory default permissions
    vfs.mkdir("/bin").unwrap();
    let perms = vfs.get_permissions("/bin").unwrap();
    assert_eq!(perms.mode, 0o755);
    assert!(perms.owner_can_execute());
}

// -----------------------------------------------------------------------
// MemoryVfs: large file handling
// -----------------------------------------------------------------------

#[test]
fn memory_vfs_large_file() {
    let mut vfs = MemoryVfs::new();

    // Write 1MB file
    let data: Vec<u8> = (0..1_048_576).map(|i| (i % 256) as u8).collect();
    vfs.write("/big.bin", &data).unwrap();

    let read_back = vfs.read("/big.bin").unwrap();
    assert_eq!(read_back.len(), 1_048_576);
    assert_eq!(read_back, data);

    let meta = vfs.stat("/big.bin").unwrap();
    assert_eq!(meta.size, 1_048_576);
}

// -----------------------------------------------------------------------
// MemoryVfs: empty file and empty directory
// -----------------------------------------------------------------------

#[test]
fn memory_vfs_empty_file_and_dir() {
    let mut vfs = MemoryVfs::new();

    // Empty file
    vfs.write("/empty.txt", b"").unwrap();
    assert_eq!(vfs.read("/empty.txt").unwrap(), b"");
    assert_eq!(vfs.stat("/empty.txt").unwrap().size, 0);

    // Empty directory
    vfs.mkdir("/empty_dir").unwrap();
    assert_eq!(vfs.readdir("/empty_dir").unwrap().len(), 0);
}

// -----------------------------------------------------------------------
// GameAssetVfs: overlay write over read-only base
// -----------------------------------------------------------------------

#[test]
fn game_asset_vfs_overlay_writes() {
    let mut vfs = GameAssetVfs::new();

    // Add base content (immutable layer)
    vfs.add_base_file("/config.ini", b"original");
    vfs.add_base_dir("/data");
    vfs.add_base_file("/data/level1.dat", b"level data");

    // Read from base
    assert_eq!(vfs.read("/config.ini").unwrap(), b"original");
    assert_eq!(vfs.read("/data/level1.dat").unwrap(), b"level data");

    // Overlay write shadows the base
    vfs.write("/config.ini", b"modified").unwrap();
    assert_eq!(vfs.read("/config.ini").unwrap(), b"modified");

    // New file in overlay
    vfs.mkdir("/saves").unwrap();
    vfs.write("/saves/save1.dat", b"save data").unwrap();
    assert_eq!(vfs.read("/saves/save1.dat").unwrap(), b"save data");

    // Base file still accessible if not overwritten
    assert_eq!(vfs.read("/data/level1.dat").unwrap(), b"level data");
}

// -----------------------------------------------------------------------
// MemoryVfs: root directory listing
// -----------------------------------------------------------------------

#[test]
fn memory_vfs_root_listing() {
    let mut vfs = MemoryVfs::new();

    vfs.mkdir("/apps").unwrap();
    vfs.mkdir("/data").unwrap();
    vfs.write("/readme.txt", b"hello").unwrap();

    let entries = vfs.readdir("/").unwrap();
    assert_eq!(entries.len(), 3);

    let dirs: Vec<&VfsEntry> = entries
        .iter()
        .filter(|e| e.kind == EntryKind::Directory)
        .collect();
    let files: Vec<&VfsEntry> = entries
        .iter()
        .filter(|e| e.kind == EntryKind::File)
        .collect();

    assert_eq!(dirs.len(), 2);
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].name, "readme.txt");
}

// -----------------------------------------------------------------------
// RealVfs: basic operations with temp directory
// -----------------------------------------------------------------------

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn real_vfs_basic_operations() {
    use oasis_vfs::RealVfs;

    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_str().unwrap().to_string();
    let mut vfs = RealVfs::new(&root).unwrap();

    // Write and read
    vfs.write("/test.txt", b"real file").unwrap();
    assert_eq!(vfs.read("/test.txt").unwrap(), b"real file");
    assert!(vfs.exists("/test.txt"));

    // Mkdir and list
    vfs.mkdir("/subdir").unwrap();
    vfs.write("/subdir/nested.txt", b"nested").unwrap();

    let entries = vfs.readdir("/subdir").unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "nested.txt");

    // Remove
    vfs.remove("/subdir/nested.txt").unwrap();
    assert!(!vfs.exists("/subdir/nested.txt"));
}

// -----------------------------------------------------------------------
// MemoryVfs: concurrent-style multi-directory workflow
// -----------------------------------------------------------------------

#[test]
fn memory_vfs_multi_directory_workflow() {
    let mut vfs = MemoryVfs::new();

    // Simulate an app creating its structure
    for dir in &["/apps", "/apps/browser", "/apps/terminal", "/data", "/tmp"] {
        vfs.mkdir(dir).unwrap();
    }

    // Write config files
    vfs.write("/apps/browser/bookmarks.json", b"[]").unwrap();
    vfs.write("/apps/terminal/history", b"ls\ncd /\n").unwrap();
    vfs.write("/data/settings.toml", b"[general]\ntheme = \"dark\"")
        .unwrap();

    // Verify structure
    let app_entries = vfs.readdir("/apps").unwrap();
    assert_eq!(app_entries.len(), 2);

    let root_entries = vfs.readdir("/").unwrap();
    assert_eq!(root_entries.len(), 3); // apps, data, tmp

    // Rename a directory's file
    vfs.rename("/apps/terminal/history", "/apps/terminal/history.bak")
        .unwrap();
    assert!(!vfs.exists("/apps/terminal/history"));
    assert!(vfs.exists("/apps/terminal/history.bak"));
}
