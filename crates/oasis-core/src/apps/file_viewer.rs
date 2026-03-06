//! File and directory viewing helpers for the app runner.

use crate::vfs::{EntryKind, Vfs};

/// List a VFS directory, returning display lines.
pub(crate) fn list_directory(vfs: &dyn Vfs, path: &str) -> Vec<String> {
    let mut lines = Vec::new();

    // Parent link (unless at root).
    if path != "/" {
        lines.push("..".to_string());
    }

    match vfs.readdir(path) {
        Ok(entries) => {
            // Directories first, then files.
            let mut dirs: Vec<_> = entries
                .iter()
                .filter(|e| e.kind == EntryKind::Directory)
                .collect();
            let mut files: Vec<_> = entries
                .iter()
                .filter(|e| e.kind == EntryKind::File)
                .collect();
            dirs.sort_by(|a, b| a.name.cmp(&b.name));
            files.sort_by(|a, b| a.name.cmp(&b.name));

            for d in &dirs {
                lines.push(format!("{}/", d.name));
            }
            for f in &files {
                let size = f.size;
                if size >= 1024 {
                    lines.push(format!("{}  ({} KB)", f.name, size / 1024));
                } else {
                    lines.push(format!("{}  ({size} B)", f.name));
                }
            }

            if dirs.is_empty() && files.is_empty() {
                lines.push("(empty directory)".to_string());
            }
        },
        Err(e) => {
            lines.push(format!("Error reading directory: {e}"));
        },
    }

    lines
}

/// View an audio file: parse headers and show track metadata.
#[cfg(test)]
pub(crate) fn view_audio_file(path: &str, data: &[u8]) -> Vec<String> {
    let filename = path.rsplit('/').next().unwrap_or(path);
    let mut lines = vec![format!("=== Now Viewing: {filename} ==="), String::new()];

    let size_kb = data.len() / 1024;
    let ext = filename.rsplit('.').next().unwrap_or("").to_lowercase();

    // Detect format and parse headers.
    if data.len() >= 4 && &data[..4] == b"RIFF" && data.len() >= 44 && &data[8..12] == b"WAVE" {
        // WAV file -- parse header.
        let channels = u16::from_le_bytes([data[22], data[23]]);
        let sample_rate = u32::from_le_bytes([data[24], data[25], data[26], data[27]]);
        let bits = u16::from_le_bytes([data[34], data[35]]);
        let data_size = if data.len() >= 44 {
            u32::from_le_bytes([data[40], data[41], data[42], data[43]])
        } else {
            0
        };
        let duration_secs = if sample_rate > 0 && channels > 0 && bits > 0 {
            data_size as f64 / (sample_rate as f64 * channels as f64 * (bits as f64 / 8.0))
        } else {
            0.0
        };

        lines.push("  Format:       WAV (PCM audio)".to_string());
        lines.push(format!("  Sample Rate:  {sample_rate} Hz"));
        lines.push(format!("  Channels:     {channels}"));
        lines.push(format!("  Bit Depth:    {bits}-bit"));
        lines.push(format!("  Duration:     {duration_secs:.1}s"));
        lines.push(format!("  File Size:    {size_kb} KB"));
    } else if data.len() >= 3 && (data[..2] == [0xFF, 0xFB] || data[..3] == *b"ID3") {
        // MP3 file.
        lines.push("  Format:       MP3 (MPEG audio)".to_string());
        lines.push(format!("  File Size:    {size_kb} KB"));

        // Try to extract ID3v2 title/artist.
        if data.len() > 10 && &data[..3] == b"ID3" {
            let id3_info = parse_id3v2_basic(data);
            if let Some(title) = id3_info.0 {
                lines.push(format!("  Title:        {title}"));
            }
            if let Some(artist) = id3_info.1 {
                lines.push(format!("  Artist:       {artist}"));
            }
        }

        // Rough duration estimate from file size (128kbps average).
        let est_secs = (data.len() as f64) / (128.0 * 1024.0 / 8.0);
        lines.push(format!("  Duration:     ~{est_secs:.0}s (estimated)"));
    } else {
        lines.push(format!("  Format:       {ext} audio"));
        lines.push(format!("  File Size:    {size_kb} KB"));
    }

    lines.push(String::new());
    lines.push("----------------------------------".to_string());
    lines.push(String::new());
    lines.push("  To play in terminal:".to_string());
    lines.push("    music play".to_string());
    lines.push("    music pause / music stop".to_string());
    lines.push("    music vol <0-100>".to_string());
    lines.push(String::new());
    lines.push("Cancel=back to library".to_string());
    lines
}

/// Try to extract title and artist from an ID3v2 tag.
/// Returns (Option<title>, Option<artist>).
#[cfg(test)]
fn parse_id3v2_basic(data: &[u8]) -> (Option<String>, Option<String>) {
    if data.len() < 10 || &data[..3] != b"ID3" {
        return (None, None);
    }
    let header_size = ((data[6] as usize & 0x7F) << 21)
        | ((data[7] as usize & 0x7F) << 14)
        | ((data[8] as usize & 0x7F) << 7)
        | (data[9] as usize & 0x7F);
    let end = (10 + header_size).min(data.len());

    let mut title = None;
    let mut artist = None;
    let mut pos = 10;

    while pos + 10 < end {
        let frame_id = &data[pos..pos + 4];
        let frame_size =
            u32::from_be_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]])
                as usize;
        if frame_size == 0 || pos + 10 + frame_size > end {
            break;
        }
        let frame_data = &data[pos + 10..pos + 10 + frame_size];
        // Skip encoding byte, extract as lossy UTF-8.
        let text = if frame_data.len() > 1 {
            String::from_utf8_lossy(&frame_data[1..])
                .trim_matches('\0')
                .to_string()
        } else {
            String::new()
        };

        if frame_id == b"TIT2" && !text.is_empty() {
            title = Some(text);
        } else if frame_id == b"TPE1" && !text.is_empty() {
            artist = Some(text);
        }

        pos += 10 + frame_size;
    }

    (title, artist)
}

/// View an image file: parse headers and show image metadata.
#[cfg(test)]
pub(crate) fn view_image_file(path: &str, data: &[u8]) -> Vec<String> {
    let filename = path.rsplit('/').next().unwrap_or(path);
    let mut lines = vec![format!("=== Photo: {filename} ==="), String::new()];

    let size_kb = data.len() / 1024;

    if data.len() >= 24 && &data[..8] == b"\x89PNG\r\n\x1a\n" {
        // PNG -- IHDR is at offset 8 (4 len + 4 type + data).
        let w = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
        let h = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
        let bit_depth = data[24];
        let color_type = data[25];
        let color_name = match color_type {
            0 => "Grayscale",
            2 => "RGB",
            3 => "Indexed",
            4 => "Grayscale+Alpha",
            6 => "RGBA",
            _ => "Unknown",
        };

        lines.push("  Format:       PNG".to_string());
        lines.push(format!("  Dimensions:   {w} x {h} pixels"));
        lines.push(format!("  Color:        {color_name} ({bit_depth}-bit)"));
        lines.push(format!("  File Size:    {size_kb} KB"));
    } else if data.len() >= 2 && data[..2] == [0xFF, 0xD8] {
        // JPEG.
        let (w, h) = parse_jpeg_dimensions(data);
        lines.push("  Format:       JPEG".to_string());
        if w > 0 && h > 0 {
            lines.push(format!("  Dimensions:   {w} x {h} pixels"));
        }
        lines.push(format!("  File Size:    {size_kb} KB"));
    } else if data.len() >= 6 && (&data[..4] == b"GIF8") {
        // GIF.
        let w = u16::from_le_bytes([data[6], data[7]]);
        let h = u16::from_le_bytes([data[8], data[9]]);
        lines.push("  Format:       GIF".to_string());
        lines.push(format!("  Dimensions:   {w} x {h} pixels"));
        lines.push(format!("  File Size:    {size_kb} KB"));
    } else if data.len() >= 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        // WebP.
        lines.push("  Format:       WebP".to_string());
        lines.push(format!("  File Size:    {size_kb} KB"));
    } else {
        let ext = filename.rsplit('.').next().unwrap_or("").to_lowercase();
        lines.push(format!("  Format:       {ext} image"));
        lines.push(format!("  File Size:    {size_kb} KB"));
    }

    lines.push(String::new());
    lines.push("----------------------------------".to_string());
    lines.push(String::new());
    lines.push("  (Image preview not available".to_string());
    lines.push("   in text mode)".to_string());
    lines.push(String::new());
    lines.push("Cancel=back to gallery".to_string());
    lines
}

/// Try to extract JPEG image dimensions from SOF markers.
#[cfg(test)]
fn parse_jpeg_dimensions(data: &[u8]) -> (u16, u16) {
    let mut pos = 2;
    while pos + 4 < data.len() {
        if data[pos] != 0xFF {
            break;
        }
        let marker = data[pos + 1];
        // SOF0..SOF3 markers contain dimensions.
        if (0xC0..=0xC3).contains(&marker) && pos + 9 < data.len() {
            let h = u16::from_be_bytes([data[pos + 5], data[pos + 6]]);
            let w = u16::from_be_bytes([data[pos + 7], data[pos + 8]]);
            return (w, h);
        }
        if marker == 0xD9 || marker == 0xDA {
            break; // End of headers.
        }
        let seg_len = u16::from_be_bytes([data[pos + 2], data[pos + 3]]) as usize;
        pos += 2 + seg_len;
    }
    (0, 0)
}

