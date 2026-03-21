//! USB PHY configuration at 0xBE4C0000.
//!
//! The PSP's USB PHY needs to be configured for host mode operation.
//! The firmware's `usb_init_main` sets these registers as part of the
//! USB host initialization sequence.
//!
//! Register map:
//!   BE4C0024 — PHY clock divisor (96MHz / N = USB clock)
//!   BE4C0028 — PHY clock divisor high
//!   BE4C002C — PHY config
//!   BE4C0030 — PHY mode (bit 0 = host enable, bits 8-9 = mode)
//!   BE4C0044 — Feature enable bitmask

use psp::hw::{hw_read32, hw_write32, USB_PHY_BASE};

const PHY_CLK_DIV: u32 = USB_PHY_BASE + 0x24;
const PHY_CLK_DIV_HI: u32 = USB_PHY_BASE + 0x28;
const PHY_CONFIG: u32 = USB_PHY_BASE + 0x2C;
const PHY_MODE: u32 = USB_PHY_BASE + 0x30;
const PHY_FEATURE: u32 = USB_PHY_BASE + 0x44;

/// Snapshot of USB PHY registers.
#[derive(Clone, Copy)]
pub struct PhySnapshot {
    pub clk_div: u32,
    pub clk_div_hi: u32,
    pub config: u32,
    pub mode: u32,
    pub feature: u32,
}

/// Read all PHY registers.
pub fn snapshot() -> PhySnapshot {
    // SAFETY: kernel-mode MMIO reads (read-only, safe).
    unsafe {
        PhySnapshot {
            clk_div: hw_read32(PHY_CLK_DIV),
            clk_div_hi: hw_read32(PHY_CLK_DIV_HI),
            config: hw_read32(PHY_CONFIG),
            mode: hw_read32(PHY_MODE),
            feature: hw_read32(PHY_FEATURE),
        }
    }
}

/// Configure PHY for host mode per firmware init sequence.
///
/// Sequence from `usb_init_main` @ 0x88604A8C:
///   BE4C0024 = 6          (96MHz / 6 = 16MHz USB clock)
///   BE4C0028 = 0          (divisor high = 0)
///   BE4C002C = 0x60       (PHY config)
///   BE4C0030 |= 0x301     (host mode enable)
///   BE4C0044 |= 0x7FF     (feature enable all)
pub unsafe fn configure_host_mode() {
    // SAFETY: kernel-mode MMIO writes to PHY registers per firmware sequence.
    hw_write32(PHY_CLK_DIV, 6);
    hw_write32(PHY_CLK_DIV_HI, 0);
    hw_write32(PHY_CONFIG, 0x60);

    let mode = hw_read32(PHY_MODE);
    hw_write32(PHY_MODE, mode | 0x301);

    // Small delay for PHY to settle
    psp::sys::sceKernelDelayThread(1_000); // 1ms

    let feat = hw_read32(PHY_FEATURE);
    hw_write32(PHY_FEATURE, feat | 0x7FF);
}
