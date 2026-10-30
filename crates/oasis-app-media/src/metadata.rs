//! Audio file metadata for the Music Player: display title (ID3v2 via
//! `oasis_app_core::id3`, else the file name), MP3 duration estimate.

/// Extract a display title and duration string from an audio file.
/// Falls back to the filename stem if ID3 tags are not available.
pub(crate) fn parse_audio_metadata(path: &str, data: &[u8]) -> (String, Option<String>) {
    let filename = path.rsplit('/').next().unwrap_or(path);
    let fallback_title = filename
        .rsplit_once('.')
        .map_or(filename, |(stem, _)| stem)
        .replace(['_', '-'], " ");
    let title = oasis_app_core::id3::read(data)
        .title
        .unwrap_or(fallback_title);
    let duration = mp3_duration(data).map(format_duration);
    (title, duration)
}

/// Estimate MP3 duration from first-frame bitrate + file size. Close
/// enough for a UI readout; we're not a metadata library.
fn mp3_duration(data: &[u8]) -> Option<u32> {
    let audio_start = oasis_app_core::id3::tag_len(data);
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
    fn huge_frame_size_falls_back_to_file_name() {
        // v2.3 tag, TIT2 frame claiming 0xFFFFFFFF bytes.
        let mut data = b"ID3\x03\x00\x00\x00\x00\x00\x14".to_vec();
        data.extend_from_slice(b"TIT2\xFF\xFF\xFF\xFF\x00\x00");
        data.extend_from_slice(&[0; 10]);
        assert_eq!(parse_audio_metadata("/m/my_song.mp3", &data).0, "my song");
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
        assert_eq!(parse_audio_metadata("/m/x.mp3", &tag).0, "Hello World");
    }

    #[test]
    fn id3v2_4_title_parses_syncsafe_frame_size() {
        // v2.4 uses syncsafe for frame sizes too — the old code read
        // them as plain big-endian and tripped `body_end > tag.len()`.
        let tag = make_id3_tag("Modern MP3", 4);
        assert_eq!(parse_audio_metadata("/m/x.mp3", &tag).0, "Modern MP3");
    }

    #[test]
    fn id3v2_returns_none_for_unknown_version() {
        // v2.2 has a different frame layout we don't support.
        let mut tag = make_id3_tag("Old", 3);
        tag[3] = 2; // pretend v2.2
        assert_eq!(parse_audio_metadata("/m/old.mp3", &tag).0, "old");
    }
}
