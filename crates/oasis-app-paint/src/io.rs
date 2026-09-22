//! File I/O for Paint: BMP decoding, unique save paths and the picture
//! folder listing used by the Open picker.
//!
//! Encoding lives next to the canvas in `canvas::encode_bmp`.

use oasis_types::backend::Color;
use oasis_vfs::{EntryKind, Vfs};

/// Folder Paint saves into and lists for File > Open.
pub const PICTURES_DIR: &str = "/home/user/pictures";

/// Largest image side Paint will open (matches `PaintApp::new_canvas`).
pub(crate) const MAX_SIDE: u32 = 256;

/// Create `dir` and all missing ancestors.
pub(crate) fn ensure_dir(vfs: &mut dyn Vfs, dir: &str) -> Result<(), String> {
    let mut cur = String::new();
    for part in dir.split('/').filter(|p| !p.is_empty()) {
        cur.push('/');
        cur.push_str(part);
        if !vfs.exists(&cur) {
            vfs.mkdir(&cur).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// First free `PICTURES_DIR/paint_NNN.bmp` path (never overwrites).
pub(crate) fn unique_save_path(vfs: &dyn Vfs) -> String {
    let mut n = 1u32;
    loop {
        let path = format!("{PICTURES_DIR}/paint_{n:03}.bmp");
        if !vfs.exists(&path) {
            return path;
        }
        n += 1;
    }
}

/// Sorted absolute paths of the `.bmp` files in [`PICTURES_DIR`].
pub(crate) fn list_bmps(vfs: &dyn Vfs) -> Vec<String> {
    let Ok(entries) = vfs.readdir(PICTURES_DIR) else {
        return Vec::new();
    };
    let mut out: Vec<String> = entries
        .into_iter()
        .filter(|e| e.kind == EntryKind::File && e.name.to_ascii_lowercase().ends_with(".bmp"))
        .map(|e| format!("{PICTURES_DIR}/{}", e.name))
        .collect();
    out.sort();
    out
}

fn u16_at(d: &[u8], off: usize) -> Option<u16> {
    Some(u16::from_le_bytes(d.get(off..off + 2)?.try_into().ok()?))
}

fn u32_at(d: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(off..off + 4)?.try_into().ok()?))
}

/// Decode an uncompressed 24- or 32-bit BMP (bottom-up or top-down).
///
/// Returns `(width, height, pixels)` in top-down row order. 32-bit files
/// whose alpha bytes are all zero (common for writers that ignore alpha)
/// are treated as fully opaque.
pub fn decode_bmp(data: &[u8]) -> Result<(u32, u32, Vec<Color>), String> {
    let bad = || "not a supported BMP file".to_string();
    if data.get(0..2) != Some(b"BM") {
        return Err(bad());
    }
    let pixel_off = u32_at(data, 10).ok_or_else(bad)? as usize;
    let width = u32_at(data, 18).ok_or_else(bad)? as i32;
    let height = u32_at(data, 22).ok_or_else(bad)? as i32;
    let bpp = u16_at(data, 28).ok_or_else(bad)?;
    let compression = u32_at(data, 30).ok_or_else(bad)?;
    // BI_RGB, or BI_BITFIELDS with the standard BGRA masks for 32 bpp.
    if !(bpp == 24 && compression == 0 || bpp == 32 && (compression == 0 || compression == 3)) {
        return Err(format!("unsupported BMP format ({bpp} bpp)"));
    }
    let top_down = height < 0;
    let (w, h) = (width.unsigned_abs(), height.unsigned_abs());
    if w == 0 || h == 0 {
        return Err(bad());
    }
    if w > MAX_SIDE || h > MAX_SIDE {
        return Err(format!(
            "image {w}x{h} is larger than {MAX_SIDE}x{MAX_SIDE}"
        ));
    }
    let bytes_pp = (bpp / 8) as usize;
    let stride = (w as usize * bytes_pp).div_ceil(4) * 4;
    // Checked: `pixel_off` comes straight from the file and could overflow
    // a 32-bit usize (WASM) when added.
    let needed = pixel_off.checked_add(stride * h as usize);
    if needed.is_none_or(|n| data.len() < n) {
        return Err("BMP file is truncated".to_string());
    }
    let mut pixels = Vec::with_capacity((w * h) as usize);
    let mut any_alpha = false;
    for row in 0..h as usize {
        let src_row = if top_down { row } else { h as usize - 1 - row };
        let base = pixel_off + src_row * stride;
        for x in 0..w as usize {
            let p = &data[base + x * bytes_pp..base + (x + 1) * bytes_pp];
            let a = if bytes_pp == 4 { p[3] } else { 255 };
            any_alpha |= a != 0;
            pixels.push(Color::rgba(p[2], p[1], p[0], a));
        }
    }
    if !any_alpha {
        for c in &mut pixels {
            c.a = 255;
        }
    }
    Ok((w, h, pixels))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::encode_bmp;
    use oasis_vfs::MemoryVfs;

    #[test]
    fn encode_decode_round_trip() {
        let px = vec![
            Color::rgb(255, 0, 0),
            Color::rgb(0, 255, 0),
            Color::rgb(0, 0, 255),
            Color::rgba(1, 2, 3, 128),
        ];
        let bmp = encode_bmp(&px, 2, 2);
        let (w, h, out) = decode_bmp(&bmp).expect("decode");
        assert_eq!((w, h), (2, 2));
        assert_eq!(out, px);
    }

    #[test]
    fn decode_24bit_padded_rows() {
        // 3x1 24-bit image: 9 data bytes padded to a 12-byte row.
        let mut bmp = vec![0u8; 54];
        bmp[0..2].copy_from_slice(b"BM");
        bmp[10..14].copy_from_slice(&54u32.to_le_bytes());
        bmp[14..18].copy_from_slice(&40u32.to_le_bytes());
        bmp[18..22].copy_from_slice(&3i32.to_le_bytes());
        bmp[22..26].copy_from_slice(&1i32.to_le_bytes());
        bmp[28..30].copy_from_slice(&24u16.to_le_bytes());
        bmp.extend_from_slice(&[0, 0, 255, 0, 255, 0, 255, 0, 0, 0, 0, 0]);
        let (w, h, out) = decode_bmp(&bmp).expect("decode");
        assert_eq!((w, h), (3, 1));
        assert_eq!(out[0], Color::rgb(255, 0, 0));
        assert_eq!(out[2], Color::rgb(0, 0, 255));
    }

    #[test]
    fn decode_rejects_garbage_and_truncation() {
        assert!(decode_bmp(b"nope").is_err());
        let bmp = encode_bmp(&[Color::rgb(1, 1, 1); 4], 2, 2);
        assert!(decode_bmp(&bmp[..bmp.len() - 1]).is_err());
    }

    #[test]
    fn unique_paths_and_listing() {
        let mut vfs = MemoryVfs::new();
        ensure_dir(&mut vfs, PICTURES_DIR).expect("mkdir");
        let first = unique_save_path(&vfs);
        assert_eq!(first, format!("{PICTURES_DIR}/paint_001.bmp"));
        vfs.write(&first, b"x").expect("write");
        assert_eq!(
            unique_save_path(&vfs),
            format!("{PICTURES_DIR}/paint_002.bmp")
        );
        vfs.write(&format!("{PICTURES_DIR}/notes.txt"), b"x")
            .expect("write");
        assert_eq!(list_bmps(&vfs), vec![first]);
    }
}
