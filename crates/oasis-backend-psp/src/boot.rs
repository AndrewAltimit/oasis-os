//! Boot splash screen rendering.

use oasis_backend_psp::render::{FONT_ATLAS_H, FONT_ATLAS_W};
use oasis_backend_psp::{Color, PspBackend, SCREEN_HEIGHT, SCREEN_WIDTH};
use psp::gu_ext::SpriteBatch;

use crate::theme::CHAR_W;

/// Draw a boot splash screen with title, status text, and progress bar.
///
/// Uses fill_rect for the background (bypasses FAST_CLEAR on PPSSPP),
/// draws progress bar with fill_rects, then renders both text lines in
/// a **single** SpriteBatch + texture bind to avoid GE state issues on
/// PPSSPP with multiple sprite draws per frame during init.
pub(crate) fn show_boot_screen(backend: &mut PspBackend, status: &str, progress: u32) {
    let bg = Color::rgba(15, 15, 25, 255);
    backend.fill_rect_inner(0, 0, SCREEN_WIDTH, SCREEN_HEIGHT, bg);

    // Progress bar (200px wide, centered).
    let title_y = SCREEN_HEIGHT as i32 / 2 - 30;
    let status_y = title_y + 16;
    let bar_w: u32 = 200;
    let bar_h: u32 = 6;
    let bar_x = (SCREEN_WIDTH as i32 - bar_w as i32) / 2;
    let bar_y = status_y + 20;
    backend.fill_rect_inner(bar_x, bar_y, bar_w, bar_h, Color::rgba(40, 40, 60, 200));
    let fill_w = (bar_w * progress.min(100)) / 100;
    if fill_w > 0 {
        backend.fill_rect_inner(bar_x, bar_y, fill_w, bar_h, Color::rgb(80, 140, 220));
    }

    // Single SpriteBatch for both title and status text.
    let title = "OASIS_OS";
    let atlas_cols: u32 = 16;
    let total_chars = title.len() + status.len();
    let mut batch = SpriteBatch::new(total_chars);

    let title_w = (title.len() as i32) * CHAR_W;
    let title_x = (SCREEN_WIDTH as i32 - title_w) / 2;
    let white_abgr = 0xFFFF_FFFFu32;
    let mut cx = title_x as f32;
    for ch in title.chars() {
        let idx = (ch as u32).wrapping_sub(32);
        let (u0, v0) = if idx < 95 {
            ((idx % atlas_cols * 8) as f32, (idx / atlas_cols * 8) as f32)
        } else {
            (0.0, 0.0)
        };
        batch.draw_rect(
            cx,
            title_y as f32,
            8.0,
            8.0,
            u0,
            v0,
            u0 + 8.0,
            v0 + 8.0,
            white_abgr,
        );
        cx += 8.0;
    }

    let status_w = (status.len() as i32) * CHAR_W;
    let status_x = (SCREEN_WIDTH as i32 - status_w) / 2;
    let status_abgr = 0xFFC8AAA0u32; // Color::rgb(160, 170, 200) in ABGR
    cx = status_x as f32;
    for ch in status.chars() {
        let idx = (ch as u32).wrapping_sub(32);
        let (u0, v0) = if idx < 95 {
            ((idx % atlas_cols * 8) as f32, (idx / atlas_cols * 8) as f32)
        } else {
            (0.0, 0.0)
        };
        batch.draw_rect(
            cx,
            status_y as f32,
            8.0,
            8.0,
            u0,
            v0,
            u0 + 8.0,
            v0 + 8.0,
            status_abgr,
        );
        cx += 8.0;
    }

    // Single texture bind + single flush for all text.
    // SAFETY: Within an active GU display list; font atlas pointer is
    // valid and non-null (set during backend.init()).
    unsafe {
        use psp::sys::{
            self, MipmapLevel, TextureColorComponent, TextureEffect, TexturePixelFormat,
        };
        use std::ffi::c_void;
        let uncached_atlas = psp::cache::UncachedPtr::from_cached_addr(backend.font_atlas())
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
        sys::sceGuTexFlush();
        sys::sceGuTexSync();
        batch.flush();
    }

    backend.swap_buffers_inner();
}
