//! File and directory viewing helpers for app content.

use oasis_vfs::{EntryKind, Vfs};

/// Get parent directory of a path.
pub fn parent_dir(path: &str) -> String {
    if path == "/" {
        "/".to_string()
    } else {
        let trimmed = path.trim_end_matches('/');
        match trimmed.rfind('/') {
            Some(0) => "/".to_string(),
            Some(pos) => trimmed[..pos].to_string(),
            None => "/".to_string(),
        }
    }
}

/// Join a directory and a name with proper path separator.
pub fn join_path(dir: &str, name: &str) -> String {
    if dir == "/" {
        format!("/{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// List a VFS directory, returning display lines.
pub fn list_directory(vfs: &dyn Vfs, path: &str) -> Vec<String> {
    let mut lines = Vec::new();

    if path != "/" {
        lines.push("..".to_string());
    }

    match vfs.readdir(path) {
        Ok(entries) => {
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
pub fn view_audio_file(path: &str, data: &[u8]) -> Vec<String> {
    let filename = path.rsplit('/').next().unwrap_or(path);
    let mut lines = vec![format!("=== Now Viewing: {filename} ==="), String::new()];

    let size_kb = data.len() / 1024;
    let ext = filename.rsplit('.').next().unwrap_or("").to_lowercase();

    if data.len() >= 4 && &data[..4] == b"RIFF" && data.len() >= 44 && &data[8..12] == b"WAVE" {
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
        lines.push("  Format:       MP3 (MPEG audio)".to_string());
        lines.push(format!("  File Size:    {size_kb} KB"));

        if data.len() > 10 && &data[..3] == b"ID3" {
            let id3_info = parse_id3v2_basic(data);
            if let Some(title) = id3_info.0 {
                lines.push(format!("  Title:        {title}"));
            }
            if let Some(artist) = id3_info.1 {
                lines.push(format!("  Artist:       {artist}"));
            }
        }

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
pub fn view_image_file(path: &str, data: &[u8]) -> Vec<String> {
    let filename = path.rsplit('/').next().unwrap_or(path);
    let mut lines = vec![format!("=== Photo: {filename} ==="), String::new()];

    let size_kb = data.len() / 1024;

    if data.len() >= 24 && &data[..8] == b"\x89PNG\r\n\x1a\n" {
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
        let (w, h) = parse_jpeg_dimensions(data);
        lines.push("  Format:       JPEG".to_string());
        if w > 0 && h > 0 {
            lines.push(format!("  Dimensions:   {w} x {h} pixels"));
        }
        lines.push(format!("  File Size:    {size_kb} KB"));
    } else if data.len() >= 6 && &data[..4] == b"GIF8" {
        let w = u16::from_le_bytes([data[6], data[7]]);
        let h = u16::from_le_bytes([data[8], data[9]]);
        lines.push("  Format:       GIF".to_string());
        lines.push(format!("  Dimensions:   {w} x {h} pixels"));
        lines.push(format!("  File Size:    {size_kb} KB"));
    } else if data.len() >= 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WEBP" {
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
fn parse_jpeg_dimensions(data: &[u8]) -> (u16, u16) {
    let mut pos = 2;
    while pos + 4 < data.len() {
        if data[pos] != 0xFF {
            break;
        }
        let marker = data[pos + 1];
        if (0xC0..=0xC3).contains(&marker) && pos + 9 < data.len() {
            let h = u16::from_be_bytes([data[pos + 5], data[pos + 6]]);
            let w = u16::from_be_bytes([data[pos + 7], data[pos + 8]]);
            return (w, h);
        }
        if marker == 0xD9 || marker == 0xDA {
            break;
        }
        let seg_len = u16::from_be_bytes([data[pos + 2], data[pos + 3]]) as usize;
        pos += 2 + seg_len;
    }
    (0, 0)
}

/// Given a file path, pick the best-fit app title to open it in. Returns
/// `None` for file types with no dedicated viewer; the caller should stay
/// in the generic hex/text viewer for those.
///
/// The mapping is intentionally small and conservative — adding a new
/// extension here silently changes File Manager's Confirm behaviour.
pub fn app_for_file(path: &str) -> Option<&'static str> {
    let lower = path.rsplit('/').next().unwrap_or(path).to_lowercase();
    let ext = lower.rsplit('.').next()?;
    match ext {
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" => Some("Photo Viewer"),
        "mp3" | "wav" | "ogg" | "flac" | "m4a" => Some("Music Player"),
        "txt" | "md" | "rs" | "toml" | "json" | "yaml" | "yml" | "html" | "htm" | "css" | "js"
        | "sh" | "py" | "c" | "cpp" | "h" | "hpp" | "log" | "ini" | "conf" => Some("Text Editor"),
        _ => None,
    }
}

/// Generic file viewer: text content or hex dump.
pub fn view_generic_file(path: &str, data: &[u8]) -> Vec<String> {
    let filename = path.rsplit('/').next().unwrap_or(path);
    let mut lines = vec![format!("--- {filename} ---"), String::new()];

    let is_text = data.len() < 64 * 1024 && std::str::from_utf8(data).is_ok();
    if is_text {
        let text = String::from_utf8_lossy(data);
        for line in text.lines() {
            lines.push(line.to_string());
        }
        if data.is_empty() {
            lines.push("(empty file)".to_string());
        }
    } else {
        lines.push(format!("Binary file  ({} bytes)", data.len()));
        lines.push(String::new());
        for (i, chunk) in data.chunks(16).enumerate().take(8) {
            let hex: Vec<String> = chunk.iter().map(|b| format!("{b:02x}")).collect();
            let ascii: String = chunk
                .iter()
                .map(|&b| {
                    if (0x20..=0x7e).contains(&b) {
                        b as char
                    } else {
                        '.'
                    }
                })
                .collect();
            lines.push(format!("{:04x}  {:<48}  {ascii}", i * 16, hex.join(" ")));
        }
        if data.len() > 128 {
            lines.push(format!("... ({} more bytes)", data.len() - 128));
        }
    }

    lines.push(String::new());
    lines.push("Cancel=back".to_string());
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- parent_dir --

    #[test]
    fn parent_dir_root() {
        assert_eq!(parent_dir("/"), "/");
    }

    #[test]
    fn parent_dir_top_level() {
        assert_eq!(parent_dir("/home"), "/");
    }

    #[test]
    fn parent_dir_nested() {
        assert_eq!(parent_dir("/home/user/docs"), "/home/user");
    }

    #[test]
    fn parent_dir_trailing_slash() {
        // Trailing slash is stripped first, so "/home/user/" -> parent of "/home/user" = "/home"
        assert_eq!(parent_dir("/home/user/"), "/home");
    }

    #[test]
    fn parent_dir_deep_path() {
        assert_eq!(parent_dir("/a/b/c/d/e"), "/a/b/c/d");
    }

    // -- join_path --

    #[test]
    fn join_path_root() {
        assert_eq!(join_path("/", "file.txt"), "/file.txt");
    }

    #[test]
    fn join_path_subdir() {
        assert_eq!(join_path("/home/user", "docs"), "/home/user/docs");
    }

    #[test]
    fn join_path_no_double_slash() {
        let result = join_path("/", "test");
        assert!(!result.starts_with("//"));
    }

    // -- view_audio_file (WAV) --

    #[test]
    fn view_audio_wav() {
        // Minimal WAV header: RIFF....WAVEfmt + data
        let mut wav = vec![0u8; 44];
        wav[..4].copy_from_slice(b"RIFF");
        wav[8..12].copy_from_slice(b"WAVE");
        // channels = 2
        wav[22] = 2;
        wav[23] = 0;
        // sample rate = 44100 (0xAC44)
        wav[24..28].copy_from_slice(&44100u32.to_le_bytes());
        // bits per sample = 16
        wav[34] = 16;
        wav[35] = 0;
        // data size = 176400 (1 second of stereo 16-bit 44100Hz)
        wav[40..44].copy_from_slice(&176400u32.to_le_bytes());

        let lines = view_audio_file("/music/song.wav", &wav);
        assert!(lines.iter().any(|l| l.contains("WAV")));
        assert!(lines.iter().any(|l| l.contains("44100")));
        assert!(lines.iter().any(|l| l.contains("16-bit")));
        assert!(lines.iter().any(|l| l.contains("song.wav")));
    }

    // -- view_audio_file (MP3 with sync bytes) --

    #[test]
    fn view_audio_mp3_sync() {
        let mp3 = vec![0xFF, 0xFB, 0x90, 0x00]; // MP3 frame sync
        let lines = view_audio_file("/music/track.mp3", &mp3);
        assert!(lines.iter().any(|l| l.contains("MP3")));
        assert!(lines.iter().any(|l| l.contains("track.mp3")));
    }

    // -- view_audio_file (MP3 with ID3v2 tag) --

    #[test]
    fn view_audio_mp3_id3() {
        let mut data = Vec::new();
        // ID3v2 header
        data.extend_from_slice(b"ID3");
        data.push(3); // version
        data.push(0); // revision
        data.push(0); // flags
        // Tag size (syncsafe): encode 30 as syncsafe
        // 30 = 0b0011110 -> syncsafe [0, 0, 0, 30]
        data.extend_from_slice(&[0, 0, 0, 30]);
        // TIT2 frame: "TestTitle"
        data.extend_from_slice(b"TIT2");
        let title_bytes = b"\x03TestTitle"; // encoding byte + text
        data.extend_from_slice(&(title_bytes.len() as u32).to_be_bytes());
        data.extend_from_slice(&[0, 0]); // flags
        data.extend_from_slice(title_bytes);
        // Pad to reach header_size
        while data.len() < 40 {
            data.push(0);
        }

        let lines = view_audio_file("/music/id3.mp3", &data);
        assert!(lines.iter().any(|l| l.contains("MP3")));
        assert!(lines.iter().any(|l| l.contains("TestTitle")));
    }

    // -- view_audio_file (unknown format) --

    #[test]
    fn view_audio_unknown_format() {
        let data = vec![0x00, 0x01, 0x02, 0x03];
        let lines = view_audio_file("/music/sound.ogg", &data);
        assert!(lines.iter().any(|l| l.contains("ogg")));
    }

    // -- view_image_file (PNG) --

    #[test]
    fn view_image_png() {
        let mut png = vec![0u8; 30];
        png[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        // Width = 1920 at offset 16
        png[16..20].copy_from_slice(&1920u32.to_be_bytes());
        // Height = 1080 at offset 20
        png[20..24].copy_from_slice(&1080u32.to_be_bytes());
        png[24] = 8; // bit depth
        png[25] = 6; // color type: RGBA

        let lines = view_image_file("/photos/img.png", &png);
        assert!(lines.iter().any(|l| l.contains("PNG")));
        assert!(lines.iter().any(|l| l.contains("1920 x 1080")));
        assert!(lines.iter().any(|l| l.contains("RGBA")));
    }

    // -- view_image_file (JPEG) --

    #[test]
    fn view_image_jpeg() {
        // Minimal JPEG with SOF0 marker
        let mut jpg = vec![0xFF, 0xD8]; // SOI
        // APP0 segment (minimal)
        jpg.extend_from_slice(&[0xFF, 0xE0, 0x00, 0x02]);
        // SOF0 marker
        jpg.extend_from_slice(&[0xFF, 0xC0]);
        jpg.extend_from_slice(&[0x00, 0x0B]); // segment length
        jpg.push(8); // precision
        jpg.extend_from_slice(&640u16.to_be_bytes()); // height
        jpg.extend_from_slice(&480u16.to_be_bytes()); // width
        jpg.push(3); // components
        // Enough padding
        jpg.extend_from_slice(&[0; 10]);

        let lines = view_image_file("/photos/pic.jpg", &jpg);
        assert!(lines.iter().any(|l| l.contains("JPEG")));
        assert!(lines.iter().any(|l| l.contains("480 x 640")));
    }

    // -- view_image_file (GIF) --

    #[test]
    fn view_image_gif() {
        let mut gif = vec![0u8; 12];
        gif[..4].copy_from_slice(b"GIF8");
        // Width = 320 at offset 6 (little-endian)
        gif[6..8].copy_from_slice(&320u16.to_le_bytes());
        // Height = 200 at offset 8
        gif[8..10].copy_from_slice(&200u16.to_le_bytes());

        let lines = view_image_file("/img/anim.gif", &gif);
        assert!(lines.iter().any(|l| l.contains("GIF")));
        assert!(lines.iter().any(|l| l.contains("320 x 200")));
    }

    // -- view_image_file (WebP) --

    #[test]
    fn view_image_webp() {
        let mut webp = vec![0u8; 16];
        webp[..4].copy_from_slice(b"RIFF");
        webp[8..12].copy_from_slice(b"WEBP");

        let lines = view_image_file("/img/photo.webp", &webp);
        assert!(lines.iter().any(|l| l.contains("WebP")));
    }

    // -- view_image_file (unknown) --

    #[test]
    fn view_image_unknown() {
        let data = vec![0x00, 0x01];
        let lines = view_image_file("/img/photo.tiff", &data);
        assert!(lines.iter().any(|l| l.contains("tiff")));
    }

    // -- view_generic_file (text) --

    #[test]
    fn view_generic_text_file() {
        let data = b"Hello\nWorld\n";
        let lines = view_generic_file("/docs/readme.txt", data);
        assert!(lines.iter().any(|l| l.contains("readme.txt")));
        assert!(lines.iter().any(|l| l == "Hello"));
        assert!(lines.iter().any(|l| l == "World"));
    }

    #[test]
    fn view_generic_empty_file() {
        let lines = view_generic_file("/docs/empty.txt", b"");
        assert!(lines.iter().any(|l| l.contains("empty file")));
    }

    // -- view_generic_file (binary) --

    #[test]
    fn view_generic_binary_file() {
        let data: Vec<u8> = (0..200).collect();
        let lines = view_generic_file("/bin/program", &data);
        assert!(lines.iter().any(|l| l.contains("Binary file")));
        assert!(lines.iter().any(|l| l.contains("200 bytes")));
        // Should have hex dump lines
        assert!(lines.iter().any(|l| l.starts_with("0000")));
        // Should show "more bytes" for data > 128
        assert!(lines.iter().any(|l| l.contains("more bytes")));
    }

    // -- parse_jpeg_dimensions --

    #[test]
    fn jpeg_dimensions_no_sof() {
        // Just SOI + SOS (no SOF marker)
        let data = vec![0xFF, 0xD8, 0xFF, 0xDA];
        assert_eq!(parse_jpeg_dimensions(&data), (0, 0));
    }

    #[test]
    fn jpeg_dimensions_truncated() {
        let data = vec![0xFF, 0xD8];
        assert_eq!(parse_jpeg_dimensions(&data), (0, 0));
    }

    // -- list_directory --

    #[test]
    fn list_directory_root_no_dotdot() {
        let vfs = oasis_vfs::MemoryVfs::new();
        let lines = list_directory(&vfs, "/");
        // Root should NOT have ".." entry
        assert!(!lines.iter().any(|l| l == ".."));
    }

    #[test]
    fn list_directory_subdir_has_dotdot() {
        let vfs = oasis_vfs::MemoryVfs::new();
        let lines = list_directory(&vfs, "/home");
        // Non-root should have ".." entry
        assert!(lines.first().map(|l| l.as_str()) == Some(".."));
    }
}
