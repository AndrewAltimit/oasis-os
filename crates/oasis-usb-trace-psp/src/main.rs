//! PSP Kernel Memory Reader — based on VBUS test v2 (known working on PRO-C2)
//!
//! Uses EXACT same structure/imports as VBUSTest2 which worked.

#![no_std]
#![no_main]

use psp::hw::hw_read32;
use psp::sys::{
    self, CtrlButtons, SceCtrlData,
    sceCtrlPeekBufferPositive,
    sceKernelDelayThread, sceUsbGetState, sceUsbStart, sceUsbStop,
};
use core::ffi::c_void;

psp::module_kernel!("VBUSTest2", 1, 0);

fn wait_cross() {
    let mut p = SceCtrlData::default();
    loop {
        unsafe { sceCtrlPeekBufferPositive(&mut p, 1) };
        if !p.buttons.intersects(CtrlButtons::CROSS) { break; }
        unsafe { sceKernelDelayThread(16_000) };
    }
    loop {
        unsafe { sceCtrlPeekBufferPositive(&mut p, 1) };
        if p.buttons.intersects(CtrlButtons::CROSS) { return; }
        unsafe { sceKernelDelayThread(16_000) };
    }
}

fn psp_main() {
    let _ = psp::callback::setup_exit_callback();
    let _ = unsafe { psp::sys::sceCtrlSetSamplingMode(psp::sys::CtrlMode::Digital) };

    psp::dprintln!("=== Kernel Mem Reader ===");
    psp::dprintln!("Based on VBUSTest2 (works)");
    psp::dprintln!("");

    // Same SYSCTL reads as VBUS test
    let r40 = unsafe { hw_read32(0xBC10_0040) };
    psp::dprintln!("SYSCTL+40 = 0x{:08X}", r40);
    let usb = unsafe { sceUsbGetState() }.bits();
    psp::dprintln!("UsbState   = 0x{:03X}", usb);
    psp::dprintln!("");

    // ── Page 1: Read kernel memory ──
    psp::dprintln!("CROSS = search for USB driver");
    wait_cross();

    // Search for sceUSB_Driver string
    psp::dprintln!("Searching 0x88000000...");
    let needle = b"sceUSB_Driv";
    let base: u32 = 0x88000000;
    let mut found: u32 = 0;

    let mut i: u32 = 0;
    while i < 0x300000 {
        let mut ok = true;
        for j in 0..needle.len() as u32 {
            let b = unsafe { *(( base + i + j) as *const u8) };
            if b != needle[j as usize] { ok = false; break; }
        }
        if ok {
            found = base + i;
            psp::dprintln!("Found: 0x{:08X}", found);
            break;
        }
        i += 4;
    }
    if found == 0 {
        psp::dprintln!("Not found in 3MB");
    }
    psp::dprintln!("CROSS = next");
    wait_cross();

    // ── Page 2: Search for Activate NID ──
    psp::dprintln!("Searching for NID 0x586DB82C");
    let nid: u32 = 0x586DB82C;
    let mut nid_addr: u32 = 0;
    i = 0;
    while i < 0x300000 {
        let v = unsafe { *((base + i) as *const u32) };
        if v == nid {
            nid_addr = base + i;
            psp::dprintln!("NID at 0x{:08X}", nid_addr);
            // Show surrounding 16 words
            let s = if i >= 24 { i - 24 } else { 0 };
            for k in 0..16u32 {
                let a = base + s + k * 4;
                let val = unsafe { *((a) as *const u32) };
                let m = if val == nid { "*" }
                    else if val == 0xAE5DE6AF { "S" }
                    else if val == 0xC21645A4 { "G" }
                    else if val >= 0x88000000 && val < 0x88400000 { "F" }
                    else { "" };
                psp::dprintln!("{:08X}={:08X}{}", a, val, m);
            }
            break;
        }
        i += 4;
    }
    if nid_addr == 0 { psp::dprintln!("NID not found"); }
    psp::dprintln!("CROSS = next");
    wait_cross();

    // ── Page 3: More of NID table ──
    if nid_addr != 0 {
        let s = nid_addr - 24;
        for k in 16..32u32 {
            let a = s + k * 4;
            let val = unsafe { *((a) as *const u32) };
            let m = if val >= 0x88000000 && val < 0x88400000 { "F" } else { "" };
            psp::dprintln!("{:08X}={:08X}{}", a, val, m);
        }
    }
    psp::dprintln!("CROSS = next");
    wait_cross();

    // ── Page 4: Search for LUI 0xBC10 near USB code ──
    psp::dprintln!("LUI BC10/BE24/BFC0 search:");
    let mut count = 0u32;
    i = 0;
    while i < 0x300000 && count < 22 {
        let instr = unsafe { *((base + i) as *const u32) };
        let op = (instr >> 26) & 0x3F;
        if op == 15 {
            let imm = instr & 0xFFFF;
            if imm == 0xBC10 || imm == 0xBE24 || imm == 0xBFC0 {
                let a = base + i;
                let rt = (instr >> 16) & 0x1F;
                let next = unsafe { *((base + i + 4) as *const u32) };
                let nop = (next >> 26) & 0x3F;
                if nop == 35 || nop == 43 {
                    let noff = next & 0xFFFF;
                    psp::dprintln!("{:08X} r{} {:04X}+{:04X}", a, rt, imm, noff);
                    count += 1;
                }
            }
        }
        i += 4;
    }
    psp::dprintln!("{} found", count);

    psp::dprintln!("");
    psp::dprintln!("Done! Photo this screen");
    unsafe { sceKernelDelayThread(60_000_000) };
}
