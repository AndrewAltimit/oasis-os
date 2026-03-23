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

/// Dump thread entry point. Polls for trigger, then dumps.
unsafe extern "C" fn dump_thread_entry(
    _args: usize,
    _argp: *mut core::ffi::c_void,
) -> i32 {
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

        // ME RPC and hook modules disabled (caused PRX crash).

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
/// Reads in 8KB chunks to avoid stack overflow in kernel thread.
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
