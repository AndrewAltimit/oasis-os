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
//! Target: <72KB total (code + data). No heap allocator -- stack + static
//! buffers only. PIP video buffers (~113KB) allocated on-demand from
//! user-memory partition 2.

#![no_std]
#![no_main]
psp::module_kernel!("OasisPlugin", 1, 0);

mod audio;
mod config;
mod font;
mod hook;
mod me_dump;
// me_hook and me_rpc disabled — their static buffers + me_boot_modules
// caused the PRX to crash the EBOOT at launch. Need lighter approach.
// mod me_hook;
// mod me_rpc;
mod overlay;
mod render;
mod video;

use core::sync::atomic::{AtomicBool, Ordering};

/// Global flag: plugin is active and hooks are installed.
static PLUGIN_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Write a debug message to ms0:/seplugins/oasis_debug.txt (append mode).
fn debug_log(msg: &[u8]) {
    // SAFETY: sceIo calls with valid null-terminated path and buffer.
    unsafe {
        let fd = psp::sys::sceIoOpen(
            b"ms0:/seplugins/oasis_debug.txt\0".as_ptr(),
            psp::sys::IoOpenFlags::APPEND
                | psp::sys::IoOpenFlags::CREAT
                | psp::sys::IoOpenFlags::WR_ONLY,
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

        // Start background audio thread (always -- handles on-demand
        // playback from the overlay menu even when autoplay is off).
        audio::start_audio_thread();

        // Start video thread (idles until first PIP menu command,
        // then scans for .rgb files and enters playback loop).
        // Must be created here in psp_main where kernel syscalls work --
        // the display hook context does not support sceKernelCreateThread.
        video::start_video_thread();

        // Start ME firmware dump thread (idles until overlay triggers it).
        me_dump::start_dump_thread();

        // Boot the Media Engine for codec use (sceMeBootStart660).
        // This is required before sceMpegGetAvcNalAu can feed NALs.
        // NID 0x5DFF5C50 = sceMeBootStart for FW 6.60/6.61.
        debug_log(b"[OASIS] calling sceMeBootStart...");
        unsafe {
            let ptr = psp::hook::find_function(
                b"sceMeCodecWrapper\0".as_ptr(),
                b"sceMeCore_driver\0".as_ptr(),
                0x5DFF5C50,
            );
            if let Some(fn_ptr) = ptr {
                let boot_fn: unsafe extern "C" fn(i32) -> i32 =
                    core::mem::transmute(fn_ptr);
                // Try mode 4 first (from our disassembly — loads full codec firmware).
                // cooleyesBridge used mode 1, but NID 0x5DFF5C50 accepts 1-4.
                let ret = boot_fn(4);
                let mut msg = [0u8; 48];
                let mut p = 0;
                let prefix = b"[OASIS] MeBootStart=0x";
                msg[..prefix.len()].copy_from_slice(prefix);
                p = prefix.len();
                let hex = b"0123456789ABCDEF";
                for shift in (0..8).rev() {
                    msg[p] = hex[((ret as u32 >> (shift * 4)) & 0xF) as usize];
                    p += 1;
                }
                debug_log(&msg[..p]);
            } else {
                debug_log(b"[OASIS] sceMeBootStart not found");
            }
        }

        // If pip_enabled is set in config, send the initial toggle command
        // so PIP starts automatically.
        let cfg = config::get_config();
        if cfg.pip_enabled {
            video::toggle_pip();
        }
    } else {
        debug_log(b"[OASIS] hook install FAILED");
    }

    // Keep the plugin thread alive (it does nothing after setup --
    // all work happens in the display hook and audio thread).
    loop {
        // SAFETY: Sleep for ~1 second to avoid busy-waiting.
        unsafe {
            psp::sys::sceKernelDelayThread(1_000_000);
        }
    }
}
