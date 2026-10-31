//! Minimal ID3v2 tag reader (title / artist) shared by the file viewer
//! and the Music Player.
//!
//! Handles v2.3 (plain big-endian frame sizes) and v2.4 (syncsafe frame
//! sizes) tags and the four ID3 text encodings. v2.2 (3-byte frame ids)
//! and anything malformed yield no tags. All size arithmetic is checked,
//! so hostile sizes can't wrap `usize` on 32-bit targets.

/// Text frames read from an ID3v2 tag.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Id3Tags {
    /// `TIT2` (title).
    pub title: Option<String>,
    /// `TPE1` (lead artist).
    pub artist: Option<String>,
}

/// Length of the ID3v2 tag at the start of `data` (header included), or
/// 0 when `data` has no tag. Where the audio frames start.
pub fn tag_len(data: &[u8]) -> usize {
    if data.len() < 10 || &data[..3] != b"ID3" {
        return 0;
    }
    // v2.4 flag 0x10: a 10-byte footer follows the tag body.
    let footer = if data[3] == 4 && data[5] & 0x10 != 0 {
        10
    } else {
        0
    };
    10 + body_len(data) + footer
}

/// Size of the tag body (frames, padding and any extended header): the
/// header's syncsafe size field, which excludes the header and footer.
fn body_len(data: &[u8]) -> usize {
    syncsafe([data[6], data[7], data[8], data[9]])
}

/// Bytes of the extended header at the start of `body` (flag 0x40), or 0.
/// Its size field counts itself in v2.4 (syncsafe) but not in v2.3.
fn ext_header_len(major: u8, flags: u8, body: &[u8]) -> usize {
    if flags & 0x40 == 0 || body.len() < 4 {
        return 0;
    }
    let b = [body[0], body[1], body[2], body[3]];
    if major == 4 {
        syncsafe(b)
    } else {
        (u32::from_be_bytes(b) as usize).saturating_add(4)
    }
}

/// Read the title / artist frames of the ID3v2 tag at the start of `data`.
pub fn read(data: &[u8]) -> Id3Tags {
    let mut tags = Id3Tags::default();
    if data.len() < 10 || &data[..3] != b"ID3" {
        return tags;
    }
    let major = data[3];
    if major != 3 && major != 4 {
        return tags;
    }
    let body = &data[10..(10 + body_len(data)).min(data.len())];
    let tag = body
        .get(ext_header_len(major, data[5], body)..)
        .unwrap_or(&[]);
    let mut i = 0;
    while i + 10 <= tag.len() {
        let id = &tag[i..i + 4];
        if id[0] == 0 {
            break; // padding
        }
        let size_bytes = [tag[i + 4], tag[i + 5], tag[i + 6], tag[i + 7]];
        let size = if major == 4 {
            syncsafe(size_bytes)
        } else {
            u32::from_be_bytes(size_bytes) as usize
        };
        let body_start = i + 10;
        let Some(body_end) = body_start.checked_add(size).filter(|&e| e <= tag.len()) else {
            break;
        };
        let slot = match id {
            b"TIT2" => Some(&mut tags.title),
            b"TPE1" => Some(&mut tags.artist),
            _ => None,
        };
        if let Some(slot) = slot
            && slot.is_none()
        {
            *slot = decode_text(&tag[body_start..body_end]);
        }
        if tags.title.is_some() && tags.artist.is_some() {
            break;
        }
        i = body_end;
    }
    tags
}

/// Decode a text frame body (encoding byte + text); `None` when empty.
fn decode_text(body: &[u8]) -> Option<String> {
    let (&encoding, text) = body.split_first()?;
    let s = match encoding {
        // ISO-8859-1: every byte is the code point.
        0 => text.iter().map(|&b| char::from(b)).collect(),
        // UTF-16 with BOM (1) / UTF-16BE without BOM (2).
        1 | 2 => {
            let (big_endian, text) = match text {
                [0xFE, 0xFF, rest @ ..] => (true, rest),
                [0xFF, 0xFE, rest @ ..] => (false, rest),
                _ => (encoding == 2, text),
            };
            let units = text.as_chunks::<2>().0.iter().map(|c| {
                if big_endian {
                    u16::from_be_bytes([c[0], c[1]])
                } else {
                    u16::from_le_bytes([c[0], c[1]])
                }
            });
            char::decode_utf16(units)
                .map(|r| r.unwrap_or(char::REPLACEMENT_CHARACTER))
                .collect()
        },
        // UTF-8.
        3 => String::from_utf8_lossy(text).into_owned(),
        _ => return None,
    };
    // NULs (terminators, padding) and whitespace in any order: `" \0 "`
    // is no title, not a NUL.
    let trimmed = s.trim_matches(|c: char| c == '\0' || c.is_whitespace());
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// Decode a 4-byte syncsafe integer (7 bits per byte).
fn syncsafe(b: [u8; 4]) -> usize {
    ((b[0] as usize & 0x7F) << 21)
        | ((b[1] as usize & 0x7F) << 14)
        | ((b[2] as usize & 0x7F) << 7)
        | (b[3] as usize & 0x7F)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tag with the given frames (`(id, encoding, text bytes)`).
    fn tag(major: u8, frames: &[(&[u8; 4], u8, &[u8])]) -> Vec<u8> {
        let mut body = Vec::new();
        for (id, enc, text) in frames {
            let size = (text.len() + 1) as u32;
            body.extend_from_slice(*id);
            if major == 4 {
                body.extend_from_slice(&[
                    ((size >> 21) & 0x7F) as u8,
                    ((size >> 14) & 0x7F) as u8,
                    ((size >> 7) & 0x7F) as u8,
                    (size & 0x7F) as u8,
                ]);
            } else {
                body.extend_from_slice(&size.to_be_bytes());
            }
            body.extend_from_slice(&[0, 0, *enc]);
            body.extend_from_slice(text);
        }
        let n = body.len();
        let mut out = vec![b'I', b'D', b'3', major, 0, 0];
        out.extend_from_slice(&[
            ((n >> 21) & 0x7F) as u8,
            ((n >> 14) & 0x7F) as u8,
            ((n >> 7) & 0x7F) as u8,
            (n & 0x7F) as u8,
        ]);
        out.extend_from_slice(&body);
        out
    }

    #[test]
    fn reads_title_and_artist_v23_and_v24() {
        for major in [3, 4] {
            let data = tag(major, &[(b"TIT2", 3, b"Song"), (b"TPE1", 0, b"Band")]);
            let t = read(&data);
            assert_eq!(t.title.as_deref(), Some("Song"), "v2.{major}");
            assert_eq!(t.artist.as_deref(), Some("Band"), "v2.{major}");
        }
    }

    #[test]
    fn decodes_utf16_and_latin1() {
        let utf16le = [0xFF, 0xFE, b'H', 0, b'i', 0];
        let data = tag(3, &[(b"TIT2", 1, &utf16le), (b"TPE1", 0, &[0xE9])]);
        let t = read(&data);
        assert_eq!(t.title.as_deref(), Some("Hi"));
        assert_eq!(t.artist.as_deref(), Some("\u{e9}"));
    }

    #[test]
    fn rejects_huge_frame_size_and_unknown_versions() {
        let mut data = b"ID3\x03\x00\x00\x00\x00\x00\x14".to_vec();
        data.extend_from_slice(b"TIT2\xFF\xFF\xFF\xFF\x00\x00");
        data.extend_from_slice(&[0; 10]);
        assert_eq!(read(&data), Id3Tags::default());
        let v22 = tag(2, &[(b"TIT2", 3, b"x")]);
        assert_eq!(read(&v22), Id3Tags::default());
        assert_eq!(read(b"not a tag"), Id3Tags::default());
    }

    #[test]
    fn tag_len_covers_header_and_body() {
        let data = tag(3, &[(b"TIT2", 3, b"abc")]);
        assert_eq!(tag_len(&data), data.len());
        assert_eq!(tag_len(b"\xFF\xFB\x90"), 0);
    }

    #[test]
    fn tag_len_includes_v24_footer() {
        let mut data = tag(4, &[(b"TIT2", 3, b"abc")]);
        let body_end = data.len();
        data[5] |= 0x10;
        data.extend_from_slice(b"3DI\x04\x00\x10\x00\x00\x00\x00");
        assert_eq!(tag_len(&data), body_end + 10);
        // The footer is not parsed as a frame.
        assert_eq!(read(&data).title.as_deref(), Some("abc"));
    }

    /// Tags come from user files: every truncation and every single-byte
    /// corruption of a valid tag must parse (to something) without
    /// panicking, and `tag_len` must never claim less than the header.
    #[test]
    fn truncated_and_corrupted_tags_never_panic() {
        let mut ext = tag(
            4,
            &[(b"TIT2", 1, &[0xFE, 0xFF, 0, b'A']), (b"TPE1", 2, b"\0B")],
        );
        ext[5] |= 0x50; // extended header + footer flags on a body without them
        for data in [
            tag(3, &[(b"TIT2", 3, b"Song"), (b"TPE1", 0, b"Band")]),
            tag(4, &[(b"TALB", 3, b"x"), (b"TIT2", 3, b"Song")]),
            ext,
        ] {
            for end in 0..=data.len() {
                let prefix = &data[..end];
                let _ = read(prefix);
                let len = tag_len(prefix);
                assert!(
                    len == 0 || len >= 10,
                    "tag_len {len} of a {end}-byte prefix"
                );
            }
            for at in 0..data.len() {
                for byte in [0x00, 0x7F, 0x80, 0xFF] {
                    let mut bad = data.clone();
                    bad[at] = byte;
                    let _ = read(&bad);
                    let _ = tag_len(&bad);
                }
            }
        }
    }

    #[test]
    fn odd_encodings_and_padding() {
        // Unknown encoding byte: no text. Odd UTF-16 byte counts drop the
        // trailing half unit; unpaired surrogates become U+FFFD.
        let data = tag(
            3,
            &[(b"TIT2", 9, b"nope"), (b"TPE1", 2, &[0, b'O', 0, b'K', 0])],
        );
        let t = read(&data);
        assert_eq!(t.title, None);
        assert_eq!(t.artist.as_deref(), Some("OK"));
        let data = tag(3, &[(b"TIT2", 1, &[0xFF, 0xFE, 0x00, 0xD8])]);
        assert_eq!(read(&data).title.as_deref(), Some("\u{FFFD}"));
        // Whitespace / NUL-only text is no title.
        let data = tag(3, &[(b"TIT2", 0, b" \0 ")]);
        assert_eq!(read(&data).title, None);
        // The first TIT2 wins over a later duplicate.
        let data = tag(3, &[(b"TIT2", 3, b"first"), (b"TIT2", 3, b"second")]);
        assert_eq!(read(&data).title.as_deref(), Some("first"));
    }

    #[test]
    fn skips_extended_header() {
        for major in [3u8, 4] {
            let plain = tag(major, &[(b"TIT2", 3, b"Song")]);
            // v2.3: 4-byte size excluding itself (6), then 6 bytes.
            // v2.4: syncsafe size including itself (6), then 2 bytes.
            let ext: &[u8] = if major == 4 {
                &[0, 0, 0, 6, 1, 0]
            } else {
                &[0, 0, 0, 6, 0, 0, 0, 0, 0, 0]
            };
            let mut data = plain[..10].to_vec();
            data[5] |= 0x40;
            let body_len = plain.len() - 10 + ext.len();
            data[6..10].copy_from_slice(&[0, 0, (body_len >> 7) as u8, (body_len & 0x7F) as u8]);
            data.extend_from_slice(ext);
            data.extend_from_slice(&plain[10..]);
            assert_eq!(read(&data).title.as_deref(), Some("Song"), "v2.{major}");
            assert_eq!(tag_len(&data), data.len(), "v2.{major}");
        }
    }
}
