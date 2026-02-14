//! OASIS Plugin PRX -- kernel-mode PSP plugin for in-game overlay + background
//! music.
//!
//! This is a companion module to the main OASIS_OS EBOOT. It compiles to a
//! relocatable PRX that CFW (ARK-4/PRO) loads via `PLUGINS.TXT` and keeps
//! resident in kernel memory alongside games.
//!
//! ## Architecture
//!
//! - Hooks `sceDisplaySetFrameBuf` to draw overlay UI on top of the game's
//!   framebuffer after each vsync
//! - Claims one PSP audio channel for background MP3 playback via the
//!   Media Engine coprocessor
//! - Reads config from `ms0:/seplugins/oasis.ini`
//! - Triggered by NOTE button (kernel-only, 0x800000)
//!
//! ## Memory Budget
//!
//! Target: <64KB total (code + data). No heap allocator -- stack + static
//! buffers only.

#![feature(restricted_std)]
#![no_main]
#![feature(asm_experimental_arch)]

psp::module_kernel!("OasisPlugin", 1, 0);

mod audio;
mod config;
mod font;
mod hook;
mod overlay;
mod render;

use core::sync::atomic::{AtomicBool, Ordering};

/// Global flag: plugin is active and hooks are installed.
static PLUGIN_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Write a debug message to ms0:/seplugins/oasis_debug.txt (append mode).
fn debug_log(msg: &[u8]) {
    // SAFETY: sceIo calls with valid null-terminated path and buffer.
    unsafe {
        let fd = psp::sys::sceIoOpen(
            b"ms0:/seplugins/oasis_debug.txt\0".as_ptr(),
            psp::sys::IoOpenFlags::APPEND | psp::sys::IoOpenFlags::CREAT | psp::sys::IoOpenFlags::WR_ONLY,
            0o777,
        );
        if fd >= psp::sys::SceUid(0) {
            psp::sys::sceIoWrite(fd, msg.as_ptr() as *const _, msg.len());
            psp::sys::sceIoWrite(fd, b"\n".as_ptr() as *const _, 1);
            psp::sys::sceIoClose(fd);
        }
    }
}

fn psp_main() {
    debug_log(b"[OASIS] psp_main entered");

    // Load configuration from ms0:/seplugins/oasis.ini
    config::load_config();
    debug_log(b"[OASIS] config loaded");

    // Install the display framebuffer hook
    debug_log(b"[OASIS] installing display hook...");
    if hook::install_display_hook() {
        PLUGIN_ACTIVE.store(true, Ordering::Release);
        debug_log(b"[OASIS] hook installed OK");

        // Start background audio thread if autoplay is enabled
        if config::get_config().autoplay {
            audio::start_audio_thread();
        }
    } else {
        debug_log(b"[OASIS] hook install FAILED");
    }

    // Also log what sctrlHENFindFunction returns for diagnostics
    hook::log_find_function_result();

    // Keep the plugin thread alive (it does nothing after setup --
    // all work happens in the display hook and audio thread).
    loop {
        // SAFETY: Sleep for ~1 second to avoid busy-waiting.
        unsafe {
            psp::sys::sceKernelDelayThread(1_000_000);
        }
    }
}
