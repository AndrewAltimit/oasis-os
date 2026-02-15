//! Direct framebuffer drawing for the overlay.
//!
//! All rendering writes directly to the game's framebuffer pointer (obtained
//! from the `sceDisplaySetFrameBuf` hook arguments). No GU, no VRAM alloc --
//! just pixel writes + dcache flush.
//!
//! Pixel format: ABGR 8888 (PSP native 32-bit).

use crate::config;
use crate::font;

/// VRAM stride in pixels (PSP hardware constant).
const VRAM_STRIDE: u32 = 512;

/// Screen dimensions.
pub const SCREEN_WIDTH: u32 = 480;
pub const SCREEN_HEIGHT: u32 = 272;

/// Draw a single character at (x, y) using the 8x8 bitmap font.
///
/// # Safety
/// `fb` must point to a valid framebuffer of at least `stride * SCREEN_HEIGHT`
/// 32-bit pixels. (x, y) must be in bounds such that x+8 <= stride and
/// y+8 <= SCREEN_HEIGHT.
pub unsafe fn draw_char(fb: *mut u32, stride: u32, x: u32, y: u32, ch: u8, color: u32) {
    let glyph_data = font::glyph(ch);
    let mut row = 0u32;
    while row < font::GLYPH_HEIGHT {
        let bits = glyph_data[row as usize];
        let mut col = 0u32;
        while col < font::GLYPH_WIDTH {
            if bits & (0x80 >> col) != 0 {
                let px = x + col;
                let py = y + row;
                if px < SCREEN_WIDTH && py < SCREEN_HEIGHT {
                    let offset = py * stride + px;
                    // SAFETY: Bounds checked above.
                    unsafe {
                        *fb.add(offset as usize) = color;
                    }
                }
            }
            col += 1;
        }
        row += 1;
    }
}

/// Draw a null-terminated byte string at (x, y).
///
/// # Safety
/// Same requirements as `draw_char`.
pub unsafe fn draw_string(fb: *mut u32, stride: u32, x: u32, y: u32, text: &[u8], color: u32) {
    let mut cx = x;
    for &ch in text {
        if ch == 0 {
            break;
        }
        if cx + font::GLYPH_WIDTH > SCREEN_WIDTH {
            break;
        }
        // SAFETY: fb is valid, cx/y bounds checked.
        unsafe {
            draw_char(fb, stride, cx, y, ch, color);
        }
        cx += font::GLYPH_WIDTH;
    }
}

/// Draw a filled rectangle with alpha blending.
///
/// `color` is ABGR 8888. The alpha channel controls blend intensity.
///
/// # Safety
/// `fb` must point to a valid framebuffer.
pub unsafe fn fill_rect_alpha(
    fb: *mut u32,
    stride: u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    color: u32,
) {
    let alpha = config::get_opacity() as u32;
    let src_r = color & 0xFF;
    let src_g = (color >> 8) & 0xFF;
    let src_b = (color >> 16) & 0xFF;
    let inv_alpha = 255 - alpha;

    let mut row = 0u32;
    while row < h {
        let py = y + row;
        if py >= SCREEN_HEIGHT {
            break;
        }
        let mut col = 0u32;
        while col < w {
            let px = x + col;
            if px >= SCREEN_WIDTH {
                break;
            }
            let offset = (py * stride + px) as usize;
            // SAFETY: Bounds checked above.
            let dst = unsafe { *fb.add(offset) };
            let dst_r = dst & 0xFF;
            let dst_g = (dst >> 8) & 0xFF;
            let dst_b = (dst >> 16) & 0xFF;

            let out_r = (src_r * alpha + dst_r * inv_alpha) / 255;
            let out_g = (src_g * alpha + dst_g * inv_alpha) / 255;
            let out_b = (src_b * alpha + dst_b * inv_alpha) / 255;

            let blended = 0xFF000000 | (out_b << 16) | (out_g << 8) | out_r;
            // SAFETY: Bounds checked above.
            unsafe {
                *fb.add(offset) = blended;
            }
            col += 1;
        }
        row += 1;
    }
}

/// Draw a filled rectangle (opaque, no blending).
///
/// # Safety
/// `fb` must point to a valid framebuffer.
pub unsafe fn fill_rect(fb: *mut u32, stride: u32, x: u32, y: u32, w: u32, h: u32, color: u32) {
    let mut row = 0u32;
    while row < h {
        let py = y + row;
        if py >= SCREEN_HEIGHT {
            break;
        }
        let mut col = 0u32;
        while col < w {
            let px = x + col;
            if px >= SCREEN_WIDTH {
                break;
            }
            let offset = (py * stride + px) as usize;
            // SAFETY: Bounds checked above.
            unsafe {
                *fb.add(offset) = color;
            }
            col += 1;
        }
        row += 1;
    }
}

/// Flush dcache for a framebuffer region to ensure writes are visible.
///
/// # Safety
/// `fb` must point to a valid memory region.
pub unsafe fn flush_framebuffer(fb: *mut u32, stride: u32, y: u32, h: u32) {
    // SAFETY: Pointer arithmetic within valid framebuffer region.
    let start = unsafe { fb.add((y * stride) as usize) } as *const u8;
    let size = (h * stride * 4) as u32;
    // SAFETY: Valid memory range within framebuffer.
    unsafe {
        psp::sys::sceKernelDcacheWritebackRange(start as *const _, size);
    }
}

/// Color constants (ABGR 8888).
pub mod colors {
    /// White.
    pub const WHITE: u32 = 0xFFFFFFFF;
    /// Black.
    pub const BLACK: u32 = 0xFF000000;
    /// Semi-transparent black for overlay background.
    pub const OVERLAY_BG: u32 = 0xFF1A1A2E;
    /// Accent blue.
    pub const ACCENT: u32 = 0xFFFF9933;
    /// Highlight / cursor color.
    pub const HIGHLIGHT: u32 = 0xFF4A3520;
    /// Green for active/enabled indicators.
    pub const GREEN: u32 = 0xFF33FF66;
    /// Yellow for warnings.
    pub const YELLOW: u32 = 0xFF00DDFF;
    /// Gray for dimmed text.
    pub const GRAY: u32 = 0xFF999999;
}
