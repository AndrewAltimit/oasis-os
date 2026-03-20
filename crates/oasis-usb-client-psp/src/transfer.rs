//! Bulk transfer management — send/receive data over USB endpoints.
//!
//! Uses sceUsbbdReqSend (bulk IN, device→host) and sceUsbbdReqRecv
//! (bulk OUT, host→device) with completion callbacks for async I/O.
//!
//! Echo mode is callback-driven: recv_complete queues the echo send,
//! send_complete re-queues the recv. No main-loop polling needed.

use crate::driver::{UsbEndpoint, UsbdDeviceReq};
use crate::usbd;

// ---------------------------------------------------------------------------
// Transfer buffers — 64-byte aligned for DMA coherency
// ---------------------------------------------------------------------------

const RECV_BUF_SIZE: usize = 16384; // 16KB

#[repr(C, align(64))]
struct AlignedRecvBuf([u8; RECV_BUF_SIZE]);

static mut RECV_BUF: AlignedRecvBuf = AlignedRecvBuf([0u8; RECV_BUF_SIZE]);

const SEND_BUF_SIZE: usize = 16384; // match recv size for echo

#[repr(C, align(64))]
struct AlignedSendBuf([u8; SEND_BUF_SIZE]);

static mut SEND_BUF: AlignedSendBuf = AlignedSendBuf([0u8; SEND_BUF_SIZE]);

/// Transfer request structs — also 64-byte aligned
#[repr(C, align(64))]
struct AlignedReq(UsbdDeviceReq);

// SAFETY: Only accessed from USB interrupt context and init thread
unsafe impl Sync for AlignedReq {}
unsafe impl Sync for AlignedRecvBuf {}
unsafe impl Sync for AlignedSendBuf {}

static mut RECV_REQ: AlignedReq = AlignedReq(UsbdDeviceReq {
    endp: core::ptr::null_mut(),
    data: core::ptr::null_mut(),
    size: 0,
    unkc: 0,
    func: None,
    recvsize: 0,
    retcode: 0,
    unk1c: 0,
    arg: core::ptr::null_mut(),
    link: core::ptr::null_mut(),
});

static mut SEND_REQ: AlignedReq = AlignedReq(UsbdDeviceReq {
    endp: core::ptr::null_mut(),
    data: core::ptr::null_mut(),
    size: 0,
    unkc: 0,
    func: None,
    recvsize: 0,
    retcode: 0,
    unk1c: 0,
    arg: core::ptr::null_mut(),
    link: core::ptr::null_mut(),
});

// ---------------------------------------------------------------------------
// State (volatile, accessed from callback + main thread)
// ---------------------------------------------------------------------------

/// Endpoint pointers — set once at init, read from callbacks
static mut EP1_PTR: *mut UsbEndpoint = core::ptr::null_mut();
static mut EP2_PTR: *mut UsbEndpoint = core::ptr::null_mut();

/// Echo mode: when true, recv callback auto-sends echo, send callback
/// auto-re-queues recv. Fully callback-driven, no main loop needed.
static mut ECHO_MODE: bool = false;

/// Counters for main loop to monitor (read-only from main)
static mut ECHO_COUNT: u32 = 0;
static mut LAST_RECV_SIZE: i32 = 0;
static mut LAST_RECV_STATUS: i32 = 0;
static mut LAST_SEND_STATUS: i32 = 0;

/// Flag for main loop to detect new echo completions
static mut ECHO_UPDATED: bool = false;

// ---------------------------------------------------------------------------
// Completion callbacks (USB interrupt context — NO file I/O)
// ---------------------------------------------------------------------------

unsafe extern "C" fn recv_complete(
    req: *mut UsbdDeviceReq,
    _arg1: i32,
    _arg2: i32,
) -> i32 {
    unsafe {
        let size = (*req).recvsize;
        let status = (*req).retcode;
        core::ptr::write_volatile(&raw mut LAST_RECV_SIZE, size);
        core::ptr::write_volatile(&raw mut LAST_RECV_STATUS, status);

        // In echo mode: immediately send the received data back
        if core::ptr::read_volatile(&raw const ECHO_MODE) && size > 0 && status == 0 {
            let len = (size as usize).min(SEND_BUF_SIZE);

            // Copy recv data to send buffer
            let src = (&raw const RECV_BUF.0) as *const u8;
            let dst = (&raw mut SEND_BUF.0) as *mut u8;
            core::ptr::copy_nonoverlapping(src, dst, len);

            // Fill send request
            core::ptr::write_bytes(&raw mut SEND_REQ.0, 0, 1);
            SEND_REQ.0.endp = core::ptr::read_volatile(&raw const EP1_PTR);
            SEND_REQ.0.data = dst;
            SEND_REQ.0.size = len as i32;
            SEND_REQ.0.func = Some(send_complete);

            flush_dcache();
            usbd::req_send(&raw mut SEND_REQ.0);
        }
    }
    0
}

unsafe extern "C" fn send_complete(
    _req: *mut UsbdDeviceReq,
    _arg1: i32,
    _arg2: i32,
) -> i32 {
    unsafe {
        let status = (*_req).retcode;
        core::ptr::write_volatile(&raw mut LAST_SEND_STATUS, status);

        let count = core::ptr::read_volatile(&raw const ECHO_COUNT);
        core::ptr::write_volatile(&raw mut ECHO_COUNT, count + 1);
        core::ptr::write_volatile(&raw mut ECHO_UPDATED, true);

        // In echo mode: immediately re-queue recv for next packet
        if core::ptr::read_volatile(&raw const ECHO_MODE) {
            core::ptr::write_bytes(&raw mut RECV_REQ.0, 0, 1);
            RECV_REQ.0.endp = core::ptr::read_volatile(&raw const EP2_PTR);
            RECV_REQ.0.data = (&raw mut RECV_BUF.0) as *mut u8;
            RECV_REQ.0.size = RECV_BUF_SIZE as i32;
            RECV_REQ.0.func = Some(recv_complete);

            flush_dcache();
            usbd::req_recv(&raw mut RECV_REQ.0);
        }
    }
    0
}

// ---------------------------------------------------------------------------
// Cache helper
// ---------------------------------------------------------------------------

unsafe fn flush_dcache() {
    unsafe extern "C" {
        fn sceKernelDcacheWritebackInvalidateAll();
    }
    unsafe { sceKernelDcacheWritebackInvalidateAll() };
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Initialize endpoint pointers for callback-driven echo.
/// Must be called before any transfers.
pub unsafe fn init_endpoints(ep1: *mut UsbEndpoint, ep2: *mut UsbEndpoint) {
    unsafe {
        core::ptr::write_volatile(&raw mut EP1_PTR, ep1);
        core::ptr::write_volatile(&raw mut EP2_PTR, ep2);
    }
}

/// Enable callback-driven echo mode.
pub fn enable_echo_mode() {
    unsafe { core::ptr::write_volatile(&raw mut ECHO_MODE, true) };
}

/// Start an async receive on EP2 (bulk OUT, host→PSP).
/// In echo mode, the recv callback will auto-send the echo.
pub unsafe fn start_recv(ep2: *mut UsbEndpoint) -> i32 {
    unsafe {
        core::ptr::write_bytes(&raw mut RECV_REQ.0, 0, 1);

        RECV_REQ.0.endp = ep2;
        RECV_REQ.0.data = (&raw mut RECV_BUF.0) as *mut u8;
        RECV_REQ.0.size = RECV_BUF_SIZE as i32;
        RECV_REQ.0.func = Some(recv_complete);

        flush_dcache();
        usbd::req_recv(&raw mut RECV_REQ.0)
    }
}

/// Send data on EP1 (bulk IN, PSP→host).
/// Copies data into the send buffer and queues the transfer.
pub unsafe fn start_send(ep1: *mut UsbEndpoint, data: &[u8]) -> i32 {
    let len = data.len().min(SEND_BUF_SIZE);

    unsafe {
        let dst = (&raw mut SEND_BUF.0) as *mut u8;
        core::ptr::copy_nonoverlapping(data.as_ptr(), dst, len);

        core::ptr::write_bytes(&raw mut SEND_REQ.0, 0, 1);
        SEND_REQ.0.endp = ep1;
        SEND_REQ.0.data = dst;
        SEND_REQ.0.size = len as i32;
        SEND_REQ.0.func = Some(send_complete);

        flush_dcache();
        usbd::req_send(&raw mut SEND_REQ.0)
    }
}

/// Check if there's a new echo completion. Returns (updated, count).
/// Clears the updated flag.
pub fn poll_echo() -> (bool, u32) {
    unsafe {
        let updated = core::ptr::read_volatile(&raw const ECHO_UPDATED);
        let count = core::ptr::read_volatile(&raw const ECHO_COUNT);
        if updated {
            core::ptr::write_volatile(&raw mut ECHO_UPDATED, false);
        }
        (updated, count)
    }
}

/// Get the last recv size and status (for logging).
pub fn last_recv_info() -> (i32, i32) {
    unsafe {
        let size = core::ptr::read_volatile(&raw const LAST_RECV_SIZE);
        let status = core::ptr::read_volatile(&raw const LAST_RECV_STATUS);
        (size, status)
    }
}
