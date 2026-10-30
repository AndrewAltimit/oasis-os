//! Audio file metadata for the Music Player: ID3v2 title, MP3 duration
//! estimate, duration formatting.

/// Extract a display title and duration string from an audio file.
/// Falls back to the filename stem if ID3 tags are not available.
pub(crate) fn parse_audio_metadata(path: &str, data: &[u8]) -> (String, Option<String>) {
    let filename = path.rsplit('/').next().unwrap_or(path);
    let fallback_title = filename
        .rsplit_once('.')
        .map_or(filename, |(stem, _)| stem)
        .replace(['_', '-'], " ");
    let title = id3v2_title(data).unwrap_or(fallback_title);
    let duration = mp3_duration(data).map(format_duration);
    (title, duration)
}

/// Small ID3v2 TIT2 (title) reader. Handles the two tag-size encodings
/// that real-world MP3s use:
///
/// * v2.3 (`data[3] == 3`): frame sizes are plain big-endian `u32`.
/// * v2.4 (`data[3] == 4`): frame sizes are 28-bit syncsafe integers
///   (same shape as the tag header size).
///
/// Returns `None` for older v2.2, unknown versions, or malformed data.
fn id3v2_title(data: &[u8]) -> Option<String> {
    if data.len() < 10 || &data[..3] != b"ID3" {
        return None;
    }
    let major = data[3];
    // We know how to parse v2.3 and v2.4 frames. v2.2 uses 3-byte
    // frame IDs with 3-byte sizes — different layout; skip it.
    if major != 3 && major != 4 {
        return None;
    }
    let size = ((data[6] as usize) << 21)
        | ((data[7] as usize) << 14)
        | ((data[8] as usize) << 7)
        | (data[9] as usize);
    let tag_end = 10 + size;
    if tag_end > data.len() {
        return None;
    }
    let tag = &data[10..tag_end];
    let mut i = 0;
    while i + 10 <= tag.len() {
        let frame_id = &tag[i..i + 4];
        if frame_id[0] == 0 {
            break;
        }
        let size_bytes = [tag[i + 4], tag[i + 5], tag[i + 6], tag[i + 7]];
        let frame_size = if major == 4 {
            syncsafe_to_usize(size_bytes)
        } else {
            u32::from_be_bytes(size_bytes) as usize
        };
        let body_start = i + 10;
        // Checked: a v2.3 size is a raw u32, which can wrap `usize` on
        // 32-bit targets (WASM) and move `body_end` backwards.
        let Some(body_end) = body_start.checked_add(frame_size) else {
            break;
        };
        if body_end > tag.len() {
            break;
        }
        if frame_id == b"TIT2" && frame_size >= 1 {
            let encoding = tag[body_start];
            let payload = &tag[body_start + 1..body_end];
            if encoding == 0 || encoding == 3 {
                let s = String::from_utf8_lossy(payload).into_owned();
                let trimmed = s.trim_end_matches('\0').trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
        }
        i = body_end;
    }
    None
}

/// Decode a 4-byte ID3 syncsafe integer (7 bits per byte).
fn syncsafe_to_usize(bytes: [u8; 4]) -> usize {
    ((bytes[0] as usize) << 21)
        | ((bytes[1] as usize) << 14)
        | ((bytes[2] as usize) << 7)
        | (bytes[3] as usize)
}

/// Estimate MP3 duration from first-frame bitrate + file size. Close
/// enough for a UI readout; we're not a metadata library.
fn mp3_duration(data: &[u8]) -> Option<u32> {
    let audio_start = if data.len() >= 10 && &data[..3] == b"ID3" {
        let size = ((data[6] as usize) << 21)
            | ((data[7] as usize) << 14)
            | ((data[8] as usize) << 7)
            | (data[9] as usize);
        10 + size
    } else {
        0
    };
    if audio_start >= data.len() {
        return None;
    }
    let audio = &data[audio_start..];
    let header_pos = audio
        .windows(2)
        .position(|w| w[0] == 0xFF && (w[1] & 0xE0) == 0xE0)?;
    let header = audio.get(header_pos..header_pos + 4)?;
    let bitrate_index = (header[2] >> 4) & 0x0F;
    const BITRATES: [u32; 16] = [
        0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0,
    ];
    let bitrate_kbps = BITRATES[bitrate_index as usize];
    if bitrate_kbps == 0 {
        return None;
    }
    let bits = (audio.len() as u64) * 8;
    Some((bits / (bitrate_kbps as u64 * 1000)) as u32)
}

fn format_duration(secs: u32) -> String {
    let m = secs / 60;
    let s = secs % 60;
    format!("{m:02}:{s:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id3v2_title_rejects_huge_frame_size() {
        // v2.3 tag, TIT2 frame claiming 0xFFFFFFFF bytes.
        let mut data = b"ID3\x03\x00\x00\x00\x00\x00\x14".to_vec();
        data.extend_from_slice(b"TIT2\xFF\xFF\xFF\xFF\x00\x00");
        data.extend_from_slice(&[0; 10]);
        assert_eq!(id3v2_title(&data), None);
    }

    /// Build a minimal ID3v2 tag containing one TIT2 frame.
    /// `major` is the version byte (3 for v2.3, 4 for v2.4).
    fn make_id3_tag(title: &str, major: u8) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.push(0u8); // encoding = ISO-8859-1
        payload.extend_from_slice(title.as_bytes());
        let frame_size = payload.len() as u32;

        let size_bytes: [u8; 4] = if major == 4 {
            [
                ((frame_size >> 21) & 0x7F) as u8,
                ((frame_size >> 14) & 0x7F) as u8,
                ((frame_size >> 7) & 0x7F) as u8,
                (frame_size & 0x7F) as u8,
            ]
        } else {
            frame_size.to_be_bytes()
        };

        let mut frame = Vec::new();
        frame.extend_from_slice(b"TIT2");
        frame.extend_from_slice(&size_bytes);
        frame.extend_from_slice(&[0u8, 0u8]); // flags
        frame.extend_from_slice(&payload);

        let tag_size = frame.len() as u32;
        // Tag size is always syncsafe (both v2.3 and v2.4).
        let tag_size_bytes = [
            ((tag_size >> 21) & 0x7F) as u8,
            ((tag_size >> 14) & 0x7F) as u8,
            ((tag_size >> 7) & 0x7F) as u8,
            (tag_size & 0x7F) as u8,
        ];

        let mut tag = Vec::new();
        tag.extend_from_slice(b"ID3");
        tag.push(major);
        tag.push(0); // minor
        tag.push(0); // flags
        tag.extend_from_slice(&tag_size_bytes);
        tag.extend_from_slice(&frame);
        tag
    }

    #[test]
    fn id3v2_3_title_parses() {
        let tag = make_id3_tag("Hello World", 3);
        assert_eq!(super::id3v2_title(&tag).as_deref(), Some("Hello World"));
    }

    #[test]
    fn id3v2_4_title_parses_syncsafe_frame_size() {
        // v2.4 uses syncsafe for frame sizes too — the old code read
        // them as plain big-endian and tripped `body_end > tag.len()`.
        let tag = make_id3_tag("Modern MP3", 4);
        assert_eq!(super::id3v2_title(&tag).as_deref(), Some("Modern MP3"));
    }

    #[test]
    fn id3v2_returns_none_for_unknown_version() {
        // v2.2 has a different frame layout we don't support.
        let mut tag = make_id3_tag("Old", 3);
        tag[3] = 2; // pretend v2.2
        assert!(super::id3v2_title(&tag).is_none());
    }
}
