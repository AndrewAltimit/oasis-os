//! GU rendering primitives: vertices, clear, fill, text, blit.

use std::ffi::c_void;
use std::mem::size_of;
use std::ptr;

use psp::sys::{
    self, ClearBuffer, GuPrimitive, MipmapLevel, TextureColorComponent, TextureEffect,
    TextureFilter,
    TexturePixelFormat, VertexType,
};

use oasis_core::backend::Color;

use crate::{ColorExt, PspBackend};

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

// ---------------------------------------------------------------------------
// PspBackend rendering methods
// ---------------------------------------------------------------------------

impl PspBackend {
    /// Build the 128x64 font atlas in a RAM buffer.
    ///
    /// 16 glyphs per row, 6 rows (95 glyphs for ASCII 32-126).
    /// Each glyph is 8x8. White where bit is set, transparent elsewhere.
    /// SAFETY: `buf` must point to a valid, 16-byte-aligned allocation of at
    /// least `FONT_ATLAS_W * FONT_ATLAS_H * 4` bytes. Caller ensures this
    /// via `alloc(atlas_layout)` with a null check.
    pub(crate) unsafe fn build_font_atlas(&self, buf: *mut u8) {
        let pixels = buf as *mut u32;
        let stride = FONT_ATLAS_W;
        let total = (FONT_ATLAS_W * FONT_ATLAS_H) as usize;

        // Zero the entire atlas first (manual loop -- see MEMORY.md footgun).
        for i in 0..total {
            unsafe { pixels.add(i).write(0u32) };
        }

        // Write a solid white pixel at the bottom-right corner of the atlas.
        // Used by fill_rect_inner to draw colored rectangles without toggling
        // Texture2D state (sample white texel * vertex color = fill color).
        let white_offset = ((FONT_ATLAS_H - 1) * stride + (FONT_ATLAS_W - 1)) as usize;
        unsafe { pixels.add(white_offset).write(0xFFFF_FFFFu32) };

        for idx in 0u32..95 {
            let col = idx % ATLAS_COLS;
            let row = idx / ATLAS_COLS;
            let glyph_data = crate::font::glyph((idx + 32) as u8 as char);

            for gy in 0..8u32 {
                let bits = glyph_data[gy as usize];
                for gx in 0..8u32 {
                    if bits & (0x80 >> gx) != 0 {
                        let px = col * 8 + gx;
                        let py = row * 8 + gy;
                        let offset = (py * stride + px) as usize;
                        unsafe { pixels.add(offset).write(0xFFFF_FFFFu32) };
                    }
                }
            }
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
        // SAFETY: sceGuGetMemory returns a display-list-embedded pointer
        // valid until sceGuFinish. We write exactly 2 TexturedColorVertex
        // structs, sampling the solid white texel at the bottom-right corner
        // of the font atlas. Modulate texfunc: white * vertex_color = fill
        // color. This avoids toggling Texture2D state on every rectangle.
        unsafe {
            let verts = sys::sceGuGetMemory((2 * size_of::<TexturedColorVertex>()) as i32)
                as *mut TexturedColorVertex;
            if verts.is_null() {
                return;
            }

            // Bind the font atlas; the white texel is at the bottom-right corner.
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
                    x: x as i16,
                    y: y as i16,
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
                    x: (x + w as i32) as i16,
                    y: (y + h as i32) as i16,
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
    fn draw_text_bitmap(&mut self, text: &str, x: i32, y: i32, font_size: u16, abgr: u32) {
        let scale = if font_size >= 8 {
            (font_size / 8) as f32
        } else {
            1.0
        };
        let glyph_w = (crate::font::GLYPH_WIDTH as f32) * scale;
        let glyph_h = 8.0 * scale;

        let mut batch = psp::gu_ext::SpriteBatch::new(text.len());

        let mut cx = x as f32;
        for ch in text.chars() {
            let idx = (ch as u32).wrapping_sub(32);
            let (u0, v0) = if idx < 95 {
                let col = idx % ATLAS_COLS;
                let row = idx / ATLAS_COLS;
                ((col * 8) as f32, (row * 8) as f32)
            } else {
                (0.0, 0.0)
            };

            batch.draw_rect(
                cx,
                y as f32,
                glyph_w,
                glyph_h,
                u0,
                v0,
                u0 + 8.0,
                v0 + 8.0,
                abgr,
            );
            cx += glyph_w;
        }

        // SAFETY: Binds the font atlas texture (RAM pointer via uncached
        // mirror) and flushes the batched sprites. font_atlas_ptr is
        // checked non-null during init(). No TexFlush/TexSync needed --
        // the atlas is in uncached RAM so the GE always reads current data.
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
        let tex_w = texture.width as i16;
        let tex_h = texture.height as i16;
        let buf_w = texture.buf_w;
        let buf_h = texture.buf_h;
        let data_ptr = texture.data;

        // SAFETY: Binds the texture (RAM pointer via uncached mirror) and
        // draws a Sprites primitive. data_ptr validity is ensured by
        // load_texture_inner (allocated and populated before insertion).
        // No TexFlush/TexSync -- uncached pointers bypass the GE cache.
        unsafe {
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
            sys::sceGuTexFunc(TextureEffect::Modulate, TextureColorComponent::Rgba);

            let verts = sys::sceGuGetMemory((2 * size_of::<TexturedColorVertex>()) as i32)
                as *mut TexturedColorVertex;
            if verts.is_null() {
                return;
            }

            let white = 0xFFFF_FFFFu32;

            ptr::write(
                verts,
                TexturedColorVertex {
                    u: 0,
                    v: 0,
                    color: white,
                    x: x as i16,
                    y: y as i16,
                    z: 0,
                    _pad: 0,
                },
            );
            ptr::write(
                verts.add(1),
                TexturedColorVertex {
                    u: tex_w,
                    v: tex_h,
                    color: white,
                    x: (x + w as i32) as i16,
                    y: (y + h as i32) as i16,
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

        unsafe {
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
            sys::sceGuTexFunc(TextureEffect::Modulate, TextureColorComponent::Rgba);
            sys::sceGuTexFilter(TextureFilter::Linear, TextureFilter::Linear);

            let verts = sys::sceGuGetMemory((2 * size_of::<TexturedColorVertex>()) as i32)
                as *mut TexturedColorVertex;
            if verts.is_null() {
                sys::sceGuTexFilter(TextureFilter::Nearest, TextureFilter::Nearest);
                return;
            }

            let white = 0xFFFF_FFFFu32;

            ptr::write(
                verts,
                TexturedColorVertex {
                    u: 0,
                    v: 0,
                    color: white,
                    x: x as i16,
                    y: y as i16,
                    z: 0,
                    _pad: 0,
                },
            );
            ptr::write(
                verts.add(1),
                TexturedColorVertex {
                    u: tex_w,
                    v: tex_h,
                    color: white,
                    x: (x + w as i32) as i16,
                    y: (y + h as i32) as i16,
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
