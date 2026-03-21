//! GPIO register read/write for VBUS MOSFET discovery.
//!
//! Uses `psp::sys` for NID-linked GPIO functions (resolved at module load)
//! and `psp::hw` for direct MMIO register snapshots.
//!
//! GPIO block base: 0xBE240000
//!   +0x00  Port 0 data read
//!   +0x04  Port 1 data read
//!   +0x08  Port 1 set
//!   +0x0C  Port 1 clear
//!   +0x10  Port 0 direction (0=input, 1=output)
//!   +0x14  Port 0 set (write 1 to set bits)
//!   +0x18  Port 0 clear (write 1 to clear bits)
//!   +0x24  Output enable (silicon-locked on TA-090v2)
//!   +0x40  Alternate function select (silicon-locked on TA-090v2)

use psp::hw::{
    self, hw_read32, hw_write32,
    GPIO_BASE, GPIO_PORT0_READ, GPIO_PORT0_DIR, GPIO_PORT0_SET,
    GPIO_PORT0_CLEAR, GPIO_OUTPUT_EN, GPIO_PORT0_ALTFUNC,
};

// Additional offsets not in psp::hw (used only for MMIO snapshots)
const GPIO_OUTPUT: u32 = GPIO_BASE + 0x08;

/// Snapshot of all GPIO registers at a point in time.
#[derive(Clone, Copy)]
pub struct GpioSnapshot {
    pub read: u32,
    pub output: u32,
    pub direction: u32,
    pub alt_func: u32,
}

/// Read all GPIO registers into a snapshot via MMIO.
pub fn snapshot() -> GpioSnapshot {
    // SAFETY: kernel-mode MMIO reads of GPIO registers (read-only, safe).
    unsafe {
        GpioSnapshot {
            read: hw_read32(GPIO_PORT0_READ),
            output: hw_read32(GPIO_OUTPUT),
            direction: hw_read32(GPIO_PORT0_DIR),
            alt_func: hw_read32(GPIO_PORT0_ALTFUNC),
        }
    }
}

/// Compare two snapshots, returning bitmask of changed bits in each register.
pub struct GpioDiff {
    pub read_changed: u32,
    pub output_changed: u32,
    pub direction_changed: u32,
    pub alt_func_changed: u32,
}

pub fn diff(before: &GpioSnapshot, after: &GpioSnapshot) -> GpioDiff {
    GpioDiff {
        read_changed: before.read ^ after.read,
        output_changed: before.output ^ after.output,
        direction_changed: before.direction ^ after.direction,
        alt_func_changed: before.alt_func ^ after.alt_func,
    }
}

/// No-op for backwards compatibility. GPIO functions are now linked at
/// module load via `psp::sys::sceGpio*` — no runtime NID resolution needed.
///
/// Returns 4 (all functions available) to match old API contract.
pub unsafe fn resolve_nids() -> u32 {
    4
}

/// Read GPIO port via psp::sys (linked at module load).
pub unsafe fn port_read() -> Option<u32> {
    Some(psp::sys::sceGpioPortRead() as u32)
}

/// Set GPIO port bits via psp::sys.
pub fn port_set(mask: u32) -> Option<i32> {
    let ret = unsafe { psp::sys::sceGpioPortSet(mask as i32) };
    Some(ret)
}

/// Clear GPIO port bits via psp::sys.
pub fn port_clear(mask: u32) -> Option<i32> {
    let ret = unsafe { psp::sys::sceGpioPortClear(mask as i32) };
    Some(ret)
}

/// Set GPIO pin direction (0=input, 1=output) via psp::sys.
///
/// Uses `sceGpioSetPortMode` (NID 0xFBC85E74) which controls the
/// Direction register. For full output enable (direction + output MUX),
/// use `set_port_mode2` instead.
pub fn set_port_mode(pin: u32, mode: u32) -> Option<i32> {
    let ret = unsafe { psp::sys::sceGpioSetPortMode(pin as i32, mode as i32) };
    Some(ret)
}

/// Set GPIO pin full output mode (0=disable, 2=output enable) via psp::sys.
///
/// Uses `sceGpioSetPortMode2` (NID 0x317D9D2C) — the function used by
/// `usb.prx` for VBUS control. Writes to both Direction and Output Enable.
pub fn set_port_mode2(pin: u32, mode: u32) -> Option<i32> {
    let ret = unsafe { psp::sys::sceGpioSetPortMode2(pin as i32, mode as i32) };
    Some(ret)
}

/// Direct MMIO write to GPIO set register (no NID needed, kernel-mode only).
pub unsafe fn mmio_set(mask: u32) {
    // SAFETY: kernel-mode MMIO write to GPIO output set register.
    hw_write32(GPIO_PORT0_SET, mask);
}

/// Direct MMIO write to GPIO clear register (no NID needed, kernel-mode only).
pub unsafe fn mmio_clear(mask: u32) {
    // SAFETY: kernel-mode MMIO write to GPIO output clear register.
    hw_write32(GPIO_PORT0_CLEAR, mask);
}

/// Direct MMIO write to GPIO direction register (no NID needed, kernel-mode only).
pub unsafe fn mmio_set_direction(val: u32) {
    // SAFETY: kernel-mode MMIO write to GPIO direction register.
    hw_write32(GPIO_PORT0_DIR, val);
}
