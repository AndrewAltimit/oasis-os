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

/// Resolve sceMpeg functions from the VSH library and write to file.
///
/// The VSH's sceMpegVsh_library exports the same sceMpeg NIDs but
/// connects to the ME through the correct kernel path. We resolve
/// these at runtime and write the addresses for the EBOOT to use.
///
/// Also boots the ME via sceMeBootStart.
unsafe fn resolve_vsh_mpeg_functions() {
    // NIDs we need to resolve from sceMpegVsh_library.
    // Format: (NID, name for logging)
    let nids: [(u32, &[u8]); 12] = [
        (0x682A619B, b"Init"),
        (0x874624D6, b"Finish"),
        (0xC132E22F, b"QueryMemSize"),
        (0xD8C5F121, b"Create"),
        (0x606A4649, b"Delete"),
        (0x167AFD9E, b"InitAu"),
        (0xA780CF7E, b"MallocEsBuf"),
        (0xCEB870B1, b"FreeEsBuf"),
        (0x11F95CF1, b"GetNalAu"),
        (0x0E3C2E9D, b"AvcDecode"),
        (0xCF3547A2, b"DecDetail2"),
        (0xA11C7026, b"DecMode"),
    ];

    let mut addrs = [0u32; 12];
    let mut resolved = 0u32;

    for (i, &(nid, _name)) in nids.iter().enumerate() {
        let ptr = psp::hook::find_function(
            b"sceMpegVsh_library\0".as_ptr(),
            b"sceMpeg\0".as_ptr(),
            nid,
        );
        if let Some(p) = ptr {
            addrs[i] = p as u32;
            resolved += 1;
        }
    }

    // Also resolve sceMpegBaseCscAvc from sceMpegbase.
    let csc_ptr = psp::hook::find_function(
        b"sceMpegVsh_library\0".as_ptr(),
        b"sceMpegbase\0".as_ptr(),
        0x91929A21, // sceMpegBaseCscAvc
    );

    // Log results.
    let mut msg = [0u8; 48];
    let mut mp = me_dump::append_bytes(&mut msg, 0, b"[OASIS] VSH mpeg: ");
    mp = me_dump::append_dec(&mut msg, mp, resolved);
    mp = me_dump::append_bytes(&mut msg, mp, b"/12 resolved");
    debug_log(&msg[..mp]);

    // Write addresses to file for EBOOT.
    let fd = psp::sys::sceIoOpen(
        b"ms0:/PSP/GAME/OASISOS/.me_vsh_addrs\0".as_ptr(),
        psp::sys::IoOpenFlags::WR_ONLY
            | psp::sys::IoOpenFlags::CREAT
            | psp::sys::IoOpenFlags::TRUNC,
        0o777,
    );
    if fd >= psp::sys::SceUid(0) {
        // Write 12 u32 addresses + 1 u32 for CscAvc = 52 bytes.
        psp::sys::sceIoWrite(
            fd,
            addrs.as_ptr() as *const _,
            48, // 12 * 4
        );
        let csc_addr = csc_ptr.map(|p| p as u32).unwrap_or(0);
        psp::sys::sceIoWrite(
            fd,
            &csc_addr as *const u32 as *const _,
            4,
        );
        psp::sys::sceIoClose(fd);
        debug_log(b"[OASIS] VSH addrs written");
    }

    // Boot the ME.
    let boot_ptr = psp::hook::find_function(
        b"sceMeCodecWrapper\0".as_ptr(),
        b"sceMeCore_driver\0".as_ptr(),
        0x5DFF5C50,
    );
    if let Some(fn_ptr) = boot_ptr {
        let boot_fn: unsafe extern "C" fn(i32) -> i32 =
            core::mem::transmute(fn_ptr);
        let ret = boot_fn(1);
        let mut m2 = [0u8; 48];
        let mut p2 = me_dump::append_bytes(&mut m2, 0, b"[OASIS] MeBoot=0x");
        p2 = me_dump::append_hex(&mut m2, p2, ret as u32);
        debug_log(&m2[..p2]);
    } else {
        debug_log(b"[OASIS] MeBoot fn not found");
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

        // Resolve sceMpeg functions from sceMpegVsh_library (loaded by VSH)
        // and write addresses to a file for the EBOOT to read.
        // Also boot the ME for codec use.
        debug_log(b"[OASIS] resolving VSH mpeg + ME boot...");
        unsafe { resolve_vsh_mpeg_functions() };

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
