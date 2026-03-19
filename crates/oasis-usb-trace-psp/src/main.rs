//! PSP USB Host Mode Phase 2e: Targeted Syscon SET commands
//!
//! Phase 2d mapped all GET commands 0x00-0x34. Cmd 0x0E (USB SET) returned
//! 0x4108 with OHCI enabled (was 0x4004 without). This version focuses on
//! SET commands with various data values to find the VBUS power switch.

#![no_std]
#![no_main]
#![feature(asm_experimental_arch)]

use psp::hw::{hw_read32, hw_write32};
use psp::sys::{
    CtrlButtons, SceCtrlData, IoOpenFlags,
    sceCtrlPeekBufferPositive,
    sceKernelDelayThread,
};
use core::ffi::c_void;

psp::module_kernel!("USBHostTest", 1, 0);

static LOG_PATH: &[u8] = b"ms0:/PSP/GAME/USBTRACE/phase2e.log\0";

struct Logger;
impl Logger {
    fn open() -> Self {
        let fd = unsafe {
            psp::sys::sceIoOpen(
                LOG_PATH.as_ptr(),
                IoOpenFlags::WR_ONLY | IoOpenFlags::CREAT | IoOpenFlags::TRUNC,
                0o777,
            )
        };
        if fd.0 >= 0 { unsafe { psp::sys::sceIoClose(fd) }; }
        Self
    }
    fn w(&self, s: &str) {
        let fd = unsafe {
            psp::sys::sceIoOpen(
                LOG_PATH.as_ptr(),
                IoOpenFlags::WR_ONLY | IoOpenFlags::APPEND,
                0o777,
            )
        };
        if fd.0 >= 0 {
            unsafe {
                psp::sys::sceIoWrite(fd, s.as_ptr() as *const c_void, s.len());
                psp::sys::sceIoClose(fd);
            }
        }
    }
}

struct Fmt { buf: [u8; 200], pos: usize }
impl Fmt {
    fn new() -> Self { Self { buf: [0u8; 200], pos: 0 } }
    fn s(&self) -> &str {
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

fn lr(l: &Logger, label: &str, val: u32) {
    let mut f = Fmt::new();
    f.p(label); f.h8(val); f.p("\n");
    psp::dprintln!("{}{:08X}", label, val);
    l.w(f.s());
}

fn ls(l: &Logger, s: &str) {
    psp::dprintln!("{}", s);
    l.w(s); l.w("\n");
}

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

fn hr(addr: u32) -> u32 { unsafe { hw_read32(addr) } }
fn hw(addr: u32, val: u32) {
    // SAFETY: kernel-mode MMIO write
    unsafe { hw_write32(addr, val) };
}

/// Execute Syscon command via known 1001 address
fn syscon_cmd(cmd: u8, data: &[u8]) -> (i32, [u8; 16]) {
    let mut pkt = [0u8; 128];
    for b in pkt[0x0C..0x1C].iter_mut() { *b = 0xFF; }
    for b in pkt[0x1C..0x2C].iter_mut() { *b = 0xFF; }

    pkt[0x0C] = cmd;
    let len = 2 + data.len() as u8;
    pkt[0x0D] = len;
    for (i, &b) in data.iter().enumerate() {
        pkt[0x0E + i] = b;
    }
    let mut sum: u8 = 0;
    for i in 0..(len as usize) {
        sum = sum.wrapping_add(pkt[0x0C + i]);
    }
    pkt[0x0C + len as usize] = !sum;

    // PSP-1001: 0x880A6D4C, PSP-3001: 0x880A6E4C (both 6.61 ARK-4)
    // Detect via Tachyon version: 0x00300000 = 1001, 0x00600000 = 3001
    type F = unsafe extern "C" fn(*mut u8, i32) -> i32;
    let tachyon = hr(0xBC10_0040) & 0xFF00_0000;
    let addr: u32 = if tachyon >= 0x0050_0000 { 0x880A6E4C } else { 0x880A6D4C };
    let func: F = unsafe { core::mem::transmute(addr) };
    let ret = unsafe { func(pkt.as_mut_ptr(), 0) };

    let mut rx = [0u8; 16];
    rx.copy_from_slice(&pkt[0x1C..0x2C]);
    (ret, rx)
}

fn log_syscon(l: &Logger, desc: &str, cmd: u8, data: &[u8]) {
    let (ret, rx) = syscon_cmd(cmd, data);
    let mut f = Fmt::new();
    f.p(desc);
    f.p(": ret=");
    f.h8(ret as u32);
    f.p(" rx=");
    for i in 0..6 {
        f.h2(rx[i]);
        f.p(" ");
    }
    f.p("\n");
    psp::dprintln!("{}: r={:08X} {:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
        desc, ret as u32, rx[0], rx[1], rx[2], rx[3], rx[4], rx[5]);
    l.w(f.s());
}

fn psp_main() {
    let _ = psp::callback::setup_exit_callback();
    let l = Logger::open();

    ls(&l, "=== USB Host Phase 2e ===");
    ls(&l, "Targeted Syscon SET");
    ls(&l, "");

    // Enable OHCI + port power
    let v50 = hr(0xBC10_0050);
    hw(0xBC10_0050, v50 | 0x2000);
    unsafe { core::arch::asm!("sync") };
    unsafe { sceKernelDelayThread(10_000) };
    hw(0xBD10_1038, 0x0303);
    unsafe { core::arch::asm!("sync") };
    lr(&l, "OHCI +38=", hr(0xBD10_1038));
    ls(&l, "");

    // Read baseline USB status
    ls(&l, "--- Baseline ---");
    log_syscon(&l, "GET USB(0C)", 0x0C, &[]);
    log_syscon(&l, "GET 0E", 0x0E, &[]);
    log_syscon(&l, "GET pwr(0B)", 0x0B, &[]);
    ls(&l, "");

    ls(&l, "CROSS = SET commands");
    wait_cross();

    // ====== SET commands on cmd 0x0E (USB SET) ======
    // Phase 0b: cmd 0x0E with length=2 (no data) returned 0x4004
    // Phase 2d: same cmd returned 0x4108 with OHCI enabled
    // Now try SET with various data values
    // The SET format: cmd=0x0E, length=3 (cmd+len+data), data=value
    ls(&l, "--- USB SET (0x0E) ---");
    let set_values: [u8; 16] = [
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x08, 0x10,
        0x20, 0x40, 0x41, 0x42, 0x80, 0xC0, 0xFE, 0xFF,
    ];
    for &val in &set_values {
        let mut desc = [0u8; 16];
        desc[..7].copy_from_slice(b"SET 0E ");
        let hex = b"0123456789ABCDEF";
        desc[7] = hex[(val >> 4) as usize];
        desc[8] = hex[(val & 0xF) as usize];
        let desc_str = unsafe { core::str::from_utf8_unchecked(&desc[..9]) };
        log_syscon(&l, desc_str, 0x0E, &[val]);
        unsafe { sceKernelDelayThread(50_000) };
    }
    // Read back USB status after SETs
    log_syscon(&l, "GET USB(0C)", 0x0C, &[]);
    log_syscon(&l, "GET 0E", 0x0E, &[]);
    ls(&l, "");

    // ====== Try SET on other known-good commands ======
    ls(&l, "--- Other SET cmds ---");

    // Cmd 0x00 (NOP) with data
    log_syscon(&l, "SET 00 01", 0x00, &[0x01]);
    // Cmd 0x04 with data
    log_syscon(&l, "SET 04 01", 0x04, &[0x01]);
    log_syscon(&l, "SET 04 03", 0x04, &[0x03]);
    // Cmd 0x05 (power related?)
    log_syscon(&l, "SET 05 01", 0x05, &[0x01]);
    log_syscon(&l, "SET 05 FF", 0x05, &[0xFF]);
    // Cmd 0x0D (ACK cmd from Phase 0b)
    log_syscon(&l, "SET 0D 01", 0x0D, &[0x01]);
    // Cmd 0x10 (ACK cmd)
    log_syscon(&l, "SET 10 01", 0x10, &[0x01]);
    // Cmd 0x12-0x1F (all ACK'd in scan)
    log_syscon(&l, "SET 12 01", 0x12, &[0x01]);
    log_syscon(&l, "SET 13 01", 0x13, &[0x01]);
    // Cmd 0x20 (worked in scan)
    log_syscon(&l, "SET 20 01", 0x20, &[0x01]);
    log_syscon(&l, "SET 20 FF", 0x20, &[0xFF]);
    // Cmd 0x22 (worked in scan)
    log_syscon(&l, "SET 22 01", 0x22, &[0x01]);
    // Cmd 0x31 (worked in scan)
    log_syscon(&l, "SET 31 01", 0x31, &[0x01]);
    ls(&l, "");

    // ====== Multi-byte SET on cmd 0x0E ======
    // Maybe VBUS needs a 2-byte value
    ls(&l, "--- 0x0E multi-byte ---");
    log_syscon(&l, "0E 41,08", 0x0E, &[0x41, 0x08]);
    log_syscon(&l, "0E 41,0C", 0x0E, &[0x41, 0x0C]);
    log_syscon(&l, "0E 41,48", 0x0E, &[0x41, 0x48]);
    log_syscon(&l, "0E 43,08", 0x0E, &[0x43, 0x08]);
    log_syscon(&l, "0E C1,08", 0x0E, &[0xC1, 0x08]);
    log_syscon(&l, "0E FF,FF", 0x0E, &[0xFF, 0xFF]);
    ls(&l, "");

    // ====== Check GPIO pins we haven't tried ======
    ls(&l, "--- GPIO scan ---");
    // Read all GPIO registers
    lr(&l, "GPIO dir  =", hr(0xBE24_0000));
    lr(&l, "GPIO out  =", hr(0xBE24_0004));
    lr(&l, "GPIO in   =", hr(0xBE24_0008));
    lr(&l, "GPIO intr =", hr(0xBE24_000C));
    lr(&l, "GPIO +10  =", hr(0xBE24_0010));
    lr(&l, "GPIO +14  =", hr(0xBE24_0014));
    lr(&l, "GPIO +18  =", hr(0xBE24_0018));
    lr(&l, "GPIO +1C  =", hr(0xBE24_001C));
    lr(&l, "GPIO +20  =", hr(0xBE24_0020));
    lr(&l, "GPIO +24  =", hr(0xBE24_0024));
    lr(&l, "GPIO +28  =", hr(0xBE24_0028));
    lr(&l, "GPIO +40  =", hr(0xBE24_0040));
    lr(&l, "GPIO +44  =", hr(0xBE24_0044));
    lr(&l, "GPIO +48  =", hr(0xBE24_0048));
    ls(&l, "");

    // Try toggling GPIO pins that might control VBUS
    // GPIO port 2 (bits 16-23) and port 3 (bits 24-31) are common for power
    ls(&l, "--- GPIO VBUS hunt ---");
    let gpio_dir = hr(0xBE24_0000);
    let gpio_out = hr(0xBE24_0004);

    // Try each GPIO bit 0-31 as output high
    for bit in 0u32..32 {
        let mask = 1u32 << bit;
        // Set direction to output
        hw(0xBE24_0000, gpio_dir | mask);
        // Set output high
        hw(0xBE24_0004, gpio_out | mask);
        unsafe { sceKernelDelayThread(20_000) };

        // Check if something happened (OHCI port status might change)
        let rhps = hr(0xBD10_1038);
        if rhps != 0x300 {
            let mut f = Fmt::new();
            f.p("  GPIO bit ");
            f.h2(bit as u8);
            f.p(": OHCI changed! +38=");
            f.h8(rhps);
            f.p("\n");
            l.w(f.s());
            psp::dprintln!("  GPIO bit {}: OHCI={:08X}", bit, rhps);
        }

        // Restore
        hw(0xBE24_0004, gpio_out);
        hw(0xBE24_0000, gpio_dir);
    }
    ls(&l, "GPIO scan done");
    ls(&l, "");

    // Final state
    ls(&l, "--- Final ---");
    log_syscon(&l, "GET USB(0C)", 0x0C, &[]);
    log_syscon(&l, "GET 0E", 0x0E, &[]);
    lr(&l, "OHCI +38=", hr(0xBD10_1038));
    lr(&l, "GPIO dir=", hr(0xBE24_0000));
    lr(&l, "GPIO out=", hr(0xBE24_0004));

    ls(&l, "");
    ls(&l, "CROSS = exit");
    l.w("=== END ===\n");
    wait_cross();
    unsafe { sceKernelDelayThread(1_000_000) };
}
