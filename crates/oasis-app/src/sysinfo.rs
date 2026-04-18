//! Tiny system probe helpers for the functional boot sequence.
//!
//! Intentionally zero-dep: reads Linux `/proc` files directly and
//! degrades gracefully on other platforms or if the files are missing.
//! The values populate BIOS-phase lines in the boot splash so users
//! see real machine info during boot instead of a scripted loop.

/// Report total system RAM in kilobytes (matches the `/proc/meminfo`
/// `MemTotal` unit for a retro-feel BIOS line).
///
/// Returns `None` on non-Linux hosts or if `/proc/meminfo` is unreadable.
pub fn total_ram_kb() -> Option<u64> {
    let data = std::fs::read_to_string("/proc/meminfo").ok()?;
    for line in data.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            // Format: "MemTotal:       16292108 kB"
            return rest
                .split_whitespace()
                .next()
                .and_then(|s| s.parse::<u64>().ok());
        }
    }
    None
}

/// Report the number of logical CPU cores.
///
/// Falls back to `std::thread::available_parallelism` if `/proc/cpuinfo`
/// is unavailable.
pub fn cpu_core_count() -> Option<usize> {
    // Prefer /proc/cpuinfo since that's what a real BIOS would read.
    if let Ok(data) = std::fs::read_to_string("/proc/cpuinfo") {
        let count = data.lines().filter(|l| l.starts_with("processor")).count();
        if count > 0 {
            return Some(count);
        }
    }
    std::thread::available_parallelism().ok().map(|n| n.get())
}

/// Count files and directories reachable from `root` in the given VFS.
///
/// Returns `(file_count, dir_count)`. Errors are swallowed so the probe
/// can't fail the boot sequence.
pub fn count_vfs_entries(vfs: &dyn oasis_core::vfs::Vfs, root: &str) -> (usize, usize) {
    use oasis_core::vfs::EntryKind;

    fn walk(
        vfs: &dyn oasis_core::vfs::Vfs,
        path: &str,
        files: &mut usize,
        dirs: &mut usize,
        depth: usize,
    ) {
        // Safety net: refuse to descend beyond a sane depth.
        if depth > 8 {
            return;
        }
        let Ok(entries) = vfs.readdir(path) else {
            return;
        };
        for entry in entries {
            let child = if path == "/" {
                format!("/{}", entry.name)
            } else {
                format!("{path}/{}", entry.name)
            };
            match entry.kind {
                EntryKind::Directory => {
                    *dirs += 1;
                    walk(vfs, &child, files, dirs, depth + 1);
                },
                EntryKind::File => {
                    *files += 1;
                },
            }
        }
    }

    let mut files = 0;
    let mut dirs = 0;
    walk(vfs, root, &mut files, &mut dirs, 0);
    (files, dirs)
}
