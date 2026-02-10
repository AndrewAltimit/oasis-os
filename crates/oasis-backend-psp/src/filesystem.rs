//! File system operations and image decoding.
//!
//! Directory listing and file reading use `psp::io` RAII wrappers.
//! JPEG decoding uses `psp::image::decode_jpeg()` hardware decoder.

/// A single entry from a directory listing.
pub struct FileEntry {
    /// File or directory name (ASCII, up to 255 chars).
    pub name: String,
    /// File size in bytes (0 for directories).
    pub size: i64,
    /// True if this entry is a directory.
    pub is_dir: bool,
}

/// List the contents of a directory path (e.g. `"ms0:/"`, `"ms0:/PSP/GAME"`).
///
/// Returns a sorted list of entries (directories first, then files,
/// alphabetically within each group). Returns an empty vec on error.
pub fn list_directory(path: &str) -> Vec<FileEntry> {
    let mut entries = Vec::new();

    let dir = match psp::io::read_dir(path) {
        Ok(d) => d,
        Err(_) => return entries,
    };

    for result in dir {
        let entry = match result {
            Ok(e) => e,
            Err(_) => break,
        };

        let name = match core::str::from_utf8(entry.name()) {
            Ok(s) => s.to_string(),
            Err(_) => continue,
        };

        // Skip . and ..
        if name == "." || name == ".." {
            continue;
        }

        let is_dir = entry.is_dir();
        let size = if is_dir { 0 } else { entry.stat().st_size };

        entries.push(FileEntry { name, size, is_dir });
    }

    // Sort: directories first, then alphabetically.
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));

    entries
}

/// Format a file size as a human-readable string.
pub fn format_size(bytes: i64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{} KB", bytes / 1024)
    } else {
        format!("{}.{} MB", bytes / (1024 * 1024), (bytes / 102400) % 10)
    }
}

/// Read an entire file into a byte vector.
///
/// Returns `None` if the file cannot be opened or read.
pub fn read_file(path: &str) -> Option<Vec<u8>> {
    psp::io::read_to_vec(path).ok()
}

/// Decode a JPEG image using the PSP's hardware MJPEG decoder.
///
/// Returns `(width, height, rgba_pixels)` on success. The output is RGBA8888.
/// `max_w` and `max_h` set the maximum decode dimensions (use 480, 272 for
/// screen-sized images).
pub fn decode_jpeg(jpeg_data: &[u8], max_w: i32, max_h: i32) -> Option<(u32, u32, Vec<u8>)> {
    let img = psp::image::decode_jpeg(jpeg_data, max_w, max_h).ok()?;
    Some((img.width, img.height, img.data))
}
