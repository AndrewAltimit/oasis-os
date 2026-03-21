//! GPIO register read/write for VBUS MOSFET discovery.
//!
//! Uses `psp::gpio` for NID-resolved GPIO functions (via find_function)
//! and `psp::hw` for direct MMIO register snapshots.

use psp::hw::{
    hw_read32, hw_write32,
    GPIO_BASE, GPIO_PORT0_READ, GPIO_PORT0_DIR, GPIO_PORT0_SET,
    GPIO_PORT0_CLEAR, GPIO_PORT0_ALTFUNC,
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

/// Initialize GPIO NID resolution via psp::gpio.
/// Returns number of resolved functions (0-6).
pub unsafe fn resolve_nids() -> u32 {
    psp::gpio::init()
}

/// Read GPIO port via psp::gpio (find_function resolved).
pub unsafe fn port_read() -> Option<u32> {
    psp::gpio::read_port()
}

/// Set GPIO port bits via psp::gpio.
pub fn port_set(mask: u32) -> Option<i32> {
    psp::gpio::set_pins(mask)
}

/// Clear GPIO port bits via psp::gpio.
pub fn port_clear(mask: u32) -> Option<i32> {
    psp::gpio::clear_pins(mask)
}

/// Set GPIO pin direction (0=input, 1=output) via psp::gpio.
pub fn set_port_mode(pin: u32, mode: u32) -> Option<i32> {
    psp::gpio::set_pin_mode(pin, mode as i32)
}

/// Set GPIO pin full output mode (0=disable, 2=output enable) via psp::gpio.
pub fn set_port_mode2(pin: u32, mode: u32) -> Option<i32> {
    psp::gpio::set_pin_mode2(pin, mode as i32)
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
