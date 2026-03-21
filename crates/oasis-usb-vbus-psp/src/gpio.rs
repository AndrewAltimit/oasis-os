//! GPIO register read/write and NID resolution for VBUS MOSFET discovery.
//!
//! GPIO block base: 0xBE240000
//!   +0x00  Port data read
//!   +0x08  Output data
//!   +0x10  Direction (0=input, 1=output)
//!   +0x14  Output set (write 1 to set bits)
//!   +0x18  Output clear (write 1 to clear bits)
//!   +0x40  Alternate function select

use psp::hw::{hw_read32, hw_write32};

// GPIO register offsets from 0xBE240000
const GPIO_BASE: u32 = 0xBE24_0000;
const GPIO_READ: u32 = GPIO_BASE + 0x00;
const GPIO_OUTPUT: u32 = GPIO_BASE + 0x08;
const GPIO_DIRECTION: u32 = GPIO_BASE + 0x10;
const GPIO_SET: u32 = GPIO_BASE + 0x14;
const GPIO_CLEAR: u32 = GPIO_BASE + 0x18;
const GPIO_ALT_FUNC: u32 = GPIO_BASE + 0x40;

/// GPIO NID constants for sceLowIO_Driver / sceGpio_driver
const NID_GPIO_PORT_READ: u32 = 0x4250D44A;
const NID_GPIO_PORT_SET: u32 = 0x310F0CCF;
const NID_GPIO_PORT_CLEAR: u32 = 0x103C3EB2;
const NID_GPIO_SET_PORT_MODE: u32 = 0xFBC85E74;

/// Function pointer types for GPIO NIDs
type GpioPortReadFn = unsafe extern "C" fn() -> u32;
type GpioPortSetFn = unsafe extern "C" fn(mask: u32) -> i32;
type GpioPortClearFn = unsafe extern "C" fn(mask: u32) -> i32;
type GpioSetPortModeFn = unsafe extern "C" fn(pin: u32, mode: u32) -> i32;

/// Resolved GPIO function pointers (None if NID not found)
static mut GPIO_PORT_READ_FN: Option<GpioPortReadFn> = None;
static mut GPIO_PORT_SET_FN: Option<GpioPortSetFn> = None;
static mut GPIO_PORT_CLEAR_FN: Option<GpioPortClearFn> = None;
static mut GPIO_SET_PORT_MODE_FN: Option<GpioSetPortModeFn> = None;

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
            read: hw_read32(GPIO_READ),
            output: hw_read32(GPIO_OUTPUT),
            direction: hw_read32(GPIO_DIRECTION),
            alt_func: hw_read32(GPIO_ALT_FUNC),
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

/// Resolve GPIO function NIDs via sctrlHENFindFunction.
/// Returns number of successfully resolved functions (0-4).
pub unsafe fn resolve_nids() -> u32 {
    let module = b"sceLowIO_Driver\0".as_ptr();
    let library = b"sceGpio_driver\0".as_ptr();
    let mut count = 0u32;

    if let Some(addr) = psp::hook::find_function(module, library, NID_GPIO_PORT_READ) {
        // SAFETY: NID-resolved function pointer from kernel driver.
        core::ptr::write_volatile(
            &raw mut GPIO_PORT_READ_FN,
            Some(core::mem::transmute(addr)),
        );
        count += 1;
    }
    if let Some(addr) = psp::hook::find_function(module, library, NID_GPIO_PORT_SET) {
        // SAFETY: NID-resolved function pointer from kernel driver.
        core::ptr::write_volatile(
            &raw mut GPIO_PORT_SET_FN,
            Some(core::mem::transmute(addr)),
        );
        count += 1;
    }
    if let Some(addr) = psp::hook::find_function(module, library, NID_GPIO_PORT_CLEAR) {
        // SAFETY: NID-resolved function pointer from kernel driver.
        core::ptr::write_volatile(
            &raw mut GPIO_PORT_CLEAR_FN,
            Some(core::mem::transmute(addr)),
        );
        count += 1;
    }
    if let Some(addr) = psp::hook::find_function(module, library, NID_GPIO_SET_PORT_MODE) {
        // SAFETY: NID-resolved function pointer from kernel driver.
        core::ptr::write_volatile(
            &raw mut GPIO_SET_PORT_MODE_FN,
            Some(core::mem::transmute(addr)),
        );
        count += 1;
    }

    count
}

/// Read GPIO port via NID-resolved function. Returns None if NID not resolved.
pub unsafe fn port_read() -> Option<u32> {
    let f = core::ptr::read_volatile(&raw const GPIO_PORT_READ_FN);
    f.map(|func| func())
}

/// Set GPIO port bits via NID-resolved function.
pub unsafe fn port_set(mask: u32) -> Option<i32> {
    let f = core::ptr::read_volatile(&raw const GPIO_PORT_SET_FN);
    f.map(|func| func(mask))
}

/// Clear GPIO port bits via NID-resolved function.
pub unsafe fn port_clear(mask: u32) -> Option<i32> {
    let f = core::ptr::read_volatile(&raw const GPIO_PORT_CLEAR_FN);
    f.map(|func| func(mask))
}

/// Set GPIO pin mode (0=input, 1=output) via NID-resolved function.
pub unsafe fn set_port_mode(pin: u32, mode: u32) -> Option<i32> {
    let f = core::ptr::read_volatile(&raw const GPIO_SET_PORT_MODE_FN);
    f.map(|func| func(pin, mode))
}

/// Direct MMIO write to GPIO set register (no NID needed, kernel-mode only).
pub unsafe fn mmio_set(mask: u32) {
    // SAFETY: kernel-mode MMIO write to GPIO output set register.
    hw_write32(GPIO_SET, mask);
}

/// Direct MMIO write to GPIO clear register (no NID needed, kernel-mode only).
pub unsafe fn mmio_clear(mask: u32) {
    // SAFETY: kernel-mode MMIO write to GPIO output clear register.
    hw_write32(GPIO_CLEAR, mask);
}

/// Direct MMIO write to GPIO direction register (no NID needed, kernel-mode only).
pub unsafe fn mmio_set_direction(val: u32) {
    // SAFETY: kernel-mode MMIO write to GPIO direction register.
    hw_write32(GPIO_DIRECTION, val);
}
