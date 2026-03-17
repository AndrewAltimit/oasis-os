//! GPU texture atlas for packing small browser images.
//!
//! Pages with many small images (icons, avatars) create excessive GPU
//! texture bind switches. This module packs images <= 128x128 into a
//! shared atlas texture, reducing bind calls during rendering.
//!
//! The atlas uses simple row-based packing: images are placed
//! left-to-right in rows, advancing to the next row when the current
//! one runs out of space. Row height equals the tallest image in that
//! row.

use std::collections::HashMap;

use oasis_types::backend::{SdiBackend, TextureId};

/// Maximum image dimension (width or height) eligible for atlas packing.
pub const MAX_ATLAS_IMAGE_SIZE: u32 = 128;

/// Default atlas texture size (width and height).
const ATLAS_SIZE: u32 = 2048;

/// A sub-region within an atlas texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AtlasRegion {
    /// X offset within the atlas texture.
    pub x: u32,
    /// Y offset within the atlas texture.
    pub y: u32,
    /// Width of the sub-image.
    pub w: u32,
    /// Height of the sub-image.
    pub h: u32,
}

/// A single atlas page backed by one GPU texture.
struct AtlasPage {
    /// GPU texture ID for this page (None until uploaded).
    texture: Option<TextureId>,
    /// Atlas width in pixels.
    width: u32,
    /// Atlas height in pixels.
    height: u32,
    /// Current X cursor (next free column in the current row).
    cursor_x: u32,
    /// Current Y cursor (top of the current row).
    cursor_y: u32,
    /// Height of the tallest image in the current row.
    row_height: u32,
    /// Raw RGBA pixel buffer (width * height * 4 bytes).
    pixels: Vec<u8>,
    /// Whether the pixel buffer has been modified since the last upload.
    dirty: bool,
}

impl AtlasPage {
    fn new(width: u32, height: u32) -> Self {
        let size = (width as usize) * (height as usize) * 4;
        Self {
            texture: None,
            width,
            height,
            cursor_x: 0,
            cursor_y: 0,
            row_height: 0,
            pixels: vec![0; size],
            dirty: false,
        }
    }

    /// Try to insert an image into this page. Returns the region on
    /// success, or `None` if there is not enough space.
    fn try_insert(&mut self, img_w: u32, img_h: u32, pixels: &[u8]) -> Option<AtlasRegion> {
        if img_w == 0 || img_h == 0 {
            return None;
        }

        // Check if the image fits in the current row.
        if self.cursor_x + img_w > self.width {
            // Move to the next row.
            self.cursor_y += self.row_height;
            self.cursor_x = 0;
            self.row_height = 0;
        }

        // Check if there is vertical space for this image.
        if self.cursor_y + img_h > self.height {
            return None;
        }

        let region = AtlasRegion {
            x: self.cursor_x,
            y: self.cursor_y,
            w: img_w,
            h: img_h,
        };

        // Blit the image pixels into the atlas buffer.
        let atlas_stride = self.width as usize * 4;
        let img_stride = img_w as usize * 4;
        for row in 0..img_h as usize {
            let src_start = row * img_stride;
            let src_end = src_start + img_stride;
            if src_end > pixels.len() {
                break;
            }
            let dst_start =
                (self.cursor_y as usize + row) * atlas_stride + self.cursor_x as usize * 4;
            let dst_end = dst_start + img_stride;
            if dst_end > self.pixels.len() {
                break;
            }
            self.pixels[dst_start..dst_end].copy_from_slice(&pixels[src_start..src_end]);
        }

        self.cursor_x += img_w;
        if img_h > self.row_height {
            self.row_height = img_h;
        }
        self.dirty = true;

        Some(region)
    }

    /// Upload or re-upload the atlas texture to the GPU.
    fn upload(&mut self, backend: &mut dyn SdiBackend) -> Option<TextureId> {
        if !self.dirty && self.texture.is_some() {
            return self.texture;
        }

        // Destroy old texture before re-uploading.
        if let Some(old) = self.texture.take() {
            let _ = backend.destroy_texture(old);
        }

        match backend.load_texture(self.width, self.height, &self.pixels) {
            Ok(tex) => {
                self.texture = Some(tex);
                self.dirty = false;
                Some(tex)
            },
            Err(_) => None,
        }
    }
}

/// Manages one or more atlas pages for packing small images.
pub struct ImageAtlas {
    /// Atlas pages (each backed by a single GPU texture).
    pages: Vec<AtlasPage>,
    /// Map from image URL to (page index, region within that page).
    entries: HashMap<String, (usize, AtlasRegion)>,
    /// Atlas page dimensions.
    atlas_size: u32,
}

impl ImageAtlas {
    /// Create a new empty atlas.
    pub fn new() -> Self {
        Self {
            pages: Vec::new(),
            entries: HashMap::new(),
            atlas_size: ATLAS_SIZE,
        }
    }

    /// Check whether an image with the given dimensions is eligible for
    /// atlas packing.
    pub fn is_eligible(width: u32, height: u32) -> bool {
        width > 0 && height > 0 && width <= MAX_ATLAS_IMAGE_SIZE && height <= MAX_ATLAS_IMAGE_SIZE
    }

    /// Look up a previously inserted image by URL.
    pub fn get(&self, url: &str) -> Option<(TextureId, AtlasRegion)> {
        let (page_idx, region) = self.entries.get(url)?;
        let page = self.pages.get(*page_idx)?;
        let tex = page.texture?;
        Some((tex, *region))
    }

    /// Returns true if the given URL is already in the atlas.
    pub fn contains(&self, url: &str) -> bool {
        self.entries.contains_key(url)
    }

    /// Insert a small image into the atlas.
    ///
    /// Returns the atlas region on success. The caller must call
    /// [`upload_dirty`] afterward to ensure the GPU texture is
    /// up-to-date.
    pub fn insert(
        &mut self,
        url: &str,
        img_w: u32,
        img_h: u32,
        pixels: &[u8],
    ) -> Option<AtlasRegion> {
        if !Self::is_eligible(img_w, img_h) {
            return None;
        }
        if self.entries.contains_key(url) {
            return self.entries.get(url).map(|(_, r)| *r);
        }

        // Try existing pages first.
        for (idx, page) in self.pages.iter_mut().enumerate() {
            if let Some(region) = page.try_insert(img_w, img_h, pixels) {
                self.entries.insert(url.to_string(), (idx, region));
                return Some(region);
            }
        }

        // Create a new page.
        let mut page = AtlasPage::new(self.atlas_size, self.atlas_size);
        let region = page.try_insert(img_w, img_h, pixels)?;
        let idx = self.pages.len();
        self.pages.push(page);
        self.entries.insert(url.to_string(), (idx, region));
        Some(region)
    }

    /// Upload any dirty atlas pages to the GPU.
    pub fn upload_dirty(&mut self, backend: &mut dyn SdiBackend) {
        for page in &mut self.pages {
            page.upload(backend);
        }
    }

    /// Clear all atlas data and destroy GPU textures.
    #[allow(dead_code)]
    pub fn clear(&mut self, backend: &mut dyn SdiBackend) {
        for page in &mut self.pages {
            if let Some(tex) = page.texture.take() {
                let _ = backend.destroy_texture(tex);
            }
        }
        self.pages.clear();
        self.entries.clear();
    }

    /// Clear all atlas data without destroying GPU textures.
    ///
    /// Used during navigation resets where the backend may have already
    /// invalidated the textures.
    pub fn clear_without_destroy(&mut self) {
        self.pages.clear();
        self.entries.clear();
    }

    /// Number of images currently packed in the atlas.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the atlas is empty.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for ImageAtlas {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eligibility_check() {
        assert!(ImageAtlas::is_eligible(64, 64));
        assert!(ImageAtlas::is_eligible(128, 128));
        assert!(!ImageAtlas::is_eligible(129, 64));
        assert!(!ImageAtlas::is_eligible(64, 129));
        assert!(!ImageAtlas::is_eligible(0, 64));
        assert!(!ImageAtlas::is_eligible(64, 0));
    }

    #[test]
    fn insert_and_lookup() {
        let mut atlas = ImageAtlas::new();
        let pixels = vec![255u8; 4 * 32 * 32];
        let region = atlas.insert("img1", 32, 32, &pixels);
        assert!(region.is_some());
        let r = region.unwrap();
        assert_eq!(r.x, 0);
        assert_eq!(r.y, 0);
        assert_eq!(r.w, 32);
        assert_eq!(r.h, 32);
        assert!(atlas.contains("img1"));
        assert!(!atlas.contains("img2"));
    }

    #[test]
    fn row_packing_advances_cursor() {
        let mut atlas = ImageAtlas::new();
        let pixels_a = vec![255u8; 4 * 64 * 32];
        let pixels_b = vec![128u8; 4 * 64 * 48];

        let ra = atlas.insert("a", 64, 32, &pixels_a).unwrap();
        let rb = atlas.insert("b", 64, 48, &pixels_b).unwrap();

        assert_eq!(ra.x, 0);
        assert_eq!(ra.y, 0);
        assert_eq!(rb.x, 64);
        assert_eq!(rb.y, 0);
    }

    #[test]
    fn row_wrap_when_full() {
        let mut atlas = ImageAtlas::new();
        // Fill most of a row with 128px-wide images in a 2048-wide atlas.
        // 2048 / 128 = 16 images per row.
        let pixels = vec![255u8; 4 * 128 * 64];
        for i in 0..16 {
            let url = format!("img{i}");
            let r = atlas.insert(&url, 128, 64, &pixels).unwrap();
            assert_eq!(r.x, (i as u32) * 128);
            assert_eq!(r.y, 0);
        }

        // 17th image should wrap to the next row.
        let r = atlas.insert("img16", 128, 64, &pixels).unwrap();
        assert_eq!(r.x, 0);
        assert_eq!(r.y, 64); // row_height of first row was 64
    }

    #[test]
    fn duplicate_insert_returns_same_region() {
        let mut atlas = ImageAtlas::new();
        let pixels = vec![255u8; 4 * 32 * 32];
        let r1 = atlas.insert("img1", 32, 32, &pixels).unwrap();
        let r2 = atlas.insert("img1", 32, 32, &pixels).unwrap();
        assert_eq!(r1, r2);
        assert_eq!(atlas.len(), 1);
    }

    #[test]
    fn clear_without_destroy_resets() {
        let mut atlas = ImageAtlas::new();
        let pixels = vec![255u8; 4 * 32 * 32];
        atlas.insert("img1", 32, 32, &pixels);
        assert!(!atlas.is_empty());
        atlas.clear_without_destroy();
        assert!(atlas.is_empty());
        assert!(!atlas.contains("img1"));
    }

    #[test]
    fn oversized_image_rejected() {
        let mut atlas = ImageAtlas::new();
        let pixels = vec![255u8; 4 * 256 * 256];
        assert!(atlas.insert("big", 256, 256, &pixels).is_none());
    }
}
