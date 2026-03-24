//! PSP backend for OASIS_OS.
//!
//! Hardware-accelerated rendering via the PSP Graphics Engine (sceGu/sceGum).
//! All rectangles, textures, and text are drawn as GU `Sprites` primitives,
//! offloading work from the 333MHz MIPS CPU to the dedicated GE hardware.
//!
//! Controller input via `sceCtrlPeekBufferPositive` with edge detection for
//! press/release events.
//!
//! Uses `restricted_std` with `RUST_PSP_BUILD_STD=1` for std support on PSP.
//! Types are imported from `oasis-core` directly.

#![feature(restricted_std)]
#![feature(asm_experimental_arch)]

// ---------------------------------------------------------------------------
// Module declarations
// ---------------------------------------------------------------------------

pub mod audio;
pub mod filesystem;
pub mod font;
pub mod input;
pub mod network;
pub mod power;
pub mod procedural;
pub mod psmf;
pub mod render;
pub mod sfx;
pub mod shapes;
pub mod status;
pub mod textures;
pub mod threading;
pub mod tls;
pub mod video;

// ---------------------------------------------------------------------------
// Re-exports from submodules (for main.rs and external users)
// ---------------------------------------------------------------------------

pub use audio::PspAudioBackend;
pub use filesystem::{FileEntry, decode_jpeg, format_size, list_directory, read_file};
pub use network::{PspNetworkBackend, PspNetworkService};
#[cfg(feature = "kernel-exception")]
pub use power::register_exception_handler;
pub use power::{check_power_resumed, power_tick, register_power_callback, set_clock};
pub use procedural::{
    CURSOR_H, CURSOR_W, WALLPAPER_TEX_H, WALLPAPER_TEX_W, generate_cursor_pixels, generate_gradient,
};
pub use sfx::SfxId;
pub use status::{StatusBarInfo, SystemInfo};
pub use threading::{
    AudioCmd, AudioHandle, IoCmd, IoHandle, IoResponse, TvCatalogRequest, spawn_workers,
};
pub use tls::PspTlsProvider;

// ---------------------------------------------------------------------------
// Re-exports from oasis-core
// ---------------------------------------------------------------------------

use oasis_core::backend::stacks::{ClipPush, ClipStack, TranslateStack};
pub use oasis_core::backend::{
    Color, SdiAlpha, SdiBackend, SdiBatch, SdiClipTransform, SdiCore, SdiGradients, SdiShapes,
    SdiText, SdiTextures, SdiVector, TextureId,
};
pub use oasis_core::error::{OasisError, Result as OasisResult};
pub use oasis_core::input::{Button, InputEvent, Trigger};
pub use oasis_core::sdi::SdiRegistry;
pub use oasis_core::wm::manager::{WindowManager, WmEvent};
pub use oasis_core::wm::window::{WindowConfig, WindowType, WmTheme};

// ---------------------------------------------------------------------------
// Imports
// ---------------------------------------------------------------------------

use std::alloc::{Layout, alloc};
use std::ffi::c_void;
use std::ptr;

use psp::sys::{
    self, BlendFactor, BlendOp, DisplayPixelFormat, GuContextType, GuState, GuSyncBehavior,
    GuSyncMode, MatrixMode, TextureColorComponent, TextureEffect, TextureFilter,
    TexturePixelFormat,
};
use psp::vram_alloc::get_vram_allocator;

use oasis_core::geometry::ClipRect;
use textures::{Texture, VolatileAllocator};

// ---------------------------------------------------------------------------
// PSP-specific color conversion
// ---------------------------------------------------------------------------

/// PSP-specific extension for Color -> ABGR conversion (used by sceGu).
pub trait ColorExt {
    fn to_abgr(&self) -> u32;
}

impl ColorExt for Color {
    fn to_abgr(&self) -> u32 {
        (self.a as u32) << 24 | (self.b as u32) << 16 | (self.g as u32) << 8 | self.r as u32
    }
}

/// Decode an ABGR u32 back to `Color` (inverse of `to_abgr`).
pub fn from_abgr(abgr: u32) -> Color {
    Color::rgba(
        (abgr & 0xFF) as u8,
        ((abgr >> 8) & 0xFF) as u8,
        ((abgr >> 16) & 0xFF) as u8,
        ((abgr >> 24) & 0xFF) as u8,
    )
}

#[cfg(test)]
mod color_tests {
    use super::*;

    #[test]
    fn to_abgr_pure_red() {
        let c = Color::rgba(255, 0, 0, 255);
        assert_eq!(c.to_abgr(), 0xFF00_00FF);
    }

    #[test]
    fn to_abgr_pure_green() {
        let c = Color::rgba(0, 255, 0, 255);
        assert_eq!(c.to_abgr(), 0xFF00_FF00);
    }

    #[test]
    fn to_abgr_pure_blue() {
        let c = Color::rgba(0, 0, 255, 255);
        assert_eq!(c.to_abgr(), 0xFFFF_0000);
    }

    #[test]
    fn to_abgr_opaque_white() {
        let c = Color::rgba(255, 255, 255, 255);
        assert_eq!(c.to_abgr(), 0xFFFF_FFFF);
    }

    #[test]
    fn to_abgr_opaque_black() {
        let c = Color::rgba(0, 0, 0, 255);
        assert_eq!(c.to_abgr(), 0xFF00_0000);
    }

    #[test]
    fn to_abgr_transparent() {
        let c = Color::rgba(0, 0, 0, 0);
        assert_eq!(c.to_abgr(), 0x0000_0000);
    }

    #[test]
    fn to_abgr_half_transparent_red() {
        let c = Color::rgba(255, 0, 0, 128);
        assert_eq!(c.to_abgr(), 0x8000_00FF);
    }

    #[test]
    fn to_abgr_arbitrary_color() {
        let c = Color::rgba(0x12, 0x34, 0x56, 0x78);
        // Expected: A=0x78, B=0x56, G=0x34, R=0x12
        assert_eq!(c.to_abgr(), 0x7856_3412);
    }

    // -- from_abgr (decode) tests --

    #[test]
    fn from_abgr_pure_red() {
        let c = from_abgr(0xFF00_00FF);
        assert_eq!(c, Color::rgba(255, 0, 0, 255));
    }

    #[test]
    fn from_abgr_pure_green() {
        let c = from_abgr(0xFF00_FF00);
        assert_eq!(c, Color::rgba(0, 255, 0, 255));
    }

    #[test]
    fn from_abgr_pure_blue() {
        let c = from_abgr(0xFFFF_0000);
        assert_eq!(c, Color::rgba(0, 0, 255, 255));
    }

    // -- Round-trip tests --

    #[test]
    fn round_trip_abgr_red() {
        let original = Color::rgba(255, 0, 0, 255);
        assert_eq!(from_abgr(original.to_abgr()), original);
    }

    #[test]
    fn round_trip_abgr_green() {
        let original = Color::rgba(0, 255, 0, 255);
        assert_eq!(from_abgr(original.to_abgr()), original);
    }

    #[test]
    fn round_trip_abgr_blue() {
        let original = Color::rgba(0, 0, 255, 255);
        assert_eq!(from_abgr(original.to_abgr()), original);
    }

    #[test]
    fn round_trip_abgr_white() {
        let original = Color::rgba(255, 255, 255, 255);
        assert_eq!(from_abgr(original.to_abgr()), original);
    }

    #[test]
    fn round_trip_abgr_black() {
        let original = Color::rgba(0, 0, 0, 255);
        assert_eq!(from_abgr(original.to_abgr()), original);
    }

    #[test]
    fn round_trip_abgr_transparent() {
        let original = Color::rgba(0, 0, 0, 0);
        assert_eq!(from_abgr(original.to_abgr()), original);
    }

    #[test]
    fn round_trip_abgr_arbitrary() {
        let original = Color::rgba(0x12, 0x34, 0x56, 0x78);
        assert_eq!(from_abgr(original.to_abgr()), original);
    }

    #[test]
    fn round_trip_abgr_max_channels() {
        let original = Color::rgba(255, 255, 255, 255);
        assert_eq!(from_abgr(original.to_abgr()), original);
    }

    #[test]
    fn round_trip_abgr_min_channels() {
        let original = Color::rgba(0, 0, 0, 0);
        assert_eq!(from_abgr(original.to_abgr()), original);
    }
}

// ---------------------------------------------------------------------------
// PSP display constants
// ---------------------------------------------------------------------------

/// Visible screen width.
pub const SCREEN_WIDTH: u32 = 480;
/// Visible screen height.
pub const SCREEN_HEIGHT: u32 = 272;
/// VRAM row stride in pixels (power-of-2 >= 480).
const BUF_WIDTH: u32 = 512;

// ---------------------------------------------------------------------------
// Display list (16-byte aligned, in BSS)
// ---------------------------------------------------------------------------

// 1 MB — browser pages generate thousands of GU commands (one per
// fill_rect, per glyph, per border edge).  256 KB overflows on any
// non-trivial HTML page, hanging sceGuSync.
const DISPLAY_LIST_SIZE: usize = 0x100000; // 1 MB

#[repr(C, align(16))]
struct Align16<T>(T);

static mut DISPLAY_LIST: Align16<[u8; DISPLAY_LIST_SIZE]> = Align16([0u8; DISPLAY_LIST_SIZE]);

// ---------------------------------------------------------------------------
// Backend
// ---------------------------------------------------------------------------

/// PSP rendering and input backend.
///
/// Draws using the PSP Graphics Engine (GE) via sceGu. All rendering calls
/// add commands to a display list; `swap_buffers()` submits the list, waits
/// for vblank, swaps framebuffers, and opens the next frame's list.
pub struct PspBackend {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) textures: Vec<Option<Texture>>,
    /// Controller input with automatic edge detection.
    pub(crate) controller: psp::input::Controller,
    /// Accumulated analog stick cursor position.
    pub(crate) cursor_x: i32,
    pub(crate) cursor_y: i32,
    /// 16-byte aligned RAM pointer to the bitmap font atlas texture (128x64 RGBA).
    pub(crate) font_atlas_ptr: *mut u8,
    /// System TrueType font renderer (None if unavailable, e.g. PPSSPP).
    pub(crate) system_font: Option<crate::font::SystemFont>,
    /// Volatile memory bump allocator (PSP-2000+ extra 4MB).
    pub(crate) volatile_alloc: Option<VolatileAllocator>,
    /// When true, `draw_text_inner` skips the system font and uses the
    /// 8x8 bitmap font. Set before drawing content that needs smaller text.
    pub force_bitmap_font: bool,
    /// Clip rectangle stack (uses GU scissor for hardware-accelerated clipping).
    clip_stack: ClipStack,
    /// Translation offset stack (applied to all rendering coordinates).
    translate_stack: TranslateStack,
}

impl PspBackend {
    /// Create a new PSP backend. Call `init()` to set up the display.
    pub fn new() -> Self {
        Self {
            width: SCREEN_WIDTH,
            height: SCREEN_HEIGHT,
            textures: Vec::new(),
            controller: psp::input::Controller::new(),
            cursor_x: (SCREEN_WIDTH / 2) as i32,
            cursor_y: (SCREEN_HEIGHT / 2) as i32,
            font_atlas_ptr: ptr::null_mut(),
            system_font: None,
            volatile_alloc: None,
            force_bitmap_font: false,
            clip_stack: ClipStack::new(SCREEN_WIDTH, SCREEN_HEIGHT),
            translate_stack: TranslateStack::new(),
        }
    }

    /// Initialize PSP display via GU and controller hardware.
    pub fn init(&mut self) {
        // SAFETY: All calls in this block are PSP firmware FFI functions
        // (sceCtrl*, sceGu*, sceGum*, sceDisplay*, sceKernelVolatileMem*)
        // and standard library `alloc`. The VRAM allocator, GU display
        // list, and framebuffer pointers are used according to the PSP SDK
        // contracts. The static DISPLAY_LIST is exclusively accessed here
        // and in swap_buffers (single-threaded main loop).
        unsafe {
            // Controller setup (enable analog stick readings).
            psp::input::enable_analog();

            // VRAM allocation: 2 framebuffers (no depth buffer for 2D).
            let allocator = get_vram_allocator().unwrap();
            let fbp0 = allocator
                .alloc_texture_pixels(BUF_WIDTH, SCREEN_HEIGHT, TexturePixelFormat::Psm8888)
                .unwrap();
            let fbp1 = allocator
                .alloc_texture_pixels(BUF_WIDTH, SCREEN_HEIGHT, TexturePixelFormat::Psm8888)
                .unwrap();

            let fbp0_zero = fbp0.as_mut_ptr_from_zero() as *mut c_void;
            let fbp1_zero = fbp1.as_mut_ptr_from_zero() as *mut c_void;

            // Font atlas in RAM (16-byte aligned).
            let atlas_size = (render::FONT_ATLAS_W * render::FONT_ATLAS_H * 4) as usize;
            let atlas_layout = Layout::from_size_align(atlas_size, 16).unwrap();
            let atlas_ptr = alloc(atlas_layout);
            if atlas_ptr.is_null() {
                // Graceful fallback: skip atlas font rendering, use bitmap-only.
                self.force_bitmap_font = true;
            } else {
                self.font_atlas_ptr = atlas_ptr;
            }

            // GU initialization.
            sys::sceGuInit();
            sys::sceGuStart(GuContextType::Direct, &raw mut DISPLAY_LIST as *mut c_void);

            // Draw buffer (render target) and display buffer.
            sys::sceGuDrawBuffer(DisplayPixelFormat::Psm8888, fbp0_zero, BUF_WIDTH as i32);
            sys::sceGuDispBuffer(
                SCREEN_WIDTH as i32,
                SCREEN_HEIGHT as i32,
                fbp1_zero,
                BUF_WIDTH as i32,
            );

            // Viewport and coordinate setup.
            sys::sceGuOffset(2048 - (SCREEN_WIDTH / 2), 2048 - (SCREEN_HEIGHT / 2));
            sys::sceGuViewport(2048, 2048, SCREEN_WIDTH as i32, SCREEN_HEIGHT as i32);

            // Scissor (full screen).
            sys::sceGuScissor(0, 0, SCREEN_WIDTH as i32, SCREEN_HEIGHT as i32);
            sys::sceGuEnable(GuState::ScissorTest);

            // Alpha blending.
            sys::sceGuEnable(GuState::Blend);
            sys::sceGuBlendFunc(
                BlendOp::Add,
                BlendFactor::SrcAlpha,
                BlendFactor::OneMinusSrcAlpha,
                0,
                0,
            );

            // Texture state.
            sys::sceGuEnable(GuState::Texture2D);
            sys::sceGuTexFunc(TextureEffect::Modulate, TextureColorComponent::Rgba);
            sys::sceGuTexFilter(TextureFilter::Nearest, TextureFilter::Nearest);

            // Projection: orthographic 2D.
            sys::sceGumMatrixMode(MatrixMode::Projection);
            sys::sceGumLoadIdentity();
            sys::sceGumOrtho(
                0.0,
                SCREEN_WIDTH as f32,
                SCREEN_HEIGHT as f32,
                0.0,
                -1.0,
                1.0,
            );

            // View and model: identity.
            sys::sceGumMatrixMode(MatrixMode::View);
            sys::sceGumLoadIdentity();
            sys::sceGumMatrixMode(MatrixMode::Model);
            sys::sceGumLoadIdentity();

            // Finalize init list, sync, enable display.
            sys::sceGuFinish();
            sys::sceGuSync(GuSyncMode::Finish, GuSyncBehavior::Wait);
            sys::sceDisplayWaitVblankStart();
            sys::sceGuDisplay(true);

            // Build bitmap font atlas in RAM (fallback).
            self.build_font_atlas(atlas_ptr);

            // Try to initialize system TrueType fonts (VRAM glyph atlas).
            // Allocate 512x512 T8 (1 byte/pixel) from VRAM for the atlas.
            let sys_font_atlas =
                allocator.alloc_texture_pixels(512, 512, TexturePixelFormat::PsmT8);
            if let Ok(atlas_chunk) = sys_font_atlas {
                let vram_ptr = atlas_chunk.as_mut_ptr_direct_to_vram();
                self.system_font = crate::font::SystemFont::try_init(vram_ptr);
                // Silently fall back to bitmap if system fonts unavailable.
            }

            // Claim volatile memory (extra 4MB on PSP-2000+) for textures.
            // Requires kernel mode; skip in user mode builds.
            #[cfg(feature = "kernel-volatile")]
            {
                let mut vol_ptr: *mut c_void = ptr::null_mut();
                let mut vol_size: i32 = 0;
                let vol_ret = sys::sceKernelVolatileMemTryLock(
                    0,
                    &mut vol_ptr as *mut *mut c_void,
                    &mut vol_size,
                );
                if vol_ret == 0 && !vol_ptr.is_null() && vol_size > 0 {
                    self.volatile_alloc = Some(VolatileAllocator::new(
                        vol_ptr as *mut u8,
                        vol_size as usize,
                    ));
                }
            }

            // Open the first frame's display list.
            sys::sceGuStart(GuContextType::Direct, &raw mut DISPLAY_LIST as *mut c_void);
        }
    }

    /// Set the clipping rectangle via GU scissor.
    pub fn set_clip_rect_inner(&mut self, x: i32, y: i32, w: u32, h: u32) {
        // SAFETY: sceGuScissor is a GU FFI call operating on the display list.
        unsafe {
            sys::sceGuScissor(x, y, x + w as i32, y + h as i32);
        }
    }

    /// Reset clipping to full screen.
    pub fn reset_clip_rect_inner(&mut self) {
        // SAFETY: sceGuScissor is a GU FFI call operating on the display list.
        unsafe {
            sys::sceGuScissor(0, 0, SCREEN_WIDTH as i32, SCREEN_HEIGHT as i32);
        }
    }

    /// Finalize the current display list, swap buffers, and open the next
    /// frame.
    ///
    /// The GE renders the display list asynchronously after `sceGuFinish`.
    /// By waiting for vsync *before* blocking on `sceGuSync`, the GE
    /// executes in parallel with the vsync wait instead of sequentially,
    /// preventing frame-doubling (60→30fps) when the display list is heavy
    /// (e.g. window dragging, music playback bus contention).
    pub fn swap_buffers_inner(&mut self) {
        // SAFETY: GU frame lifecycle calls. DISPLAY_LIST is exclusively
        // accessed from the single-threaded main loop (init/swap_buffers).
        unsafe {
            sys::sceGuFinish();
            // Vsync first: GE renders in parallel while CPU waits for vblank.
            sys::sceDisplayWaitVblankStart();
            // GE is likely done by now; block only if it isn't.
            sys::sceGuSync(GuSyncMode::Finish, GuSyncBehavior::Wait);
            sys::sceGuSwapBuffers();
            sys::sceGuStart(GuContextType::Direct, &raw mut DISPLAY_LIST as *mut c_void);
        }
    }

    /// Restore the full GU rendering state after a utility dialog.
    ///
    /// PSP utility dialogs (`psp::osk`, `psp::dialog`) run their own GU
    /// frames with different blend, texture, scissor, viewport, and matrix
    /// settings. They do **not** restore the caller's GE state on exit.
    /// This method re-opens the display list AND re-applies every GU state
    /// that `init()` configures, so subsequent rendering works correctly.
    pub fn reinit_gu_frame(&self) {
        // SAFETY: Restores the GU display list and GE rendering state
        // after a utility dialog. DISPLAY_LIST is exclusively accessed
        // from the main loop. All sceGu/sceGum calls match init().
        unsafe {
            sys::sceGuStart(GuContextType::Direct, &raw mut DISPLAY_LIST as *mut c_void);

            // Viewport and coordinate setup (matches init).
            sys::sceGuOffset(2048 - (SCREEN_WIDTH / 2), 2048 - (SCREEN_HEIGHT / 2));
            sys::sceGuViewport(2048, 2048, SCREEN_WIDTH as i32, SCREEN_HEIGHT as i32);

            // Scissor (full screen).
            sys::sceGuScissor(0, 0, SCREEN_WIDTH as i32, SCREEN_HEIGHT as i32);
            sys::sceGuEnable(GuState::ScissorTest);

            // Alpha blending.
            sys::sceGuEnable(GuState::Blend);
            sys::sceGuBlendFunc(
                BlendOp::Add,
                BlendFactor::SrcAlpha,
                BlendFactor::OneMinusSrcAlpha,
                0,
                0,
            );

            // Texture state.
            sys::sceGuEnable(GuState::Texture2D);
            sys::sceGuTexFunc(TextureEffect::Modulate, TextureColorComponent::Rgba);
            sys::sceGuTexFilter(TextureFilter::Nearest, TextureFilter::Nearest);

            // Projection: orthographic 2D.
            sys::sceGumMatrixMode(MatrixMode::Projection);
            sys::sceGumLoadIdentity();
            sys::sceGumOrtho(
                0.0,
                SCREEN_WIDTH as f32,
                SCREEN_HEIGHT as f32,
                0.0,
                -1.0,
                1.0,
            );

            // View and model: identity.
            sys::sceGumMatrixMode(MatrixMode::View);
            sys::sceGumLoadIdentity();
            sys::sceGumMatrixMode(MatrixMode::Model);
            sys::sceGumLoadIdentity();
        }
    }

    /// Current cursor position (for rendering the cursor sprite).
    pub fn cursor_pos(&self) -> (i32, i32) {
        (self.cursor_x, self.cursor_y)
    }

    /// Check if a controller button is currently held down.
    pub fn is_button_held(&self, button: psp::sys::CtrlButtons) -> bool {
        self.controller.is_held(button)
    }

    /// Check if any button OTHER than `exclude` is currently held.
    pub fn is_any_other_button_held(&self, exclude: psp::sys::CtrlButtons) -> bool {
        self.controller.raw().buttons.intersects(
            psp::sys::CtrlButtons::from_bits_truncate(!exclude.bits()),
        )
    }

    /// Query volatile memory cache status.
    ///
    /// Returns `(total_bytes, remaining_bytes)` if volatile memory was
    /// claimed, or `None` on PSP-1000 / if already locked.
    /// Raw pointer to the bitmap font atlas texture in RAM.
    ///
    /// The atlas is a 128x64 RGBA8888 image, 16-byte aligned, built during
    /// `init()`. Use via `psp::cache::UncachedPtr::from_cached_addr` for
    /// GE texture binding.
    pub fn font_atlas(&self) -> *mut u8 {
        self.font_atlas_ptr
    }

    pub fn volatile_mem_info(&self) -> Option<(usize, usize)> {
        self.volatile_alloc
            .as_ref()
            .map(|va| (va.size, va.remaining()))
    }
}

// ---------------------------------------------------------------------------
// SdiBackend trait implementation
// ---------------------------------------------------------------------------

impl SdiCore for PspBackend {
    fn init(&mut self, _width: u32, _height: u32) -> OasisResult<()> {
        // PSP backend initializes during PspBackend::init().
        Ok(())
    }

    fn clear(&mut self, color: Color) -> OasisResult<()> {
        self.clear_inner(color);
        Ok(())
    }

    fn blit(&mut self, tex: TextureId, x: i32, y: i32, w: u32, h: u32) -> OasisResult<()> {
        let (tx, ty) = self.translate_stack.translate(x, y);
        self.blit_inner(tex, tx, ty, w, h);
        Ok(())
    }

    fn fill_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: Color) -> OasisResult<()> {
        let (tx, ty) = self.translate_stack.translate(x, y);
        self.fill_rect_inner(tx, ty, w, h, color);
        Ok(())
    }

    fn draw_text(
        &mut self,
        text: &str,
        x: i32,
        y: i32,
        font_size: u16,
        color: Color,
    ) -> OasisResult<()> {
        let (tx, ty) = self.translate_stack.translate(x, y);
        self.draw_text_inner(text, tx, ty, font_size, color);
        Ok(())
    }

    fn swap_buffers(&mut self) -> OasisResult<()> {
        self.swap_buffers_inner();
        Ok(())
    }

    fn load_texture(
        &mut self,
        width: u32,
        height: u32,
        rgba_data: &[u8],
    ) -> OasisResult<TextureId> {
        self.load_texture_inner(width, height, rgba_data)
            .ok_or_else(|| OasisError::Backend("PSP texture allocation failed".to_string().into()))
    }

    fn destroy_texture(&mut self, tex: TextureId) -> OasisResult<()> {
        self.destroy_texture_inner(tex);
        Ok(())
    }

    fn set_clip_rect(&mut self, x: i32, y: i32, w: u32, h: u32) -> OasisResult<()> {
        let (tx, ty) = self.translate_stack.translate(x, y);
        self.set_clip_rect_inner(tx, ty, w, h);
        Ok(())
    }

    fn reset_clip_rect(&mut self) -> OasisResult<()> {
        self.reset_clip_rect_inner();
        Ok(())
    }

    fn measure_text(&self, text: &str, font_size: u16) -> u32 {
        oasis_core::backend::bitmap_measure_text(text, font_size)
    }

    fn read_pixels(&self, _x: i32, _y: i32, _w: u32, _h: u32) -> OasisResult<Vec<u8>> {
        Err(OasisError::Backend(
            "read_pixels not supported on PSP".to_string().into(),
        ))
    }

    fn shutdown(&mut self) -> OasisResult<()> {
        Ok(())
    }
}

// -------------------------------------------------------------------
// Extension trait implementations (GU-accelerated where possible)
// -------------------------------------------------------------------

impl SdiShapes for PspBackend {
    fn fill_rounded_rect(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        radius: u16,
        color: Color,
    ) -> OasisResult<()> {
        let (tx, ty) = self.translate_stack.translate(x, y);
        self.fill_rounded_rect_inner(tx, ty, w, h, radius, color);
        Ok(())
    }

    fn fill_circle(&mut self, cx: i32, cy: i32, radius: u16, color: Color) -> OasisResult<()> {
        let (tx, ty) = self.translate_stack.translate(cx, cy);
        self.fill_circle_inner(tx, ty, radius, color);
        Ok(())
    }

    fn draw_line(
        &mut self,
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        width: u16,
        color: Color,
    ) -> OasisResult<()> {
        let (tx1, ty1) = self.translate_stack.translate(x1, y1);
        let (tx2, ty2) = self.translate_stack.translate(x2, y2);
        self.draw_line_inner(tx1, ty1, tx2, ty2, width, color);
        Ok(())
    }
}

impl SdiGradients for PspBackend {
    fn fill_rect_gradient(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        gradient: &oasis_core::backend::GradientStyle,
    ) -> OasisResult<()> {
        let (tx, ty) = self.translate_stack.translate(x, y);
        self.fill_rect_gradient_inner(tx, ty, w, h, gradient);
        Ok(())
    }
}

impl SdiAlpha for PspBackend {
    fn viewport_size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn dim_screen(&mut self, alpha: u8) -> OasisResult<()> {
        self.dim_screen_inner(alpha);
        Ok(())
    }
}

impl SdiText for PspBackend {}
impl SdiTextures for PspBackend {}

impl SdiClipTransform for PspBackend {
    fn push_clip_rect(&mut self, x: i32, y: i32, w: u32, h: u32) -> OasisResult<()> {
        let (tx, ty) = self.translate_stack.translate(x, y);
        let new_clip = ClipRect { x: tx, y: ty, w, h };
        match self.clip_stack.push(new_clip) {
            ClipPush::Clip(c) => {
                self.set_clip_rect_inner(c.x, c.y, c.w, c.h);
            },
            ClipPush::Empty => {
                self.set_clip_rect_inner(0, 0, 0, 0);
            },
        }
        Ok(())
    }

    fn pop_clip_rect(&mut self) -> OasisResult<()> {
        match self.clip_stack.pop() {
            Some(prev) => {
                self.set_clip_rect_inner(prev.x, prev.y, prev.w, prev.h);
            },
            None => {
                self.reset_clip_rect_inner();
            },
        }
        Ok(())
    }

    fn current_clip_rect(&self) -> Option<(i32, i32, u32, u32)> {
        self.clip_stack.current_tuple()
    }

    fn push_translate(&mut self, dx: i32, dy: i32) -> OasisResult<()> {
        self.translate_stack.push(dx, dy);
        Ok(())
    }

    fn pop_translate(&mut self) -> OasisResult<()> {
        self.translate_stack.pop();
        Ok(())
    }

    fn current_translate(&self) -> (i32, i32) {
        self.translate_stack.current()
    }
}

impl SdiVector for PspBackend {}
impl SdiBatch for PspBackend {}

// ---------------------------------------------------------------------------
// PSP-tuned WM theme (compact for 480x272 screen)
// ---------------------------------------------------------------------------

/// Create a compact WmTheme tuned for the PSP's 480x272 display.
pub fn psp_wm_theme() -> WmTheme {
    WmTheme {
        titlebar_height: 12,
        border_width: 1,
        titlebar_active_color: Color::rgba(40, 70, 130, 230),
        titlebar_inactive_color: Color::rgba(60, 60, 60, 200),
        titlebar_text_color: Color::WHITE,
        frame_color: Color::rgba(30, 30, 30, 200),
        content_bg_color: Color::rgba(20, 20, 30, 220),
        btn_close_color: Color::rgb(180, 50, 50),
        btn_minimize_color: Color::rgb(180, 160, 50),
        btn_maximize_color: Color::rgb(50, 160, 50),
        button_size: 8,
        resize_handle_size: 3,
        titlebar_font_size: 10,
        ..WmTheme::default()
    }
}

// ---------------------------------------------------------------------------
// Status bar helpers
// ---------------------------------------------------------------------------

/// Draw a PSIX-style status bar at the top of the screen.
pub fn draw_status_bar(backend: &mut PspBackend, version: &str) {
    let bar_color = Color::rgba(30, 80, 30, 200);
    backend.fill_rect_inner(0, 0, SCREEN_WIDTH, 18, bar_color);
    backend.draw_text_inner(version, 4, 4, 8, Color::WHITE);
}

/// Draw a PSIX-style bottom bar with navigation hints.
pub fn draw_bottom_bar(backend: &mut PspBackend, hint: &str) {
    let bar_y = (SCREEN_HEIGHT - 18) as i32;
    let bar_color = Color::rgba(30, 80, 30, 200);
    backend.fill_rect_inner(0, bar_y, SCREEN_WIDTH, 18, bar_color);
    backend.draw_text_inner(hint, 4, bar_y + 4, 8, Color::WHITE);
}
