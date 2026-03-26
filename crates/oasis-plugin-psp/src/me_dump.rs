//! ME firmware dump module.
//!
//! Dumps loaded kernel modules (mpeg/avcodec/codec) and EDRAM memory from
//! a running game. The display hook's menu triggers an atomic flag, and a
//! dedicated kernel thread performs the actual I/O.
//!
//! ## Purpose
//!
//! During game execution, `avcodec.prx` ME submission stubs are populated
//! (games use them for H.264 decode). Dumping avcodec.prx at runtime lets
//! us compare against the homebrew-time dump (where stubs are empty) and
//! reverse-engineer the ME command protocol.
//!
//! The 2MB EDRAM dump (0x04000000) captures the ME firmware binary that
//! gets loaded during sceMpegCreate.

use core::sync::atomic::{AtomicU8, Ordering};

/// Dump command: 0=idle, 1=requested, 2=in-progress, 3=done.
static DUMP_STATE: AtomicU8 = AtomicU8::new(0);

/// Request a firmware dump (called from overlay menu).
pub fn trigger_dump() {
    // Only trigger if idle.
    let _ = DUMP_STATE.compare_exchange(0, 1, Ordering::AcqRel, Ordering::Relaxed);
}

/// Check if dump is in progress or completed. Returns status message.
pub fn dump_status() -> &'static [u8] {
    match DUMP_STATE.load(Ordering::Relaxed) {
        0 => b"Ready",
        1 => b"Queued...",
        2 => b"Dumping...",
        3 => b"Done!",
        _ => b"???",
    }
}

/// Start the dump thread. Called once from psp_main.
pub fn start_dump_thread() {
    // SAFETY: Creating a kernel thread for ME firmware dump.
    unsafe {
        let thid = psp::sys::sceKernelCreateThread(
            b"OasisMEDump\0".as_ptr(),
            dump_thread_entry,
            0x20, // low priority (background)
            0x10000, // 64KB stack (needs room for file copy buffer)
            psp::sys::ThreadAttributes::empty(),
            core::ptr::null_mut(),
        );
        if thid.0 >= 0 {
            psp::sys::sceKernelStartThread(thid, 0, core::ptr::null_mut());
            crate::debug_log(b"[ME-DUMP] thread started");
        } else {
            crate::debug_log(b"[ME-DUMP] thread create FAILED");
        }
    }
}

/// Auto-flush counter: flush spy log every ~5 seconds (50 ticks at 10Hz).
static mut SPY_FLUSH_CTR: u32 = 0;

/// Dump thread entry point. Polls for trigger, then dumps.
unsafe extern "C" fn dump_thread_entry(
    _args: usize,
    _argp: *mut core::ffi::c_void,
) -> i32 {
    // Load cooleyesBridge.prx early — PMPlayer requires it before mpeg_vsh.
    unsafe { load_cooleyes_bridge() };

    // NOTE: Kernel K1 patches previously here have been REMOVED.
    // The root cause (EABI32 vs O32 ABI mismatch) is fixed in rust-psp
    // by adding i5/i6/i7 EABI mappers to 5+ arg PSP functions.
    // See docs/psp-child-module-investigation.md for the full story.

    loop {
        // Wait for trigger.
        if DUMP_STATE.compare_exchange(
            1, 2, Ordering::AcqRel, Ordering::Relaxed
        ).is_ok() {
            // SAFETY: All dump functions use kernel-mode APIs.
            unsafe { do_dump() };
            DUMP_STATE.store(3, Ordering::Release);

            // Reset to idle after 5 seconds so menu shows "Done!" briefly.
            // SAFETY: PSP kernel syscall.
            unsafe { psp::sys::sceKernelDelayThread(5_000_000) };
            DUMP_STATE.store(0, Ordering::Release);
        }

        // Check for ME decode hook request from EBOOT (file-based IPC).
        // EBOOT writes ".me_patch" after loading AV modules, we hook
        // sceMpegAvcDecode syscall and write ".me_patched" when done.
        unsafe { check_patch_request() };

        // Check for VSH module load + address resolution request from EBOOT.
        unsafe { check_vsh_load_request() };
        unsafe { check_vsh_addr_request() };

        // Check for spy dump request from overlay menu.
        if SPY_DUMP_STATE.compare_exchange(
            1, 0, Ordering::AcqRel, Ordering::Relaxed
        ).is_ok() {
            crate::debug_log(b"[SPY] menu dump triggered");
            unsafe {
                auto_flush_spy();
                dump_all_modules_to_file();
            }
        }

        // Auto-flush spy log every ~5 seconds if there's data.
        // Also auto-dump modules once ~30s after boot for comparison.
        unsafe {
            let ctr = core::ptr::read_volatile(&raw const SPY_FLUSH_CTR);
            core::ptr::write_volatile(&raw mut SPY_FLUSH_CTR, ctr + 1);
            if ctr % 50 == 49 {
                auto_flush_spy();
            }
            // Auto-dump modules once ~30s after boot (300 ticks at 10Hz).
            if ctr == 300 {
                crate::debug_log(b"[SPY] auto module dump");
                dump_all_modules_to_file();
            }
        }

        // SAFETY: PSP kernel syscall to sleep.
        unsafe { psp::sys::sceKernelDelayThread(100_000) }; // poll at 10Hz
    }
}

/// Create dump output directory.
unsafe fn ensure_dump_dir() {
    // SAFETY: sceIoMkdir with valid path. Ignores error if dir exists.
    unsafe {
        let _ = psp::sys::sceIoMkdir(
            b"ms0:/seplugins/me_dump\0".as_ptr(),
            0o777,
        );
    }
}

/// Write a buffer to a file on the memory stick.
unsafe fn write_file(path: &[u8], data: &[u8]) -> bool {
    // SAFETY: sceIo calls with valid pointers.
    unsafe {
        let fd = psp::sys::sceIoOpen(
            path.as_ptr(),
            psp::sys::IoOpenFlags::WR_ONLY
                | psp::sys::IoOpenFlags::CREAT
                | psp::sys::IoOpenFlags::TRUNC,
            0o777,
        );
        if fd < psp::sys::SceUid(0) {
            return false;
        }

        // Write in chunks (large writes may fail on FAT32).
        let chunk = 32 * 1024;
        let mut offset = 0;
        while offset < data.len() {
            let end = (offset + chunk).min(data.len());
            let n = psp::sys::sceIoWrite(
                fd,
                data[offset..end].as_ptr() as *const _,
                end - offset,
            );
            if n <= 0 {
                break;
            }
            offset += n as usize;
        }
        psp::sys::sceIoClose(fd);
        offset == data.len()
    }
}

/// Perform the full dump sequence.
unsafe fn do_dump() {
    crate::debug_log(b"[ME-DUMP] starting dump...");
    // SAFETY: Creating output directory.
    unsafe { ensure_dump_dir() };

    // 1. Extract ME firmware images from flash0 (kernel mode can access kd/).
    // SAFETY: sceIo calls from kernel thread context.
    unsafe { extract_me_firmware() };

    // 2. Dump all loaded kernel modules (binaries + text index).
    // SAFETY: Kernel module enumeration and memory read.
    unsafe { dump_kernel_modules() };

    // 3. Probe for codec driver functions via sctrlHENFindFunction.
    // SAFETY: CFW API calls.
    unsafe { probe_codec_drivers() };

    crate::debug_log(b"[ME-DUMP] dump complete!");
}

/// Extract ME firmware images from flash0:/kd/resource/ to memory stick.
///
/// These are the encrypted firmware binaries that sceMeCodecWrapper loads
/// onto the Media Engine during codec initialization.
unsafe fn extract_me_firmware() {
    crate::debug_log(b"[ME-DUMP] extracting ME firmware...");

    // List flash0:/kd/resource/ to find actual filenames.
    unsafe { list_kd_resource() };

    // Also list flash0:/kd/ to find all kernel modules.
    unsafe { list_dir(b"flash0:/kd/\0", b"kd") };

    // Copy ME firmware image (only me_t2img.img exists on FW 6.61).
    crate::debug_log(b"[ME-DUMP] fw: me_t2img.img");
    unsafe {
        copy_file(
            b"flash0:/kd/resource/me_t2img.img\0",
            b"ms0:/seplugins/me_dump/me_t2img.img\0",
        );
    }

    // Copy all codec-related PRX files from flash0:/kd/.
    crate::debug_log(b"[ME-DUMP] fw: me_wrapper.prx");
    unsafe {
        copy_file(
            b"flash0:/kd/me_wrapper.prx\0",
            b"ms0:/seplugins/me_dump/me_wrapper.prx\0",
        );
    }

    crate::debug_log(b"[ME-DUMP] fw: avcodec.prx");
    unsafe {
        copy_file(
            b"flash0:/kd/avcodec.prx\0",
            b"ms0:/seplugins/me_dump/avcodec.prx\0",
        );
    }

    crate::debug_log(b"[ME-DUMP] fw: mpeg.prx");
    unsafe {
        copy_file(
            b"flash0:/kd/mpeg.prx\0",
            b"ms0:/seplugins/me_dump/mpeg.prx\0",
        );
    }

    crate::debug_log(b"[ME-DUMP] fw: mpeg_vsh.prx");
    unsafe {
        copy_file(
            b"flash0:/kd/mpeg_vsh.prx\0",
            b"ms0:/seplugins/me_dump/mpeg_vsh.prx\0",
        );
    }

    crate::debug_log(b"[ME-DUMP] fw: videocodec_260.prx");
    unsafe {
        copy_file(
            b"flash0:/kd/videocodec_260.prx\0",
            b"ms0:/seplugins/me_dump/videocodec_260.prx\0",
        );
    }

    crate::debug_log(b"[ME-DUMP] fw: codec_09g.prx");
    unsafe {
        copy_file(
            b"flash0:/kd/codec_09g.prx\0",
            b"ms0:/seplugins/me_dump/codec_09g.prx\0",
        );
    }

    crate::debug_log(b"[ME-DUMP] fw: mpegbase_260.prx");
    unsafe {
        copy_file(
            b"flash0:/kd/mpegbase_260.prx\0",
            b"ms0:/seplugins/me_dump/mpegbase_260.prx\0",
        );
    }

    crate::debug_log(b"[ME-DUMP] firmware extraction done");
}

/// List a directory and write results to a text file + debug log.
unsafe fn list_dir(path: &[u8], label: &[u8]) {
    let dir_fd = unsafe {
        psp::sys::sceIoDopen(path.as_ptr())
    };
    if dir_fd < psp::sys::SceUid(0) {
        let mut msg = [0u8; 80];
        let mut mp = append_bytes(&mut msg, 0, b"[ME-DUMP] dir fail: ");
        mp = append_bytes(&mut msg, mp, label);
        mp = append_bytes(&mut msg, mp, b" err=0x");
        mp = append_hex(&mut msg, mp, dir_fd.0 as u32);
        crate::debug_log(&msg[..mp]);
        return;
    }

    let mut list = [0u8; 4096];
    let mut lp = 0;
    let mut count = 0u32;

    loop {
        let mut entry: psp::sys::SceIoDirent = unsafe { core::mem::zeroed() };
        let ret = unsafe { psp::sys::sceIoDread(dir_fd, &mut entry) };
        if ret <= 0 {
            break;
        }
        let name_len = entry.d_name.iter().position(|&b| b == 0)
            .unwrap_or(entry.d_name.len());
        let name = &entry.d_name[..name_len];
        let size = entry.d_stat.st_size as u32;

        lp = append_bytes(&mut list, lp, name);
        lp = append_bytes(&mut list, lp, b" (");
        lp = append_dec(&mut list, lp, size);
        lp = append_bytes(&mut list, lp, b")\n");

        // Also log to debug.
        let mut msg = [0u8; 80];
        let mut mp = append_bytes(&mut msg, 0, b"[ME-DUMP] ");
        mp = append_bytes(&mut msg, mp, label);
        mp = append_bytes(&mut msg, mp, b"/ ");
        mp = append_bytes(&mut msg, mp, name);
        mp = append_bytes(&mut msg, mp, b" (");
        mp = append_dec(&mut msg, mp, size);
        mp = append_bytes(&mut msg, mp, b")");
        crate::debug_log(&msg[..mp]);

        count += 1;
        if count > 200 {
            break;
        }
    }
    unsafe { psp::sys::sceIoDclose(dir_fd) };

    // Write listing to file.
    let mut out_path = [0u8; 64];
    let mut op = append_bytes(&mut out_path, 0, b"ms0:/seplugins/me_dump/");
    op = append_bytes(&mut out_path, op, label);
    op = append_bytes(&mut out_path, op, b"_listing.txt\0");
    let _ = op;
    unsafe { write_file(&out_path, &list[..lp]) };
}

/// List flash0:/kd/resource/ specifically.
unsafe fn list_kd_resource() {
    unsafe { list_dir(b"flash0:/kd/resource/\0", b"kd_resource") };
}

/// Copy a file from src to dst using sceIo.
/// Reads in 4KB chunks to avoid stack overflow in kernel thread.
unsafe fn copy_file(src: &[u8], dst: &[u8]) {
    crate::debug_log(b"[ME-DUMP] sceIoOpen src...");

    // SAFETY: Opening source file for reading.
    let src_fd = unsafe {
        psp::sys::sceIoOpen(
            src.as_ptr(),
            psp::sys::IoOpenFlags::RD_ONLY,
            0,
        )
    };

    log_hex(b"[ME-DUMP] src fd=", src_fd.0 as u32);

    if src_fd < psp::sys::SceUid(0) {
        crate::debug_log(b"[ME-DUMP] src open FAILED");
        return;
    }

    crate::debug_log(b"[ME-DUMP] sceIoLseek...");

    // Get file size via seek.
    // SAFETY: sceIoLseek to end and back.
    let file_size = unsafe {
        let end = psp::sys::sceIoLseek(src_fd, 0, psp::sys::IoWhence::End);
        psp::sys::sceIoLseek(src_fd, 0, psp::sys::IoWhence::Set);
        end as usize
    };

    log_dec(b"[ME-DUMP] file size=", file_size as u32);

    if file_size == 0 {
        // SAFETY: Close source fd.
        unsafe { psp::sys::sceIoClose(src_fd) };
        crate::debug_log(b"[ME-DUMP] empty file, skipped");
        return;
    }

    // SAFETY: Opening destination file for writing.
    let dst_fd = unsafe {
        psp::sys::sceIoOpen(
            dst.as_ptr(),
            psp::sys::IoOpenFlags::WR_ONLY
                | psp::sys::IoOpenFlags::CREAT
                | psp::sys::IoOpenFlags::TRUNC,
            0o777,
        )
    };
    if dst_fd < psp::sys::SceUid(0) {
        // SAFETY: Close source fd on error.
        unsafe { psp::sys::sceIoClose(src_fd) };
        crate::debug_log(b"[ME-DUMP] dst open FAILED");
        return;
    }

    // Copy in 4KB chunks (stack buffer, safe with 64KB thread stack).
    let mut buf = [0u8; 4096];
    let mut total_written: usize = 0;

    loop {
        // SAFETY: Reading from flash0 file.
        let n = unsafe {
            psp::sys::sceIoRead(
                src_fd,
                buf.as_mut_ptr() as *mut _,
                buf.len() as u32,
            )
        };
        if n <= 0 {
            break;
        }
        let n = n as usize;

        // SAFETY: Writing to memory stick file.
        let w = unsafe {
            psp::sys::sceIoWrite(
                dst_fd,
                buf.as_ptr() as *const _,
                n,
            )
        };
        if w <= 0 {
            crate::debug_log(b"[ME-DUMP] write error during copy");
            break;
        }
        total_written += w as usize;
    }

    // SAFETY: Close both file descriptors.
    unsafe {
        psp::sys::sceIoClose(src_fd);
        psp::sys::sceIoClose(dst_fd);
    }

    log_dec(b"[ME-DUMP] copied bytes=", total_written as u32);
}

/// Enumerate and dump ALL loaded kernel modules as binary files.
///
/// During game execution, mpeg/avcodec modules may have populated ME
/// stubs (unlike homebrew where they're empty). Dumping everything lets
/// us diff against homebrew-time dumps.
unsafe fn dump_kernel_modules() {
    let mut mod_ids = [psp::sys::SceUid(0); 128];
    let mut count: i32 = 0;

    // SAFETY: Kernel syscall to enumerate modules.
    // Second param is buffer size in BYTES (not element count).
    let buf_bytes = 128 * core::mem::size_of::<psp::sys::SceUid>() as i32;
    let ret = unsafe {
        psp::sys::sceKernelGetModuleIdList(
            mod_ids.as_mut_ptr(),
            buf_bytes,
            &mut count,
        )
    };

    log_hex(b"[ME-DUMP] modules: ret=", ret as u32);
    log_dec(b"[ME-DUMP] modules: count=", count as u32);

    if ret < 0 || count <= 0 {
        return;
    }

    // Module list text file — 8KB for up to ~60 modules.
    let mut list_buf = [0u8; 8192];
    let mut list_pos = 0;
    let mut dumped = 0u32;

    for i in 0..count as usize {
        let mid = mod_ids[i];
        let mut info: psp::sys::SceKernelModuleInfo = unsafe {
            core::mem::zeroed()
        };
        info.size = core::mem::size_of::<psp::sys::SceKernelModuleInfo>();

        // SAFETY: Kernel syscall to query module info.
        let ret = unsafe { psp::sys::sceKernelQueryModuleInfo(mid, &mut info) };
        if ret < 0 {
            // Log the failure with module ID for debugging.
            log_hex(b"[ME-DUMP] query failed mid=", mid.0 as u32);
            log_hex(b"[ME-DUMP] query err=", ret as u32);
            continue;
        }

        let name_len = info.name.iter().position(|&b| b == 0)
            .unwrap_or(info.name.len());
        let name = &info.name[..name_len];

        let text_addr = info.text_addr as u32;
        let text_size = info.text_size as u32;
        let data_size = info.data_size as u32;
        let total = (text_size + data_size) as usize;

        // Append to module list (every module, whether dumped or not).
        list_pos = append_bytes(&mut list_buf, list_pos, name);
        list_pos = append_bytes(&mut list_buf, list_pos, b" @0x");
        list_pos = append_hex(&mut list_buf, list_pos, text_addr);
        list_pos = append_bytes(&mut list_buf, list_pos, b" t=");
        list_pos = append_dec(&mut list_buf, list_pos, text_size);
        list_pos = append_bytes(&mut list_buf, list_pos, b" d=");
        list_pos = append_dec(&mut list_buf, list_pos, data_size);
        list_pos = append_bytes(&mut list_buf, list_pos, b"\n");

        // Skip modules with no text section.
        if total == 0 || text_addr == 0 {
            continue;
        }

        // Dump both kernel-space (0x88xxxxxx) and user-space (0x08xxxxxx)
        // modules. Our kernel PRX can read both address ranges.
        // Skip anything outside these ranges (null, etc).
        let in_kernel = text_addr >= 0x8800_0000 && text_addr < 0x8C00_0000;
        let in_user = text_addr >= 0x0800_0000 && text_addr < 0x0C00_0000;
        if !in_kernel && !in_user {
            continue;
        }

        // Build filename: me_dump/<index>_<name>.bin
        let mut path = [0u8; 96];
        let mut p = append_bytes(&mut path, 0, b"ms0:/seplugins/me_dump/");
        // Two-digit index prefix for sorting.
        p = append_dec(&mut path, p, i as u32);
        p = append_bytes(&mut path, p, b"_");
        p = append_bytes(&mut path, p, name);
        p = append_bytes(&mut path, p, b".bin\0");
        let _ = p;

        // SAFETY: text_addr is the loaded module's base in kernel memory.
        // We're in kernel mode so we can read kernel-space addresses.
        let slice = unsafe {
            core::slice::from_raw_parts(text_addr as *const u8, total)
        };

        // SAFETY: Write dump file.
        let ok = unsafe { write_file(&path, slice) };
        if ok {
            dumped += 1;
        } else {
            crate::debug_log(b"[ME-DUMP] write failed");
        }
    }

    log_dec(b"[ME-DUMP] modules dumped=", dumped);

    // Write module list.
    // SAFETY: Write module list file.
    unsafe {
        write_file(
            b"ms0:/seplugins/me_dump/modules.txt\0",
            &list_buf[..list_pos],
        );
    }
    crate::debug_log(b"[ME-DUMP] modules.txt written");
}

/// Probe for codec driver functions using sctrlHENFindFunction.
///
/// Uses only module/library names we KNOW exist on the system (from
/// hook.rs — sceDisplay, sceCtrl, scePower all work). We only add
/// codec-specific names that are likely to be loaded during video
/// playback.
unsafe fn probe_codec_drivers() {
    crate::debug_log(b"[ME-DUMP] probing codec drivers...");

    let mut result_buf = [0u8; 2048];
    let mut rp = 0;

    // Probe one function at a time, logging before each call so we
    // know exactly which one crashes if any.

    // Use the same module/library pattern that works for hook.rs:
    // sctrlHENFindFunction returns null for missing modules (no crash).
    // But some firmware versions may crash — log before each probe.

    struct Probe {
        module: &'static [u8],
        library: &'static [u8],
        nid: u32,
        label: &'static [u8],
    }

    let probes = [
        // sceAudiocodec — known working, use as canary
        Probe {
            module: b"sceAudiocodec_driver\0",
            library: b"sceAudiocodec\0",
            nid: 0x9D3F790C,
            label: b"audiocodec_CheckNeedMem",
        },
        // sceMpeg — kernel driver
        Probe {
            module: b"sceMpeg_driver\0",
            library: b"sceMpeg\0",
            nid: 0xD8C5F121,
            label: b"sceMpeg_Create",
        },
        // sceVideocodec
        Probe {
            module: b"sceVideocodec_driver\0",
            library: b"sceVideocodec\0",
            nid: 0xC01EC829,
            label: b"sceVideocodec_Open",
        },
        // sceMpegbase
        Probe {
            module: b"sceMpegbase_driver\0",
            library: b"sceMpegbase\0",
            nid: 0xBEA18F91,
            label: b"sceMpegbase_init",
        },
        // sceAVcodec_driver (avcodec.prx)
        Probe {
            module: b"sceAVcodec_driver\0",
            library: b"sceVideocodec\0",
            nid: 0xC01EC829,
            label: b"avcodec_VcodecOpen",
        },
    ];

    for probe in &probes {
        // Log before each probe so crash pinpoints the culprit.
        let mut pre = [0u8; 64];
        let mut pp = append_bytes(&mut pre, 0, b"[ME-DUMP] probe: ");
        pp = append_bytes(&mut pre, pp, probe.label);
        crate::debug_log(&pre[..pp]);

        let ptr = unsafe {
            psp::hook::find_function(
                probe.module.as_ptr(),
                probe.library.as_ptr(),
                probe.nid,
            )
        };

        rp = append_bytes(&mut result_buf, rp, probe.label);
        rp = append_bytes(&mut result_buf, rp, b": ");

        if let Some(addr) = ptr {
            let addr_u32 = addr as *const u8 as u32;
            rp = append_bytes(&mut result_buf, rp, b"0x");
            rp = append_hex(&mut result_buf, rp, addr_u32);

            let mut lg = [0u8; 64];
            let mut lp = append_bytes(&mut lg, 0, b"[ME-DUMP] FOUND ");
            lp = append_bytes(&mut lg, lp, probe.label);
            lp = append_bytes(&mut lg, lp, b" @0x");
            lp = append_hex(&mut lg, lp, addr_u32);
            crate::debug_log(&lg[..lp]);
        } else {
            rp = append_bytes(&mut result_buf, rp, b"NOT FOUND");
        }
        rp = append_bytes(&mut result_buf, rp, b"\n");
    }

    // SAFETY: Write probe results.
    unsafe {
        write_file(
            b"ms0:/seplugins/me_dump/codec_probes.txt\0",
            &result_buf[..rp],
        );
    }
    crate::debug_log(b"[ME-DUMP] codec probes written");
}

// ---------------------------------------------------------------------------
// Helpers (no_std, no alloc)
// ---------------------------------------------------------------------------

/// Check if `haystack` contains `needle` (byte-level substring search).
pub fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.len() > haystack.len() {
        return false;
    }
    let mut i = 0;
    while i + needle.len() <= haystack.len() {
        let mut matched = true;
        let mut j = 0;
        while j < needle.len() {
            if haystack[i + j] != needle[j] {
                matched = false;
                break;
            }
            j += 1;
        }
        if matched {
            return true;
        }
        i += 1;
    }
    false
}

pub fn append_bytes(buf: &mut [u8], pos: usize, s: &[u8]) -> usize {
    let mut p = pos;
    for &b in s {
        if p >= buf.len() {
            break;
        }
        buf[p] = b;
        p += 1;
    }
    p
}

pub fn append_hex(buf: &mut [u8], pos: usize, val: u32) -> usize {
    let hex = b"0123456789ABCDEF";
    let mut p = pos;
    let mut i = 0;
    while i < 8 {
        if p >= buf.len() {
            break;
        }
        let nibble = (val >> (28 - i * 4)) & 0xF;
        buf[p] = hex[nibble as usize];
        p += 1;
        i += 1;
    }
    p
}

pub fn append_dec(buf: &mut [u8], pos: usize, val: u32) -> usize {
    if val == 0 {
        if pos < buf.len() {
            buf[pos] = b'0';
            return pos + 1;
        }
        return pos;
    }
    let mut digits = [0u8; 10];
    let mut n = val;
    let mut count = 0;
    while n > 0 {
        digits[count] = b'0' + (n % 10) as u8;
        n /= 10;
        count += 1;
    }
    let mut p = pos;
    while count > 0 {
        count -= 1;
        if p >= buf.len() {
            break;
        }
        buf[p] = digits[count];
        p += 1;
    }
    p
}

fn log_hex(prefix: &[u8], val: u32) {
    let mut buf = [0u8; 64];
    let mut p = append_bytes(&mut buf, 0, prefix);
    p = append_bytes(&mut buf, p, b"0x");
    p = append_hex(&mut buf, p, val);
    crate::debug_log(&buf[..p]);
}

fn log_dec(prefix: &[u8], val: u32) {
    let mut buf = [0u8; 64];
    let mut p = append_bytes(&mut buf, 0, prefix);
    p = append_dec(&mut buf, p, val);
    crate::debug_log(&buf[..p]);
}

// ---------------------------------------------------------------------------
// Spy log: captures sceMpeg call arguments for runtime analysis
// ---------------------------------------------------------------------------

/// 4KB ring buffer for spy log entries. Each hook appends a line with
/// function name, arguments, and return value. Flushed to file on demand
/// via the overlay menu ("Spy: Dump Log").
static mut SPY_BUF: [u8; 4096] = [0u8; 4096];
static mut SPY_POS: usize = 0;

/// Append a raw byte string to the spy log buffer.
pub fn spy_append(msg: &[u8]) {
    // SAFETY: Only called from syscall hook context (serialized by kernel).
    unsafe {
        let pos = core::ptr::read_volatile(&raw const SPY_POS);
        let buf = &mut *(&raw mut SPY_BUF);
        let new_pos = append_bytes(buf, pos, msg);
        core::ptr::write_volatile(&raw mut SPY_POS, new_pos);
    }
}

/// Append a newline to the spy log.
pub fn spy_newline() {
    // SAFETY: Same as spy_append.
    unsafe {
        let pos = core::ptr::read_volatile(&raw const SPY_POS);
        let buf = &mut *(&raw mut SPY_BUF);
        let new_pos = append_bytes(buf, pos, b"\n");
        core::ptr::write_volatile(&raw mut SPY_POS, new_pos);
    }
}

/// Append a hex value to the spy log.
pub fn spy_hex(val: u32) {
    // SAFETY: Same as spy_append.
    unsafe {
        let pos = core::ptr::read_volatile(&raw const SPY_POS);
        let buf = &mut *(&raw mut SPY_BUF);
        let p = append_bytes(buf, pos, b"0x");
        let new_pos = append_hex(buf, p, val);
        core::ptr::write_volatile(&raw mut SPY_POS, new_pos);
    }
}

/// Append a decimal value to the spy log.
pub fn spy_dec(val: u32) {
    // SAFETY: Same as spy_append.
    unsafe {
        let pos = core::ptr::read_volatile(&raw const SPY_POS);
        let buf = &mut *(&raw mut SPY_BUF);
        let new_pos = append_dec(buf, pos, val);
        core::ptr::write_volatile(&raw mut SPY_POS, new_pos);
    }
}

/// Log a sceMpegCreate call to the spy buffer.
pub fn spy_log_create(
    mpeg: u32, data: u32, size: i32, rb: u32,
    fw: i32, mode: i32, ddr: i32, ret: i32, tag: &[u8],
) {
    spy_append(tag);
    spy_append(b"Create: mpeg=");
    spy_hex(mpeg);
    spy_append(b" data=");
    spy_hex(data);
    spy_append(b" sz=");
    spy_dec(size as u32);
    spy_append(b" rb=");
    spy_hex(rb);
    spy_append(b" fw=");
    spy_dec(fw as u32);
    spy_append(b" mode=");
    spy_dec(mode as u32);
    spy_append(b" ddr=");
    spy_hex(ddr as u32);
    spy_append(b" ret=");
    spy_hex(ret as u32);
    spy_newline();
}

/// Log a sceMpegAvcDecode call to the spy buffer.
pub fn spy_log_decode(
    mpeg: u32, au: u32, fw: i32, buf: u32,
    init: i32, ret: i32, tag: &[u8],
) {
    spy_append(tag);
    spy_append(b"Decode: mpeg=");
    spy_hex(mpeg);
    spy_append(b" au=");
    spy_hex(au);
    spy_append(b" fw=");
    spy_dec(fw as u32);
    spy_append(b" buf=");
    spy_hex(buf);
    spy_append(b" init=");
    spy_dec(init as u32);
    spy_append(b" ret=");
    spy_hex(ret as u32);
    spy_newline();
}

/// Spy dump command: 0=idle, 1=requested.
static SPY_DUMP_STATE: AtomicU8 = AtomicU8::new(0);

/// Request a spy dump (called from overlay menu — display hook context).
/// Actual I/O happens in the dump thread where file operations are safe.
pub fn trigger_spy_dump() {
    let _ = SPY_DUMP_STATE.compare_exchange(0, 1, Ordering::AcqRel, Ordering::Relaxed);
}

/// Auto-flush spy log to file if there's data. Called periodically from
/// the dump thread (~every 5 seconds). Appends to mpeg_spy.txt and also
/// writes a fresh module snapshot on the first flush.
unsafe fn auto_flush_spy() {
    let pos = core::ptr::read_volatile(&raw const SPY_POS);
    if pos == 0 {
        return;
    }

    // Diagnostic: log that we detected data.
    let mut m = [0u8; 48];
    let mut p = append_bytes(&mut m, 0, b"[SPY] flush pos=");
    p = append_dec(&mut m, p, pos as u32);
    crate::debug_log(&m[..p]);

    // Append spy log to file (not truncate -- accumulates across flushes).
    let buf = &*(&raw const SPY_BUF);
    let fd = psp::sys::sceIoOpen(
        b"ms0:/seplugins/mpeg_spy.txt\0".as_ptr(),
        psp::sys::IoOpenFlags::APPEND
            | psp::sys::IoOpenFlags::CREAT
            | psp::sys::IoOpenFlags::WR_ONLY,
        0o777,
    );
    if fd >= psp::sys::SceUid(0) {
        psp::sys::sceIoWrite(fd, buf.as_ptr() as *const _, pos);
        psp::sys::sceIoClose(fd);
    }

    // Reset buffer.
    core::ptr::write_volatile(&raw mut SPY_POS, 0);

    // Dump modules on first flush only (avoid repeated large writes).
    static mut MODULES_DUMPED: bool = false;
    if !core::ptr::read_volatile(&raw const MODULES_DUMPED) {
        core::ptr::write_volatile(&raw mut MODULES_DUMPED, true);
        dump_all_modules_to_file();
    }
}

/// Dump all loaded modules (kernel + user) to a text file for comparison.
unsafe fn dump_all_modules_to_file() {
    let mut mod_ids = [psp::sys::SceUid(0); 128];
    let mut count: i32 = 0;
    let buf_bytes = 128 * core::mem::size_of::<psp::sys::SceUid>() as i32;
    let ret = psp::sys::sceKernelGetModuleIdList(
        mod_ids.as_mut_ptr(), buf_bytes, &mut count,
    );
    if ret < 0 || count <= 0 {
        crate::debug_log(b"[SPY] no modules");
        return;
    }

    let mut list = [0u8; 8192];
    let mut lp = 0;

    for i in 0..count as usize {
        let mut info: psp::sys::SceKernelModuleInfo = core::mem::zeroed();
        info.size = core::mem::size_of::<psp::sys::SceKernelModuleInfo>();
        if psp::sys::sceKernelQueryModuleInfo(mod_ids[i], &mut info) < 0 {
            continue;
        }
        let name_len = info.name.iter().position(|&b| b == 0)
            .unwrap_or(info.name.len());

        // Tag kernel vs user modules.
        let tag = if info.text_addr >= 0x8800_0000 {
            b"[K] "
        } else if info.text_addr >= 0x0800_0000 {
            b"[U] "
        } else {
            b"[?] "
        };

        lp = append_bytes(&mut list, lp, tag);
        lp = append_bytes(&mut list, lp, &info.name[..name_len]);
        lp = append_bytes(&mut list, lp, b" @0x");
        lp = append_hex(&mut list, lp, info.text_addr);
        lp = append_bytes(&mut list, lp, b" t=");
        lp = append_dec(&mut list, lp, info.text_size);
        lp = append_bytes(&mut list, lp, b" d=");
        lp = append_dec(&mut list, lp, info.data_size);
        lp = append_bytes(&mut list, lp, b" attr=0x");
        lp = append_hex(&mut list, lp, info.attribute as u32);
        lp = append_bytes(&mut list, lp, b"\n");
    }

    lp = append_bytes(&mut list, lp, b"\nTotal: ");
    lp = append_dec(&mut list, lp, count as u32);
    lp = append_bytes(&mut list, lp, b" modules\n");

    write_file(
        b"ms0:/seplugins/mpeg_spy_modules.txt\0",
        &list[..lp],
    );

    let mut m = [0u8; 48];
    let mut p = append_bytes(&mut m, 0, b"[SPY] dumped ");
    p = append_dec(&mut m, p, count as u32);
    p = append_bytes(&mut m, p, b" modules");
    crate::debug_log(&m[..p]);
}

// ---------------------------------------------------------------------------
// ME stub patching via file-based IPC with EBOOT
// ---------------------------------------------------------------------------

const PATCH_REQUEST: &[u8] = b"ms0:/PSP/GAME/OASISOS/.me_patch\0";
const PATCH_DONE: &[u8] = b"ms0:/PSP/GAME/OASISOS/.me_patched\0";

static PATCH_DONE_FLAG: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

/// Check for patch request file and perform kernel stub patching.
unsafe fn check_patch_request() {
    if PATCH_DONE_FLAG.load(core::sync::atomic::Ordering::Relaxed) {
        return;
    }

    let fd = psp::sys::sceIoOpen(
        PATCH_REQUEST.as_ptr(), psp::sys::IoOpenFlags::RD_ONLY, 0,
    );
    if fd < psp::sys::SceUid(0) {
        return;
    }
    psp::sys::sceIoClose(fd);
    psp::sys::sceIoRemove(PATCH_REQUEST.as_ptr());

    crate::debug_log(b"[ME-PATCH] hooking sceMpegAvcDecode...");

    // Find AvcDecode — try kernel driver first (sctrlHENPatchSyscall
    // only works on kernel syscall table entries, not user-mode stubs).
    let avc_ptr = psp::hook::find_function(
        b"sceMpeg_driver\0".as_ptr(),
        b"sceMpeg\0".as_ptr(),
        0x0E3C2E9D,
    ).or_else(|| psp::hook::find_function(
        b"sceMpegVsh_library\0".as_ptr(),
        b"sceMpeg\0".as_ptr(),
        0x0E3C2E9D,
    )).or_else(|| psp::hook::find_function(
        b"sceMpeg_library\0".as_ptr(),
        b"sceMpeg\0".as_ptr(),
        0x0E3C2E9D,
    ));
    if let Some(ptr) = avc_ptr {
        let mut m = [0u8; 64];
        let mut p = append_bytes(&mut m, 0, b"[ME-PATCH] AvcDecode @0x");
        p = append_hex(&mut m, p, ptr as u32);
        crate::debug_log(&m[..p]);

        // Store original and hook the syscall.
        crate::ORIGINAL_AVC_DECODE = ptr;
        let ret = psp::sys::sctrlHENPatchSyscall(
            ptr,
            crate::hooked_avc_decode as *mut u8,
        );
        let ok = ret >= 0;
        if ok {
            crate::debug_log(b"[ME-PATCH] AvcDecode HOOKED!");
        } else {
            crate::debug_log(b"[ME-PATCH] hook failed");
        }
        write_done_file(ok);
    } else {
        crate::debug_log(b"[ME-PATCH] sceMpeg_library AvcDecode NOT found");
        write_done_file(false);
    }

    PATCH_DONE_FLAG.store(true, core::sync::atomic::Ordering::Release);
}

/// Dump all user-space modules (0x08xxxxxx) for debugging.
unsafe fn dump_user_modules() {
    let mut mod_ids = [psp::sys::SceUid(0); 128];
    let mut count: i32 = 0;
    let buf_bytes = 128 * core::mem::size_of::<psp::sys::SceUid>() as i32;
    let ret = psp::sys::sceKernelGetModuleIdList(
        mod_ids.as_mut_ptr(), buf_bytes, &mut count,
    );
    if ret < 0 || count <= 0 { return; }

    for i in 0..count as usize {
        let mut info: psp::sys::SceKernelModuleInfo = core::mem::zeroed();
        info.size = core::mem::size_of::<psp::sys::SceKernelModuleInfo>();
        if psp::sys::sceKernelQueryModuleInfo(mod_ids[i], &mut info) < 0 {
            continue;
        }
        // Only log user-space modules
        if info.text_addr < 0x0800_0000 || info.text_addr >= 0x0C00_0000 {
            continue;
        }
        let name_len = info.name.iter().position(|&b| b == 0)
            .unwrap_or(info.name.len());
        let mut msg = [0u8; 80];
        let mut mp = append_bytes(&mut msg, 0, b"[ME-PATCH] user mod: ");
        mp = append_bytes(&mut msg, mp, &info.name[..name_len]);
        mp = append_bytes(&mut msg, mp, b" @0x");
        mp = append_hex(&mut msg, mp, info.text_addr);
        mp = append_bytes(&mut msg, mp, b" sz=");
        mp = append_dec(&mut msg, mp, info.text_size);
        crate::debug_log(&msg[..mp]);
    }
}

/// Find the kernel sceAvcodec_wrapper module size.
fn find_kernel_avcodec_size() -> u32 {
    let mut mod_ids = [psp::sys::SceUid(0); 128];
    let mut count: i32 = 0;
    let buf_bytes = 128 * core::mem::size_of::<psp::sys::SceUid>() as i32;
    let ret = unsafe {
        psp::sys::sceKernelGetModuleIdList(
            mod_ids.as_mut_ptr(), buf_bytes, &mut count,
        )
    };
    if ret < 0 || count <= 0 { return 0; }
    for i in 0..count as usize {
        let mut info: psp::sys::SceKernelModuleInfo = unsafe { core::mem::zeroed() };
        info.size = core::mem::size_of::<psp::sys::SceKernelModuleInfo>();
        if unsafe { psp::sys::sceKernelQueryModuleInfo(mod_ids[i], &mut info) } < 0 {
            continue;
        }
        let name_len = info.name.iter().position(|&b| b == 0).unwrap_or(info.name.len());
        let name = &info.name[..name_len];
        if contains_bytes(name, b"vcodec") && info.text_addr >= 0x8800_0000 {
            return info.text_size + info.data_size;
        }
    }
    0
}

/// Find the kernel sceAvcodec_wrapper module base address.
fn find_kernel_avcodec() -> u32 {
    let mut mod_ids = [psp::sys::SceUid(0); 128];
    let mut count: i32 = 0;
    let buf_bytes = 128 * core::mem::size_of::<psp::sys::SceUid>() as i32;

    let ret = unsafe {
        psp::sys::sceKernelGetModuleIdList(
            mod_ids.as_mut_ptr(), buf_bytes, &mut count,
        )
    };
    if ret < 0 || count <= 0 {
        return 0;
    }

    for i in 0..count as usize {
        let mut info: psp::sys::SceKernelModuleInfo = unsafe {
            core::mem::zeroed()
        };
        info.size = core::mem::size_of::<psp::sys::SceKernelModuleInfo>();
        let ret = unsafe {
            psp::sys::sceKernelQueryModuleInfo(mod_ids[i], &mut info)
        };
        if ret < 0 { continue; }

        let name_len = info.name.iter().position(|&b| b == 0)
            .unwrap_or(info.name.len());
        let name = &info.name[..name_len];

        // Match "sceAvcodec_wrapper" (kernel module, addr >= 0x88000000).
        if contains_bytes(name, b"vcodec") && info.text_addr >= 0x8800_0000 {
            return info.text_addr;
        }
    }
    0
}

/// Patch the 5 ME submission stubs in kernel sceAvcodec_wrapper.
///
/// Replaces J instructions (that jump to user-space sceMpeg) with
/// J instructions to the real sceMeVideo_driver functions.
///
/// Returns true if all stubs were patched successfully.
unsafe fn patch_kernel_stubs(base: u32) -> bool {
    // Stub offsets and their target sceMeVideo_driver NIDs.
    let stubs: [(u32, u32); 5] = [
        (0x4414, 0xC441994C), // me_open → MeVideo::Init
        (0x4424, 0x8768915D), // me_scan → MeVideo::ScanHeader
        (0x4434, 0xE8CD3C75), // me_init → MeVideo::Init2
        (0x4394, 0x6D68B223), // me_worker → MeVideo::Decode
        (0x438c, 0x4D78330C), // me_wait → MeVideo::GetEdram
    ];

    let mut patched = 0u32;

    for (offset, nid) in stubs {
        let stub_addr = base + offset;

        // Log current instruction at this address.
        let current = core::ptr::read_volatile(stub_addr as *const u32);
        let mut pre = [0u8; 64];
        let mut pp = append_bytes(&mut pre, 0, b"[ME-PATCH] +0x");
        pp = append_hex(&mut pre, pp, offset);
        pp = append_bytes(&mut pre, pp, b" was=0x");
        pp = append_hex(&mut pre, pp, current);
        crate::debug_log(&pre[..pp]);

        // Resolve the real ME function.
        let me_fn = psp::hook::find_function(
            b"sceMeCodecWrapper\0".as_ptr(),
            b"sceMeVideo_driver\0".as_ptr(),
            nid,
        );

        if let Some(target_ptr) = me_fn {
            let target = target_ptr as u32;
            let j_insn = 0x0800_0000 | ((target >> 2) & 0x03FF_FFFF);

            // Write the J instruction over the stub.
            core::ptr::write_volatile(stub_addr as *mut u32, j_insn);

            // Flush caches.
            psp::sys::sceKernelDcacheWritebackInvalidateRange(
                stub_addr as *const core::ffi::c_void, 8,
            );
            psp::sys::sceKernelIcacheInvalidateRange(
                stub_addr as *const core::ffi::c_void, 8,
            );

            patched += 1;
        }
    }

    log_dec(b"[ME-PATCH] patched ", patched);
    crate::debug_log(b"[ME-PATCH] done");
    patched == 5
}

// ---------------------------------------------------------------------------
// VSH address resolution via file-based IPC
// ---------------------------------------------------------------------------

const VSH_LOAD_REQ: &[u8] = b"ms0:/PSP/GAME/OASISOS/.vsh_load\0";
const VSH_REQ: &[u8] = b"ms0:/PSP/GAME/OASISOS/.vsh_req\0";
const VSH_ADDRS: &[u8] = b"ms0:/PSP/GAME/OASISOS/.vsh_addrs\0";

static VSH_LOAD_DONE: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

/// Load cooleyesBridge.prx as kernel module (PMPlayer prerequisite).
/// Called once at dump thread startup.
static BRIDGE_LOADED: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

unsafe fn load_cooleyes_bridge() {
    if BRIDGE_LOADED.load(core::sync::atomic::Ordering::Relaxed) {
        return;
    }
    BRIDGE_LOADED.store(true, core::sync::atomic::Ordering::Release);

    let paths: [&[u8]; 2] = [
        b"ms0:/PSP/GAME/OASISOS/cooleyesBridge.prx\0",
        b"ms0:/PSP/GAME/UoPMPlayer_660/cooleyesBridge.prx\0",
    ];

    for path in &paths {
        let mod_id = psp::sys::sceKernelLoadModule(
            path.as_ptr(), 0, core::ptr::null_mut(),
        );
        if mod_id < psp::sys::SceUid(0) {
            continue;
        }
        let mut status: i32 = 0;
        let ret = psp::sys::sceKernelStartModule(
            mod_id, 0, core::ptr::null_mut(),
            &mut status, core::ptr::null_mut(),
        );
        let mut m = [0u8; 64];
        let mut p = append_bytes(&mut m, 0, b"[BRIDGE] start=0x");
        p = append_hex(&mut m, p, ret as u32);
        crate::debug_log(&m[..p]);
        if ret >= 0 {
            crate::debug_log(b"[BRIDGE] cooleyesBridge loaded OK");
            return;
        }
        psp::sys::sceKernelUnloadModule(mod_id);
    }
    crate::debug_log(b"[BRIDGE] cooleyesBridge not found");
}

/// Check for VSH syscall resolution request from EBOOT.
/// Reads syscall numbers from the VSH library's user-mode stubs (0x0A0A*)
/// and writes them so the EBOOT can make direct syscalls.
unsafe fn check_vsh_load_request() {
    if VSH_LOAD_DONE.load(core::sync::atomic::Ordering::Relaxed) {
        return;
    }

    let fd = psp::sys::sceIoOpen(
        VSH_LOAD_REQ.as_ptr(), psp::sys::IoOpenFlags::RD_ONLY, 0,
    );
    if fd < psp::sys::SceUid(0) {
        return;
    }
    psp::sys::sceIoClose(fd);
    psp::sys::sceIoRemove(VSH_LOAD_REQ.as_ptr());

    crate::debug_log(b"[VSH-SC] extracting syscall numbers...");

    // Resolve VSH function addresses (in VSH process space 0x0A0A*).
    let nids: [u32; 8] = [
        0xD8C5F121, 0x606A4649, 0x167AFD9E, 0xA780CF7E,
        0xCEB870B1, 0x11F95CF1, 0x0E3C2E9D, 0xC132E22F,
    ];

    // Output: 8 x u32 syscall numbers (or 0 if not found).
    let mut syscalls = [0u32; 8];
    let mut resolved = 0u32;

    for (i, &nid) in nids.iter().enumerate() {
        let ptr = psp::hook::find_function(
            b"sceMpegVsh_library\0".as_ptr(),
            b"sceMpeg\0".as_ptr(),
            nid,
        );
        if let Some(fn_ptr) = ptr {
            // Read the syscall stub. PSP user-mode stubs look like:
            //   jr $ra        (03E00008)
            //   syscall N     (0000000C | (N << 6))
            // OR:
            //   nop           (00000000)
            //   syscall N     (0000000C | (N << 6))
            // The syscall instruction is at offset +4.
            let stub_addr = fn_ptr as u32;
            let insn0 = core::ptr::read_volatile(stub_addr as *const u32);
            let insn1 = core::ptr::read_volatile((stub_addr + 4) as *const u32);

            // Extract syscall number from whichever instruction is the syscall.
            let sc_num = if (insn1 & 0x3F) == 0x0C {
                // insn1 is syscall
                (insn1 >> 6) & 0xFFFFF
            } else if (insn0 & 0x3F) == 0x0C {
                // insn0 is syscall
                (insn0 >> 6) & 0xFFFFF
            } else {
                0
            };

            if sc_num != 0 {
                syscalls[i] = sc_num;
                resolved += 1;

                let mut m = [0u8; 80];
                let mut p = append_bytes(&mut m, 0, b"[VSH-SC] [");
                p = append_dec(&mut m, p, i as u32);
                p = append_bytes(&mut m, p, b"] @0x");
                p = append_hex(&mut m, p, stub_addr);
                p = append_bytes(&mut m, p, b" insn=0x");
                p = append_hex(&mut m, p, insn0);
                p = append_bytes(&mut m, p, b",0x");
                p = append_hex(&mut m, p, insn1);
                p = append_bytes(&mut m, p, b" sc=");
                p = append_dec(&mut m, p, sc_num);
                crate::debug_log(&m[..p]);
            }
        }
    }

    let mut m = [0u8; 48];
    let mut p = append_bytes(&mut m, 0, b"[VSH-SC] resolved ");
    p = append_dec(&mut m, p, resolved);
    p = append_bytes(&mut m, p, b"/8 syscalls");
    crate::debug_log(&m[..p]);

    // Write syscall numbers to file (8 x u32 = 32 bytes).
    let out_fd = psp::sys::sceIoOpen(
        VSH_ADDRS.as_ptr(),
        psp::sys::IoOpenFlags::WR_ONLY
            | psp::sys::IoOpenFlags::CREAT
            | psp::sys::IoOpenFlags::TRUNC,
        0o777,
    );
    if out_fd >= psp::sys::SceUid(0) {
        psp::sys::sceIoWrite(
            out_fd,
            syscalls.as_ptr() as *const core::ffi::c_void,
            32,
        );
        psp::sys::sceIoClose(out_fd);
    }

    VSH_LOAD_DONE.store(true, core::sync::atomic::Ordering::Release);
}

static VSH_ADDRS_DONE: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

/// Check for VSH address request from EBOOT and resolve sceMpegVsh_library NIDs.
///
/// The EBOOT creates `.vsh_req` after starting mpeg_vsh370.prx. This function
/// resolves all 24 sceMpeg NIDs via sctrlHENFindFunction and writes the
/// function addresses to `.vsh_addrs` (24 x u32 = 96 bytes).
/// NID order matches `psp::sys::mpeg_stubs::NIDS`.
unsafe fn check_vsh_addr_request() {
    if VSH_ADDRS_DONE.load(core::sync::atomic::Ordering::Relaxed) {
        return;
    }

    let fd = psp::sys::sceIoOpen(
        VSH_REQ.as_ptr(), psp::sys::IoOpenFlags::RD_ONLY, 0,
    );
    if fd < psp::sys::SceUid(0) {
        return;
    }
    psp::sys::sceIoClose(fd);
    psp::sys::sceIoRemove(VSH_REQ.as_ptr());

    crate::debug_log(b"[VSH-ADDR] resolving 24 sceMpegVsh NIDs...");

    // All 24 NIDs — same order as psp::sys::mpeg_stubs::NIDS.
    let nids: [u32; 24] = [
        0x682A619B, // 0:  sceMpegInit
        0x874624D6, // 1:  sceMpegFinish
        0xC132E22F, // 2:  sceMpegQueryMemSize
        0xD8C5F121, // 3:  sceMpegCreate
        0x606A4649, // 4:  sceMpegDelete
        0x42560F23, // 5:  sceMpegRegistStream
        0x591A4AA2, // 6:  sceMpegUnRegistStream
        0xA780CF7E, // 7:  sceMpegMallocAvcEsBuf
        0xCEB870B1, // 8:  sceMpegFreeAvcEsBuf
        0x167AFD9E, // 9:  sceMpegInitAu
        0xFE246728, // 10: sceMpegGetAvcAu
        0xA11C7026, // 11: sceMpegAvcDecodeMode
        0x0E3C2E9D, // 12: sceMpegAvcDecode
        0x740FCCD1, // 13: sceMpegAvcDecodeStop
        0x11F95CF1, // 14: sceMpegGetAvcNalAu
        0xCF3547A2, // 15: sceMpegAvcDecodeDetail2
        0x707B7629, // 16: sceMpegFlushAllStream
        0xD7A29F46, // 17: sceMpegRingbufferQueryMemSize
        0x37295ED8, // 18: sceMpegRingbufferConstruct
        0x13407F13, // 19: sceMpegRingbufferDestruct
        0xB5F6DC87, // 20: sceMpegRingbufferAvailableSize
        0xB240A59E, // 21: sceMpegRingbufferPut
        0x21FF80E4, // 22: sceMpegQueryStreamOffset
        0x611E9E11, // 23: sceMpegQueryStreamSize
    ];

    let mut addrs = [0u32; 24];
    let mut resolved = 0u32;

    for (i, &nid) in nids.iter().enumerate() {
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

    let mut m = [0u8; 48];
    let mut p = append_bytes(&mut m, 0, b"[VSH-ADDR] resolved ");
    p = append_dec(&mut m, p, resolved);
    p = append_bytes(&mut m, p, b"/24");
    crate::debug_log(&m[..p]);

    // Write addresses to dedicated file (24 x u32 = 96 bytes, LE).
    // Uses ".mpeg_fns" NOT ".vsh_addrs" to avoid conflict with boot-time
    // syscall extraction that also writes to ".vsh_addrs".
    let out_fd = psp::sys::sceIoOpen(
        b"ms0:/PSP/GAME/OASISOS/.mpeg_fns\0".as_ptr(),
        psp::sys::IoOpenFlags::WR_ONLY
            | psp::sys::IoOpenFlags::CREAT
            | psp::sys::IoOpenFlags::TRUNC,
        0o777,
    );
    if out_fd >= psp::sys::SceUid(0) {
        psp::sys::sceIoWrite(
            out_fd,
            addrs.as_ptr() as *const core::ffi::c_void,
            96, // 24 * 4
        );
        psp::sys::sceIoClose(out_fd);
        crate::debug_log(b"[VSH-ADDR] 96 bytes written");
    }

    VSH_ADDRS_DONE.store(true, core::sync::atomic::Ordering::Release);
}

/// Write the done file (EBOOT polls for this).
unsafe fn write_done_file(success: bool) {
    let fd = psp::sys::sceIoOpen(
        PATCH_DONE.as_ptr(),
        psp::sys::IoOpenFlags::WR_ONLY
            | psp::sys::IoOpenFlags::CREAT
            | psp::sys::IoOpenFlags::TRUNC,
        0o777,
    );
    if fd >= psp::sys::SceUid(0) {
        let msg: &[u8] = if success { b"OK" } else { b"FAIL" };
        psp::sys::sceIoWrite(fd, msg.as_ptr() as *const _, msg.len());
        psp::sys::sceIoClose(fd);
    }
}

// ---------------------------------------------------------------------------
// Kernel-mode child module load test
// ---------------------------------------------------------------------------

static KERNEL_LOAD_DONE: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

/// Check for kernel-mode module load test trigger file.
/// EBOOT creates `.kload_test`, PRX loads+starts modules from kernel context.
unsafe fn check_kernel_load_test() {
    if KERNEL_LOAD_DONE.load(core::sync::atomic::Ordering::Relaxed) {
        return;
    }

    let trigger = b"ms0:/PSP/GAME/OASISOS/.kload_test\0";
    let fd = psp::sys::sceIoOpen(trigger.as_ptr(), psp::sys::IoOpenFlags::RD_ONLY, 0);
    if fd < psp::sys::SceUid(0) {
        return;
    }
    psp::sys::sceIoClose(fd);
    psp::sys::sceIoRemove(trigger.as_ptr());
    KERNEL_LOAD_DONE.store(true, core::sync::atomic::Ordering::Relaxed);

    crate::debug_log(b"[KLOAD] patching kernel linking wrapper");

    // Patch the sceKernelLinkLibraryEntriesWithModule wrapper at 0x8805e5e8.
    // At 0x8805e620: "beq $a2, $zero, 0x8805e658" returns 0x800200D3 when
    // stubSize (a2) is 0. We NOP this check to allow modules with 0 imports.
    // Also NOP the delay slot instruction at 0x8805e624.
    let patch_addr1 = 0x8805E620 as *mut u32;
    let patch_addr2 = 0x8805E624 as *mut u32;

    let old1 = core::ptr::read_volatile(patch_addr1);
    let old2 = core::ptr::read_volatile(patch_addr2);
    let mut m = [0u8; 80];
    let mut p = append_bytes(&mut m, 0, b"[KLOAD] old @0x8805E620=0x");
    p = append_hex(&mut m, p, old1);
    p = append_bytes(&mut m, p, b" @0x8805E624=0x");
    p = append_hex(&mut m, p, old2);
    crate::debug_log(&m[..p]);

    // NOP both instructions
    core::ptr::write_volatile(patch_addr1, 0x00000000u32); // nop
    // DON'T nop the delay slot — it sets a3 = (K1<0)?1:0 which is needed later
    // Just nop the branch itself

    crate::debug_log(b"[KLOAD] patched beq $a2,$zero to NOP");

    // Flush caches to ensure patch takes effect
    psp::sys::sceKernelDcacheWritebackInvalidateAll();
    psp::sys::sceKernelIcacheInvalidateAll();

    // Read module ID from trigger file (EBOOT writes u32 module ID)
    let fd = psp::sys::sceIoOpen(
        b"ms0:/PSP/GAME/OASISOS/.kload_test\0".as_ptr(),
        psp::sys::IoOpenFlags::RD_ONLY, 0,
    );
    // File was already removed above, so reopen the new version the EBOOT writes
    // Actually: EBOOT writes .kload_modid with the module ID
    let fd = psp::sys::sceIoOpen(
        b"ms0:/PSP/GAME/OASISOS/.kload_modid\0".as_ptr(),
        psp::sys::IoOpenFlags::RD_ONLY, 0,
    );
    if fd < psp::sys::SceUid(0) {
        crate::debug_log(b"[KLOAD] no .kload_modid file");
        return;
    }
    let mut mod_id_bytes = [0u8; 4];
    psp::sys::sceIoRead(fd, mod_id_bytes.as_mut_ptr() as *mut _, 4);
    psp::sys::sceIoClose(fd);
    psp::sys::sceIoRemove(b"ms0:/PSP/GAME/OASISOS/.kload_modid\0".as_ptr());

    let mod_id_val = u32::from_le_bytes(mod_id_bytes);
    let mod_id = psp::sys::SceUid(mod_id_val as i32);

    let mut msg = [0u8; 60];
    let mut p = append_bytes(&mut msg, 0, b"[KLOAD] mod_id=0x");
    p = append_hex(&mut msg, p, mod_id_val);
    crate::debug_log(&msg[..p]);

    // The kernel patch (NOP at 0x8805E620) should now allow child module start.
    // The EBOOT does sceKernelStartModule directly — no need to do it here.

    // Write empty result file to signal completion
    let result_fd = psp::sys::sceIoOpen(
        b"ms0:/PSP/GAME/OASISOS/.kload_result\0".as_ptr(),
        psp::sys::IoOpenFlags::CREAT | psp::sys::IoOpenFlags::WR_ONLY | psp::sys::IoOpenFlags::TRUNC,
        0o777,
    );
    if result_fd >= psp::sys::SceUid(0) {
        psp::sys::sceIoWrite(result_fd, b"DONE".as_ptr() as *const _, 4);
        psp::sys::sceIoClose(result_fd);
    }

    crate::debug_log(b"[KLOAD] test complete");
}

/// Dump a kernel module's text+data segments to a file for Ghidra analysis.
/// `mod_name` must be null-terminated. `out_path` must be null-terminated.
unsafe fn dump_kernel_module(mod_name: &[u8], out_path: &[u8]) {
    // Get list of all loaded module IDs
    let mut ids = [psp::sys::SceUid(0); 256];
    let mut count: i32 = 0;
    let ret = psp::sys::sceKernelGetModuleIdList(
        ids.as_mut_ptr(), 256, &mut count,
    );
    if ret < 0 {
        let mut m = [0u8; 60];
        let mut p = append_bytes(&mut m, 0, b"[KDUMP] GetModuleIdList err=0x");
        p = append_hex(&mut m, p, ret as u32);
        crate::debug_log(&m[..p]);
        return;
    }

    let mut m = [0u8; 60];
    let mut p = append_bytes(&mut m, 0, b"[KDUMP] ");
    p = append_dec(&mut m, p, count as u32);
    p = append_bytes(&mut m, p, b" modules loaded, searching for ");
    // Append module name (without null terminator)
    let name_len = mod_name.iter().position(|&b| b == 0).unwrap_or(mod_name.len());
    p = append_bytes(&mut m, p, &mod_name[..name_len]);
    crate::debug_log(&m[..p]);

    // Search for the target module
    let mut found_id = psp::sys::SceUid(-1);
    let mut info: psp::sys::SceKernelModuleInfo = core::mem::zeroed();

    for i in 0..(count as usize).min(256) {
        info = core::mem::zeroed();
        info.size = core::mem::size_of::<psp::sys::SceKernelModuleInfo>();
        let ret = psp::sys::sceKernelQueryModuleInfo(ids[i], &mut info);
        if ret < 0 {
            continue;
        }

        // Compare name (info.name is [u8; 28])
        let mut matches = true;
        for j in 0..name_len {
            if j >= 28 || info.name[j] != mod_name[j] {
                matches = false;
                break;
            }
        }
        // Also check null terminator
        if matches && name_len < 28 && info.name[name_len] != 0 {
            matches = false;
        }

        if matches {
            found_id = ids[i];
            break;
        }
    }

    if found_id < psp::sys::SceUid(0) {
        let mut m = [0u8; 60];
        let mut p = append_bytes(&mut m, 0, b"[KDUMP] module not found: ");
        p = append_bytes(&mut m, p, &mod_name[..name_len]);
        crate::debug_log(&m[..p]);
        return;
    }

    // Log module info
    let text_addr = info.text_addr;
    let text_size = info.text_size;
    let data_size = info.data_size;
    let total_size = text_size + data_size;
    let seg0_addr = info.segment_addr[0] as u32;
    let seg0_size = info.segment_size[0] as u32;

    let mut m = [0u8; 120];
    let mut p = append_bytes(&mut m, 0, b"[KDUMP] found! text=0x");
    p = append_hex(&mut m, p, text_addr);
    p = append_bytes(&mut m, p, b" tsize=0x");
    p = append_hex(&mut m, p, text_size);
    p = append_bytes(&mut m, p, b" dsize=0x");
    p = append_hex(&mut m, p, data_size);
    p = append_bytes(&mut m, p, b" seg0=0x");
    p = append_hex(&mut m, p, seg0_addr);
    p = append_bytes(&mut m, p, b"+0x");
    p = append_hex(&mut m, p, seg0_size);
    crate::debug_log(&m[..p]);

    // Use the larger of (text+data) or seg0_size for dump
    let dump_size = if seg0_size > total_size { seg0_size } else { total_size };
    let dump_addr = if seg0_addr != 0 { seg0_addr } else { text_addr };

    if dump_size == 0 || dump_size > 0x100000 {
        crate::debug_log(b"[KDUMP] invalid size, skipping");
        return;
    }

    // Write raw memory to file
    let fd = psp::sys::sceIoOpen(
        out_path.as_ptr(),
        psp::sys::IoOpenFlags::CREAT | psp::sys::IoOpenFlags::WR_ONLY | psp::sys::IoOpenFlags::TRUNC,
        0o777,
    );
    if fd < psp::sys::SceUid(0) {
        crate::debug_log(b"[KDUMP] failed to open output file");
        return;
    }

    // Write a small header: [base_addr: u32] [dump_size: u32] [module_name: 28 bytes]
    let hdr_addr = dump_addr.to_le_bytes();
    let hdr_size = dump_size.to_le_bytes();
    psp::sys::sceIoWrite(fd, hdr_addr.as_ptr() as *const _, 4);
    psp::sys::sceIoWrite(fd, hdr_size.as_ptr() as *const _, 4);
    psp::sys::sceIoWrite(fd, info.name.as_ptr() as *const _, 28);
    // Total header: 36 bytes, raw dump follows

    // Dump memory in chunks (kernel memory may need uncached access)
    let src = dump_addr as *const u8;
    let mut written = 0u32;
    while written < dump_size {
        let chunk = if dump_size - written > 4096 { 4096 } else { dump_size - written };
        let ret = psp::sys::sceIoWrite(
            fd,
            src.add(written as usize) as *const _,
            chunk as usize,
        );
        if ret < 0 {
            break;
        }
        written += chunk;
    }

    psp::sys::sceIoClose(fd);

    let mut m = [0u8; 60];
    let mut p = append_bytes(&mut m, 0, b"[KDUMP] dumped 0x");
    p = append_hex(&mut m, p, written);
    p = append_bytes(&mut m, p, b" bytes to file");
    crate::debug_log(&m[..p]);
}
