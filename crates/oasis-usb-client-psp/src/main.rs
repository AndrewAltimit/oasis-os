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

use psp::sys::{
    sceKernelDelayThread, sceUsbGetState, sceUsbStart, sceUsbActivate,
    sceUsbDeactivate, sceUsbStop, IoOpenFlags,
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

    // Main loop: show USB state + wait for connection
    loop {
        let state = unsafe { sceUsbGetState() };
        let bits = state.bits();

        if bits & 0x002 != 0 {
            // ESTABLISHED - host has selected our configuration
            dbg_log!("CONNECTED! state={:03X}", bits);

            // TODO: Start transfer threads
            // For now, just loop and show state
            loop {
                unsafe { sceKernelDelayThread(1_000_000) };
                let s = unsafe { sceUsbGetState() }.bits();
                if s & 0x002 == 0 {
                    dbg_log!("Disconnected. state={:03X}", s);
                    break;
                }
            }
        }

        unsafe { sceKernelDelayThread(100_000) };
    }
}
