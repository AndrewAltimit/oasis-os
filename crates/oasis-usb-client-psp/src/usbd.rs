//! sceUsbbd kernel API bindings — resolved at runtime via NID.
//!
//! These functions are kernel-only and not in the rust-psp SDK.
//! We resolve them at runtime using sctrlHENFindFunction (CFW API).

use crate::driver::{UsbDriver, UsbEndpoint, UsbdDeviceReq};

// ---------------------------------------------------------------------------
// NID constants (from PSPSDK pspusbbus.h / USBHostFS source)
// ---------------------------------------------------------------------------

// NIDs verified from PSP-3001 6.61 ARK-4 kernel memory dump
// (NID table at 0x88190324, library "sceUsbBus_driver")
const NID_USBBD_REGISTER: u32 = 0xB1644BE7;    // was 0xB1644BE4 in old PSPSDK
const NID_USBBD_UNREGISTER: u32 = 0xC1E2A540;
const NID_USBBD_REQ_SEND: u32 = 0x913EC15D;
const NID_USBBD_REQ_RECV: u32 = 0x7B87815D;    // was 0x7C765D1A in old PSPSDK
const NID_USBBD_REQ_CANCEL_ALL: u32 = 0xC5E53685;
const NID_USBBD_CLEAR_FIFO: u32 = 0xE65441C1;
#[allow(dead_code)]
const NID_USBBD_STALL: u32 = 0x951A24CC;       // from NID table index 6

// Module/library pairs to search
const USB_MODULES: &[(&[u8], &[u8])] = &[
    (b"sceUSB_Driver\0", b"sceUsbBus_driver\0"),
    (b"sceUsb_Driver\0", b"sceUsbBus_driver\0"),
    (b"sceUSB_Driver\0", b"sceUsb_driver\0"),
];

// ---------------------------------------------------------------------------
// Resolved function pointers (static muts, written once at init)
// ---------------------------------------------------------------------------

type RegisterFn = unsafe extern "C" fn(*mut UsbDriver) -> i32;
type UnregisterFn = unsafe extern "C" fn(*mut UsbDriver) -> i32;
type ReqSendFn = unsafe extern "C" fn(*mut UsbdDeviceReq) -> i32;
type ReqRecvFn = unsafe extern "C" fn(*mut UsbdDeviceReq) -> i32;
type CancelAllFn = unsafe extern "C" fn(*mut UsbEndpoint) -> i32;

static mut REGISTER_FN: Option<RegisterFn> = None;
static mut UNREGISTER_FN: Option<UnregisterFn> = None;
static mut REQ_SEND_FN: Option<ReqSendFn> = None;
static mut REQ_RECV_FN: Option<ReqRecvFn> = None;
#[allow(dead_code)]
static mut CANCEL_ALL_FN: Option<CancelAllFn> = None;

// ---------------------------------------------------------------------------
// NID resolution
// ---------------------------------------------------------------------------

unsafe fn resolve_nid(nid: u32) -> Option<*mut u8> {
    for &(module, library) in USB_MODULES {
        if let Some(ptr) =
            unsafe { psp::hook::find_function(module.as_ptr(), library.as_ptr(), nid) }
        {
            return Some(ptr);
        }
    }
    // Fallback: null module (searches all loaded modules on ARK-4)
    for &(_, library) in USB_MODULES {
        if let Some(ptr) =
            unsafe { psp::hook::find_function(core::ptr::null(), library.as_ptr(), nid) }
        {
            return Some(ptr);
        }
    }
    None
}

/// Try resolving via direct addresses from kernel memory dump (PSP-3001 6.61)
/// Falls back to NID resolution if direct addresses fail sanity check.
unsafe fn try_direct_addresses() -> bool {
    use psp::hw::hw_read32;
    // Only use direct addresses on PSP-3001 (Tachyon >= 0x00500000)
    let tachyon = unsafe { hw_read32(0xBC10_0040) } & 0xFF00_0000;
    if tachyon < 0x0050_0000 {
        return false;
    }

    // Addresses from NID table at 0x88190324 in PSP-3001 6.61 ARK-4 dump
    // NID 0xB1644BE7 (Register)    -> 0x8818F024
    // NID 0xC1E2A540 (Unregister)  -> 0x8818F164
    // NID 0x913EC15D (ReqSend)     -> 0x88189244
    // NID 0x7B87815D (ReqRecv)     -> 0x8818F884
    // NID 0xC5E53685 (CancelAll)   -> 0x88189E94

    // Sanity check: read first instruction at Register address
    // Should be 0x27BDFFE0 (addiu $sp, $sp, -32)
    let first_insn = unsafe { hw_read32(0x8818F024) };
    if first_insn != 0x27BDFFE0 {
        psp::dprintln!("Direct addr check fail: {:08X}", first_insn);
        return false;
    }

    unsafe {
        core::ptr::write_volatile(&raw mut REGISTER_FN, Some(core::mem::transmute(0x8818F024u32)));
        core::ptr::write_volatile(&raw mut UNREGISTER_FN, Some(core::mem::transmute(0x8818F164u32)));
        core::ptr::write_volatile(&raw mut REQ_SEND_FN, Some(core::mem::transmute(0x88189244u32)));
        core::ptr::write_volatile(&raw mut REQ_RECV_FN, Some(core::mem::transmute(0x8818F884u32)));
        core::ptr::write_volatile(&raw mut CANCEL_ALL_FN, Some(core::mem::transmute(0x88189E94u32)));
    }
    psp::dprintln!("Using direct addresses");
    true
}

/// Resolve all sceUsbbd NIDs. Returns true if all critical ones resolved.
pub unsafe fn resolve_all() -> bool {
    // Try direct addresses first (faster, avoids NID resolution issues)
    if unsafe { try_direct_addresses() } {
        return true;
    }
    psp::dprintln!("Direct addrs failed, trying NID...");

    unsafe {
        let nids: &[(u32, &str)] = &[
            (NID_USBBD_REGISTER, "Register"),
            (NID_USBBD_UNREGISTER, "Unregister"),
            (NID_USBBD_REQ_SEND, "ReqSend"),
            (NID_USBBD_REQ_RECV, "ReqRecv"),
            (NID_USBBD_REQ_CANCEL_ALL, "CancelAll"),
        ];

        let ptrs: [*mut Option<*mut u8>; 5] = [
            (&raw mut REGISTER_FN) as *mut Option<*mut u8>,
            (&raw mut UNREGISTER_FN) as *mut Option<*mut u8>,
            (&raw mut REQ_SEND_FN) as *mut Option<*mut u8>,
            (&raw mut REQ_RECV_FN) as *mut Option<*mut u8>,
            (&raw mut CANCEL_ALL_FN) as *mut Option<*mut u8>,
        ];

        for (i, &(nid, name)) in nids.iter().enumerate() {
            if let Some(p) = resolve_nid(nid) {
                psp::dprintln!("  {} = {:08X}", name, p as u32);
                // SAFETY: transmuting resolved function pointer
                core::ptr::write_volatile(ptrs[i], Some(p));
            } else {
                psp::dprintln!("  {} FAIL ({:08X})", name, nid);
                // Only Register, ReqSend, ReqRecv are critical
                if i < 4 && i != 1 {
                    return false;
                }
            }
        }

        true
    }
}

// ---------------------------------------------------------------------------
// Safe wrappers
// ---------------------------------------------------------------------------

pub unsafe fn register_driver(drv: *mut UsbDriver) -> i32 {
    unsafe {
        if let Some(f) = core::ptr::read_volatile(&raw const REGISTER_FN) {
            f(drv)
        } else {
            -1
        }
    }
}

pub unsafe fn unregister_driver(drv: *mut UsbDriver) -> i32 {
    unsafe {
        if let Some(f) = core::ptr::read_volatile(&raw const UNREGISTER_FN) {
            f(drv)
        } else {
            -1
        }
    }
}

pub unsafe fn req_send(req: *mut UsbdDeviceReq) -> i32 {
    unsafe {
        if let Some(f) = core::ptr::read_volatile(&raw const REQ_SEND_FN) {
            f(req)
        } else {
            -1
        }
    }
}

pub unsafe fn req_recv(req: *mut UsbdDeviceReq) -> i32 {
    unsafe {
        if let Some(f) = core::ptr::read_volatile(&raw const REQ_RECV_FN) {
            f(req)
        } else {
            -1
        }
    }
}
