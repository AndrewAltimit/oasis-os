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
pub mod render;
pub mod sfx;
pub mod status;
pub mod textures;
pub mod threading;

// ---------------------------------------------------------------------------
// Re-exports from submodules (for main.rs and external users)
// ---------------------------------------------------------------------------

pub use audio::PspAudioBackend;
pub use network::{PspNetworkBackend, PspNetworkService};
pub use filesystem::{list_directory, format_size, read_file, decode_jpeg, FileEntry};
pub use power::{
    check_power_resumed, power_tick, register_exception_handler,
    register_power_callback, set_clock,
};
pub use procedural::{
    generate_cursor_pixels, generate_gradient, CURSOR_H, CURSOR_W,
};
pub use status::{StatusBarInfo, SystemInfo};
pub use sfx::SfxId;
pub use threading::{
    spawn_workers, AudioCmd, AudioHandle, IoCmd, IoHandle, IoResponse,
};

// ---------------------------------------------------------------------------
// Re-exports from oasis-core
// ---------------------------------------------------------------------------

pub use oasis_core::backend::{Color, SdiBackend, TextureId};
pub use oasis_core::error::{OasisError, Result as OasisResult};
pub use oasis_core::input::{Button, InputEvent, Trigger};
pub use oasis_core::sdi::SdiRegistry;
pub use oasis_core::wm::manager::{WindowManager, WmEvent};
pub use oasis_core::wm::window::{WindowConfig, WindowType, WmTheme};

// ---------------------------------------------------------------------------
// Imports
// ---------------------------------------------------------------------------

use std::alloc::{alloc, Layout};
use std::ffi::c_void;
use std::ptr;

use psp::sys::{
    self, BlendFactor, BlendOp, DisplayPixelFormat, GuContextType, GuState,
    GuSyncBehavior, GuSyncMode, MatrixMode, TextureColorComponent,
    TextureEffect, TextureFilter, TexturePixelFormat,
};
use psp::vram_alloc::get_vram_allocator;

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
        (self.a as u32) << 24
            | (self.b as u32) << 16
            | (self.g as u32) << 8
            | self.r as u32
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

const DISPLAY_LIST_SIZE: usize = 0x40000; // 256 KB

#[repr(C, align(16))]
struct Align16<T>(T);

static mut DISPLAY_LIST: Align16<[u8; DISPLAY_LIST_SIZE]> =
    Align16([0u8; DISPLAY_LIST_SIZE]);

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
                .alloc_texture_pixels(
                    BUF_WIDTH,
                    SCREEN_HEIGHT,
                    TexturePixelFormat::Psm8888,
                )
                .unwrap();
            let fbp1 = allocator
                .alloc_texture_pixels(
                    BUF_WIDTH,
                    SCREEN_HEIGHT,
                    TexturePixelFormat::Psm8888,
                )
                .unwrap();

            let fbp0_zero = fbp0.as_mut_ptr_from_zero() as *mut c_void;
            let fbp1_zero = fbp1.as_mut_ptr_from_zero() as *mut c_void;

            // Font atlas in RAM (16-byte aligned).
            let atlas_size =
                (render::FONT_ATLAS_W * render::FONT_ATLAS_H * 4) as usize;
            let atlas_layout =
                Layout::from_size_align(atlas_size, 16).unwrap();
            let atlas_ptr = alloc(atlas_layout);
            if atlas_ptr.is_null() {
                panic!(
                    "OASIS_OS: FATAL - font atlas allocation failed (OOM)"
                );
            }
            self.font_atlas_ptr = atlas_ptr;

            // GU initialization.
            sys::sceGuInit();
            sys::sceGuStart(
                GuContextType::Direct,
                &raw mut DISPLAY_LIST as *mut c_void,
            );

            // Draw buffer (render target) and display buffer.
            sys::sceGuDrawBuffer(
                DisplayPixelFormat::Psm8888,
                fbp0_zero,
                BUF_WIDTH as i32,
            );
            sys::sceGuDispBuffer(
                SCREEN_WIDTH as i32,
                SCREEN_HEIGHT as i32,
                fbp1_zero,
                BUF_WIDTH as i32,
            );

            // Viewport and coordinate setup.
            sys::sceGuOffset(
                2048 - (SCREEN_WIDTH / 2),
                2048 - (SCREEN_HEIGHT / 2),
            );
            sys::sceGuViewport(
                2048,
                2048,
                SCREEN_WIDTH as i32,
                SCREEN_HEIGHT as i32,
            );

            // Scissor (full screen).
            sys::sceGuScissor(
                0,
                0,
                SCREEN_WIDTH as i32,
                SCREEN_HEIGHT as i32,
            );
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
            sys::sceGuTexFunc(
                TextureEffect::Modulate,
                TextureColorComponent::Rgba,
            );
            sys::sceGuTexFilter(
                TextureFilter::Nearest,
                TextureFilter::Nearest,
            );

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
            let sys_font_atlas = allocator.alloc_texture_pixels(
                512,
                512,
                TexturePixelFormat::PsmT8,
            );
            if let Ok(atlas_chunk) = sys_font_atlas {
                let vram_ptr = atlas_chunk.as_mut_ptr_direct_to_vram();
                self.system_font =
                    crate::font::SystemFont::try_init(vram_ptr);
                // Silently fall back to bitmap if system fonts unavailable.
            }

            // Claim volatile memory (extra 4MB on PSP-2000+) for textures.
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

            // Open the first frame's display list.
            sys::sceGuStart(
                GuContextType::Direct,
                &raw mut DISPLAY_LIST as *mut c_void,
            );
        }
    }

    /// Set the clipping rectangle via GU scissor.
    pub fn set_clip_rect_inner(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
    ) {
        // SAFETY: sceGuScissor is a GU FFI call operating on the display list.
        unsafe {
            sys::sceGuScissor(x, y, x + w as i32, y + h as i32);
        }
    }

    /// Reset clipping to full screen.
    pub fn reset_clip_rect_inner(&mut self) {
        // SAFETY: sceGuScissor is a GU FFI call operating on the display list.
        unsafe {
            sys::sceGuScissor(
                0,
                0,
                SCREEN_WIDTH as i32,
                SCREEN_HEIGHT as i32,
            );
        }
    }

    /// Finalize the current display list, swap buffers, and open the next
    /// frame.
    pub fn swap_buffers_inner(&mut self) {
        // SAFETY: GU frame lifecycle calls. DISPLAY_LIST is exclusively
        // accessed from the single-threaded main loop (init/swap_buffers).
        unsafe {
            sys::sceGuFinish();
            sys::sceGuSync(GuSyncMode::Finish, GuSyncBehavior::Wait);
            sys::sceDisplayWaitVblankStart();
            sys::sceGuSwapBuffers();
            sys::sceGuStart(
                GuContextType::Direct,
                &raw mut DISPLAY_LIST as *mut c_void,
            );
        }
    }

    /// Current cursor position (for rendering the cursor sprite).
    pub fn cursor_pos(&self) -> (i32, i32) {
        (self.cursor_x, self.cursor_y)
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

impl SdiBackend for PspBackend {
    fn init(&mut self, _width: u32, _height: u32) -> OasisResult<()> {
        // PSP backend initializes during PspBackend::init().
        Ok(())
    }

    fn clear(&mut self, color: Color) -> OasisResult<()> {
        self.clear_inner(color);
        Ok(())
    }

    fn blit(
        &mut self,
        tex: TextureId,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
    ) -> OasisResult<()> {
        self.blit_inner(tex, x, y, w, h);
        Ok(())
    }

    fn fill_rect(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        color: Color,
    ) -> OasisResult<()> {
        self.fill_rect_inner(x, y, w, h, color);
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
        self.draw_text_inner(text, x, y, font_size, color);
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
            .ok_or_else(|| {
                OasisError::Backend("PSP texture allocation failed".into())
            })
    }

    fn destroy_texture(&mut self, tex: TextureId) -> OasisResult<()> {
        self.destroy_texture_inner(tex);
        Ok(())
    }

    fn set_clip_rect(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
    ) -> OasisResult<()> {
        self.set_clip_rect_inner(x, y, w, h);
        Ok(())
    }

    fn reset_clip_rect(&mut self) -> OasisResult<()> {
        self.reset_clip_rect_inner();
        Ok(())
    }

    fn read_pixels(
        &self,
        _x: i32,
        _y: i32,
        _w: u32,
        _h: u32,
    ) -> OasisResult<Vec<u8>> {
        Err(OasisError::Backend(
            "read_pixels not supported on PSP".into(),
        ))
    }

    fn shutdown(&mut self) -> OasisResult<()> {
        Ok(())
    }
}

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
        titlebar_font_size: 8,
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
