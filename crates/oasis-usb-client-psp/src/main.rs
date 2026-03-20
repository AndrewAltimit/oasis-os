//! PSP USB Thin Client — kernel-mode PRX
//!
//! Registers a custom USB device driver with bulk IN/OUT endpoints.
//! When connected to a USB host (e.g., Luckfox Pico), the PSP acts as
//! a thin client: receives framebuffer data, sends controller input.
//!
//! Architecture: PSP = USB device (always), Host = USB master (Luckfox/PC).
//!
//! Phase 1: Diagnostics — MMIO state dump, queue validation tracing.
//! Phase 2: Transfer fixes — ClearFIFO, aligned buffers, speed detection.

#![no_std]
#![no_main]

mod usbd;
mod descriptors;
mod driver;
mod transfer;

use psp::sys::{
    sceKernelDelayThread, sceUsbGetState, sceUsbStart, sceUsbActivate,
    sceUsbDeactivate, sceUsbStop, IoOpenFlags,
    SceCtrlData, CtrlButtons, sceCtrlPeekBufferPositive,
};
use core::ffi::c_void;

/// Read a 32-bit value from a memory-mapped hardware address.
/// SAFETY: Only call with valid MMIO or kernel memory addresses.
#[inline(always)]
unsafe fn hw_read32(addr: u32) -> u32 {
    unsafe { core::ptr::read_volatile(addr as *const u32) }
}

psp::module_kernel!("OasisUSBClient", 1, 0);

/// Product ID for our custom USB device (avoid Sony reserved range 0x1C8-0x1CC)
pub const OASIS_USB_PID: u32 = 0x1337;

/// USB storage PID (for comparison dump in Step 1.3)
const USB_STORAGE_PID: u32 = 0x1C8;

static LOG_PATH: &[u8] = b"ms0:/PSP/GAME/USBCLIENT/usb.log\0";

fn log_init() {
    let fd = unsafe {
        psp::sys::sceIoOpen(
            LOG_PATH.as_ptr(),
            IoOpenFlags::WR_ONLY | IoOpenFlags::CREAT | IoOpenFlags::TRUNC,
            0o777,
        )
    };
    if fd.0 >= 0 { unsafe { psp::sys::sceIoClose(fd) }; }
}

pub fn log_str(s: &str) {
    psp::dprintln!("{}", s);
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
            psp::sys::sceIoWrite(fd, b"\n".as_ptr() as *const c_void, 1);
            psp::sys::sceIoClose(fd);
        }
    }
}

pub fn log_hex(label: &str, val: u32) {
    let hex = b"0123456789ABCDEF";
    let mut buf = [0u8; 60];
    let lb = label.as_bytes();
    let n = lb.len().min(50);
    buf[..n].copy_from_slice(&lb[..n]);
    for i in 0..8 {
        buf[n + i] = hex[((val >> (28 - i * 4)) & 0xF) as usize];
    }
    let s = unsafe { core::str::from_utf8_unchecked(&buf[..n + 8]) };
    log_str(s);
}

// ---------------------------------------------------------------------------
// Phase 1.1: MMIO state dump — USB bus driver internal state
// ---------------------------------------------------------------------------

/// Dump the USB bus driver's internal state block at 0x88190600-0x88190700.
/// Key addresses from RE:
///   0x881906E8 = speed param 1 (expect 0x200 for hi-speed, 0x100 for full)
///   0x881906EC = speed param 2 (expect 0x20 for hi-speed, 0x10 for full)
///   0x881906F0 = additional state
fn dump_bus_driver_state(label: &str) {
    log_str(label);

    // Critical speed validation addresses
    let s1 = unsafe { hw_read32(0x881906E8) };
    let s2 = unsafe { hw_read32(0x881906EC) };
    let s3 = unsafe { hw_read32(0x881906F0) };
    log_hex("  [6E8] speed1=", s1);
    log_hex("  [6EC] speed2=", s2);
    log_hex("  [6F0] state3=", s3);

    // Dump full 0x88190600-0x88190700 range (64 words)
    log_str("  --- 0x88190600 block ---");
    let base: u32 = 0x88190600;
    for row in 0..16 {
        let off = row * 16;
        let w0 = unsafe { hw_read32(base + off) };
        let w1 = unsafe { hw_read32(base + off + 4) };
        let w2 = unsafe { hw_read32(base + off + 8) };
        let w3 = unsafe { hw_read32(base + off + 12) };
        // Log as "  +XX: WWWWWWWW WWWWWWWW WWWWWWWW WWWWWWWW"
        log_hex_row(off, w0, w1, w2, w3);
    }
}

/// Log a row of 4 words with offset label.
fn log_hex_row(off: u32, w0: u32, w1: u32, w2: u32, w3: u32) {
    let hex = b"0123456789ABCDEF";
    let mut buf = [b' '; 48]; // "  +XX: WWWWWWWW WWWWWWWW WWWWWWWW WWWWWWWW"
    buf[0] = b' ';
    buf[1] = b' ';
    buf[2] = b'+';
    buf[3] = hex[((off >> 4) & 0xF) as usize];
    buf[4] = hex[(off & 0xF) as usize];
    buf[5] = b':';
    buf[6] = b' ';
    fn write_word(buf: &mut [u8], pos: usize, w: u32) {
        let hex = b"0123456789ABCDEF";
        for i in 0..8 {
            buf[pos + i] = hex[((w >> (28 - i * 4)) & 0xF) as usize];
        }
    }
    write_word(&mut buf, 7, w0);
    buf[15] = b' ';
    write_word(&mut buf, 16, w1);
    buf[24] = b' ';
    write_word(&mut buf, 25, w2);
    buf[33] = b' ';
    write_word(&mut buf, 34, w3);

    let s = unsafe { core::str::from_utf8_unchecked(&buf[..42]) };
    log_str(s);
}

// ---------------------------------------------------------------------------
// Phase 1.3: USB storage comparison — dump regs with known-good driver
// ---------------------------------------------------------------------------

/// Activate USB storage mode, dump the bus driver state, then deactivate.
/// This gives us the known-good values for speed params with a working driver.
fn dump_usb_storage_comparison() {
    log_str("=== USB Storage Comparison ===");

    // Start USB storage driver
    let r = unsafe {
        sceUsbStart(
            b"USBStorDriver\0".as_ptr(),
            0,
            core::ptr::null_mut::<c_void>(),
        )
    };
    log_hex("StorStart=", r as u32);

    // Activate USB storage with standard PID
    let r = unsafe { sceUsbActivate(USB_STORAGE_PID) };
    log_hex("StorActivate=", r as u32);

    // Wait for connection
    log_str("Plug USB cable for storage test...");
    let mut connected = false;
    for i in 0..50 {
        unsafe { sceKernelDelayThread(200_000) };
        let state = unsafe { sceUsbGetState() }.bits();
        if state & 0x8 != 0 {
            // USB_STATE_CONFIGURED
            log_hex("Storage configured at tick=", i);
            connected = true;
            break;
        }
    }

    // Dump state whether connected or not
    dump_bus_driver_state("Storage driver state:");

    // Deactivate and stop
    unsafe { sceUsbDeactivate(USB_STORAGE_PID) };
    unsafe { sceKernelDelayThread(100_000) };
    unsafe {
        sceUsbStop(
            b"USBStorDriver\0".as_ptr(),
            0,
            core::ptr::null_mut::<c_void>(),
        )
    };

    if !connected {
        log_str("(storage never connected, values are pre-connect)");
    }
    log_str("=== End Storage Comparison ===");
}

// ---------------------------------------------------------------------------
// Main entry point
// ---------------------------------------------------------------------------

fn psp_main() {
    let _ = psp::callback::setup_exit_callback();
    log_init();

    log_str("=== OASIS USB Client v2 ===");
    log_str("Phase 1: Diagnostics + Phase 2: Transfer fixes");
    log_str("");

    let mut pad = SceCtrlData::default();

    // Step 1: Load USB modules + start bus driver
    let r1 = unsafe {
        psp::sys::sceUtilityLoadUsbModule(psp::sys::UsbModule::UsbPspCm)
    };
    log_hex("LoadPspCm=", r1 as u32);
    let r2 = unsafe {
        psp::sys::sceUtilityLoadUsbModule(psp::sys::UsbModule::UsbAcc)
    };
    log_hex("LoadAcc=", r2 as u32);

    log_str("Starting USBBusDriver...");
    let r = unsafe {
        sceUsbStart(
            b"USBBusDriver\0".as_ptr(),
            0,
            core::ptr::null_mut::<c_void>(),
        )
    };
    log_hex("USBBusDriver=", r as u32);
    if (r as i32) < 0 {
        log_str("Bus start failed");
        unsafe { sceKernelDelayThread(5_000_000) };
        return;
    }

    // Log critical baseline values (before driver)
    let s1 = unsafe { hw_read32(0x881906E8) };
    let s2 = unsafe { hw_read32(0x881906EC) };
    log_hex("baseline [6E8]=", s1);
    log_hex("baseline [6EC]=", s2);

    // Step 2: Resolve sceUsbbd NIDs
    log_str("Resolving USB NIDs...");
    let resolved = unsafe { usbd::resolve_all() };
    if !resolved {
        log_str("FATAL: USB NID resolve fail");
        unsafe { sceKernelDelayThread(5_000_000) };
        return;
    }
    log_str("NIDs resolved OK");

    // Step 3: Register our custom USB device driver
    log_str("Calling sceUsbbdRegister...");
    unsafe {
        let d = &raw const driver::DRIVER_STATIC;
        log_hex("drv=", d as u32);
        log_hex("name=", (*d).name as u32);
        log_hex("endp=", (*d).endp as u32);
        log_hex("intp=", (*d).intp as u32);
    }

    let r = unsafe { driver::register() };
    log_hex("Register=", r as u32);
    if (r as i32) < 0 {
        log_str("Register failed");
        unsafe { sceKernelDelayThread(5_000_000) };
        return;
    }
    log_str("Register OK!");

    // Step 4: Start our driver
    log_str("Calling sceUsbStart(driver)...");
    let r = unsafe {
        sceUsbStart(
            driver::DRIVER_NAME.as_ptr(),
            0,
            core::ptr::null_mut::<c_void>(),
        )
    };
    log_hex("DriverStart=", r as u32);

    // Step 5: Activate with our custom PID
    log_str("Calling sceUsbActivate...");
    let r = unsafe { sceUsbActivate(OASIS_USB_PID) };
    log_hex("Activate=", r as u32);

    // Brief delay to let USB hardware settle before any more I/O
    unsafe { sceKernelDelayThread(500_000) };

    // Log critical speed values (no full dump — callbacks may fire)
    let s1 = unsafe { hw_read32(0x881906E8) };
    let s2 = unsafe { hw_read32(0x881906EC) };
    log_hex("post-activate [6E8]=", s1);
    log_hex("post-activate [6EC]=", s2);

    log_str("");
    log_str("USB active. Connect cable!");
    log_str("");

    // Initialize endpoint pointers for callback-driven echo
    unsafe {
        let ep1 = driver::get_endpoint(1);
        let ep2 = driver::get_endpoint(2);
        transfer::init_endpoints(ep1, ep2);
    }

    // Monitor loop — echo is callback-driven, main loop just monitors
    let mut last_state: i32 = 0;
    let mut last_attached = false;
    let mut tick: u32 = 0;
    let mut attach_done = false;
    let mut last_echo_count: u32 = 0;

    loop {
        let state = unsafe { sceUsbGetState() }.bits();
        let attached = driver::is_attached();

        // Log state changes
        if state != last_state {
            log_hex("state=", state as u32);
            last_state = state;
        }

        // Handle attach/detach transitions
        if attached != last_attached {
            if attached {
                log_str(">> ATTACHED");
                attach_done = false;
                last_echo_count = 0;
            } else {
                log_str(">> DETACHED");
            }
            last_attached = attached;
        }

        // On first attach: send proactive message, then start echo chain
        if attached && !attach_done {
            unsafe { sceKernelDelayThread(200_000) };

            let speed = driver::attach_speed();
            log_hex("speed=", speed as u32);

            // Step 1: Proactive send "PSP READY" (NOT in echo mode yet)
            let ep1 = unsafe { driver::get_endpoint(1) };
            let mut buf = [0u8; 512];
            buf[..9].copy_from_slice(b"PSP READY");
            let r = unsafe { transfer::start_send(ep1, &buf) };
            log_hex("proactive send=", r as u32);

            // Step 2: Wait for proactive send to complete (host reads it)
            // Don't enable echo mode until this send is done
            // (only one SEND_REQ at a time)
            log_str("Waiting for host to read PSP READY...");
            for _ in 0..100 {
                // 10 seconds max
                let (updated, _) = transfer::poll_echo();
                if updated {
                    break;
                }
                unsafe { sceKernelDelayThread(100_000) };
            }

            // Step 3: Enable echo mode and queue first recv
            transfer::enable_echo_mode();
            let ep2 = unsafe { driver::get_endpoint(2) };
            let r = unsafe { transfer::start_recv(ep2) };
            log_hex("echo recv=", r as u32);
            log_str("Echo mode ON");

            attach_done = true;
        }

        // Log echo completions (driven by callbacks, we just monitor)
        let (updated, count) = transfer::poll_echo();
        if updated && count != last_echo_count {
            let (size, status) = transfer::last_recv_info();
            log_hex("echo #", count);
            log_hex("  size=", size as u32);
            last_echo_count = count;
        }

        tick += 1;

        // Heartbeat every 10 seconds
        if tick % 100 == 0 {
            log_hex("tick=", tick);
            log_hex("  echoes=", last_echo_count);
        }

        unsafe { sceKernelDelayThread(10_000) }; // 10ms poll

        // Home to exit
        unsafe { sceCtrlPeekBufferPositive(&mut pad, 1) };
        if pad.buttons.intersects(CtrlButtons::HOME) {
            log_str("Exiting...");
            break;
        }
    }

    // Cleanup
    log_str("Cleaning up...");
    unsafe {
        sceUsbDeactivate(OASIS_USB_PID);
        sceKernelDelayThread(100_000);
        sceUsbStop(
            driver::DRIVER_NAME.as_ptr(),
            0,
            core::ptr::null_mut::<c_void>(),
        );
        sceKernelDelayThread(100_000);
        sceUsbStop(
            b"USBBusDriver\0".as_ptr(),
            0,
            core::ptr::null_mut::<c_void>(),
        );
    }
    log_str("Done.");
}
