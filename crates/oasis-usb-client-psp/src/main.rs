//! PSP USB Thin Client — kernel-mode PRX
//!
//! Registers a custom USB device driver with bulk IN/OUT endpoints.
//! When connected to a USB host (e.g., Luckfox Pico), the PSP acts as
//! a thin client: receives framebuffer data, sends controller input.
//!
//! Architecture: PSP = USB device (always), Host = USB master (Luckfox/PC).

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

psp::module_kernel!("OasisUSBClient", 1, 0);

/// Product ID for our custom USB device (avoid Sony reserved range 0x1C8-0x1CC)
pub const OASIS_USB_PID: u32 = 0x1337;

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

fn log_str(s: &str) {
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

fn log_hex(label: &str, val: u32) {
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

macro_rules! dbg_log {
    ($($arg:tt)*) => {
        psp::dprintln!($($arg)*);
    };
}

fn psp_main() {
    let _ = psp::callback::setup_exit_callback();
    log_init();

    log_str("=== OASIS USB Client ===");
    log_str("Initializing...");

    // Step 1: Load USB modules + start bus driver FIRST
    let r1 = unsafe { psp::sys::sceUtilityLoadUsbModule(psp::sys::UsbModule::UsbPspCm) };
    log_hex("LoadPspCm=", r1 as u32);
    let r2 = unsafe { psp::sys::sceUtilityLoadUsbModule(psp::sys::UsbModule::UsbAcc) };
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
    // Log the driver struct address and key fields
    unsafe {
        let d = &raw const driver::DRIVER_STATIC;
        log_hex("drv=", d as u32);
        log_hex("name=", (*d).name as u32);
        log_hex("endp=", (*d).endp as u32);
        log_hex("intp=", (*d).intp as u32);
        log_hex("str=", (*d).str_desc as u32);
        log_hex("recvctl=", (*d).recvctl.map(|f| f as *const () as u32).unwrap_or(0));
        log_hex("attach=", (*d).attach.map(|f| f as *const () as u32).unwrap_or(0));
        log_hex("start=", (*d).start_func.map(|f| f as *const () as u32).unwrap_or(0));
        log_hex("stop=", (*d).stop_func.map(|f| f as *const () as u32).unwrap_or(0));
    }

    let r = unsafe { driver::register() };
    log_hex("Register=", r as u32);
    if (r as i32) < 0 {
        log_str("Register failed (not crash)");
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

    log_str("");
    log_str("USB device active.");
    log_str("Connect to host!");

    // Main loop: wait for connection, then run echo protocol
    loop {
        let state = unsafe { sceUsbGetState() };
        let bits = state.bits();

        if bits & 0x002 != 0 {
            // ESTABLISHED - host has selected our configuration
            log_str("CONNECTED!");
            log_hex("state=", bits as u32);

            // Run the echo loop
            echo_loop();

            log_str("Disconnected, waiting...");
        }

        unsafe { sceKernelDelayThread(100_000) };
    }
}

/// Echo loop: receive data from host, echo it back + send controller state.
/// This is the simplest bidirectional test — proves bulk transfers work.
fn echo_loop() {
    // EP1 = bulk IN (index 1), EP2 = bulk OUT (index 2)
    let ep1 = unsafe { driver::get_endpoint(1) };
    let ep2 = unsafe { driver::get_endpoint(2) };

    log_str("Waiting 3s for host...");
    unsafe { sceKernelDelayThread(3_000_000) };

    if unsafe { sceUsbGetState() }.bits() & 0x002 == 0 {
        log_str("Lost connection");
        return;
    }

    // Continuously send "PING" every second so host can read
    // This tests the IN (PSP→host) direction
    log_str("Sending PINGs on EP1...");
    let mut ping_count: u32 = 0;

    loop {
        // Check USB still connected
        if unsafe { sceUsbGetState() }.bits() & 0x002 == 0 {
            return;
        }

        // Build ping message
        let mut msg = [0u8; 32];
        msg[..5].copy_from_slice(b"PING ");
        // Add counter as ASCII digits
        let hex = b"0123456789ABCDEF";
        for i in 0..8 {
            msg[5 + i] = hex[((ping_count >> (28 - i * 4)) & 0xF) as usize];
        }
        msg[13] = b'\n';

        // Send on EP1
        let r = unsafe { transfer::start_send(ep1, &msg[..14]) };
        if r == 0 {
            // Wait for completion (5 second timeout)
            let mut completed = false;
            for _ in 0..50000 {
                let (done, status) = transfer::send_poll();
                if done {
                    if ping_count < 3 {
                        log_hex("ping sent status=", status as u32);
                    }
                    completed = true;
                    break;
                }
                unsafe { sceKernelDelayThread(100) };
            }
            if !completed && ping_count < 3 {
                log_str("ping send timeout");
            }
            ping_count += 1;
        } else if ping_count < 3 {
            log_hex("ping start fail=", r as u32);
        }

        // Also try to receive on EP2 (non-blocking check)
        if ping_count == 1 {
            log_str("Also starting recv...");
            let r = unsafe { transfer::start_recv(ep2) };
            log_hex("recv start=", r as u32);
        }
        let (rdone, rsize, rstatus) = transfer::recv_poll();
        if rdone && rsize > 0 && rstatus == 0 {
            log_hex("GOT DATA! bytes=", rsize as u32);
        }

        unsafe { sceKernelDelayThread(1_000_000) }; // 1 second between pings

        let mut pad = SceCtrlData::default();
        unsafe { sceCtrlPeekBufferPositive(&mut pad, 1) };
        if pad.buttons.intersects(CtrlButtons::HOME) {
            log_str("Home pressed, exiting");
            return;
        }
    }
}
