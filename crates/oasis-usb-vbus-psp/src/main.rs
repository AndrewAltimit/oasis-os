//! PSP USB VBUS Power Output — Reverse Engineering Tool
//!
//! Interactive kernel-mode EBOOT for discovering and enabling VBUS output
//! on the PSP-3001's USB Mini-B port. D-pad selects phase/step, X runs it.
//!
//! Phases:
//!   1. GPIO Discovery — dump & compare GPIO registers, monitor during USB init
//!   2. Syscon USB Power — resolve sceSysconCtrlUsbPower NID, test SET commands
//!   3. Direct Register Host Init — clocks, PHY, OHCI, MUSB per firmware sequence
//!   4. GPIO VBUS Toggle — set identified GPIO pin to output + drive high
//!
//! Controls:
//!   UP/DOWN  — select phase/step
//!   X        — execute selected step
//!   TRIANGLE — exit
//!
//! Log output: ms0:/PSP/GAME/USBVBUS/vbus.log

#![no_std]
#![no_main]

mod gpio;
mod ohci;
mod phy;
mod screen;
mod syscon;

use core::ffi::c_void;
use psp::hw::hw_read32;
use psp::sys::{
    self, CtrlButtons, CtrlMode, IoOpenFlags, SceCtrlData,
    sceCtrlPeekBufferPositive, sceCtrlSetSamplingMode,
    sceKernelDelayThread,
    sceUsbActivate, sceUsbGetState, sceUsbStart,
};

psp::module_kernel!("USBVbusTest", 1, 0);

// ── Logging ─────────────────────────────────────────────────────────────

static LOG_PATH: &[u8] = b"ms0:/PSP/GAME/USBVBUS/vbus.log\0";

struct Logger;
impl Logger {
    fn init() {
        // Append mode — don't lose previous run's data
        let fd = unsafe {
            psp::sys::sceIoOpen(
                LOG_PATH.as_ptr(),
                IoOpenFlags::WR_ONLY | IoOpenFlags::CREAT | IoOpenFlags::APPEND,
                0o777,
            )
        };
        if fd.0 >= 0 {
            unsafe {
                psp::sys::sceIoWrite(
                    fd,
                    b"\n========== NEW RUN ==========\n".as_ptr()
                        as *const c_void,
                    31,
                );
                psp::sys::sceIoClose(fd);
            }
        }
    }

    /// Write to both screen (dprintln) and log file.
    fn log(s: &str) {
        psp::dprintln!("{}", s);
        Self::file(s);
    }

    /// Write to log file only (no screen output).
    fn file(s: &str) {
        let fd = unsafe {
            psp::sys::sceIoOpen(
                LOG_PATH.as_ptr(),
                IoOpenFlags::WR_ONLY | IoOpenFlags::APPEND,
                0o777,
            )
        };
        if fd.0 >= 0 {
            unsafe {
                psp::sys::sceIoWrite(
                    fd, s.as_ptr() as *const c_void, s.len(),
                );
                psp::sys::sceIoWrite(
                    fd, b"\n".as_ptr() as *const c_void, 1,
                );
                psp::sys::sceIoClose(fd);
            }
        }
    }

    /// Write to screen only (no log file).
    fn screen(s: &str) {
        psp::dprintln!("{}", s);
    }

    /// Clear screen by flushing dprintln's 27-line character buffer.
    fn clear_screen() {
        // dprintln's CharBuffer has ROWS = 272/10 = 27 lines.
        // Writing 27 blank lines pushes all old content out.
        for _ in 0..27 {
            psp::dprintln!("");
        }
    }
}

// ── Formatting ──────────────────────────────────────────────────────────

struct Fmt {
    buf: [u8; 256],
    pos: usize,
}

impl Fmt {
    fn new() -> Self {
        Self {
            buf: [0u8; 256],
            pos: 0,
        }
    }

    fn s(&self) -> &str {
        // SAFETY: we only write ASCII bytes via p/h8/h2/decimal.
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

    fn decimal(&mut self, mut v: u32) {
        if v == 0 {
            if self.pos < self.buf.len() {
                self.buf[self.pos] = b'0';
                self.pos += 1;
            }
            return;
        }
        let mut digits = [0u8; 10];
        let mut n = 0usize;
        while v > 0 {
            digits[n] = b'0' + (v % 10) as u8;
            v /= 10;
            n += 1;
        }
        for i in (0..n).rev() {
            if self.pos < self.buf.len() {
                self.buf[self.pos] = digits[i];
                self.pos += 1;
            }
        }
    }
}

fn lr(label: &str, val: u32) {
    let mut f = Fmt::new();
    f.p(label);
    f.h8(val);
    Logger::log(f.s());
}

fn log_cmd(desc: &str, ret: i32, rx: &[u8; 16]) {
    let mut f = Fmt::new();
    f.p(desc);
    f.p(": r=");
    f.h8(ret as u32);
    f.p(" [");
    for i in 0..6 {
        if i > 0 {
            f.p(" ");
        }
        f.h2(rx[i]);
    }
    f.p("]");
    Logger::log(f.s());
}

// ── Input helpers ───────────────────────────────────────────────────────

fn wait_cross() {
    let mut p = SceCtrlData::default();
    // Wait for release
    loop {
        unsafe { sceCtrlPeekBufferPositive(&mut p, 1) };
        if !p.buttons.intersects(CtrlButtons::CROSS) {
            break;
        }
        unsafe { sceKernelDelayThread(16_000) };
    }
    // Wait for press
    loop {
        unsafe { sceCtrlPeekBufferPositive(&mut p, 1) };
        if p.buttons.intersects(CtrlButtons::CROSS) {
            return;
        }
        unsafe { sceKernelDelayThread(16_000) };
    }
}

/// Buttons we care about for menu navigation.
const NAV_BUTTONS: CtrlButtons = CtrlButtons::from_bits_truncate(
    CtrlButtons::UP.bits()
    | CtrlButtons::DOWN.bits()
    | CtrlButtons::CROSS.bits()
    | CtrlButtons::TRIANGLE.bits()
    | CtrlButtons::CIRCLE.bits()
    | CtrlButtons::SQUARE.bits()
    | CtrlButtons::START.bits()
);

/// Wait for any navigation button press (edge-triggered).
fn wait_any_button() -> CtrlButtons {
    let mut p = SceCtrlData::default();
    // Wait for nav buttons released
    loop {
        unsafe { sceCtrlPeekBufferPositive(&mut p, 1) };
        if !p.buttons.intersects(NAV_BUTTONS) {
            break;
        }
        unsafe { sceKernelDelayThread(16_000) };
    }
    // Wait for any nav button press
    loop {
        unsafe { sceCtrlPeekBufferPositive(&mut p, 1) };
        let pressed = p.buttons & NAV_BUTTONS;
        if !pressed.is_empty() {
            return pressed;
        }
        unsafe { sceKernelDelayThread(16_000) };
    }
}

// ── GPIO snapshot logging ───────────────────────────────────────────────

fn log_gpio_snapshot(label: &str, snap: &gpio::GpioSnapshot) {
    Logger::log(label);
    lr("  Read     = ", snap.read);
    lr("  Output   = ", snap.output);
    lr("  Direction= ", snap.direction);
    lr("  AltFunc  = ", snap.alt_func);
}

fn log_gpio_diff(before: &gpio::GpioSnapshot, after: &gpio::GpioSnapshot) {
    let d = gpio::diff(before, after);
    if d.read_changed == 0 && d.output_changed == 0
        && d.direction_changed == 0 && d.alt_func_changed == 0
    {
        Logger::log("  (no GPIO changes)");
        return;
    }
    if d.read_changed != 0 {
        lr("  Read     CHANGED: ", d.read_changed);
    }
    if d.output_changed != 0 {
        lr("  Output   CHANGED: ", d.output_changed);
    }
    if d.direction_changed != 0 {
        lr("  Direction CHANGED: ", d.direction_changed);
    }
    if d.alt_func_changed != 0 {
        lr("  AltFunc  CHANGED: ", d.alt_func_changed);
    }
}

// ── OHCI snapshot logging ───────────────────────────────────────────────

fn log_ohci_snapshot(label: &str, snap: &ohci::OhciSnapshot) {
    Logger::log(label);
    lr("  SysClk1  = ", snap.sys_clk1);
    lr("  SysClk2  = ", snap.sys_clk2);
    lr("  OhciClk  = ", snap.ohci_clk);
    lr("  Tachyon  = ", snap.tachyon);
    lr("  Revision = ", snap.ohci_revision);
    lr("  Control  = ", snap.ohci_control);
    lr("  CmdStatus= ", snap.ohci_cmd_status);
    lr("  RhStatus = ", snap.ohci_rh_status);
    lr("  PortStatus=", snap.ohci_rh_port_status);
    lr("  DevCtl   = ", snap.musb_devctl);
}

fn log_phy_snapshot(label: &str, snap: &phy::PhySnapshot) {
    Logger::log(label);
    lr("  ClkDiv   = ", snap.clk_div);
    lr("  ClkDivHi = ", snap.clk_div_hi);
    lr("  Config   = ", snap.config);
    lr("  Mode     = ", snap.mode);
    lr("  Feature  = ", snap.feature);
}

// ── Phase 1: GPIO Discovery ────────────────────────────────────────────

/// Step 1.1: Dump PSP-3001 GPIO registers
fn phase1_step1_dump_gpio() {
    Logger::log("=== Phase 1.1: GPIO Register Dump ===");
    let snap = gpio::snapshot();
    log_gpio_snapshot("GPIO registers:", &snap);

    // PSP-1001 baseline for comparison (from memory)
    Logger::log("");
    Logger::log("PSP-1001 baseline:");
    Logger::log("  Direction= 020000EF");
    Logger::log("  Output   = 01000067");
    Logger::log("  Read     = 05000010");
    Logger::log("");

    // Find candidate VBUS bits
    let psp1_dir: u32 = 0x020000EF;
    let new_bits = snap.direction & !psp1_dir;
    if new_bits != 0 {
        lr("VBUS candidates (dir bits new in 3001): ", new_bits);
        // Log individual bit positions
        for bit in 0..32 {
            if new_bits & (1 << bit) != 0 {
                let mut f = Fmt::new();
                f.p("  -> GPIO pin ");
                f.decimal(bit);
                f.p(" (bit ");
                f.decimal(bit);
                f.p(")");
                Logger::log(f.s());
            }
        }
    } else {
        Logger::log("No new direction bits vs PSP-1001");
    }
}

/// Step 1.2: Resolve GPIO NIDs
fn phase1_step2_resolve_nids() {
    Logger::log("=== Phase 1.2: Resolve GPIO NIDs ===");
    let count = unsafe { gpio::resolve_nids() };
    let mut f = Fmt::new();
    f.p("Resolved ");
    f.decimal(count);
    f.p("/4 GPIO NIDs");
    Logger::log(f.s());

    // Test port read via NID
    if let Some(val) = unsafe { gpio::port_read() } {
        lr("sceGpioPortRead() = ", val);
    } else {
        Logger::log("sceGpioPortRead: NOT RESOLVED");
    }
}

/// Step 1.3: Monitor GPIO during USB camera module init
fn phase1_step3_monitor_usb_init() {
    Logger::log("=== Phase 1.3: Monitor GPIO During USB Init ===");

    let snap_before = gpio::snapshot();
    log_gpio_snapshot("Before USB init:", &snap_before);

    // Step A: Load USB modules
    Logger::log("");
    Logger::log("Loading UsbPspCm...");
    let snap_a = gpio::snapshot();
    let ret = unsafe { sys::sceUtilityLoadUsbModule(sys::UsbModule::UsbPspCm) };
    lr("  ret=", ret as u32);
    let snap_a2 = gpio::snapshot();
    log_gpio_diff(&snap_a, &snap_a2);

    Logger::log("Loading UsbAcc...");
    let snap_b = gpio::snapshot();
    let ret = unsafe { sys::sceUtilityLoadUsbModule(sys::UsbModule::UsbAcc) };
    lr("  ret=", ret as u32);
    let snap_b2 = gpio::snapshot();
    log_gpio_diff(&snap_b, &snap_b2);

    Logger::log("Loading UsbCam...");
    let snap_c = gpio::snapshot();
    let ret = unsafe { sys::sceUtilityLoadUsbModule(sys::UsbModule::UsbCam) };
    lr("  ret=", ret as u32);
    let snap_c2 = gpio::snapshot();
    log_gpio_diff(&snap_c, &snap_c2);

    unsafe { sceKernelDelayThread(100_000) };

    // Step B: Start USB bus driver
    Logger::log("");
    Logger::log("Starting USBBusDriver...");
    let snap_d = gpio::snapshot();
    let ret = unsafe {
        sceUsbStart(
            b"USBBusDriver\0".as_ptr(),
            0,
            core::ptr::null_mut::<c_void>(),
        )
    };
    lr("  ret=", ret as u32);
    unsafe { sceKernelDelayThread(100_000) };
    let snap_d2 = gpio::snapshot();
    log_gpio_diff(&snap_d, &snap_d2);

    // Step C: Start USB camera driver
    Logger::log("");
    Logger::log("Starting USBCamDriver...");
    let snap_e = gpio::snapshot();
    let ret = unsafe {
        sceUsbStart(
            b"USBCamDriver\0".as_ptr(),
            0,
            core::ptr::null_mut::<c_void>(),
        )
    };
    lr("  ret=", ret as u32);
    unsafe { sceKernelDelayThread(100_000) };
    let snap_e2 = gpio::snapshot();
    log_gpio_diff(&snap_e, &snap_e2);

    // Step D: Activate with camera PID
    Logger::log("");
    Logger::log("Activating USB (PID=0x282)...");
    let snap_f = gpio::snapshot();
    let ret = unsafe { sceUsbActivate(0x282) };
    lr("  ret=", ret as u32);
    unsafe { sceKernelDelayThread(500_000) };
    let snap_f2 = gpio::snapshot();
    log_gpio_diff(&snap_f, &snap_f2);

    // Log USB state
    let bits = unsafe { sceUsbGetState() }.bits();
    lr("USB state=", bits as u32);

    // Overall diff
    let snap_after = gpio::snapshot();
    Logger::log("");
    Logger::log("Overall GPIO changes (before → after):");
    log_gpio_diff(&snap_before, &snap_after);

    // Cleanup
    Logger::log("");
    Logger::log("Cleaning up...");
    unsafe {
        sys::sceUsbDeactivate(0x282);
        sceKernelDelayThread(100_000);
        sys::sceUsbStop(
            b"USBCamDriver\0".as_ptr(),
            0,
            core::ptr::null_mut::<c_void>(),
        );
        sys::sceUsbStop(
            b"USBBusDriver\0".as_ptr(),
            0,
            core::ptr::null_mut::<c_void>(),
        );
        sceKernelDelayThread(100_000);
        sys::sceUtilityUnloadUsbModule(sys::UsbModule::UsbCam);
        sys::sceUtilityUnloadUsbModule(sys::UsbModule::UsbAcc);
        sys::sceUtilityUnloadUsbModule(sys::UsbModule::UsbPspCm);
    }
    Logger::log("Cleanup done.");
}

// ── Phase 2: Syscon USB Power Commands ─────────────────────────────────

/// Step 2.1: Resolve sceSysconCtrlUsbPower NID
fn phase2_step1_resolve_syscon() {
    Logger::log("=== Phase 2.1: Resolve sceSysconCtrlUsbPower ===");
    let found = unsafe { syscon::resolve_nids() };
    if found {
        Logger::log("sceSysconCtrlUsbPower: FOUND!");
        lr("  addr=", syscon::resolved_addr());
        Logger::log("X=call it, TRI=skip");
        let btn = wait_any_button();
        if btn.intersects(CtrlButtons::TRIANGLE) {
            Logger::log("Skipped call.");
            return;
        }

        let gpio_before = gpio::snapshot();
        lr("GPIO before Read=", gpio_before.read);
        lr("GPIO before Out =", gpio_before.output);
        lr("GPIO before Dir =", gpio_before.direction);
        lr("BC100050 before =", unsafe { hw_read32(0xBC10_0050) });

        Logger::log("Calling sceSysconCtrlUsbPower(1)...");
        if let Some(ret) = unsafe { syscon::ctrl_usb_power(1) } {
            lr("  ret=", ret as u32);
        }
        Logger::log("Call returned.");

        unsafe { sceKernelDelayThread(500_000) };

        let gpio_after = gpio::snapshot();
        Logger::log("GPIO changes:");
        log_gpio_diff(&gpio_before, &gpio_after);
        lr("BC100050 after  =", unsafe { hw_read32(0xBC10_0050) });

        // OHCI/MUSB registers bus-fault unless OHCI clock is
        // fully enabled (BC100078 bit 19). Don't attempt here —
        // use Phase 3.1 to enable clocks first.
        lr("BC100078 (OHCI clk)=", unsafe { hw_read32(0xBC10_0078) });
    } else {
        Logger::log("sceSysconCtrlUsbPower: NOT FOUND");
        Logger::log("Will use raw Syscon packets in step 2.2");
    }
}

/// Step 2.2: Test Syscon GET commands for USB power status
fn phase2_step2_syscon_status() {
    Logger::log("=== Phase 2.2: Syscon USB Power Status ===");

    let (r, rx) = syscon::syscon_get(0x44);
    log_cmd("GET 0x44 (USB power?)", r, &rx);

    let (r, rx) = syscon::syscon_get(0x46);
    log_cmd("GET 0x46 (USB state?)", r, &rx);

    // Also try 0x07 (Baryon status) and 0x09 (power status)
    let (r, rx) = syscon::syscon_get(0x07);
    log_cmd("GET 0x07 (Baryon?)", r, &rx);

    let (r, rx) = syscon::syscon_get(0x09);
    log_cmd("GET 0x09 (power?)", r, &rx);

    lr("BC100050=", unsafe { hw_read32(0xBC10_0050) });
    lr("BC100078=", unsafe { hw_read32(0xBC10_0078) });

    // GPIO state (safe reads)
    let snap = gpio::snapshot();
    lr("GPIO Read=", snap.read);
    lr("GPIO Out =", snap.output);
    lr("GPIO Dir =", snap.direction);
}

/// Step 2.3: Test Syscon SET 0x47 with correct format
fn phase2_step3_syscon_set() {
    Logger::log("=== Phase 2.3: Syscon SET Commands ===");

    // Log baseline (safe reads only — no OHCI/MUSB)
    lr("BC100050 before=", unsafe { hw_read32(0xBC10_0050) });
    lr("BC100078 before=", unsafe { hw_read32(0xBC10_0078) });
    let gpio_before = gpio::snapshot();

    // Test SET 0x47 value=0
    Logger::log("");
    Logger::log("SET 0x47 v=0:");
    let (r, rx) = syscon::syscon_set(0x47, 0);
    log_cmd("  result", r, &rx);
    unsafe { sceKernelDelayThread(200_000) };
    lr("  BC100050=", unsafe { hw_read32(0xBC10_0050) });

    // Test SET 0x47 value=1
    Logger::log("");
    Logger::log("SET 0x47 v=1:");
    let (r, rx) = syscon::syscon_set(0x47, 1);
    log_cmd("  result", r, &rx);
    unsafe { sceKernelDelayThread(500_000) };
    lr("  BC100050=", unsafe { hw_read32(0xBC10_0050) });

    // Test SET 0x47 value=2
    Logger::log("");
    Logger::log("SET 0x47 v=2:");
    let (r, rx) = syscon::syscon_set(0x47, 2);
    log_cmd("  result", r, &rx);
    unsafe { sceKernelDelayThread(500_000) };
    lr("  BC100050=", unsafe { hw_read32(0xBC10_0050) });

    // GPIO diff
    let gpio_after = gpio::snapshot();
    Logger::log("");
    Logger::log("GPIO changes after Syscon SET tests:");
    log_gpio_diff(&gpio_before, &gpio_after);
    lr("BC100078 after=", unsafe { hw_read32(0xBC10_0078) });
}

// ── Phase 3: Direct Register Host Mode Init ────────────────────────────

/// Step 3.1: Clock/PHY init per firmware sequence
fn phase3_step1_clock_phy() {
    Logger::log("=== Phase 3.1: Clock + PHY Init ===");

    let gpio_before = gpio::snapshot();
    lr("BC100050=", unsafe { hw_read32(0xBC10_0050) });
    lr("BC100058=", unsafe { hw_read32(0xBC10_0058) });
    lr("BC100078=", unsafe { hw_read32(0xBC10_0078) });
    log_phy_snapshot("PHY before:", &phy::snapshot());

    Logger::log("");
    Logger::log("Enabling clocks...");
    unsafe { ohci::enable_clocks() };
    unsafe { sceKernelDelayThread(10_000) };

    Logger::log("Configuring PHY...");
    unsafe { phy::configure_host_mode() };
    unsafe { sceKernelDelayThread(10_000) };

    log_phy_snapshot("PHY after:", &phy::snapshot());

    // Try WRITE to OHCI first (trace crate does this successfully)
    Logger::log("");
    Logger::log("Writing OHCI PortStatus=0x0303...");
    unsafe { psp::hw::hw_write32(0xBD10_1038, 0x0303) };
    Logger::log("Write OK!");
    unsafe { sceKernelDelayThread(10_000) };

    // Now try reading it back
    Logger::log("Reading OHCI PortStatus...");
    lr("OHCI +38=", unsafe { psp::hw::hw_read32(0xBD10_1038) });
    Logger::log("Read OK!");

    // MUSB at 0xBD80xxxx is a separate peripheral — skip for now
    // (bus-faults even when OHCI works)
    Logger::log("(MUSB 0xBD80xxxx skipped — separate bus enable needed)");

    let gpio_after = gpio::snapshot();
    Logger::log("GPIO changes:");
    log_gpio_diff(&gpio_before, &gpio_after);
}

/// Step 3.2: OHCI controller init (port power)
fn phase3_step2_ohci_init() {
    Logger::log("=== Phase 3.2: OHCI Port Power ===");

    lr("OHCI PortStatus before=", ohci::port_status());

    Logger::log("Enabling OHCI clock bit 1...");
    unsafe { ohci::enable_ohci_clock_bit1() };
    unsafe { sceKernelDelayThread(10_000) };

    Logger::log("Setting port power (0x0303)...");
    unsafe { ohci::set_port_power() };
    unsafe { sceKernelDelayThread(100_000) };

    lr("OHCI PortStatus after=", ohci::port_status());

    let vbus = ohci::vbus_level();
    let mut f = Fmt::new();
    f.p("VBUS level: ");
    f.decimal(vbus);
    f.p(" ");
    f.p(ohci::vbus_level_str(vbus));
    Logger::log(f.s());
}

/// Step 3.3: MUSBMHDRC host session
fn phase3_step3_musb_host() {
    Logger::log("=== Phase 3.3: MUSB Host Session ===");
    Logger::log("MUSB at 0xBD80xxxx bus-faults on this PSP.");
    Logger::log("Skipping — need to find MUSB bus enable first.");
}

/// Step 3.4: Tachyon mode bit (BC100040)
fn phase3_step4_tachyon_mode() {
    Logger::log("=== Phase 3.4: Tachyon Mode Bit ===");
    Logger::log("WARNING: This writes BC100040 mode bits");
    Logger::log("X=proceed, TRIANGLE=skip");

    let btn = wait_any_button();
    if btn.intersects(CtrlButtons::TRIANGLE) {
        Logger::log("Skipped.");
        return;
    }

    let gpio_before = gpio::snapshot();
    lr("BC100040 before=", ohci::tachyon_mode());

    Logger::log("Setting host mode bit...");
    unsafe { ohci::set_tachyon_host_mode() };
    unsafe { sceKernelDelayThread(1_000_000) };

    lr("BC100040 after=", ohci::tachyon_mode());

    let gpio_after = gpio::snapshot();
    Logger::log("GPIO changes:");
    log_gpio_diff(&gpio_before, &gpio_after);

    let vbus = ohci::vbus_level();
    let mut f = Fmt::new();
    f.p("VBUS level: ");
    f.decimal(vbus);
    f.p(" ");
    f.p(ohci::vbus_level_str(vbus));
    Logger::log(f.s());
}

/// Step 3.5: Full firmware init sequence (all of Phase 3 combined)
fn phase3_step5_full_sequence() {
    Logger::log("=== Phase 3.5: Full Firmware Init Sequence ===");

    let gpio_before = gpio::snapshot();
    let ohci_before = ohci::snapshot();
    let phy_before = phy::snapshot();

    log_gpio_snapshot("GPIO before:", &gpio_before);
    log_ohci_snapshot("OHCI before:", &ohci_before);
    log_phy_snapshot("PHY before:", &phy_before);

    Logger::log("");
    Logger::log("Step 1: Clocks...");
    unsafe { ohci::enable_clocks() };
    unsafe { sceKernelDelayThread(10_000) };

    Logger::log("Step 2: PHY host mode...");
    unsafe { phy::configure_host_mode() };
    unsafe { sceKernelDelayThread(10_000) };

    Logger::log("Step 3: OHCI clock bit 1...");
    unsafe { ohci::enable_ohci_clock_bit1() };
    unsafe { sceKernelDelayThread(10_000) };

    Logger::log("Step 4: OHCI port power...");
    unsafe { ohci::set_port_power() };
    unsafe { sceKernelDelayThread(100_000) };

    Logger::log("Step 5: MUSB host session...");
    let devctl = unsafe { ohci::set_musb_host_session() };
    lr("DevCtl=", devctl);

    unsafe { sceKernelDelayThread(500_000) };

    let gpio_after = gpio::snapshot();
    let ohci_after = ohci::snapshot();
    let phy_after = phy::snapshot();

    Logger::log("");
    log_gpio_snapshot("GPIO after:", &gpio_after);
    log_ohci_snapshot("OHCI after:", &ohci_after);
    log_phy_snapshot("PHY after:", &phy_after);

    Logger::log("");
    Logger::log("GPIO changes:");
    log_gpio_diff(&gpio_before, &gpio_after);

    let vbus = ohci::vbus_level();
    let mut f = Fmt::new();
    f.p("VBUS level: ");
    f.decimal(vbus);
    f.p(" ");
    f.p(ohci::vbus_level_str(vbus));
    Logger::log(f.s());

    // Monitor for 10 seconds
    Logger::log("");
    Logger::log("Monitoring for 10s (watch FNB58)...");
    for sec in 0u32..10 {
        unsafe { sceKernelDelayThread(1_000_000) };
        let v = ohci::vbus_level();
        let ps = ohci::port_status();
        let mut f = Fmt::new();
        f.p("  t=");
        f.decimal(sec + 1);
        f.p("s VBUS=");
        f.decimal(v);
        f.p(" Port=");
        f.h8(ps);
        Logger::log(f.s());

        // Check for triangle to skip
        let mut p = SceCtrlData::default();
        unsafe { sceCtrlPeekBufferPositive(&mut p, 1) };
        if p.buttons.intersects(CtrlButtons::TRIANGLE) {
            Logger::log("  (skipped)");
            break;
        }
    }
}

// ── Phase 4: GPIO VBUS Toggle ──────────────────────────────────────────

/// Step 4.1: Try each candidate GPIO pin as VBUS output.
///
/// Tests pins that are in PSP-3001 direction register but NOT in PSP-1001.
/// For each: set to output, drive high, check VBUS level, restore.
fn phase4_step1_gpio_toggle() {
    Logger::log("=== Phase 4.1: GPIO VBUS Toggle ===");

    // Resolve NIDs first for proper GPIO control
    let nid_count = unsafe { gpio::resolve_nids() };
    let mut f = Fmt::new();
    f.p("GPIO NIDs resolved: ");
    f.decimal(nid_count);
    f.p("/4");
    Logger::log(f.s());

    Logger::log("Testing pins 0-31 (skip 19,23)");
    Logger::log("Watch FNB58 for VBUS!");
    Logger::log("");

    let snap = gpio::snapshot();
    log_gpio_snapshot("Baseline:", &snap);
    Logger::log("");

    // Known dangerous pins:
    //   3  = LCD backlight/power (screen turns off)
    //   19 = USB PHY transceiver (disrupts USB)
    //   23 = crash
    // Test safe pins, high pins first (more likely USB-related)
    let skip: [u32; 6] = [3, 4, 19, 23, 24, 26];
    for pin in (0..32u32).rev() {
        if skip.contains(&pin) {
            let mut f = Fmt::new();
            f.p("  SKIP pin ");
            f.decimal(pin);
            Logger::log(f.s());
            continue;
        }
        test_gpio_pin(pin, &snap);
    }
}

/// Test a single GPIO pin: set output, drive high, check VBUS, restore.
fn test_gpio_pin(pin: u32, original: &gpio::GpioSnapshot) {
    let mask = 1u32 << pin;

    let mut f = Fmt::new();
    f.p("Testing GPIO pin ");
    f.decimal(pin);
    f.p(" (mask=");
    f.h8(mask);
    f.p(")...");
    Logger::log(f.s());

    unsafe {
        // Method 1: Try NID functions (preferred)
        if let Some(ret) = gpio::set_port_mode(pin, 1) {
            lr("  NID SetMode ret=", ret as u32);
            if let Some(ret) = gpio::port_set(mask) {
                lr("  NID PortSet ret=", ret as u32);
            }
        } else {
            // Method 2: MMIO — set direction + write output directly
            let dir = psp::hw::hw_read32(0xBE24_0010);
            // SAFETY: kernel-mode MMIO write to GPIO direction.
            psp::hw::hw_write32(0xBE24_0010, dir | mask);
            // Write to Set register AND Output register
            // SAFETY: kernel-mode MMIO write to GPIO set + output.
            psp::hw::hw_write32(0xBE24_0014, mask); // Set
            let out = psp::hw::hw_read32(0xBE24_0008);
            psp::hw::hw_write32(0xBE24_0008, out | mask); // Output
            Logger::log("  MMIO dir+set+out");
        }
    }

    // Wait and check
    unsafe { sceKernelDelayThread(500_000) };

    let new_snap = gpio::snapshot();
    // Only log if something changed (reduce log spam)
    if new_snap.read != original.read
        || new_snap.output != original.output
    {
        lr("  Read  =", new_snap.read);
        lr("  Output=", new_snap.output);
        Logger::log("  *** CHANGE DETECTED ***");
    }

    // Restore
    unsafe {
        if gpio::port_clear(mask).is_none() {
            psp::hw::hw_write32(0xBE24_0018, mask); // Clear
            let out = psp::hw::hw_read32(0xBE24_0008);
            // SAFETY: kernel-mode MMIO write to restore GPIO output.
            psp::hw::hw_write32(0xBE24_0008, out & !mask);
        }
        if gpio::set_port_mode(pin, 0).is_none() {
            // SAFETY: kernel-mode MMIO write to restore GPIO direction.
            psp::hw::hw_write32(0xBE24_0010, original.direction);
        }
    }

    unsafe { sceKernelDelayThread(50_000) };
}

/// Step 4.2: Syscon 0x45+0x47 + OHCI + GPIO sweep
fn phase4_step2_gpio_plus_init() {
    Logger::log("=== Phase 4.2: Combined Approach ===");
    Logger::log("Syscon prepare + activate + clocks + GPIO sweep");
    Logger::log("");

    // Step A: Syscon prepare (0x45) then activate (0x47)
    // Syscon 0x45 can cause black screen — skip it.
    // Only use 0x47 (which returned success in 2.3).
    Logger::log("Syscon SET 0x47 v=1 (activate)...");
    let (r, rx) = syscon::syscon_set(0x47, 1);
    log_cmd("  0x47", r, &rx);
    unsafe { sceKernelDelayThread(500_000) };

    // Step B: Enable clocks + PHY + OHCI port power
    Logger::log("Enabling clocks + PHY + OHCI...");
    unsafe { ohci::enable_clocks() };
    unsafe { sceKernelDelayThread(10_000) };
    unsafe { phy::configure_host_mode() };
    unsafe { sceKernelDelayThread(10_000) };
    unsafe { ohci::enable_ohci_clock_bit1() };
    unsafe { sceKernelDelayThread(10_000) };
    unsafe { ohci::set_port_power() };
    unsafe { sceKernelDelayThread(100_000) };

    // Check if anything changed
    let snap = gpio::snapshot();
    lr("GPIO Read after Syscon+clocks=", snap.read);
    lr("BC100050=", unsafe { hw_read32(0xBC10_0050) });
    Logger::log("");

    // Step C: Try GPIO pins with everything enabled
    Logger::log("Sweeping GPIO pins 0-31...");
    for pin in 0..32u32 {
        if pin == 19 {
            continue;
        }
        test_gpio_pin(pin, &snap);
    }
}

// ── Phase 5: VBUS Enable (from firmware RE) ─────────────────────────────

/// NID 0x317D9D2C — GPIO port mode function used by usb.prx
/// Signature: int func(int pin, int mode)
/// Mode 0 = input/disable, Mode 2 = output for VBUS
const NID_GPIO_PORT_MODE_2: u32 = 0x317D9D2C;

type GpioPortMode2Fn = unsafe extern "C" fn(pin: i32, mode: i32) -> i32;
static mut GPIO_PORT_MODE_2_FN: Option<GpioPortMode2Fn> = None;

/// Step 5.1: Clean VBUS test with full register dump
fn phase5_vbus_enable() {
    Logger::log("=== 5.1: VBUS ENABLE (Pin 23) ===");

    // Resolve all NIDs upfront
    unsafe { gpio::resolve_nids() };
    let mode2_found = unsafe {
        let m = b"sceLowIO_Driver\0".as_ptr();
        let l = b"sceGpio_driver\0".as_ptr();
        if let Some(addr) = psp::hook::find_function(m, l, NID_GPIO_PORT_MODE_2) {
            core::ptr::write_volatile(
                &raw mut GPIO_PORT_MODE_2_FN,
                Some(core::mem::transmute(addr)),
            );
            true
        } else { false }
    };
    if !mode2_found {
        Logger::log("ERROR: NID 0x317D9D2C not found!");
        return;
    }

    // Full register dump BEFORE
    Logger::log("--- BEFORE ---");
    dump_all_gpio();

    // Step 1: sceSysreg enables
    Logger::log("");
    Logger::log("1. sceSysreg GPIO+USB enable...");
    unsafe {
        let m = b"sceLowIO_Driver\0".as_ptr();
        let l = b"sceSysreg_driver\0".as_ptr();
        type F = unsafe extern "C" fn(i32) -> i32;
        for (nid, name) in [
            (0x72C1CA96u32, "GpioIoEn"),
            (0xEC03F6E2, "GpioClkEn"),
            (0x9306F27B, "UsbIoEn"),
            (0x9A6E7BB8, "UsbBusClkEn"),
            (0x1561BCD2, "UsbClkEn"),
        ] {
            if let Some(addr) = psp::hook::find_function(m, l, nid) {
                let func: F = core::mem::transmute(addr);
                let ret = func(0);
                let mut f = Fmt::new();
                f.p("  "); f.p(name); f.p(" ret="); f.h8(ret as u32);
                Logger::log(f.s());
            }
        }
        sceKernelDelayThread(10_000);
    }

    // Step 2: GPIO pin 23 mode + set (NID path)
    Logger::log("");
    Logger::log("2. NID: SetMode(23,2) + PortSet(0x800000)...");
    unsafe {
        let func = core::ptr::read_volatile(
            &raw const GPIO_PORT_MODE_2_FN,
        ).expect("resolved");
        let r1 = func(23, 2);
        lr("  SetMode ret=", r1 as u32);
    }
    if let Some(ret) = unsafe { gpio::port_set(0x0080_0000) } {
        lr("  PortSet ret=", ret as u32);
    }

    // Step 3: Also MMIO set (belt and suspenders)
    Logger::log("");
    Logger::log("3. MMIO: Dir+Set+Out for pin 23...");
    unsafe {
        let dir = psp::hw::hw_read32(0xBE24_0010);
        psp::hw::hw_write32(0xBE24_0010, dir | 0x0080_0000);
        psp::hw::hw_write32(0xBE24_0014, 0x0080_0000);
        let out = psp::hw::hw_read32(0xBE24_0008);
        psp::hw::hw_write32(0xBE24_0008, out | 0x0080_0000);
    }

    unsafe { sceKernelDelayThread(500_000) };

    // Full register dump AFTER
    Logger::log("");
    Logger::log("--- AFTER ---");
    dump_all_gpio();

    // Monitor
    Logger::log("");
    Logger::log("Monitoring 15s — watch FNB58!");
    Logger::log("(VBUS is ON, TRI=stop early)");
    for sec in 0u32..15 {
        unsafe { sceKernelDelayThread(1_000_000) };
        let p0r = unsafe { psp::hw::hw_read32(0xBE24_0000) };
        let p0o = unsafe { psp::hw::hw_read32(0xBE24_0008) };
        let mut f = Fmt::new();
        f.p("  t="); f.decimal(sec + 1);
        f.p(" R="); f.h8(p0r);
        f.p(" O="); f.h8(p0o);
        Logger::log(f.s());

        let mut p = SceCtrlData::default();
        unsafe { sceCtrlPeekBufferPositive(&mut p, 1) };
        if p.buttons.intersects(CtrlButtons::TRIANGLE) {
            Logger::log("  (stopped)");
            break;
        }
    }

    // Disable
    Logger::log("");
    Logger::log("Disabling VBUS...");
    unsafe {
        let _ = gpio::port_clear(0x0080_0000);
        psp::hw::hw_write32(0xBE24_0018, 0x0080_0000);
        psp::hw::hw_write32(0xBE24_0008,
            psp::hw::hw_read32(0xBE24_0008) & !0x0080_0000);
        let func = core::ptr::read_volatile(
            &raw const GPIO_PORT_MODE_2_FN,
        ).expect("resolved");
        func(23, 0);
    }
    Logger::log("Done. Check FNB58 reading.");
}

/// Dump all GPIO and key system registers
fn dump_all_gpio() {
    unsafe {
        lr("P0 Read   =", psp::hw::hw_read32(0xBE24_0000));
        lr("P0 Output =", psp::hw::hw_read32(0xBE24_0008));
        lr("P0 Dir    =", psp::hw::hw_read32(0xBE24_0010));
        lr("P0 Set    =", psp::hw::hw_read32(0xBE24_0014));
        lr("P0 Clear  =", psp::hw::hw_read32(0xBE24_0018));
        lr("P0 AltFunc=", psp::hw::hw_read32(0xBE24_0040));
        lr("P1 Read   =", psp::hw::hw_read32(0xBE24_0004));
        lr("P1 Output =", psp::hw::hw_read32(0xBE24_000C));
        lr("P1 Dir    =", psp::hw::hw_read32(0xBE24_001C));
        lr("P1 AltFunc=", psp::hw::hw_read32(0xBE24_0048));
        lr("+0x20     =", psp::hw::hw_read32(0xBE24_0020));
        lr("+0x24     =", psp::hw::hw_read32(0xBE24_0024));
        lr("+0x28     =", psp::hw::hw_read32(0xBE24_0028));
        lr("+0x2C     =", psp::hw::hw_read32(0xBE24_002C));
        lr("+0x30     =", psp::hw::hw_read32(0xBE24_0030));
        lr("+0x34     =", psp::hw::hw_read32(0xBE24_0034));
        lr("+0x44     =", psp::hw::hw_read32(0xBE24_0044));
        lr("BC1000B8  =", psp::hw::hw_read32(0xBC10_00B8));
        lr("BC100050  =", psp::hw::hw_read32(0xBC10_0050));
        lr("BC100074  =", psp::hw::hw_read32(0xBC10_0074));
        lr("BC10004C  =", psp::hw::hw_read32(0xBC10_004C));
    }
}

// ── Menu ────────────────────────────────────────────────────────────────

struct MenuItem {
    label: &'static str,
    func: fn(),
}

const MENU: &[MenuItem] = &[
    MenuItem { label: "1.1 Dump GPIO registers", func: phase1_step1_dump_gpio },
    MenuItem { label: "1.2 Resolve GPIO NIDs", func: phase1_step2_resolve_nids },
    MenuItem { label: "1.3 Monitor GPIO (needs camera)", func: phase1_step3_monitor_usb_init },
    MenuItem { label: "2.1 Resolve sceSysconCtrlUsbPower", func: phase2_step1_resolve_syscon },
    MenuItem { label: "2.2 Syscon GET status", func: phase2_step2_syscon_status },
    MenuItem { label: "2.3 Syscon SET 0x47", func: phase2_step3_syscon_set },
    MenuItem { label: "3.1 Clock + PHY init", func: phase3_step1_clock_phy },
    MenuItem { label: "3.2 OHCI port power", func: phase3_step2_ohci_init },
    MenuItem { label: "3.3 MUSB (disabled)", func: phase3_step3_musb_host },
    MenuItem { label: "3.4 Tachyon mode bit", func: phase3_step4_tachyon_mode },
    MenuItem { label: "3.5 Full firmware init", func: phase3_step5_full_sequence },
    MenuItem { label: "4.1 GPIO sweep (all pins)", func: phase4_step1_gpio_toggle },
    MenuItem { label: "4.2 Syscon+clocks+GPIO sweep", func: phase4_step2_gpio_plus_init },
    MenuItem { label: "5.1 >>> VBUS ENABLE (pin 23) <<<", func: phase5_vbus_enable },
];

/// Collect menu labels into a fixed array for screen rendering.
const MENU_LABELS: [&str; 14] = [
    "1.1 Dump GPIO registers",
    "1.2 Resolve GPIO NIDs",
    "1.3 Monitor GPIO (needs camera)",
    "2.1 Resolve sceSysconCtrlUsbPower",
    "2.2 Syscon GET status",
    "2.3 Syscon SET 0x47",
    "3.1 Clock + PHY init",
    "3.2 OHCI port power",
    "3.3 MUSB (disabled)",
    "3.4 Tachyon mode bit",
    "3.5 Full firmware init",
    "4.1 GPIO sweep (all pins)",
    "4.2 Syscon+clocks+GPIO sweep",
    "5.1 >>> VBUS ENABLE (pin 23) <<<",
];

fn psp_main() {
    let _ = psp::callback::setup_exit_callback();
    unsafe { sceCtrlSetSamplingMode(CtrlMode::Digital) };
    Logger::init();

    // Log hardware info to file only
    Logger::file("=== USB VBUS Power Output Tool ===");
    lr("Tachyon=", unsafe { hw_read32(0xBC10_0040) });
    unsafe {
        let percent = sys::scePowerGetBatteryLifePercent();
        let voltage = sys::scePowerGetBatteryVolt();
        let ac = sys::scePowerIsPowerOnline();
        let mut f = Fmt::new();
        f.p("Battery: ");
        f.decimal(percent as u32);
        f.p("%  ");
        f.decimal(voltage as u32);
        f.p("mV  AC:");
        f.p(if ac != 0 { "Y" } else { "N" });
        Logger::file(f.s());
    }

    let mut cursor: usize = 0;

    loop {
        // Instant full-screen menu redraw
        screen::draw_menu(&MENU_LABELS, cursor);

        // Wait for input
        let btn = wait_any_button();

        if btn.intersects(CtrlButtons::TRIANGLE) {
            Logger::file("Exiting.");
            break;
        }

        if btn.intersects(CtrlButtons::UP) {
            if cursor > 0 {
                cursor -= 1;
            }
            continue;
        }

        if btn.intersects(CtrlButtons::DOWN) {
            if cursor + 1 < MENU.len() {
                cursor += 1;
            }
            continue;
        }

        if btn.intersects(CtrlButtons::CROSS) {
            screen::clear_screen();
            (MENU[cursor].func)();
            Logger::log("");
            Logger::log("--- Done. Press any button ---");
            wait_any_button();
        }
    }

    // Clean exit — try multiple methods since kernel mode is tricky
    screen::clear_screen();
    unsafe {
        // sceKernelExitGame may not work in kernel mode.
        // Try sceKernelExitThread first (exits our thread), then ExitGame.
        sys::sceKernelExitDeleteThread(0);
    }
    // Fallback if above didn't exit
    unsafe { sys::sceKernelExitGame() };
}
