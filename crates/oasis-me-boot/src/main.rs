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
#![feature(asm_experimental_arch)]

psp::module_kernel!("OasisMeBoot", 1, 0);

// K1 register manipulation (equivalent to pspSdkSetK1).
// K1 ($k1, register $27) controls kernel address validation.
unsafe extern "C" {
    fn set_k1(val: u32) -> u32;
}

core::arch::global_asm!(
    r#"
    .section .text
    .global set_k1
    set_k1:
        move $v0, $k1
        move $k1, $a0
        jr $ra
        nop
    "#
);

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
        // SAFETY: sceMeBootStart660 takes one i32 arg: mebooterType (0 or 1).
        // cooleyesBridge passes mebooterType=0 and clears K1 protection bits.
        // K1 clearing is critical — kernel functions check K1 for address validation.
        let me_boot: unsafe extern "C" fn(i32) -> i32 =
            unsafe { core::mem::transmute(ptr) };

        // Clear K1 protection bits (like pspSdkSetK1(0) in cooleyesBridge).
        let old_k1 = unsafe { set_k1(0) };
        let ret = unsafe { me_boot(0) }; // mebooterType = 0
        unsafe { set_k1(old_k1); }

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
