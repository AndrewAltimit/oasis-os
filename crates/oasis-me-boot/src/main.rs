//! Minimal kernel PRX that boots the PSP Media Engine.
//!
//! Replaces cooleyesBridge.prx (GPL, by cooleyes) with a clean Rust
//! implementation. Resolves `sceMeBootStart660` via `sctrlHENFindFunction`
//! and calls it during `module_start`. The EBOOT loads this PRX from the
//! video thread before attempting H.264 decode.
//!
//! Build: cd crates/oasis-me-boot && RUST_PSP_BUILD_STD=1 cargo +nightly psp --release
//! Output: target/mipsel-sony-psp-std/release/oasis-me-boot.prx

#![no_std]
#![no_main]

psp::module_kernel!("OasisMeBoot", 1, 0);

/// NID for sceMeBootStart660 in sceMeCore_driver.
const NID_ME_BOOT_START_660: u32 = 0x5DFF5C50;

fn psp_main() {
    // Resolve sceMeBootStart660 from the kernel's ME driver.
    // Module: sceMeCodecWrapper (or null to search all modules)
    // Library: sceMeCore_driver
    // SAFETY: find_function calls sctrlHENFindFunction (CFW kernel API).
    let boot_fn = unsafe {
        psp::hook::find_function(
            b"sceMeCodecWrapper\0".as_ptr(),
            b"sceMeCore_driver\0".as_ptr(),
            NID_ME_BOOT_START_660,
        )
    };

    if let Some(ptr) = boot_fn {
        // SAFETY: sceMeBootStart660 takes one i32 arg: the devkit version.
        // FW 6.60 = 0x06060010, FW 6.61 = 0x06060110.
        // cooleyesBridge passes the devkit version from its caller.
        let me_boot: unsafe extern "C" fn(i32) -> i32 =
            unsafe { core::mem::transmute(ptr) };
        let ret = unsafe { me_boot(0x06060010) };

        // Log result to file (kernel mode — can't use std I/O).
        log_result(b"[ME-BOOT] sceMeBootStart660 = ", ret as u32);
    } else {
        log_msg(b"[ME-BOOT] sceMeBootStart660 not found");
    }
}

/// Write a log message + hex value to the EBOOT's log file.
fn log_result(prefix: &[u8], val: u32) {
    unsafe {
        let fd = psp::sys::sceIoOpen(
            b"ms0:/PSP/GAME/OASISOS/eboot.log\0".as_ptr(),
            psp::sys::IoOpenFlags::APPEND
                | psp::sys::IoOpenFlags::CREAT
                | psp::sys::IoOpenFlags::WR_ONLY,
            0o777,
        );
        if fd >= psp::sys::SceUid(0) {
            psp::sys::sceIoWrite(fd, prefix.as_ptr() as *const _, prefix.len());
            let mut hex = [0u8; 10];
            hex[0] = b'0';
            hex[1] = b'x';
            for i in 0..8 {
                let nibble = ((val >> (28 - i * 4)) & 0xF) as u8;
                hex[2 + i] = if nibble < 10 { b'0' + nibble } else { b'a' + nibble - 10 };
            }
            psp::sys::sceIoWrite(fd, hex.as_ptr() as *const _, 10);
            psp::sys::sceIoWrite(fd, b"\n".as_ptr() as *const _, 1);
            psp::sys::sceIoClose(fd);
        }
    }
}

fn log_msg(msg: &[u8]) {
    unsafe {
        let fd = psp::sys::sceIoOpen(
            b"ms0:/PSP/GAME/OASISOS/eboot.log\0".as_ptr(),
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
