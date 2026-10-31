//! GU rendering primitives: vertices, clear, fill, text, blit.

use std::ffi::c_void;
use std::mem::size_of;
use std::ptr;

use psp::gu_ext::SpriteVertex;
use psp::sys::{
    self, ClearBuffer, GuPrimitive, MipmapLevel, TextureColorComponent, TextureEffect,
    TextureFilter, TexturePixelFormat, VertexType,
};

use oasis_core::backend::Color;

use crate::gu_mem::{self, coord};
use crate::{PspBackend, SCREEN_WIDTH};

// ---------------------------------------------------------------------------
// Vertex types for 2D GU rendering
// ---------------------------------------------------------------------------

/// Textured + colored vertex for blit, draw_text, and fill_rect.
#[repr(C, align(4))]
pub(crate) struct TexturedColorVertex {
    u: i16,
    v: i16,
    color: u32,
    x: i16,
    y: i16,
    z: i16,
    _pad: i16,
}

/// Vertex type flags for TexturedColorVertex.
pub(crate) const TEXTURED_COLOR_VTYPE: VertexType = VertexType::from_bits_truncate(
    VertexType::TEXTURE_16BIT.bits()
        | VertexType::COLOR_8888.bits()
        | VertexType::VERTEX_16BIT.bits()
        | VertexType::TRANSFORM_2D.bits(),
);

// ---------------------------------------------------------------------------
// Font atlas constants
// ---------------------------------------------------------------------------

/// Font atlas dimensions.
pub const FONT_ATLAS_W: u32 = 128;
pub const FONT_ATLAS_H: u32 = 64;
/// Glyphs per row in the atlas.
const ATLAS_COLS: u32 = 16;

/// Fully opaque white, as a `0xAABBGGRR` color (used for untinted texture
/// sampling).
const COLOR_WHITE_ABGR: u32 = 0xFFFF_FFFF;

// ---------------------------------------------------------------------------
// PspBackend rendering methods
// ---------------------------------------------------------------------------

impl PspBackend {
    /// Build the 128x64 font atlas in a RAM buffer.
    ///
    /// 16 glyphs per row, 6 rows (95 glyphs for ASCII 32-126).
    /// Each glyph is 8x8. White where bit is set, transparent elsewhere.
    ///
    /// # Safety
    ///
    /// `buf` must point to a valid, 16-byte-aligned allocation of at least
    /// `FONT_ATLAS_W * FONT_ATLAS_H * 4` bytes.
    pub(crate) unsafe fn build_font_atlas(&self, buf: *mut u8) {
        let pixels = buf as *mut u32;
        let stride = FONT_ATLAS_W;
        let total = (FONT_ATLAS_W * FONT_ATLAS_H) as usize;

        // Zero the entire atlas first (deliberately a manual loop rather
        // than `write_bytes`).
        for i in 0..total {
            // SAFETY: i < total, the atlas size in u32s (caller contract).
            unsafe { pixels.add(i).write(0u32) };
        }

        // Write a solid white pixel at the bottom-right corner of the atlas.
        // Used by fill_rect_inner to draw colored rectangles without toggling
        // Texture2D state (sample white texel * vertex color = fill color).
        let white_offset = ((FONT_ATLAS_H - 1) * stride + (FONT_ATLAS_W - 1)) as usize;
        // SAFETY: the last pixel of the atlas.
        unsafe { pixels.add(white_offset).write(COLOR_WHITE_ABGR) };

        for idx in 0u32..95 {
            let col = idx % ATLAS_COLS;
            let row = idx / ATLAS_COLS;
            let glyph_data = oasis_core::bitmap_font::glyph((idx + 32) as u8 as char);

            for gy in 0..8u32 {
                let bits = glyph_data[gy as usize];
                for gx in 0..8u32 {
                    if bits & (0x80 >> gx) != 0 {
                        let px = col * 8 + gx;
                        let py = row * 8 + gy;
                        let offset = (py * stride + px) as usize;
                        // SAFETY: 95 glyphs in 16 columns x 6 rows of 8x8
                        // cells fit inside the 128x64 atlas.
                        unsafe { pixels.add(offset).write(COLOR_WHITE_ABGR) };
                    }
                }
            }
        }

        // The GE samples the atlas through the uncached mirror, so the
        // writes above must reach RAM (the white texel, written early, may
        // otherwise still sit in the data cache).
        // SAFETY: `buf` is the atlas allocation of `total` u32s (caller
        // contract).
        unsafe {
            psp::cache::dcache_writeback_range(buf as *const c_void, (total * 4) as u32);
        }
    }

    /// Clear the screen to a solid color.
    pub fn clear_inner(&mut self, color: Color) {
        // SAFETY: sceGuClearColor/sceGuClear are GU FFI calls that operate
        // on the current display list. Called within a valid GU frame.
        unsafe {
            sys::sceGuClearColor(color.to_abgr());
            sys::sceGuClear(ClearBuffer::COLOR_BUFFER_BIT | ClearBuffer::FAST_CLEAR_BIT);
        }
    }

    /// Draw a filled rectangle.
    ///
    /// Uses a 1x1 white texel instead of toggling Texture2D state, avoiding
    /// expensive GE state changes on every call.
    pub fn fill_rect_inner(&mut self, x: i32, y: i32, w: u32, h: u32, color: Color) {
        // SAFETY: gu_mem::alloc returns display-list memory for 2 vertices,
        // valid until the GE has run the frame. We write exactly 2 TexturedColorVertex
        // structs, sampling the solid white texel at the bottom-right corner
        // of the font atlas. Modulate texfunc: white * vertex_color = fill
        // color. This avoids toggling Texture2D state on every rectangle.
        unsafe {
            let Some(verts) = gu_mem::alloc::<TexturedColorVertex>(2, "fill_rect") else {
                return;
            };

            // Bind the font atlas; the white texel is at the bottom-right corner.
            // Validate that the atlas pointer is in cached KSEG0 range before
            // converting to uncached KSEG1 (ORing with 0x4000_0000).
            debug_assert!(
                !self.font_atlas_ptr.is_null(),
                "font_atlas_ptr must be initialized before rendering"
            );
            let uncached_atlas = psp::cache::UncachedPtr::from_cached_addr(self.font_atlas_ptr)
                .as_ptr() as *const c_void;
            sys::sceGuTexMode(TexturePixelFormat::Psm8888, 0, 0, 0);
            sys::sceGuTexImage(
                MipmapLevel::None,
                FONT_ATLAS_W as i32,
                FONT_ATLAS_H as i32,
                FONT_ATLAS_W as i32,
                uncached_atlas,
            );

            let abgr = color.to_abgr();
            let white_u = (FONT_ATLAS_W - 1) as i16;
            let white_v = (FONT_ATLAS_H - 1) as i16;

            ptr::write(
                verts,
                TexturedColorVertex {
                    u: white_u,
                    v: white_v,
                    color: abgr,
                    x: coord(x),
                    y: coord(y),
                    z: 0,
                    _pad: 0,
                },
            );
            ptr::write(
                verts.add(1),
                TexturedColorVertex {
                    u: white_u,
                    v: white_v,
                    color: abgr,
                    x: coord(x.saturating_add_unsigned(w)),
                    y: coord(y.saturating_add_unsigned(h)),
                    z: 0,
                    _pad: 0,
                },
            );

            sys::sceGuDrawArray(
                GuPrimitive::Sprites,
                TEXTURED_COLOR_VTYPE,
                2,
                ptr::null(),
                verts as *const c_void,
            );
        }
    }

    /// Draw text using system TrueType fonts (if available) or the 8x8
    /// bitmap font as fallback.
    pub fn draw_text_inner(&mut self, text: &str, x: i32, y: i32, font_size: u16, color: Color) {
        if text.is_empty() {
            return;
        }

        let abgr = color.to_abgr();

        // System font path: anti-aliased TrueType via VRAM glyph atlas.
        if !self.force_bitmap_font
            && let Some(sf) = &mut self.system_font
        {
            // Up to one sprite (2 vertices) per char, plus the CLUT and
            // texture state its flush emits.
            if !gu_mem::has_room(text.len() * 2 * size_of::<SpriteVertex>() + 512, "text") {
                return;
            }
            sf.draw_text(x as f32, y as f32, abgr, text);
            // SAFETY: Within an active GU display list (between
            // sceGuStart and sceGuFinish in the main frame loop).
            unsafe { sf.flush() };
            return;
        }

        // Bitmap font fallback: 8x8 glyphs via SpriteBatch.
        self.draw_text_bitmap(text, x, y, font_size, abgr);
    }

    /// Draw text using the embedded 8x8 bitmap font via the GU font atlas.
    ///
    /// Uses proportional glyph advances from `oasis_types::bitmap_font`
    /// so character spacing matches `bitmap_measure_text`. The atlas
    /// stores each glyph at a fixed 8x8 cell; advance widths vary per
    /// character (e.g. space = 4px, 'W' = 8px at base scale).
    fn draw_text_bitmap(&mut self, text: &str, x: i32, y: i32, font_size: u16, abgr: u32) {
        let scale = if font_size >= 8 {
            (font_size / 8) as f32
        } else {
            1.0
        };
        let glyph_h = 8.0 * scale;

        let mut batch = psp::gu_ext::SpriteBatch::new(text.len());

        let mut cx = x as f32;
        let mut sprites = 0usize;
        for ch in text.chars() {
            // Glyphs past the right edge are never visible (no render
            // targets on PSP); don't spend display-list space on them.
            if cx >= SCREEN_WIDTH as f32 {
                break;
            }
            let idx = (ch as u32).wrapping_sub(32);
            let (u0, v0) = if idx < 95 {
                let col = idx % ATLAS_COLS;
                let row = idx / ATLAS_COLS;
                ((col * 8) as f32, (row * 8) as f32)
            } else {
                (0.0, 0.0)
            };

            // Use proportional advance width so character spacing matches
            // `bitmap_measure_text`. This fixes space characters (advance=4)
            // rendering at full 8px width.
            let advance = oasis_core::bitmap_font::glyph_advance_scaled(ch, font_size) as f32;

            // Blit the full 8x8 atlas cell but only advance by the
            // proportional width. The atlas cell's rightmost columns are
            // blank for narrow glyphs, so the visual result is correct.
            let blit_w = (oasis_core::bitmap_font::GLYPH_WIDTH as f32) * scale;
            batch.draw_rect(
                cx,
                y as f32,
                blit_w,
                glyph_h,
                u0,
                v0,
                u0 + 8.0,
                v0 + 8.0,
                abgr,
            );
            sprites += 1;
            cx += advance;
        }
        if sprites == 0 || !gu_mem::has_room(sprites * 2 * size_of::<SpriteVertex>(), "text") {
            return;
        }

        // SAFETY: Binds the font atlas texture (RAM pointer via uncached
        // mirror) and flushes the batched sprites. font_atlas_ptr is
        // checked non-null during init(), its contents were written back to
        // RAM by build_font_atlas, and `has_room` above reserved display-list
        // space for the batch's vertices.
        unsafe {
            let uncached_atlas = psp::cache::UncachedPtr::from_cached_addr(self.font_atlas_ptr)
                .as_ptr() as *const c_void;
            sys::sceGuTexMode(TexturePixelFormat::Psm8888, 0, 0, 0);
            sys::sceGuTexImage(
                MipmapLevel::None,
                FONT_ATLAS_W as i32,
                FONT_ATLAS_H as i32,
                FONT_ATLAS_W as i32,
                uncached_atlas,
            );
            sys::sceGuTexFunc(TextureEffect::Modulate, TextureColorComponent::Rgba);

            batch.flush();
        }
    }

    /// Blit a loaded texture at the given position and size.
    pub fn blit_inner(
        &mut self,
        tex: oasis_core::backend::TextureId,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
    ) {
        let idx = tex.0 as usize;
        let Some(Some(texture)) = self.textures.get(idx) else {
            return;
        };
        let buf_w = texture.buf_w;
        let buf_h = texture.buf_h;
        let data_ptr = texture.data;

        // For video textures, use actual content dimensions for UV so
        // only video pixels are sampled (not stride padding).
        let is_video = self.video_tex == Some(tex);
        let tex_w = if is_video && self.video_content_w > 0 {
            self.video_content_w as i16
        } else {
            texture.width as i16
        };
        let tex_h = if is_video && self.video_content_h > 0 {
            self.video_content_h as i16
        } else {
            texture.height as i16
        };

        // SAFETY: Binds the texture (RAM pointer via uncached mirror) and
        // draws a Sprites primitive. data_ptr validity is ensured by
        // load_texture_inner (allocated, populated and written back to RAM
        // before insertion); `verts` holds the 2 vertices written below.
        unsafe {
            let Some(verts) = gu_mem::alloc::<TexturedColorVertex>(2, "blit") else {
                return;
            };
            let uncached_ptr =
                psp::cache::UncachedPtr::from_cached_addr(data_ptr).as_ptr() as *const c_void;
            sys::sceGuTexMode(TexturePixelFormat::Psm8888, 0, 0, 0);
            sys::sceGuTexImage(
                MipmapLevel::None,
                buf_w as i32,
                buf_h as i32,
                buf_w as i32,
                uncached_ptr,
            );
            // Video textures: use Rgb to ignore alpha (CSC outputs 0x00).
            let tcc = if is_video {
                TextureColorComponent::Rgb
            } else {
                TextureColorComponent::Rgba
            };
            sys::sceGuTexFunc(TextureEffect::Modulate, tcc);

            ptr::write(
                verts,
                TexturedColorVertex {
                    u: 0,
                    v: 0,
                    color: COLOR_WHITE_ABGR,
                    x: coord(x),
                    y: coord(y),
                    z: 0,
                    _pad: 0,
                },
            );
            ptr::write(
                verts.add(1),
                TexturedColorVertex {
                    u: tex_w,
                    v: tex_h,
                    color: COLOR_WHITE_ABGR,
                    x: coord(x.saturating_add_unsigned(w)),
                    y: coord(y.saturating_add_unsigned(h)),
                    z: 0,
                    _pad: 0,
                },
            );

            sys::sceGuDrawArray(
                GuPrimitive::Sprites,
                TEXTURED_COLOR_VTYPE,
                2,
                ptr::null(),
                verts as *const c_void,
            );
        }
    }

    /// Blit a texture scaled to the given size with bilinear filtering.
    ///
    /// Used for the wallpaper: a small texture (64x64) scaled to fullscreen.
    /// Bilinear filtering smooths the upscale. Filter state is restored to
    /// Nearest after the draw for subsequent pixel-art text/icons.
    pub fn blit_scaled(
        &mut self,
        tex: oasis_core::backend::TextureId,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
    ) {
        let idx = tex.0 as usize;
        let Some(Some(texture)) = self.textures.get(idx) else {
            return;
        };
        let tex_w = texture.width as i16;
        let tex_h = texture.height as i16;
        let buf_w = texture.buf_w;
        let buf_h = texture.buf_h;
        let data_ptr = texture.data;

        // SAFETY: Binds the texture (RAM pointer via uncached mirror) and
        // draws a Sprites primitive with bilinear filtering. data_ptr
        // validity is ensured by load_texture_inner; `verts` holds the 2
        // vertices written below. Filter state is restored to Nearest after
        // the draw.
        unsafe {
            let Some(verts) = gu_mem::alloc::<TexturedColorVertex>(2, "blit_scaled") else {
                return;
            };
            let uncached_ptr =
                psp::cache::UncachedPtr::from_cached_addr(data_ptr).as_ptr() as *const c_void;
            sys::sceGuTexMode(TexturePixelFormat::Psm8888, 0, 0, 0);
            sys::sceGuTexImage(
                MipmapLevel::None,
                buf_w as i32,
                buf_h as i32,
                buf_w as i32,
                uncached_ptr,
            );
            // Video textures skip alpha fixup — CSC outputs alpha=0x00.
            // Use Rgb color component so GU ignores texture alpha and
            // uses vertex alpha (0xFF) instead.
            let is_video = self.video_tex == Some(tex);
            let tcc = if is_video {
                TextureColorComponent::Rgb
            } else {
                TextureColorComponent::Rgba
            };
            sys::sceGuTexFunc(TextureEffect::Modulate, tcc);
            sys::sceGuTexFilter(TextureFilter::Linear, TextureFilter::Linear);

            ptr::write(
                verts,
                TexturedColorVertex {
                    u: 0,
                    v: 0,
                    color: COLOR_WHITE_ABGR,
                    x: coord(x),
                    y: coord(y),
                    z: 0,
                    _pad: 0,
                },
            );
            ptr::write(
                verts.add(1),
                TexturedColorVertex {
                    u: tex_w,
                    v: tex_h,
                    color: COLOR_WHITE_ABGR,
                    x: coord(x.saturating_add_unsigned(w)),
                    y: coord(y.saturating_add_unsigned(h)),
                    z: 0,
                    _pad: 0,
                },
            );

            sys::sceGuDrawArray(
                GuPrimitive::Sprites,
                TEXTURED_COLOR_VTYPE,
                2,
                ptr::null(),
                verts as *const c_void,
            );

            // Restore nearest filtering for pixel-art text/icons.
            sys::sceGuTexFilter(TextureFilter::Nearest, TextureFilter::Nearest);
        }
    }
}
