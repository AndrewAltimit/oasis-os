//! Syscon command helpers for USB power control.
//!
//! Uses `psp::sys::sceSyscon*` for NID-linked functions (resolved at module
//! load) and raw packet construction for direct Syscon SPI commands.
//!
//! Key USB power commands:
//!   0x44 — GET USB power status
//!   0x45 — SET USB power prepare (DANGEROUS: causes shutdown)
//!   0x46 — GET USB power state
//!   0x47 — SET USB power activate

use psp::hw::hw_read32;

static mut CTRL_USB_POWER_FN: Option<unsafe extern "C" fn(i32) -> i32> = None;
static mut RESOLVED_ADDR: u32 = 0;

/// Get the resolved function address (for logging).
pub fn resolved_addr() -> u32 {
    // SAFETY: volatile read of a fixed-address register or module-static.
    unsafe { core::ptr::read_volatile(&raw const RESOLVED_ADDR) }
}

/// Resolve sceSysconCtrlUsbPower via find_function.
/// Returns true if found.
pub unsafe fn resolve_nids() -> bool {
    let modules: [&[u8]; 2] = [
        psp::sys::syscon::SYSCON_MODULE,
        psp::sys::syscon::SYSCON_MODULE_ALT,
    ];
    let library = psp::sys::syscon::SYSCON_LIBRARY.as_ptr();
    let nid = psp::sys::syscon::NID_SYSCON_CTRL_USB_POWER;

    for module in &modules {
        if let Some(addr) = psp::hook::find_function(module.as_ptr(), library, nid) {
            core::ptr::write_volatile(
                &raw mut CTRL_USB_POWER_FN,
                Some(core::mem::transmute(addr)),
            );
            core::ptr::write_volatile(&raw mut RESOLVED_ADDR, addr as u32);
            return true;
        }
    }
    false
}

/// Call sceSysconCtrlUsbPower(enable) via resolved function pointer.
///
/// Note: On PSP-3001 6.61, this NID resolves to a getter stub — it reads
/// a cached value but may not actually control USB power. The raw Syscon
/// command path (syscon_set 0x47) is more reliable for testing.
pub unsafe fn ctrl_usb_power(enable: i32) -> Option<i32> {
    let f = core::ptr::read_volatile(&raw const CTRL_USB_POWER_FN)?;
    Some(f(enable))
}

/// Get the _sceSysconCommonWrite function address based on Tachyon version.
fn syscon_common_write_addr() -> u32 {
    // SAFETY: reading Tachyon version register (read-only MMIO).
    let tachyon = unsafe { hw_read32(psp::hw::SYSREG_TACHYON_VER) } & 0xFF00_0000;
    if tachyon >= 0x0050_0000 {
        0x880A6E4C // PSP-3001 (Tachyon TA-090v2+)
    } else {
        0x880A6D4C // PSP-1000 variants
    }
}

/// Prepare a Syscon packet, send it via _sceSysconCommonWrite, and
/// return (return_code, response_bytes[16]).
///
/// `data` contains the payload bytes AFTER the command byte. For GET
/// commands this is empty; for SET commands it contains the value byte.
fn syscon_cmd(cmd: u8, data: &[u8]) -> (i32, [u8; 16]) {
    let mut pkt = [0u8; 128];
    // Pre-fill response regions with 0xFF
    for b in pkt[0x0C..0x1C].iter_mut() {
        *b = 0xFF;
    }
    for b in pkt[0x1C..0x2C].iter_mut() {
        *b = 0xFF;
    }

    // Build packet: [cmd, length, ...data, checksum]
    let pkt_len = 2 + data.len(); // cmd + len_byte + data
    pkt[0x0C] = cmd;
    pkt[0x0D] = pkt_len as u8;
    for (i, &b) in data.iter().enumerate() {
        pkt[0x0E + i] = b;
    }
    let mut sum = 0u8;
    for i in 0..pkt_len {
        sum = sum.wrapping_add(pkt[0x0C + i]);
    }
    pkt[0x0C + pkt_len] = !sum;

    type F = unsafe extern "C" fn(*mut u8, i32) -> i32;
    let addr = syscon_common_write_addr();
    // SAFETY: calling kernel Syscon driver function at known address.
    let func: F = unsafe { core::mem::transmute(addr) };
    let ret = unsafe { func(pkt.as_mut_ptr(), 0) };

    let mut rx = [0u8; 16];
    rx.copy_from_slice(&pkt[0x1C..0x2C]);
    (ret, rx)
}

/// Send a Syscon SET command (cmd, value).
///
/// Returns (return_code, response_bytes[16]).
pub fn syscon_set(cmd: u8, value: u8) -> (i32, [u8; 16]) {
    syscon_cmd(cmd, &[value])
}

/// Send a Syscon GET command (cmd only, no payload).
///
/// Returns (return_code, response_bytes[16]).
pub fn syscon_get(cmd: u8) -> (i32, [u8; 16]) {
    syscon_cmd(cmd, &[])
}
