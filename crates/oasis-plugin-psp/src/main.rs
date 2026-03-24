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

/// Resolved sceMeVideo_driver::Decode function pointer.
static mut ME_VIDEO_DECODE_FN: Option<unsafe extern "C" fn(i32, *mut u32) -> i32> = None;

/// Original sceMpegAvcDecode function pointer (for chaining).
static mut ORIGINAL_AVC_DECODE: *mut u8 = core::ptr::null_mut();

/// Our hook for sceMpegAvcDecode.
///
/// When the EBOOT calls sceMpegAvcDecode, this intercept runs instead.
/// We call the original first — if it returns AVC_DECODE_FATAL (0x80628002),
/// we try calling sceMeVideo_driver::Decode directly as a fallback.
unsafe extern "C" fn hooked_avc_decode(
    mpeg: *mut core::ffi::c_void,
    au: *mut core::ffi::c_void,
    frame_width: i32,
    buffer: *mut core::ffi::c_void,
    init: *mut i32,
) -> i32 {
    // Call the original sceMpegAvcDecode.
    let original: unsafe extern "C" fn(
        *mut core::ffi::c_void, *mut core::ffi::c_void, i32,
        *mut core::ffi::c_void, *mut i32,
    ) -> i32 = core::mem::transmute(ORIGINAL_AVC_DECODE);
    let ret = original(mpeg, au, frame_width, buffer, init);

    // If it succeeds, great — return the result.
    if ret >= 0 {
        return ret;
    }

    // If FATAL (0x80628002), try ME direct decode.
    if ret == 0x80628002_u32 as i32 {
        if let Some(me_decode) = core::ptr::read_volatile(&raw const ME_VIDEO_DECODE_FN) {
            // sceMeVideo::Decode takes (sub_cmd, codec_buf_ptr).
            // The codec_buf is inside the sceMpeg AU structure.
            // For now, just call with the AU pointer — the ME driver
            // should extract what it needs.
            // TODO: map sceMpeg AU → sceMeVideo codec_buf correctly
            return me_decode(0x26, au as *mut u32); // cmd 0x26 = decode
        }
    }

    ret
}

/// Boot the ME and hook sceMpegAvcDecode syscall.
unsafe fn setup_me_decode_hook() {
    debug_log(b"[OASIS] setting up ME decode hook...");

    // Boot the ME.
    let boot_ptr = psp::hook::find_function(
        b"sceMeCodecWrapper\0".as_ptr(),
        b"sceMeCore_driver\0".as_ptr(),
        0x5DFF5C50, // sceMeBootStart660
    );
    if let Some(fn_ptr) = boot_ptr {
        let boot_fn: unsafe extern "C" fn(i32) -> i32 =
            core::mem::transmute(fn_ptr);
        let ret = boot_fn(1);
        let mut m = [0u8; 48];
        let mut p = me_dump::append_bytes(&mut m, 0, b"[OASIS] MeBoot=0x");
        p = me_dump::append_hex(&mut m, p, ret as u32);
        debug_log(&m[..p]);
    } else {
        debug_log(b"[OASIS] MeBoot fn not found");
    }

    // Resolve sceMeVideo_driver::Decode.
    let decode_ptr = psp::hook::find_function(
        b"sceMeCodecWrapper\0".as_ptr(),
        b"sceMeVideo_driver\0".as_ptr(),
        0x6D68B223, // Decode function
    );
    if let Some(ptr) = decode_ptr {
        ME_VIDEO_DECODE_FN = Some(core::mem::transmute(ptr));
        debug_log(b"[OASIS] MeVideoDecode resolved");
    } else {
        debug_log(b"[OASIS] MeVideoDecode NOT found");
        return;
    }

    // Find sceMpegAvcDecode in the loaded sceMpeg_library.
    // NID 0x0E3C2E9D
    let avc_decode_ptr = psp::hook::find_function(
        b"sceMpeg_library\0".as_ptr(),
        b"sceMpeg\0".as_ptr(),
        0x0E3C2E9D,
    );
    if let Some(ptr) = avc_decode_ptr {
        ORIGINAL_AVC_DECODE = ptr;
        let mut m = [0u8; 48];
        let mut p = me_dump::append_bytes(&mut m, 0, b"[OASIS] AvcDecode @0x");
        p = me_dump::append_hex(&mut m, p, ptr as u32);
        debug_log(&m[..p]);

        // Hook the syscall.
        let ret = psp::sys::sctrlHENPatchSyscall(
            ptr,
            hooked_avc_decode as *mut u8,
        );
        let mut m2 = [0u8; 48];
        let mut p2 = me_dump::append_bytes(&mut m2, 0, b"[OASIS] hook ret=0x");
        p2 = me_dump::append_hex(&mut m2, p2, ret as u32);
        debug_log(&m2[..p2]);

        if ret >= 0 {
            debug_log(b"[OASIS] AvcDecode HOOKED!");
        }
    } else {
        // Try sceMpegVsh_library.
        let vsh_ptr = psp::hook::find_function(
            b"sceMpegVsh_library\0".as_ptr(),
            b"sceMpeg\0".as_ptr(),
            0x0E3C2E9D,
        );
        if let Some(ptr) = vsh_ptr {
            ORIGINAL_AVC_DECODE = ptr;
            debug_log(b"[OASIS] found AvcDecode in VSH lib");
            let ret = psp::sys::sctrlHENPatchSyscall(
                ptr,
                hooked_avc_decode as *mut u8,
            );
            if ret >= 0 {
                debug_log(b"[OASIS] AvcDecode HOOKED (VSH)!");
            }
        } else {
            debug_log(b"[OASIS] AvcDecode NOT found in any lib");
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

        // Boot ME and hook sceMpegAvcDecode to use sceMeVideo_driver.
        unsafe { setup_me_decode_hook() };

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
