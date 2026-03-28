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
mod devloop;
mod font;
mod hook;
mod me_dump;
mod me_watchdog;
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

/// Original VSH function pointers saved BEFORE spy hooks are installed.
/// Used by spy hooks to call through to the real VSH implementation.
static mut ORIG_VSH_CREATE: *mut u8 = core::ptr::null_mut();
static mut ORIG_VSH_DECODE: *mut u8 = core::ptr::null_mut();

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
// The Create and AvcDecode hooks include spy logging for Phase A analysis.

/// sceMpegCreate hook: logs all 7 args + return value, then redirects to VSH.
pub unsafe extern "C" fn hook_create(
    mpeg: *mut core::ffi::c_void,
    data: *mut core::ffi::c_void,
    size: i32,
    rb: *mut core::ffi::c_void,
    fw: i32,
    mode: i32,
    ddr: i32,
) -> i32 {
    let vsh = core::ptr::read_volatile(
        (&raw const VSH_FN).cast::<*mut u8>().add(0)
    );
    if !vsh.is_null() {
        let f: unsafe extern "C" fn(
            *mut core::ffi::c_void, *mut core::ffi::c_void, i32,
            *mut core::ffi::c_void, i32, i32, i32,
        ) -> i32 = core::mem::transmute(vsh);
        let ret = f(mpeg, data, size, rb, fw, mode, ddr);
        // Spy log with [OASIS] tag (this hook is on sceMpeg_library).
        me_dump::spy_log_create(
            mpeg as u32, data as u32, size, rb as u32,
            fw, mode, ddr, ret, b"[OASIS] ",
        );
        return ret;
    }
    -1
}

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

/// Counter for hooked decode calls (diagnostic).
static mut DECODE_HOOK_COUNT: u32 = 0;

/// sceMpegAvcDecode hook: logs args + return value, redirects to VSH.
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
    let init_val = if !init.is_null() {
        core::ptr::read_volatile(init)
    } else {
        -1
    };

    // Diagnostic: log first 3 calls to debug log to verify hook fires.
    let count = core::ptr::read_volatile(&raw const DECODE_HOOK_COUNT);
    core::ptr::write_volatile(&raw mut DECODE_HOOK_COUNT, count + 1);
    if count < 3 {
        let mut m = [0u8; 96];
        let mut p = me_dump::append_bytes(&mut m, 0, b"[SPY-HOOK] Decode #");
        p = me_dump::append_dec(&mut m, p, count);
        p = me_dump::append_bytes(&mut m, p, b" mpeg=0x");
        p = me_dump::append_hex(&mut m, p, mpeg as u32);
        p = me_dump::append_bytes(&mut m, p, b" fw=");
        p = me_dump::append_dec(&mut m, p, frame_width as u32);
        p = me_dump::append_bytes(&mut m, p, b" vsh=0x");
        p = me_dump::append_hex(&mut m, p, vsh as u32);
        debug_log(&m[..p]);
    }

    if !vsh.is_null() {
        let f: unsafe extern "C" fn(
            *mut core::ffi::c_void, *mut core::ffi::c_void, i32,
            *mut core::ffi::c_void, *mut i32,
        ) -> i32 = core::mem::transmute(vsh);
        let ret = f(mpeg, au, frame_width, buffer, init);
        // Spy log with [OASIS] tag.
        me_dump::spy_log_decode(
            mpeg as u32, au as u32, frame_width,
            buffer as u32, init_val, ret, b"[OASIS] ",
        );
        if count < 3 {
            let mut m = [0u8; 48];
            let mut p = me_dump::append_bytes(&mut m, 0,
                b"[SPY-HOOK] ret=0x");
            p = me_dump::append_hex(&mut m, p, ret as u32);
            debug_log(&m[..p]);
        }
        return ret;
    }
    let orig = core::ptr::read_volatile(&raw const ORIGINAL_AVC_DECODE);
    let f: unsafe extern "C" fn(
        *mut core::ffi::c_void, *mut core::ffi::c_void, i32,
        *mut core::ffi::c_void, *mut i32,
    ) -> i32 = core::mem::transmute(orig);
    let ret = f(mpeg, au, frame_width, buffer, init);
    me_dump::spy_log_decode(
        mpeg as u32, au as u32, frame_width,
        buffer as u32, init_val, ret, b"[OASIS] ",
    );
    ret
}

// ---------------------------------------------------------------------------
// VSH spy hooks: installed on sceMpegVsh_library syscalls to capture
// PMPlayer's arguments. These log + pass through to the original function.
// ---------------------------------------------------------------------------

/// Spy hook for sceMpegCreate via sceMpegVsh_library.
/// Logs all args, calls the original VSH function, logs return.
pub unsafe extern "C" fn spy_vsh_create(
    mpeg: *mut core::ffi::c_void,
    data: *mut core::ffi::c_void,
    size: i32,
    rb: *mut core::ffi::c_void,
    fw: i32,
    mode: i32,
    ddr: i32,
) -> i32 {
    let orig = core::ptr::read_volatile(&raw const ORIG_VSH_CREATE);
    if orig.is_null() {
        return -1;
    }
    let f: unsafe extern "C" fn(
        *mut core::ffi::c_void, *mut core::ffi::c_void, i32,
        *mut core::ffi::c_void, i32, i32, i32,
    ) -> i32 = core::mem::transmute(orig);
    let ret = f(mpeg, data, size, rb, fw, mode, ddr);
    // Spy log with [VSH] tag (catches PMPlayer and other VSH callers).
    me_dump::spy_log_create(
        mpeg as u32, data as u32, size, rb as u32,
        fw, mode, ddr, ret, b"[VSH] ",
    );
    ret
}

/// Spy hook for sceMpegAvcDecode via sceMpegVsh_library.
pub unsafe extern "C" fn spy_vsh_decode(
    mpeg: *mut core::ffi::c_void,
    au: *mut core::ffi::c_void,
    frame_width: i32,
    buffer: *mut core::ffi::c_void,
    init: *mut i32,
) -> i32 {
    let init_val = if !init.is_null() {
        core::ptr::read_volatile(init)
    } else {
        -1
    };
    let orig = core::ptr::read_volatile(&raw const ORIG_VSH_DECODE);
    if orig.is_null() {
        return -1;
    }
    let f: unsafe extern "C" fn(
        *mut core::ffi::c_void, *mut core::ffi::c_void, i32,
        *mut core::ffi::c_void, *mut i32,
    ) -> i32 = core::mem::transmute(orig);
    let ret = f(mpeg, au, frame_width, buffer, init);
    // Spy log with [VSH] tag.
    me_dump::spy_log_decode(
        mpeg as u32, au as u32, frame_width,
        buffer as u32, init_val, ret, b"[VSH] ",
    );
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

    // Hook sceMpeg functions at the KERNEL DRIVER level.
    // sceMpeg_library (user-mode) can't be hooked via sctrlHENPatchSyscall
    // — it only patches syscall table entries. The kernel driver
    // "sceMpeg_driver" has the actual syscall implementations that all
    // user-mode wrappers call into.
    //
    // Try module names in order: kernel driver → VSH library → user library.
    // Only kernel-mode entries (0x88xxxxxx) will work with PatchSyscall.
    let hooks: [(u32, *mut u8, &[u8]); 5] = [
        (0xD8C5F121, hook_create as *mut u8, b"Create"),
        (0x167AFD9E, hook_init_au as *mut u8, b"InitAu"),
        (0xA780CF7E, hook_malloc_es as *mut u8, b"MallocEs"),
        (0x11F95CF1, hook_get_nal_au as *mut u8, b"GetNalAu"),
        (0x0E3C2E9D, hooked_avc_decode as *mut u8, b"AvcDecode"),
    ];

    // Module names to try (kernel driver first for reliable syscall hook).
    let modules: [&[u8]; 3] = [
        b"sceMpeg_driver\0",
        b"sceMpegVsh_library\0",
        b"sceMpeg_library\0",
    ];

    let mut hooked = 0u32;
    for &(nid, hook_fn, _name) in &hooks {
        let mut found = None;
        for module in &modules {
            let ptr = psp::hook::find_function(
                module.as_ptr(),
                b"sceMpeg\0".as_ptr(),
                nid,
            );
            if let Some(p) = ptr {
                found = Some(p);
                // Log which module we found it in.
                let addr = p as u32;
                let mut m = [0u8; 80];
                let mut mp = me_dump::append_bytes(&mut m, 0, b"[OASIS] NID 0x");
                mp = me_dump::append_hex(&mut m, mp, nid);
                mp = me_dump::append_bytes(&mut m, mp, b" @0x");
                mp = me_dump::append_hex(&mut m, mp, addr);
                mp = me_dump::append_bytes(&mut m, mp,
                    if addr >= 0x8800_0000 { b" [K]" } else { b" [U]" }
                );
                debug_log(&m[..mp]);
                break;
            }
        }

        if let Some(ptr) = found {
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

    // Extract syscall numbers from VSH stubs while they're accessible
    // (VSH process space 0x0A0A* is only valid during XMB, before game launch).
    // Write them to a file for the EBOOT to use instead of sceMpeg_library.
    {
        let mut syscalls = [0u32; 8];
        let mut sc_count = 0u32;
        for idx in 0..8usize {
            let vsh = core::ptr::read_volatile(
                (&raw const VSH_FN).cast::<*mut u8>().add(idx)
            );
            if vsh.is_null() { continue; }
            let addr = vsh as u32;
            // PSP stub: jr $ra; syscall N  OR  nop; syscall N
            let insn0 = core::ptr::read_volatile(addr as *const u32);
            let insn1 = core::ptr::read_volatile((addr + 4) as *const u32);
            let sc = if (insn1 & 0x3F) == 0x0C {
                (insn1 >> 6) & 0xFFFFF
            } else if (insn0 & 0x3F) == 0x0C {
                (insn0 >> 6) & 0xFFFFF
            } else { 0 };
            syscalls[idx] = sc;
            if sc != 0 { sc_count += 1; }
        }

        let mut m3 = [0u8; 48];
        let mut p3 = me_dump::append_bytes(&mut m3, 0, b"[OASIS] VSH syscalls: ");
        p3 = me_dump::append_dec(&mut m3, p3, sc_count);
        p3 = me_dump::append_bytes(&mut m3, p3, b"/8");
        debug_log(&m3[..p3]);

        // Write syscall numbers to file for EBOOT to read.
        let sc_fd = psp::sys::sceIoOpen(
            b"ms0:/PSP/GAME/OASISOS/.vsh_addrs\0".as_ptr(),
            psp::sys::IoOpenFlags::WR_ONLY
                | psp::sys::IoOpenFlags::CREAT
                | psp::sys::IoOpenFlags::TRUNC,
            0o777,
        );
        if sc_fd >= psp::sys::SceUid(0) {
            psp::sys::sceIoWrite(
                sc_fd,
                syscalls.as_ptr() as *const core::ffi::c_void,
                32,
            );
            psp::sys::sceIoClose(sc_fd);
            debug_log(b"[OASIS] VSH syscalls written to .vsh_addrs");
        }
    }

    // Resolve all 24 sceMpegVsh_library function addresses for EBOOT.
    // NID order matches psp::sys::mpeg_stubs::NIDS.
    // Written to .mpeg_fns (24 x u32 = 96 bytes).
    {
        let all_nids: [u32; 24] = [
            0x682A619B, 0x874624D6, 0xC132E22F, 0xD8C5F121,
            0x606A4649, 0x42560F23, 0x591A4AA2, 0xA780CF7E,
            0xCEB870B1, 0x167AFD9E, 0xFE246728, 0xA11C7026,
            0x0E3C2E9D, 0x740FCCD1, 0x11F95CF1, 0xCF3547A2,
            0x707B7629, 0xD7A29F46, 0x37295ED8, 0x13407F13,
            0xB5F6DC87, 0xB240A59E, 0x21FF80E4, 0x611E9E11,
        ];
        let mut addrs24 = [0u32; 24];
        let mut resolved24 = 0u32;
        for (i, &nid) in all_nids.iter().enumerate() {
            let ptr = psp::hook::find_function(
                b"sceMpegVsh_library\0".as_ptr(),
                b"sceMpeg\0".as_ptr(),
                nid,
            );
            if let Some(p) = ptr {
                addrs24[i] = p as u32;
                resolved24 += 1;
            }
        }
        let mut m4 = [0u8; 48];
        let mut p4 = me_dump::append_bytes(&mut m4, 0, b"[OASIS] mpeg_fns: ");
        p4 = me_dump::append_dec(&mut m4, p4, resolved24);
        p4 = me_dump::append_bytes(&mut m4, p4, b"/24");
        debug_log(&m4[..p4]);

        let fn_fd = psp::sys::sceIoOpen(
            b"ms0:/PSP/GAME/OASISOS/.mpeg_fns\0".as_ptr(),
            psp::sys::IoOpenFlags::WR_ONLY
                | psp::sys::IoOpenFlags::CREAT
                | psp::sys::IoOpenFlags::TRUNC,
            0o777,
        );
        if fn_fd >= psp::sys::SceUid(0) {
            psp::sys::sceIoWrite(
                fn_fd,
                addrs24.as_ptr() as *const core::ffi::c_void,
                96,
            );
            psp::sys::sceIoClose(fn_fd);
            debug_log(b"[OASIS] .mpeg_fns written (96 bytes)");
        }
    }

    // Phase A: Install spy hooks on sceMpegVsh_library's Create and
    // AvcDecode syscalls. These capture PMPlayer's arguments when it
    // calls through VSH (log + pass through to original).
    // We saved VSH_FN[0] and VSH_FN[6] above — use those as originals.
    let vsh_create = core::ptr::read_volatile(
        (&raw const VSH_FN).cast::<*mut u8>().add(0)
    );
    let vsh_decode = core::ptr::read_volatile(
        (&raw const VSH_FN).cast::<*mut u8>().add(6)
    );

    if !vsh_create.is_null() {
        ORIG_VSH_CREATE = vsh_create;
        let ret = psp::sys::sctrlHENPatchSyscall(
            vsh_create, spy_vsh_create as *mut u8,
        );
        if ret >= 0 {
            debug_log(b"[OASIS] VSH Create spy hook installed");
        }
    }

    if !vsh_decode.is_null() {
        ORIG_VSH_DECODE = vsh_decode;
        let ret = psp::sys::sctrlHENPatchSyscall(
            vsh_decode, spy_vsh_decode as *mut u8,
        );
        if ret >= 0 {
            debug_log(b"[OASIS] VSH Decode spy hook installed");
        }
    }
}

fn psp_main() {
    debug_log(b"[OASIS] psp_main entered");

    // Load configuration from ms0:/seplugins/oasis.ini
    config::load_config();
    debug_log(b"[OASIS] config loaded");

    // Start remote development loop if enabled in config.
    // When devloop is active, skip ALL hooks (display, ctrl, spy, ME watchdog)
    // to avoid XMB crashes. Only run the devloop command processor.
    if config::get_config().devloop {
        debug_log(b"[OASIS] devloop mode - skipping hooks");
        devloop::start();
        // Park the main thread — devloop runs in its own thread.
        loop {
            unsafe { psp::sys::sceKernelDelayThread(10_000_000) };
        }
    }

    // Install ME decode watchdog (hook WaitEventFlag with timeout).
    debug_log(b"[OASIS] installing ME watchdog...");
    me_watchdog::install();

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
