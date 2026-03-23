//! Hook empty avcodec.prx ME submission stubs.
//!
//! The user-mode `avcodec.prx` has empty ME submission functions that just
//! return (void stubs). This module patches those stubs at runtime to
//! redirect to the real `sceMeVideo_driver` functions in `sceMeCodecWrapper`.
//!
//! ## Empty stubs in avcodec.prx (from Ghidra analysis)
//!
//! | Offset | Used by | Original |
//! |--------|---------|----------|
//! | 0x4414 | Open | `void { return; }` |
//! | 0x4424 | ScanHeader | `void { return; }` |
//! | 0x4434 | Init | `void { return; }` |
//! | 0x4394 | ME worker loop | `void { return; }` |
//! | 0x438c | ME wait/poll | `void { return; }` |
//!
//! After patching, sceMpeg/sceVideocodec calls from user-mode will
//! route through the real ME driver path.

use core::ffi::c_void;

/// Attempt to hook the avcodec empty stubs.
///
/// Must be called from kernel mode after ME modules are loaded.
///
/// # Safety
/// Patches kernel memory (instruction rewriting).
pub unsafe fn hook_avcodec_stubs() {
    crate::debug_log(b"[ME-HOOK] searching for avcodec.prx stubs...");

    // List codec-related modules for debugging.
    list_all_modules();

    // The kernel sceAvcodec_wrapper has stubs that jump BACK to user-space
    // (sceMpeg_library), which then tries to submit to ME via the user-mode
    // avcodec's empty stubs. Fix: patch the KERNEL stubs to jump directly
    // to sceMeVideo_driver functions instead.
    //
    // We already found sceAvcodec_wrapper by module name in earlier runs.
    // Find it again and patch its stubs.
    let names: &[&[u8]] = &[b"Avcodec", b"avcodec", b"sceAvcodec"];
    let mut avcodec_base = 0u32;
    for name in names {
        avcodec_base = find_module_base(name);
        if avcodec_base != 0 && avcodec_base >= 0x8800_0000 {
            break; // Only want kernel module
        }
        avcodec_base = 0;
    }

    if avcodec_base != 0 {
        let mut msg = [0u8; 64];
        let mut mp = crate::me_dump::append_bytes(&mut msg, 0, b"[ME-HOOK] kernel avcodec @0x");
        mp = crate::me_dump::append_hex(&mut msg, mp, avcodec_base);
        crate::debug_log(&msg[..mp]);

        // Patch the kernel stubs to jump to ME driver functions.
        unsafe { patch_kernel_avcodec(avcodec_base) };
    } else {
        crate::debug_log(b"[ME-HOOK] kernel avcodec not found");
        // Fallback: scan user space.
        scan_and_patch_stubs();
    }
}

/// List all loaded modules (for debugging module name discovery).
fn list_all_modules() {
    let mut mod_ids = [psp::sys::SceUid(0); 128];
    let mut count: i32 = 0;
    let buf_bytes = 128 * core::mem::size_of::<psp::sys::SceUid>() as i32;

    let ret = unsafe {
        psp::sys::sceKernelGetModuleIdList(
            mod_ids.as_mut_ptr(),
            buf_bytes,
            &mut count,
        )
    };
    if ret < 0 || count <= 0 {
        return;
    }

    for i in 0..count as usize {
        let mid = mod_ids[i];
        let mut info: psp::sys::SceKernelModuleInfo = unsafe {
            core::mem::zeroed()
        };
        info.size = core::mem::size_of::<psp::sys::SceKernelModuleInfo>();

        let ret = unsafe { psp::sys::sceKernelQueryModuleInfo(mid, &mut info) };
        if ret < 0 {
            continue;
        }

        let name_len = info.name.iter().position(|&b| b == 0)
            .unwrap_or(info.name.len());
        let mod_name = &info.name[..name_len];
        let text_addr = info.text_addr;
        let text_size = info.text_size;

        // Only log codec/mpeg/video related modules.
        let dominated = [
            b"codec" as &[u8], b"Codec", b"mpeg", b"Mpeg",
            b"video", b"Video", b"avcodec", b"Avcodec",
            b"memlmd", b"MeCodec", b"wrapper",
        ];
        let is_relevant = dominated.iter().any(|t|
            crate::me_dump::contains_bytes(mod_name, t)
        );
        if is_relevant {
            let mut msg = [0u8; 80];
            let mut mp = crate::me_dump::append_bytes(&mut msg, 0, b"[ME-HOOK] mod: ");
            mp = crate::me_dump::append_bytes(&mut msg, mp, mod_name);
            mp = crate::me_dump::append_bytes(&mut msg, mp, b" @0x");
            mp = crate::me_dump::append_hex(&mut msg, mp, text_addr);
            mp = crate::me_dump::append_bytes(&mut msg, mp, b" sz=");
            mp = crate::me_dump::append_dec(&mut msg, mp, text_size);
            crate::debug_log(&msg[..mp]);
        }
    }
}

/// Find a loaded module's text base address by name.
fn find_module_base(name: &[u8]) -> u32 {
    let mut mod_ids = [psp::sys::SceUid(0); 128];
    let mut count: i32 = 0;
    let buf_bytes = 128 * core::mem::size_of::<psp::sys::SceUid>() as i32;

    let ret = unsafe {
        psp::sys::sceKernelGetModuleIdList(
            mod_ids.as_mut_ptr(),
            buf_bytes,
            &mut count,
        )
    };
    if ret < 0 || count <= 0 {
        return 0;
    }

    for i in 0..count as usize {
        let mid = mod_ids[i];
        let mut info: psp::sys::SceKernelModuleInfo = unsafe {
            core::mem::zeroed()
        };
        info.size = core::mem::size_of::<psp::sys::SceKernelModuleInfo>();

        let ret = unsafe { psp::sys::sceKernelQueryModuleInfo(mid, &mut info) };
        if ret < 0 {
            continue;
        }

        let name_len = info.name.iter().position(|&b| b == 0)
            .unwrap_or(info.name.len());
        let mod_name = &info.name[..name_len];

        // Check if module name contains our search term.
        if crate::me_dump::contains_bytes(mod_name, name) {
            return info.text_addr;
        }
    }
    0
}

/// Patch kernel sceAvcodec_wrapper stubs to call sceMeVideo_driver.
///
/// The kernel avcodec stubs currently contain J instructions that jump
/// back to user-mode sceMpeg functions. We replace them with J instructions
/// to the real sceMeVideo_driver functions in sceMeCodecWrapper.
unsafe fn patch_kernel_avcodec(base: u32) {
    crate::debug_log(b"[ME-HOOK] patching kernel avcodec stubs...");

    let stubs: &[(u32, &[u8], u32)] = &[
        // (offset, label, sceMeVideo NID)
        (0x4414, b"open", 0xC441994C),
        (0x4424, b"scan", 0x8768915D),
        (0x4434, b"init", 0xE8CD3C75),
        (0x4394, b"work", 0x6D68B223),
        (0x438c, b"wait", 0x4D78330C),
    ];

    for &(offset, label, nid) in stubs {
        let stub_addr = base + offset;
        let current = unsafe { core::ptr::read_volatile(stub_addr as *const u32) };

        // Resolve the real ME function.
        let me_fn = unsafe {
            psp::hook::find_function(
                b"sceMeCodecWrapper\0".as_ptr(),
                b"sceMeVideo_driver\0".as_ptr(),
                nid,
            )
        };

        if let Some(target_ptr) = me_fn {
            let target = target_ptr as u32;

            // Build J instruction: opcode=000010, target=(addr>>2)
            let j_insn = 0x0800_0000 | ((target >> 2) & 0x03FF_FFFF);

            let mut msg = [0u8; 80];
            let mut mp = crate::me_dump::append_bytes(&mut msg, 0, b"[ME-HOOK] ");
            mp = crate::me_dump::append_bytes(&mut msg, mp, label);
            mp = crate::me_dump::append_bytes(&mut msg, mp, b" 0x");
            mp = crate::me_dump::append_hex(&mut msg, mp, current);
            mp = crate::me_dump::append_bytes(&mut msg, mp, b" -> 0x");
            mp = crate::me_dump::append_hex(&mut msg, mp, j_insn);
            crate::debug_log(&msg[..mp]);

            // Write the patch.
            unsafe {
                core::ptr::write_volatile(stub_addr as *mut u32, j_insn);
                // Keep the delay slot (nop) intact.
            }

            // Flush caches.
            unsafe {
                psp::sys::sceKernelDcacheWritebackInvalidateRange(
                    stub_addr as *const core::ffi::c_void, 8,
                );
                psp::sys::sceKernelIcacheInvalidateRange(
                    stub_addr as *const core::ffi::c_void, 8,
                );
            }
        } else {
            let mut msg = [0u8; 64];
            let mut mp = crate::me_dump::append_bytes(&mut msg, 0, b"[ME-HOOK] ");
            mp = crate::me_dump::append_bytes(&mut msg, mp, label);
            mp = crate::me_dump::append_bytes(&mut msg, mp, b" ME fn not found");
            crate::debug_log(&msg[..mp]);
        }
    }

    crate::debug_log(b"[ME-HOOK] kernel patch complete!");
}

/// Analyze stubs at known offsets from avcodec base (diagnostic only).
#[allow(dead_code)]
unsafe fn patch_stubs_at_base(base: u32) {
    // The empty stubs are `jr $ra; nop` (2 instructions = 8 bytes).
    // MIPS encoding: jr $ra = 0x03E00008, nop = 0x00000000
    //
    // We replace them with `j <target>; nop` to redirect to the real
    // sceMeVideo_driver functions.
    //
    // However, the empty stubs don't directly map 1:1 to sceMeVideo_driver
    // functions — the avcodec code calls these internal stubs from various
    // places with specific register setups. We need to understand what
    // each stub is supposed to do.
    //
    // The simplest approach: find the empty stubs by pattern matching
    // (they're `jr $ra; nop` sequences at specific offsets), then patch
    // them to call through to the sceMeCodecWrapper functions that do
    // the actual ME communication.
    //
    // From the Ghidra analysis, the key stub that causes 0x806201FE:
    // FUN_00004414 is called by sceVideocodecOpen's internal init path.
    // When it returns void (no ME submission), the calling code sees
    // uninitialized return values and produces the error.

    // First, verify the stubs are actually empty (jr $ra; nop).
    let stubs: &[(u32, &[u8])] = &[
        (0x438c, b"me_wait"),
        (0x4394, b"me_worker"),
        (0x4414, b"me_open"),
        (0x4424, b"me_scan"),
        (0x4434, b"me_init"),
    ];

    let jr_ra: u32 = 0x03E0_0008; // jr $ra
    let nop: u32 = 0x0000_0000;   // nop

    for &(offset, label) in stubs {
        let addr = base + offset;
        let insn0 = unsafe { core::ptr::read_volatile(addr as *const u32) };
        let insn1 = unsafe { core::ptr::read_volatile((addr + 4) as *const u32) };

        let is_empty = insn0 == jr_ra && insn1 == nop;

        let mut msg = [0u8; 80];
        let mut mp = crate::me_dump::append_bytes(&mut msg, 0, b"[ME-HOOK] ");
        mp = crate::me_dump::append_bytes(&mut msg, mp, label);
        mp = crate::me_dump::append_bytes(&mut msg, mp, b" @0x");
        mp = crate::me_dump::append_hex(&mut msg, mp, addr);
        if is_empty {
            mp = crate::me_dump::append_bytes(&mut msg, mp, b" EMPTY");
        } else {
            mp = crate::me_dump::append_bytes(&mut msg, mp, b" =0x");
            mp = crate::me_dump::append_hex(&mut msg, mp, insn0);
        }
        crate::debug_log(&msg[..mp]);
    }

    // Now find the real ME functions we want to redirect to.
    // The sceMeCodecWrapper exports sceMeVideo_driver with the actual
    // ME RPC implementation. We need to find the right function for
    // each stub.
    //
    // From our RPC analysis:
    // - me_open (0x4414): should call sceMeVideo::C441994C (RPC cmd 0x24)
    // - me_scan (0x4424): should call sceMeVideo::8768915D (RPC cmd 0x20)
    // - me_init (0x4434): should call sceMeVideo::E8CD3C75
    // - me_worker (0x4394): should call sceMeVideo::6D68B223 (RPC cmd 0x27)
    // - me_wait (0x438c): should poll/wait for ME completion
    //
    // But these aren't simple redirects — the avcodec stubs are called
    // with specific internal register states, not the public API signature.
    // We need to understand the calling convention of each stub.
    //
    // For now, log what we found. The actual patching requires matching
    // the avcodec internal calling convention to the ME driver functions.

    crate::debug_log(b"[ME-HOOK] stub analysis complete");
    crate::debug_log(b"[ME-HOOK] TODO: patch stubs with ME driver redirects");
}

/// Scan user-mode memory for avcodec's empty ME stubs.
///
/// The user-mode avcodec.prx has 5 empty stubs at offsets 0x438C-0x4434.
/// They're `jr $ra; nop` sequences. We find the module by looking for
/// these 5 stubs in the right relative positions.
fn scan_and_patch_stubs() {
    crate::debug_log(b"[ME-HOOK] scanning for user avcodec stubs...");

    let jr_ra: u32 = 0x03E0_0008;
    let nop: u32 = 0x0000_0000;

    // Known stub offsets within avcodec.prx (from Ghidra):
    // 0x438C, 0x4394, 0x4414, 0x4424, 0x4434
    // Gaps: +8, +0x80, +0x10, +0x10
    // We search for any base where ALL 5 offsets have jr $ra; nop.

    let stub_offsets: &[u32] = &[0x438C, 0x4394, 0x4414, 0x4424, 0x4434];

    // Search user module space. avcodec loads around 0x08A3xxxx-0x08C0xxxx.
    let search_start: u32 = 0x0880_0000;
    let search_end: u32 = 0x08D0_0000;
    // Step by 0x100 (modules are page-aligned).
    let mut base = search_start;

    while base < search_end - 0x4500 {
        let mut all_empty = true;
        for &off in stub_offsets {
            let addr = base + off;
            let insn0 = unsafe { core::ptr::read_volatile(addr as *const u32) };
            let insn1 = unsafe { core::ptr::read_volatile((addr + 4) as *const u32) };
            if insn0 != jr_ra || insn1 != nop {
                all_empty = false;
                break;
            }
        }

        if all_empty {
            let mut msg = [0u8; 64];
            let mut mp = crate::me_dump::append_bytes(
                &mut msg, 0, b"[ME-HOOK] FOUND avcodec base=0x",
            );
            mp = crate::me_dump::append_hex(&mut msg, mp, base);
            crate::debug_log(&msg[..mp]);

            // Verify: check for the version string "Lib-PSP avcodec" nearby.
            // It's at offset ~0x4A80 in avcodec.prx.
            let magic_off = base + 0x4A80;
            let magic = unsafe {
                core::slice::from_raw_parts(magic_off as *const u8, 15)
            };
            if magic.starts_with(b"Lib-PSP avcodec") {
                crate::debug_log(b"[ME-HOOK] version string confirmed!");
            } else {
                // Try nearby
                let alt = base + 0x4A60;
                let alt_slice = unsafe {
                    core::slice::from_raw_parts(alt as *const u8, 30)
                };
                if crate::me_dump::contains_bytes(alt_slice, b"Lib-PSP") {
                    crate::debug_log(b"[ME-HOOK] version string near match");
                }
            }

            // Now patch the stubs!
            unsafe { patch_user_avcodec(base) };
            return;
        }

        base += 0x100;
    }

    crate::debug_log(b"[ME-HOOK] empty stub pattern not found");
    crate::debug_log(b"[ME-HOOK] scanning for avcodec string...");

    // Find "Lib-PSP avcodec" string in user memory to locate the module.
    let needle = b"Lib-PSP avcodec";
    let mut addr = search_start;
    while addr < search_end - 20 {
        let byte = unsafe { core::ptr::read_volatile(addr as *const u8) };
        if byte == b'L' {
            let slice = unsafe {
                core::slice::from_raw_parts(addr as *const u8, 15)
            };
            if slice == needle {
                let mut msg = [0u8; 64];
                let mut mp = crate::me_dump::append_bytes(
                    &mut msg, 0, b"[ME-HOOK] avcodec str @0x",
                );
                mp = crate::me_dump::append_hex(&mut msg, mp, addr);
                crate::debug_log(&msg[..mp]);

                // The string is typically at offset ~0x4A80 from module base.
                // So module base ≈ addr - 0x4A80.
                let est_base = addr - 0x4A80;
                let mut msg2 = [0u8; 64];
                let mut mp2 = crate::me_dump::append_bytes(
                    &mut msg2, 0, b"[ME-HOOK] est base=0x",
                );
                mp2 = crate::me_dump::append_hex(&mut msg2, mp2, est_base);
                crate::debug_log(&msg2[..mp2]);

                // Dump what's at the stub offsets from this base.
                let offsets: &[(u32, &[u8])] = &[
                    (0x438c, b"wait"),
                    (0x4394, b"worker"),
                    (0x4414, b"open"),
                    (0x4424, b"scan"),
                    (0x4434, b"init"),
                ];
                for &(off, label) in offsets {
                    let saddr = est_base + off;
                    let insn = unsafe { core::ptr::read_volatile(saddr as *const u32) };
                    let mut msg3 = [0u8; 80];
                    let mut mp3 = crate::me_dump::append_bytes(&mut msg3, 0, b"[ME-HOOK] ");
                    mp3 = crate::me_dump::append_bytes(&mut msg3, mp3, label);
                    mp3 = crate::me_dump::append_bytes(&mut msg3, mp3, b" @0x");
                    mp3 = crate::me_dump::append_hex(&mut msg3, mp3, saddr);
                    mp3 = crate::me_dump::append_bytes(&mut msg3, mp3, b" =0x");
                    mp3 = crate::me_dump::append_hex(&mut msg3, mp3, insn);
                    crate::debug_log(&msg3[..mp3]);
                }
                break;
            }
        }
        addr += 1;
    }
}

/// Patch user-mode avcodec empty stubs to call sceMeVideo_driver.
///
/// We overwrite the `jr $ra; nop` with `j <me_driver_fn>; nop`.
///
/// # Safety
/// Patches user-mode code from kernel context.
unsafe fn patch_user_avcodec(base: u32) {
    crate::debug_log(b"[ME-HOOK] patching user avcodec stubs...");

    // Resolve the real ME video driver functions.
    let me_fns: &[(&[u8], u32, u32)] = &[
        // (label, avcodec_offset, sceMeVideo NID)
        // me_open at 0x4414: needs sceMeVideo::C441994C
        (b"open", 0x4414, 0xC441994C),
        // me_scan at 0x4424: needs sceMeVideo::8768915D
        (b"scan", 0x4424, 0x8768915D),
        // me_init at 0x4434: needs sceMeVideo::E8CD3C75
        (b"init", 0x4434, 0xE8CD3C75),
        // me_worker at 0x4394: needs sceMeVideo::6D68B223 (decode)
        (b"work", 0x4394, 0x6D68B223),
        // me_wait at 0x438c: needs sceMeVideo::4D78330C (getedram/wait)
        (b"wait", 0x438c, 0x4D78330C),
    ];

    for &(label, offset, nid) in me_fns {
        let stub_addr = base + offset;

        // Resolve the real ME function.
        let me_fn = unsafe {
            psp::hook::find_function(
                b"sceMeCodecWrapper\0".as_ptr(),
                b"sceMeVideo_driver\0".as_ptr(),
                nid,
            )
        };

        if let Some(target_ptr) = me_fn {
            let target = target_ptr as u32;

            // Build J instruction: opcode=000010, target=(addr>>2)&0x03FFFFFF
            let j_insn = 0x0800_0000 | ((target >> 2) & 0x03FF_FFFF);

            // Write the patch.
            unsafe {
                core::ptr::write_volatile(stub_addr as *mut u32, j_insn);
                core::ptr::write_volatile((stub_addr + 4) as *mut u32, 0); // nop delay slot
            }

            // Flush I-cache + D-cache for the patched region.
            unsafe {
                psp::sys::sceKernelDcacheWritebackInvalidateRange(
                    stub_addr as *const core::ffi::c_void, 8,
                );
                psp::sys::sceKernelIcacheInvalidateRange(
                    stub_addr as *const core::ffi::c_void, 8,
                );
            }

            let mut msg = [0u8; 80];
            let mut mp = crate::me_dump::append_bytes(&mut msg, 0, b"[ME-HOOK] patched ");
            mp = crate::me_dump::append_bytes(&mut msg, mp, label);
            mp = crate::me_dump::append_bytes(&mut msg, mp, b" @0x");
            mp = crate::me_dump::append_hex(&mut msg, mp, stub_addr);
            mp = crate::me_dump::append_bytes(&mut msg, mp, b" -> 0x");
            mp = crate::me_dump::append_hex(&mut msg, mp, target);
            crate::debug_log(&msg[..mp]);
        } else {
            let mut msg = [0u8; 64];
            let mut mp = crate::me_dump::append_bytes(&mut msg, 0, b"[ME-HOOK] ");
            mp = crate::me_dump::append_bytes(&mut msg, mp, label);
            mp = crate::me_dump::append_bytes(&mut msg, mp, b" ME fn not found");
            crate::debug_log(&msg[..mp]);
        }
    }

    crate::debug_log(b"[ME-HOOK] patching complete!");
}

fn append_dec(buf: &mut [u8], pos: usize, val: u32) -> usize {
    crate::me_dump::append_dec(buf, pos, val)
}
