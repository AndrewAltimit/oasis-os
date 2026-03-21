//! Syscon command helpers for USB power control.
//!
//! Syscon is the PMU/power controller on PSP. It communicates via a custom
//! packet protocol through a shared memory region. The kernel function
//! `_sceSysconCommonWrite` at a Tachyon-version-dependent address handles
//! the actual SPI transaction.
//!
//! Key USB power commands:
//!   0x44 — GET USB power status
//!   0x45 — SET USB power prepare
//!   0x46 — GET USB power state
//!   0x47 — SET USB power activate
//!
//! NID for official API: sceSysconCtrlUsbPower = 0xC8D97773

use psp::hw::hw_read32;

/// NID for sceSysconCtrlUsbPower (may not be exported on all FW versions)
const NID_SYSCON_CTRL_USB_POWER: u32 = 0xC8D97773;

/// Resolved sceSysconCtrlUsbPower function pointer
type SysconCtrlUsbPowerFn = unsafe extern "C" fn(enable: i32) -> i32;
static mut SYSCON_CTRL_USB_POWER_FN: Option<SysconCtrlUsbPowerFn> = None;
static mut RESOLVED_ADDR: u32 = 0;

/// Get the resolved function address (for logging).
pub fn resolved_addr() -> u32 {
    unsafe { core::ptr::read_volatile(&raw const RESOLVED_ADDR) }
}

/// Attempt to resolve sceSysconCtrlUsbPower via NID.
/// Returns true if found.
pub unsafe fn resolve_nids() -> bool {
    let modules: [(*const u8, *const u8); 2] = [
        (b"sceSyscon_Driver\0".as_ptr(), b"sceSyscon_driver\0".as_ptr()),
        (b"sceSYSCON_Driver\0".as_ptr(), b"sceSyscon_driver\0".as_ptr()),
    ];

    for (module, library) in &modules {
        if let Some(addr) =
            psp::hook::find_function(*module, *library, NID_SYSCON_CTRL_USB_POWER)
        {
            // SAFETY: NID-resolved function pointer from kernel Syscon driver.
            core::ptr::write_volatile(
                &raw mut SYSCON_CTRL_USB_POWER_FN,
                Some(core::mem::transmute(addr)),
            );
            core::ptr::write_volatile(
                &raw mut RESOLVED_ADDR,
                addr as u32,
            );
            return true;
        }
    }

    false
}

/// Call sceSysconCtrlUsbPower(enable) if resolved.
/// Returns Some(ret_code) or None if NID not found.
pub unsafe fn ctrl_usb_power(enable: i32) -> Option<i32> {
    let f = core::ptr::read_volatile(&raw const SYSCON_CTRL_USB_POWER_FN);
    f.map(|func| func(enable))
}

/// Get the _sceSysconCommonWrite function address based on Tachyon version.
fn syscon_common_write_addr() -> u32 {
    // SAFETY: reading Tachyon version register (read-only MMIO).
    let tachyon = unsafe { hw_read32(0xBC10_0040) } & 0xFF00_0000;
    if tachyon >= 0x0050_0000 {
        0x880A6E4C // PSP-3001 (Tachyon TA-090v2+)
    } else {
        0x880A6D4C // PSP-1000 variants
    }
}

/// Send a Syscon SET command (length=3 packet: cmd, len, value, checksum).
///
/// Returns (return_code, response_bytes[16]).
pub fn syscon_set(cmd: u8, value: u8) -> (i32, [u8; 16]) {
    let mut pkt = [0u8; 128];
    // Pre-fill response regions with 0xFF
    for b in pkt[0x0C..0x1C].iter_mut() {
        *b = 0xFF;
    }
    for b in pkt[0x1C..0x2C].iter_mut() {
        *b = 0xFF;
    }

    pkt[0x0C] = cmd;
    pkt[0x0D] = 3; // length for SET commands
    pkt[0x0E] = value;
    let sum = pkt[0x0C].wrapping_add(pkt[0x0D]).wrapping_add(pkt[0x0E]);
    pkt[0x0F] = !sum;

    type F = unsafe extern "C" fn(*mut u8, i32) -> i32;
    let addr = syscon_common_write_addr();
    // SAFETY: calling kernel Syscon driver function at known address.
    let func: F = unsafe { core::mem::transmute(addr) };
    let ret = unsafe { func(pkt.as_mut_ptr(), 0) };

    let mut rx = [0u8; 16];
    rx.copy_from_slice(&pkt[0x1C..0x2C]);
    (ret, rx)
}

/// Send a Syscon GET command (length=2 packet: cmd, len, checksum).
///
/// Returns (return_code, response_bytes[16]).
pub fn syscon_get(cmd: u8) -> (i32, [u8; 16]) {
    let mut pkt = [0u8; 128];
    for b in pkt[0x0C..0x1C].iter_mut() {
        *b = 0xFF;
    }
    for b in pkt[0x1C..0x2C].iter_mut() {
        *b = 0xFF;
    }

    pkt[0x0C] = cmd;
    pkt[0x0D] = 2; // length for GET commands
    let sum = pkt[0x0C].wrapping_add(pkt[0x0D]);
    pkt[0x0E] = !sum;

    type F = unsafe extern "C" fn(*mut u8, i32) -> i32;
    let addr = syscon_common_write_addr();
    // SAFETY: calling kernel Syscon driver function at known address.
    let func: F = unsafe { core::mem::transmute(addr) };
    let ret = unsafe { func(pkt.as_mut_ptr(), 0) };

    let mut rx = [0u8; 16];
    rx.copy_from_slice(&pkt[0x1C..0x2C]);
    (ret, rx)
}
