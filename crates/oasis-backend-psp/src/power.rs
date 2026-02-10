//! Power management, clock control, and exception handling.
//!
//! Clock and power callbacks use `psp::power` high-level wrappers.
//! Exception handler uses raw syscalls (no high-level wrapper).

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};

use psp::sys;

/// Flag indicating a resume-from-sleep event occurred.
static POWER_RESUMED: AtomicBool = AtomicBool::new(false);

/// Set CPU and bus clock frequencies.
///
/// Common presets:
/// - `set_clock(333, 166)` -- maximum performance
/// - `set_clock(266, 133)` -- balanced
/// - `set_clock(222, 111)` -- power saving (default)
///
/// Returns 0 on success, < 0 on error (for backwards compatibility with
/// main.rs callers that check `ret >= 0`).
pub fn set_clock(cpu: i32, bus: i32) -> i32 {
    match psp::power::set_clock(cpu, bus) {
        Ok(_) => 0,
        Err(e) => e.0,
    }
}

/// Register a power callback for suspend/resume notification.
///
/// Returns a RAII handle that unregisters the callback on drop.
/// The caller must keep the handle alive for the duration of the program.
pub fn register_power_callback() -> Option<psp::power::PowerCallbackHandle> {
    psp::power::on_power_event(power_callback).ok()
}

/// Check and clear the "resumed from sleep" flag.
pub fn check_power_resumed() -> bool {
    POWER_RESUMED.swap(false, Ordering::AcqRel)
}

/// Prevent the PSP from auto-suspending due to idle timeout.
/// Call once per frame during active use.
pub fn power_tick() {
    psp::power::prevent_sleep();
}

/// SAFETY: Called by the PSP firmware on power state changes. POWER_RESUMED
/// is an AtomicBool, so cross-thread access is safe without unsafe.
unsafe extern "C" fn power_callback(_arg1: i32, power_info: i32, _arg: *mut c_void) -> i32 {
    let info = sys::PowerInfo::from_bits_truncate(power_info as u32);
    if info.contains(sys::PowerInfo::RESUME_COMPLETE) {
        psp::dprintln!("OASIS_OS: Resumed from sleep");
        POWER_RESUMED.store(true, Ordering::Release);
    }
    if info.contains(sys::PowerInfo::SUSPENDING) {
        psp::dprintln!("OASIS_OS: Entering suspend");
    }
    0
}

/// Register a default exception handler that prints the exception type
/// via debug output. Prevents silent crashes on real hardware.
pub fn register_exception_handler() {
    // SAFETY: Registers a static function pointer as the default exception
    // handler. The callback signature matches the PSP SDK contract.
    unsafe {
        sys::sceKernelRegisterDefaultExceptionHandler(exception_handler);
    }
}

/// Exception handler callback -- prints exception info and halts.
/// SAFETY: Called by the PSP firmware on unhandled CPU exceptions.
/// Only reads the exception code; does not dereference the context pointer.
unsafe extern "C" fn exception_handler(exception: u32, _context: *mut c_void) -> i32 {
    let name = match exception {
        0 => "Interrupt",
        1 => "TLB Modification",
        2 => "TLB Load Miss",
        3 => "TLB Store Miss",
        4 => "Address Error (Load)",
        5 => "Address Error (Store)",
        6 => "Bus Error (Insn)",
        7 => "Bus Error (Data)",
        8 => "Syscall",
        9 => "Breakpoint",
        10 => "Reserved Instruction",
        11 => "Coprocessor Unusable",
        12 => "Overflow",
        _ => "Unknown",
    };
    psp::dprintln!("OASIS_OS EXCEPTION: {} (code {})", name, exception);
    // Spin forever -- the debug output is visible in PPSSPP console
    // and on real hardware via psplink. Returning -1 passes to next handler.
    -1
}
