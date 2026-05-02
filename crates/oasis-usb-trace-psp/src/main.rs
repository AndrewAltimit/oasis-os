//! PSP USB Host Mode Phase 3G: Cmd 0x45 retry with correct format
//!
//! Phase 3F with buggy length=6 caused black screen + solid memory card light
//! on cmd 0x45 — this matches the USB firmware reboot sequence!
//! This version sends cmd 0x45 with correct length=3 format, then waits
//! 60 seconds to let the sequence complete without interruption.
//!
//! IMPORTANT: Keep FNB58 plugged in. Do NOT reboot during the wait.

#![no_std]
#![no_main]

use psp::hw::{hw_read32, hw_write32};
use psp::sys::{
    CtrlButtons, SceCtrlData, IoOpenFlags,
    sceCtrlPeekBufferPositive,
    sceKernelDelayThread,
};
use core::ffi::c_void;

psp::module_kernel!("USBHostTest", 1, 0);

static LOG_PATH: &[u8] = b"ms0:/PSP/GAME/USBTRACE/phase3g.log\0";

struct Logger;
impl Logger {
    fn init() {
        // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
        let fd = unsafe {
            psp::sys::sceIoOpen(
                LOG_PATH.as_ptr(),
                IoOpenFlags::WR_ONLY | IoOpenFlags::CREAT | IoOpenFlags::TRUNC,
                0o777,
            )
        };
        // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
        if fd.0 >= 0 { unsafe { psp::sys::sceIoClose(fd) }; }
    }
    fn log(s: &str) {
        psp::dprintln!("{}", s);
        // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
        let fd = unsafe {
            psp::sys::sceIoOpen(
                LOG_PATH.as_ptr(),
                IoOpenFlags::WR_ONLY | IoOpenFlags::APPEND,
                0o777,
            )
        };
        if fd.0 >= 0 {
            // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
            unsafe {
                psp::sys::sceIoWrite(fd, s.as_ptr() as *const c_void, s.len());
                psp::sys::sceIoWrite(fd, b"\n".as_ptr() as *const c_void, 1);
                psp::sys::sceIoClose(fd);
            }
        }
    }
}

struct Fmt { buf: [u8; 200], pos: usize }
impl Fmt {
    fn new() -> Self { Self { buf: [0u8; 200], pos: 0 } }
    fn s(&self) -> &str {
        // SAFETY: all bytes written into this buffer above are ASCII.
        unsafe { core::str::from_utf8_unchecked(&self.buf[..self.pos]) }
    }
    fn p(&mut self, s: &str) {
        let n = s.len().min(self.buf.len() - self.pos);
        self.buf[self.pos..self.pos + n].copy_from_slice(&s.as_bytes()[..n]);
        self.pos += n;
    }
    fn h8(&mut self, v: u32) {
        let hex = b"0123456789ABCDEF";
        for i in (0..8).rev() {
            if self.pos < self.buf.len() {
                self.buf[self.pos] = hex[((v >> (i * 4)) & 0xF) as usize];
                self.pos += 1;
            }
        }
    }
    fn h2(&mut self, v: u8) {
        let hex = b"0123456789ABCDEF";
        if self.pos + 1 < self.buf.len() {
            self.buf[self.pos] = hex[(v >> 4) as usize];
            self.buf[self.pos + 1] = hex[(v & 0xF) as usize];
            self.pos += 2;
        }
    }
}

fn lr(label: &str, val: u32) {
    let mut f = Fmt::new();
    f.p(label); f.h8(val);
    Logger::log(f.s());
}

fn wait_cross() {
    let mut p = SceCtrlData::default();
    loop {
        // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
        unsafe { sceCtrlPeekBufferPositive(&mut p, 1) };
        if !p.buttons.intersects(CtrlButtons::CROSS) { break; }
        // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
        unsafe { sceKernelDelayThread(16_000) };
    }
    loop {
        // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
        unsafe { sceCtrlPeekBufferPositive(&mut p, 1) };
        if p.buttons.intersects(CtrlButtons::CROSS) { return; }
        // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
        unsafe { sceKernelDelayThread(16_000) };
    }
}

// SAFETY: MMIO read of a documented PSP hardware register at a fixed physical address.
fn hr(addr: u32) -> u32 { unsafe { hw_read32(addr) } }
fn hw(addr: u32, val: u32) {
    // SAFETY: kernel-mode MMIO write
    unsafe { hw_write32(addr, val) };
}

fn syscon_set(cmd: u8, value: u8) -> (i32, [u8; 16]) {
    let mut pkt = [0u8; 128];
    for b in pkt[0x0C..0x1C].iter_mut() { *b = 0xFF; }
    for b in pkt[0x1C..0x2C].iter_mut() { *b = 0xFF; }

    pkt[0x0C] = cmd;
    pkt[0x0D] = 3;
    pkt[0x0E] = value;
    let sum = pkt[0x0C].wrapping_add(pkt[0x0D]).wrapping_add(pkt[0x0E]);
    pkt[0x0F] = !sum;

    type F = unsafe extern "C" fn(*mut u8, i32) -> i32;
    let tachyon = hr(0xBC10_0040) & 0xFF00_0000;
    let addr: u32 = if tachyon >= 0x0050_0000 { 0x880A6E4C } else { 0x880A6D4C };
    // SAFETY: transmuting a u32 address to a fn pointer of the documented prototype.
    let func: F = unsafe { core::mem::transmute(addr) };
    // SAFETY: PSP-specific unsafe op (kernel-mode hardware / syscall access).
    let ret = unsafe { func(pkt.as_mut_ptr(), 0) };

    let mut rx = [0u8; 16];
    rx.copy_from_slice(&pkt[0x1C..0x2C]);
    (ret, rx)
}

fn syscon_get(cmd: u8) -> (i32, [u8; 16]) {
    let mut pkt = [0u8; 128];
    for b in pkt[0x0C..0x1C].iter_mut() { *b = 0xFF; }
    for b in pkt[0x1C..0x2C].iter_mut() { *b = 0xFF; }

    pkt[0x0C] = cmd;
    pkt[0x0D] = 2;
    let sum = pkt[0x0C].wrapping_add(pkt[0x0D]);
    pkt[0x0E] = !sum;

    type F = unsafe extern "C" fn(*mut u8, i32) -> i32;
    let tachyon = hr(0xBC10_0040) & 0xFF00_0000;
    let addr: u32 = if tachyon >= 0x0050_0000 { 0x880A6E4C } else { 0x880A6D4C };
    // SAFETY: transmuting a u32 address to a fn pointer of the documented prototype.
    let func: F = unsafe { core::mem::transmute(addr) };
    // SAFETY: PSP-specific unsafe op (kernel-mode hardware / syscall access).
    let ret = unsafe { func(pkt.as_mut_ptr(), 0) };

    let mut rx = [0u8; 16];
    rx.copy_from_slice(&pkt[0x1C..0x2C]);
    (ret, rx)
}

fn log_cmd(desc: &str, ret: i32, rx: &[u8; 16]) {
    let mut f = Fmt::new();
    f.p(desc);
    f.p(": r=");
    f.h8(ret as u32);
    f.p(" ");
    for i in 0..6 { f.h2(rx[i]); f.p(" "); }
    Logger::log(f.s());
}

fn psp_main() {
    let _ = psp::callback::setup_exit_callback();
    Logger::init();

    Logger::log("=== USB Host Phase 3G ===");
    Logger::log("CMD 0x45 retry (fixed)");
    Logger::log("");
    Logger::log("!! DO NOT REBOOT !!");
    Logger::log("!! WAIT FOR TIMER !!");
    Logger::log("");

    lr("Tachyon=", hr(0xBC10_0040));

    // Enable OHCI + port power first
    let v50 = hr(0xBC10_0050);
    hw(0xBC10_0050, v50 | 0x2000);
    // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
    unsafe { sceKernelDelayThread(10_000) };
    hw(0xBD10_1038, 0x0303);
    // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
    unsafe { sceKernelDelayThread(10_000) };
    lr("OHCI +38=", hr(0xBD10_1038));

    // Baseline
    let (r, rx) = syscon_get(0x44);
    log_cmd("GET 0x44", r, &rx);
    let (r, rx) = syscon_get(0x46);
    log_cmd("GET 0x46", r, &rx);
    Logger::log("");

    Logger::log("CROSS = send 0x45 SET");
    Logger::log("(then WAIT 60s, watch FNB58)");
    wait_cross();

    // ====== THE TEST: cmd 0x45 with correct length=3 ======
    Logger::log("Sending SET 0x45 v=1...");
    let (r, rx) = syscon_set(0x45, 1);
    log_cmd("SET 0x45 v=1", r, &rx);

    // Log state immediately after
    lr("OHCI +38=", hr(0xBD10_1038));
    lr("BC10+50=", hr(0xBC10_0050));
    lr("BC10+40=", hr(0xBC10_0040));

    // Now WAIT — the PSP might enter a different mode
    // Show countdown on screen
    Logger::log("");
    Logger::log("WAITING 60s...");
    Logger::log("Watch FNB58 for VBUS!");
    Logger::log("");

    for sec in 0u32..60 {
        // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
        unsafe { sceKernelDelayThread(1_000_000) };

        // Every 10 seconds, log state
        if sec % 10 == 9 {
            let mut f = Fmt::new();
            f.p("t=");
            if sec + 1 >= 10 {
                f.buf[f.pos] = b'0' + ((sec + 1) / 10) as u8;
                f.pos += 1;
            }
            f.buf[f.pos] = b'0' + ((sec + 1) % 10) as u8;
            f.pos += 1;
            f.p("s +38=");
            f.h8(hr(0xBD10_1038));
            f.p(" +50=");
            f.h8(hr(0xBC10_0050));
            Logger::log(f.s());
        }

        // Check for triangle to skip wait
        let mut p = SceCtrlData::default();
        // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
        unsafe { sceCtrlPeekBufferPositive(&mut p, 1) };
        if p.buttons.intersects(CtrlButtons::TRIANGLE) {
            Logger::log("(skipped by user)");
            break;
        }
    }

    // Also try cmd 0x47 after 0x45, in case 0x45 is "prepare" and 0x47 is "activate"
    Logger::log("");
    Logger::log("Now trying 0x47 after 0x45:");
    // Re-enable OHCI
    hw(0xBC10_0050, hr(0xBC10_0050) | 0x2000);
    // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
    unsafe { sceKernelDelayThread(10_000) };
    hw(0xBD10_1038, 0x0303);
    // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
    unsafe { sceKernelDelayThread(10_000) };

    let (r, rx) = syscon_set(0x47, 1);
    log_cmd("SET 0x47 v=1", r, &rx);
    // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
    unsafe { sceKernelDelayThread(500_000) };
    lr("OHCI +38=", hr(0xBD10_1038));

    // Try the sequence: 0x45 then 0x47 quickly
    Logger::log("");
    Logger::log("Sequence: 0x45 then 0x47:");
    hw(0xBC10_0050, hr(0xBC10_0050) | 0x2000);
    // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
    unsafe { sceKernelDelayThread(10_000) };
    hw(0xBD10_1038, 0x0303);
    // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
    unsafe { sceKernelDelayThread(10_000) };

    let (r1, rx1) = syscon_set(0x45, 1);
    log_cmd("SET 0x45 v=1", r1, &rx1);
    // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
    unsafe { sceKernelDelayThread(50_000) };
    let (r2, rx2) = syscon_set(0x47, 1);
    log_cmd("SET 0x47 v=1", r2, &rx2);
    // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
    unsafe { sceKernelDelayThread(500_000) };
    lr("OHCI +38=", hr(0xBD10_1038));
    lr("BC10+50=", hr(0xBC10_0050));

    // Wait another 30 seconds
    Logger::log("");
    Logger::log("WAITING 30s more...");
    for sec in 0u32..30 {
        // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
        unsafe { sceKernelDelayThread(1_000_000) };
        if sec % 10 == 9 {
            let mut f = Fmt::new();
            f.p("t=");
            f.buf[f.pos] = b'0' + ((sec + 1) / 10) as u8;
            f.pos += 1;
            f.buf[f.pos] = b'0' + ((sec + 1) % 10) as u8;
            f.pos += 1;
            f.p("s +38=");
            f.h8(hr(0xBD10_1038));
            Logger::log(f.s());
        }
        let mut p = SceCtrlData::default();
        // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
        unsafe { sceCtrlPeekBufferPositive(&mut p, 1) };
        if p.buttons.intersects(CtrlButtons::TRIANGLE) {
            Logger::log("(skipped)");
            break;
        }
    }

    Logger::log("");
    Logger::log("--- Final ---");
    lr("OHCI +38=", hr(0xBD10_1038));
    lr("BC10+50=", hr(0xBC10_0050));
    lr("BC10+40=", hr(0xBC10_0040));
    let (r, rx) = syscon_get(0x46);
    log_cmd("GET 0x46", r, &rx);

    Logger::log("");
    Logger::log("CROSS = exit");
    wait_cross();
    // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
    unsafe { sceKernelDelayThread(1_000_000) };
}
