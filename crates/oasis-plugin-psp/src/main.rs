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

/// Original sceMpegAvcDecode function pointer (for chaining).
pub static mut ORIGINAL_AVC_DECODE: *mut u8 = core::ptr::null_mut();

/// Resolved VSH function pointers for ALL critical sceMpeg functions.
static mut VSH_FN: [*mut u8; 8] = [core::ptr::null_mut(); 8];
// Index: 0=Create, 1=Delete, 2=InitAu, 3=MallocEsBuf, 4=FreeEsBuf,
//        5=GetAvcNalAu, 6=AvcDecode, 7=QueryMemSize

/// Generic hook that redirects any sceMpeg function to its VSH equivalent.
/// Each hook function reads the VSH pointer from VSH_FN[index].
macro_rules! make_vsh_hook {
    ($name:ident, $idx:expr, ($($arg:ident: $ty:ty),*) -> $ret:ty) => {
        pub unsafe extern "C" fn $name($($arg: $ty),*) -> $ret {
            let vsh = core::ptr::read_volatile(
                (&raw const VSH_FN).cast::<*mut u8>().add($idx)
            );
            if !vsh.is_null() {
                let f: unsafe extern "C" fn($($ty),*) -> $ret =
                    core::mem::transmute(vsh);
                return f($($arg),*);
            }
            -1 as $ret // VSH not available
        }
    };
}

// Hook functions for each sceMpeg function.
// These get installed via sctrlHENPatchSyscall on the sceMpeg_library entries.
make_vsh_hook!(hook_create, 0,
    (mpeg: *mut core::ffi::c_void, data: *mut core::ffi::c_void,
     size: i32, rb: *mut core::ffi::c_void, fw: i32, mode: i32, ddr: i32) -> i32);
make_vsh_hook!(hook_init_au, 2,
    (mpeg: *mut core::ffi::c_void, es: *mut core::ffi::c_void,
     au: *mut core::ffi::c_void) -> i32);
pub unsafe extern "C" fn hook_malloc_es(
    mpeg: *mut core::ffi::c_void,
) -> *mut core::ffi::c_void {
    let vsh = core::ptr::read_volatile(
        (&raw const VSH_FN).cast::<*mut u8>().add(3)
    );
    if !vsh.is_null() {
        let f: unsafe extern "C" fn(*mut core::ffi::c_void) -> *mut core::ffi::c_void =
            core::mem::transmute(vsh);
        return f(mpeg);
    }
    core::ptr::null_mut()
}
make_vsh_hook!(hook_get_nal_au, 5,
    (mpeg: *mut core::ffi::c_void, nal: *mut core::ffi::c_void,
     au: *mut core::ffi::c_void) -> i32);

pub unsafe extern "C" fn hooked_avc_decode(
    mpeg: *mut core::ffi::c_void,
    au: *mut core::ffi::c_void,
    frame_width: i32,
    buffer: *mut core::ffi::c_void,
    init: *mut i32,
) -> i32 {
    let vsh = core::ptr::read_volatile(
        (&raw const VSH_FN).cast::<*mut u8>().add(6)
    );
    if !vsh.is_null() {
        let f: unsafe extern "C" fn(
            *mut core::ffi::c_void, *mut core::ffi::c_void, i32,
            *mut core::ffi::c_void, *mut i32,
        ) -> i32 = core::mem::transmute(vsh);
        return f(mpeg, au, frame_width, buffer, init);
    }
    let orig: unsafe extern "C" fn(
        *mut core::ffi::c_void, *mut core::ffi::c_void, i32,
        *mut core::ffi::c_void, *mut i32,
    ) -> i32 = core::mem::transmute(ORIGINAL_AVC_DECODE);
    orig(mpeg, au, frame_width, buffer, init)
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

    // Resolve ALL critical sceMpeg functions from sceMpegVsh_library.
    let vsh_nids: [(usize, u32, &[u8]); 8] = [
        (0, 0xD8C5F121, b"Create"),       // sceMpegCreate
        (1, 0x606A4649, b"Delete"),        // sceMpegDelete
        (2, 0x167AFD9E, b"InitAu"),        // sceMpegInitAu
        (3, 0xA780CF7E, b"MallocEs"),      // sceMpegMallocAvcEsBuf
        (4, 0xCEB870B1, b"FreeEs"),        // sceMpegFreeAvcEsBuf
        (5, 0x11F95CF1, b"GetNalAu"),      // sceMpegGetAvcNalAu
        (6, 0x0E3C2E9D, b"AvcDecode"),     // sceMpegAvcDecode
        (7, 0xC132E22F, b"QueryMem"),      // sceMpegQueryMemSize
    ];

    let mut vsh_resolved = 0u32;
    for &(idx, nid, _name) in &vsh_nids {
        let ptr = psp::hook::find_function(
            b"sceMpegVsh_library\0".as_ptr(),
            b"sceMpeg\0".as_ptr(),
            nid,
        );
        if let Some(p) = ptr {
            VSH_FN[idx] = p;
            vsh_resolved += 1;
        }
    }

    let mut m = [0u8; 48];
    let mut p = me_dump::append_bytes(&mut m, 0, b"[OASIS] VSH fns: ");
    p = me_dump::append_dec(&mut m, p, vsh_resolved);
    p = me_dump::append_bytes(&mut m, p, b"/8");
    debug_log(&m[..p]);

    if vsh_resolved < 6 {
        debug_log(b"[OASIS] not enough VSH fns, skipping hooks");
        return;
    }

    // Hook ALL sceMpeg functions in sceMpeg_library to redirect to VSH.
    // NID → (hook function, name)
    let hooks: [(u32, *mut u8, &[u8]); 5] = [
        (0xD8C5F121, hook_create as *mut u8, b"Create"),
        (0x167AFD9E, hook_init_au as *mut u8, b"InitAu"),
        (0xA780CF7E, hook_malloc_es as *mut u8, b"MallocEs"),
        (0x11F95CF1, hook_get_nal_au as *mut u8, b"GetNalAu"),
        (0x0E3C2E9D, hooked_avc_decode as *mut u8, b"AvcDecode"),
    ];

    let mut hooked = 0u32;
    for &(nid, hook_fn, name) in &hooks {
        // Find in sceMpeg_library first, then VSH.
        let orig = psp::hook::find_function(
            b"sceMpeg_library\0".as_ptr(),
            b"sceMpeg\0".as_ptr(),
            nid,
        ).or_else(|| psp::hook::find_function(
            b"sceMpegVsh_library\0".as_ptr(),
            b"sceMpeg\0".as_ptr(),
            nid,
        ));

        if let Some(ptr) = orig {
            if nid == 0x0E3C2E9D {
                ORIGINAL_AVC_DECODE = ptr;
            }
            let ret = psp::sys::sctrlHENPatchSyscall(ptr, hook_fn);
            if ret >= 0 {
                hooked += 1;
            }
        }
    }

    let mut m2 = [0u8; 48];
    let mut p2 = me_dump::append_bytes(&mut m2, 0, b"[OASIS] hooked ");
    p2 = me_dump::append_dec(&mut m2, p2, hooked);
    p2 = me_dump::append_bytes(&mut m2, p2, b"/5 sceMpeg fns");
    debug_log(&m2[..p2]);
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
