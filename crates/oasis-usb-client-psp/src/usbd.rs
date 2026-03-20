//! sceUsbbd kernel API bindings — resolved at runtime via NID.
//!
//! These functions are kernel-only and not in the rust-psp SDK.
//! We resolve them at runtime using sctrlHENFindFunction (CFW API).

use crate::driver::{UsbDriver, UsbEndpoint, UsbdDeviceReq};

// ---------------------------------------------------------------------------
// NID constants (from PSPSDK pspusbbus.h / USBHostFS source)
// ---------------------------------------------------------------------------

// NIDs from PSPSDK pspusbbus.h (sceUsbBus_driver library)
// Verified against kernel NID table at 0x88190324 in PSP-3001 6.61 dump
const NID_USBBD_REGISTER: u32 = 0xB1644BE7;
const NID_USBBD_UNREGISTER: u32 = 0xC1E2A540;
const NID_USBBD_REQ_SEND: u32 = 0x23E51D8F;     // 0x88189128
const NID_USBBD_REQ_RECV: u32 = 0x913EC15D;     // 0x88189244
const NID_USBBD_REQ_CANCEL: u32 = 0xCC57EC9D;   // cancel single request
const NID_USBBD_REQ_CANCEL_ALL: u32 = 0xC5E53685;
const NID_USBBD_CLEAR_FIFO: u32 = 0x951A24CC;   // was swapped with Stall
#[allow(dead_code)]
const NID_USBBD_STALL: u32 = 0xE65441C1;        // was swapped with ClearFIFO

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
type ClearFifoFn = unsafe extern "C" fn(*mut UsbEndpoint) -> i32;
type StallFn = unsafe extern "C" fn(*mut UsbEndpoint) -> i32;

static mut REGISTER_FN: Option<RegisterFn> = None;
static mut UNREGISTER_FN: Option<UnregisterFn> = None;
static mut REQ_SEND_FN: Option<ReqSendFn> = None;
static mut REQ_RECV_FN: Option<ReqRecvFn> = None;
static mut CANCEL_ALL_FN: Option<CancelAllFn> = None;
static mut CLEAR_FIFO_FN: Option<ClearFifoFn> = None;
static mut STALL_FN: Option<StallFn> = None;

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
    // Only use direct addresses on PSP-3001 (Tachyon >= 0x00500000)
    let tachyon = unsafe {
        core::ptr::read_volatile(0xBC10_0040u32 as *const u32)
    } & 0xFF00_0000;
    if tachyon < 0x0050_0000 {
        return false;
    }

    // Addresses from NID table at 0x88190324 in PSP-3001 6.61 ARK-4 dump
    // NID 0xB1644BE7 (Register)    -> 0x8818F024
    // NID 0xC1E2A540 (Unregister)  -> 0x8818F164
    // NID 0x913EC15D (ReqSend)     -> 0x88189244
    // NID 0x7B87815D (ReqRecv)     -> 0x88189128
    // NID 0xC5E53685 (CancelAll)   -> 0x88189E94
    //
    // NOTE: Trying SWAPPED Send/Recv — retcode values (1,2) were swapped
    // relative to endpoint numbers, suggesting the NID mapping may be wrong.

    // Sanity check: read first instruction at Register address
    // Should be 0x27BDFFE0 (addiu $sp, $sp, -32)
    let first_insn = unsafe {
        core::ptr::read_volatile(0x8818F024u32 as *const u32)
    };
    if first_insn != 0x27BDFFE0 {
        psp::dprintln!("Direct addr check fail: {:08X}", first_insn);
        return false;
    }

    unsafe {
        core::ptr::write_volatile(&raw mut REGISTER_FN, Some(core::mem::transmute(0x8818F024u32)));
        core::ptr::write_volatile(&raw mut UNREGISTER_FN, Some(core::mem::transmute(0x8818F164u32)));
        // Confirmed by PSPSDK NIDs + Ghidra disassembly:
        // 0x88189128 = ReqSend (NID 0x23E51D8F): stores direction=1 (OUT), calls TX finalize
        // 0x88189244 = ReqRecv (NID 0x913EC15D): stores direction=2 (IN), calls RX finalize
        // Achieved 105 FPS / 28 MB/s with this mapping.
        core::ptr::write_volatile(&raw mut REQ_SEND_FN, Some(core::mem::transmute(0x88189128u32)));
        core::ptr::write_volatile(&raw mut REQ_RECV_FN, Some(core::mem::transmute(0x88189244u32)));
        core::ptr::write_volatile(&raw mut CANCEL_ALL_FN, Some(core::mem::transmute(0x88189E94u32)));
    }

    // Resolve ClearFIFO and Stall via NID (not in direct address table)
    if let Some(p) = unsafe { resolve_nid(NID_USBBD_CLEAR_FIFO) } {
        unsafe {
            core::ptr::write_volatile(
                &raw mut CLEAR_FIFO_FN,
                Some(core::mem::transmute(p as u32)),
            );
        }
        psp::dprintln!("  ClearFIFO = {:08X}", p as u32);
    }
    if let Some(p) = unsafe { resolve_nid(NID_USBBD_STALL) } {
        unsafe {
            core::ptr::write_volatile(
                &raw mut STALL_FN,
                Some(core::mem::transmute(p as u32)),
            );
        }
        psp::dprintln!("  Stall = {:08X}", p as u32);
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
    crate::log_str("Direct addrs failed, trying NID...");

    unsafe {
        let nids: &[(u32, &str)] = &[
            (NID_USBBD_REGISTER, "Register"),
            (NID_USBBD_UNREGISTER, "Unregister"),
            (NID_USBBD_REQ_SEND, "ReqSend"),
            (NID_USBBD_REQ_RECV, "ReqRecv"),
            (NID_USBBD_REQ_CANCEL_ALL, "CancelAll"),
            (NID_USBBD_CLEAR_FIFO, "ClearFIFO"),
            (NID_USBBD_STALL, "Stall"),
        ];

        let ptrs: [*mut Option<*mut u8>; 7] = [
            (&raw mut REGISTER_FN) as *mut Option<*mut u8>,
            (&raw mut UNREGISTER_FN) as *mut Option<*mut u8>,
            (&raw mut REQ_SEND_FN) as *mut Option<*mut u8>,
            (&raw mut REQ_RECV_FN) as *mut Option<*mut u8>,
            (&raw mut CANCEL_ALL_FN) as *mut Option<*mut u8>,
            (&raw mut CLEAR_FIFO_FN) as *mut Option<*mut u8>,
            (&raw mut STALL_FN) as *mut Option<*mut u8>,
        ];

        for (i, &(nid, name)) in nids.iter().enumerate() {
            if let Some(p) = resolve_nid(nid) {
                crate::log_hex("  NID addr=", p as u32);
                // SAFETY: transmuting resolved function pointer
                core::ptr::write_volatile(ptrs[i], Some(p));
            } else {
                crate::log_str("  NID FAIL");
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

pub unsafe fn cancel_all(endp: *mut UsbEndpoint) -> i32 {
    unsafe {
        if let Some(f) = core::ptr::read_volatile(&raw const CANCEL_ALL_FN) {
            f(endp)
        } else {
            -1
        }
    }
}

pub unsafe fn clear_fifo(endp: *mut UsbEndpoint) -> i32 {
    unsafe {
        if let Some(f) = core::ptr::read_volatile(&raw const CLEAR_FIFO_FN) {
            f(endp)
        } else {
            -1
        }
    }
}

/// Dump endpoint descriptor state for recv debugging.
/// The recv finalize at 0x8818B128 checks descriptor flags that can
/// prevent RX hardware from being enabled even when req_recv returns 0.
pub fn dump_ep_descriptor_state(ep_index: u32) {
    unsafe {
        // Descriptor table at 0x88190850, 36 bytes per entry
        let desc_base = (0x88190850u32 + ep_index * 36) as *const u8;
        crate::log_hex("ep_desc_base=", desc_base as u32);

        // Flags byte at offset 0 of descriptor
        let flags = core::ptr::read_volatile(desc_base as *const u32);
        crate::log_hex("  desc_flags=", flags);

        // field4 at offset 4 (checked by recv finalize 0x8818B730)
        let field4 = core::ptr::read_volatile(desc_base.add(4) as *const u32);
        crate::log_hex("  desc_field4=", field4);

        // Counter halfwords at desc+0x21E and desc+0x220
        // Both must be zero for recv hardware enable
        // These are relative to the descriptor BASE structure, not the 36-byte entry
        // The desc entries are small but reference a larger structure
        // Let's dump more of the descriptor area
        for i in 0..9 {
            let val = core::ptr::read_volatile(desc_base.add(i * 4) as *const u32);
            crate::log_hex("  desc+", (i * 4) as u32);
            crate::log_hex("    =", val);
        }

        // Also dump the USB driver state at 0x88190600
        let state_base = 0x88190600u32 as *const u8;
        let driver_state = core::ptr::read_volatile(state_base.add(0xE8) as *const u32);
        let sub_state = core::ptr::read_volatile(state_base.add(0xEC) as *const u32);
        let phase = core::ptr::read_volatile(state_base.add(0xF0) as *const u32);
        let blocked = core::ptr::read_volatile(state_base.add(0xE4) as *const u32);
        crate::log_hex("  drv_state[E8]=", driver_state);
        crate::log_hex("  drv_sub[EC]=", sub_state);
        crate::log_hex("  drv_phase[F0]=", phase);
        crate::log_hex("  drv_blocked[E4]=", blocked);
    }
}

pub unsafe fn stall(endp: *mut UsbEndpoint) -> i32 {
    unsafe {
        if let Some(f) = core::ptr::read_volatile(&raw const STALL_FN) {
            f(endp)
        } else {
            -1
        }
    }
}
