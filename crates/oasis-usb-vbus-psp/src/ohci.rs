//! OHCI controller and clock initialization for USB host mode.
//!
//! The PSP has an OHCI USB controller at 0xBD100000 and a MUSBMHDRC
//! (Mentor USB controller) at 0xBD800000. The firmware's `usb_init_main`
//! at 0x88604A8C sets up clocks, PHY, and OHCI before the reboot.
//!
//! Register map:
//!   BC100050 — System clock control 1
//!   BC100058 — System clock control 2
//!   BC100078 — OHCI clock enable
//!   BD100000 — OHCI base (HcRevision, etc.)
//!   BD101038 — HcRhPortStatus (port power + enable)
//!   BD800060 — MUSBMHDRC DevCtl (session/host mode)

use psp::hw::{
    hw_read32, hw_write32,
    OHCI_BASE, MUSB_BASE, SYSREG_TACHYON_VER,
    SYSREG_PERIPH_CLK1, SYSREG_PERIPH_CLK2, SYSREG_USB_CLK,
};

// Clock registers (using psp::hw constants where available)
const SYS_CLK1: u32 = SYSREG_PERIPH_CLK1;
const SYS_CLK2: u32 = SYSREG_PERIPH_CLK2;
const OHCI_CLK: u32 = SYSREG_USB_CLK;
const TACHYON_REG: u32 = SYSREG_TACHYON_VER;

// OHCI registers (offsets from OHCI_BASE)
const OHCI_REVISION: u32 = OHCI_BASE;
const OHCI_CONTROL: u32 = OHCI_BASE + 0x004;
const OHCI_CMDSTATUS: u32 = OHCI_BASE + 0x008;
const OHCI_RH_STATUS: u32 = OHCI_BASE + 0x1034;
const OHCI_RH_PORT_STATUS: u32 = OHCI_BASE + 0x1038;

// MUSBMHDRC registers (offsets from MUSB_BASE)
const MUSB_DEVCTL: u32 = MUSB_BASE + 0x060;

/// Snapshot of USB controller registers.
#[derive(Clone, Copy)]
pub struct OhciSnapshot {
    pub sys_clk1: u32,
    pub sys_clk2: u32,
    pub ohci_clk: u32,
    pub tachyon: u32,
    pub ohci_revision: u32,
    pub ohci_control: u32,
    pub ohci_cmd_status: u32,
    pub ohci_rh_status: u32,
    pub ohci_rh_port_status: u32,
    pub musb_devctl: u32,
}

/// Read all OHCI/MUSB/clock registers.
/// Automatically enables clocks if not already done.
pub fn snapshot() -> OhciSnapshot {
    // SAFETY: kernel-mode MMIO reads; ensure_clocks prevents bus-fault.
    unsafe {
        ensure_clocks();
        OhciSnapshot {
            sys_clk1: hw_read32(SYS_CLK1),
            sys_clk2: hw_read32(SYS_CLK2),
            ohci_clk: hw_read32(OHCI_CLK),
            tachyon: hw_read32(TACHYON_REG),
            ohci_revision: hw_read32(OHCI_REVISION),
            ohci_control: hw_read32(OHCI_CONTROL),
            ohci_cmd_status: hw_read32(OHCI_CMDSTATUS),
            ohci_rh_status: hw_read32(OHCI_RH_STATUS),
            ohci_rh_port_status: hw_read32(OHCI_RH_PORT_STATUS),
            // MUSB at 0xBD80xxxx bus-faults — needs separate enable
            musb_devctl: 0,
        }
    }
}

static mut CLOCKS_ENABLED: bool = false;

/// Ensure clocks are enabled (idempotent).
pub unsafe fn ensure_clocks() {
    if !core::ptr::read_volatile(&raw const CLOCKS_ENABLED) {
        enable_clocks();
        psp::sys::sceKernelDelayThread(10_000);
        core::ptr::write_volatile(&raw mut CLOCKS_ENABLED, true);
    }
}

/// Enable USB peripheral clocks.
///
/// Sets bits in BC100050, BC100058, BC100078 per firmware init sequence.
pub unsafe fn enable_clocks() {
    let v50 = hw_read32(SYS_CLK1);
    // SAFETY: kernel-mode MMIO write — enable USB peripheral clock.
    hw_write32(SYS_CLK1, v50 | 0x4000);

    let v58 = hw_read32(SYS_CLK2);
    // SAFETY: kernel-mode MMIO write — enable USB clock 2.
    hw_write32(SYS_CLK2, v58 | 0x200);

    let v78 = hw_read32(OHCI_CLK);
    // SAFETY: kernel-mode MMIO write — enable OHCI clock.
    hw_write32(OHCI_CLK, v78 | 0x8_0000);

    // Memory barrier
    core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
}

/// Enable OHCI clock bit 1 (separate from the main OHCI clock enable).
pub unsafe fn enable_ohci_clock_bit1() {
    ensure_clocks();
    let v78 = hw_read32(OHCI_CLK);
    // SAFETY: kernel-mode MMIO write — enable OHCI clock bit 1.
    hw_write32(OHCI_CLK, v78 | 2);
}

/// Set OHCI root hub port status to enable port power + port enable.
///
/// HcRhPortStatus bits:
///   bit 0 = CurrentConnectStatus (read) / ClearPortEnable (write)
///   bit 1 = PortEnableStatus (read) / SetPortEnable (write)
///   bit 8 = PortPowerStatus (read) / SetPortPower (write)
///   bit 9 = LowSpeedDeviceAttached (read) / ClearPortPower (write)
pub unsafe fn set_port_power() {
    ensure_clocks();
    // SAFETY: kernel-mode MMIO write — set OHCI port power + enable.
    hw_write32(OHCI_RH_PORT_STATUS, 0x0303);
}

/// Set MUSBMHDRC DevCtl to host mode + session.
///
/// DevCtl bits:
///   bit 0 = Session (start OTG session)
///   bit 2 = HostMode
///   bit 5 = Host request
///   bits 3-4 = VBUS level (read-only): 0=below SessionEnd, 1=above SessionEnd,
///              2=above AValid, 3=above VBusValid
///
/// Returns the DevCtl value after write (read back).
pub unsafe fn set_musb_host_session() -> u32 {
    // MUSB at 0xBD80xxxx bus-faults — disabled for now.
    // TODO: find MUSB bus enable, then write DevCtl | 0x21.
    0
}

/// Read MUSB DevCtl VBUS level (bits 3-4).
///
/// Returns 0-3:
///   0 = below SessionEnd (<0.5V)
///   1 = above SessionEnd (>0.8V)
///   2 = above AValid (>2.0V)
///   3 = above VBusValid (>4.4V) — this means VBUS is powered!
pub fn vbus_level() -> u32 {
    // MUSB DevCtl at 0xBD800060 bus-faults unless MUSB is enabled.
    // Return 0xFF to indicate "not readable".
    0xFF
}

/// Try reading MUSB DevCtl. Returns None if not accessible.
/// Call only after confirming MUSB bus is enabled.
pub unsafe fn try_musb_devctl() -> Option<u32> {
    // For now, MUSB is not accessible — return None.
    // TODO: find MUSB bus enable register.
    None
}

/// Get human-readable VBUS level description.
pub fn vbus_level_str(level: u32) -> &'static str {
    match level {
        0 => "<0.5V (below SessionEnd)",
        1 => ">0.8V (above SessionEnd)",
        2 => ">2.0V (above AValid)",
        3 => ">4.4V (VBusValid - POWERED!)",
        _ => "unknown",
    }
}

/// Read back OHCI root hub port status register.
pub fn port_status() -> u32 {
    // SAFETY: kernel-mode MMIO read; ensure_clocks prevents bus-fault.
    unsafe { ensure_clocks() };
    unsafe { hw_read32(OHCI_RH_PORT_STATUS) }
}

/// Read Tachyon mode register (BC100040).
pub fn tachyon_mode() -> u32 {
    // SAFETY: kernel-mode MMIO read.
    unsafe { hw_read32(TACHYON_REG) }
}

/// Set Tachyon mode bit 0 (USB host mode flag).
///
/// Firmware writes: BC100040 = (BC100040 & 0xFFFFFFFC) | 1
/// WARNING: Tachyon may monitor this continuously. Test carefully.
pub unsafe fn set_tachyon_host_mode() {
    let val = hw_read32(TACHYON_REG);
    // SAFETY: kernel-mode MMIO write — set Tachyon USB host mode bit.
    hw_write32(TACHYON_REG, (val & 0xFFFF_FFFC) | 1);
}
