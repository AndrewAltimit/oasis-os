//! Bulk transfer management — send/receive data over USB endpoints.
//!
//! Uses sceUsbbdReqSend (bulk IN, device→host) and sceUsbbdReqRecv
//! (bulk OUT, host→device) with completion callbacks and event flags
//! for synchronization.

use crate::driver::{UsbEndpoint, UsbdDeviceReq};
use crate::usbd;
use core::ffi::c_void;

// ---------------------------------------------------------------------------
// Transfer buffers (static, in kernel memory)
// ---------------------------------------------------------------------------

/// Receive buffer — host→PSP (bulk OUT, EP2)
const RECV_BUF_SIZE: usize = 16384; // 16KB
static mut RECV_BUF: [u8; RECV_BUF_SIZE] = [0u8; RECV_BUF_SIZE];

/// Send buffer — PSP→host (bulk IN, EP1)
const SEND_BUF_SIZE: usize = 512; // one USB packet for now
static mut SEND_BUF: [u8; SEND_BUF_SIZE] = [0u8; SEND_BUF_SIZE];

/// Transfer request structs
static mut RECV_REQ: UsbdDeviceReq = UsbdDeviceReq {
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
};

static mut SEND_REQ: UsbdDeviceReq = UsbdDeviceReq {
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
};

// ---------------------------------------------------------------------------
// State flags (volatile, accessed from callback + main thread)
// ---------------------------------------------------------------------------

static mut RECV_DONE: bool = false;
static mut RECV_SIZE: i32 = 0;
static mut RECV_STATUS: i32 = 0;

static mut SEND_DONE: bool = false;
static mut SEND_STATUS: i32 = 0;

// ---------------------------------------------------------------------------
// Completion callbacks (called from USB interrupt context)
// ---------------------------------------------------------------------------

unsafe extern "C" fn recv_complete(req: *mut UsbdDeviceReq, _arg1: i32, _arg2: i32) -> i32 {
    unsafe {
        let r = &*req;
        core::ptr::write_volatile(&raw mut RECV_SIZE, r.recvsize);
        core::ptr::write_volatile(&raw mut RECV_STATUS, r.retcode);
        core::ptr::write_volatile(&raw mut RECV_DONE, true);
    }
    0
}

unsafe extern "C" fn send_complete(req: *mut UsbdDeviceReq, _arg1: i32, _arg2: i32) -> i32 {
    unsafe {
        let r = &*req;
        core::ptr::write_volatile(&raw mut SEND_STATUS, r.retcode);
        core::ptr::write_volatile(&raw mut SEND_DONE, true);
    }
    0
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Flush data cache for a memory range (ensures DMA coherency).
unsafe fn flush_dcache(addr: *const u8, len: usize) {
    // sceKernelDcacheWritebackInvalidateRange
    unsafe extern "C" {
        fn sceKernelDcacheWritebackInvalidateAll();
    }
    unsafe { sceKernelDcacheWritebackInvalidateAll() };
}

/// Start an async receive on EP2 (bulk OUT, host→PSP).
/// Returns 0 on success, negative on error.
pub unsafe fn start_recv(ep2: *mut UsbEndpoint) -> i32 {
    unsafe {
        core::ptr::write_volatile(&raw mut RECV_DONE, false);
        core::ptr::write_volatile(&raw mut RECV_SIZE, 0);
        core::ptr::write_volatile(&raw mut RECV_STATUS, 0);

        // Zero and fill the request struct (cached memory — driver handles DMA)
        core::ptr::write_bytes(&raw mut RECV_REQ, 0, 1);

        RECV_REQ.endp = ep2;
        RECV_REQ.data = (&raw mut RECV_BUF) as *mut u8;
        RECV_REQ.size = RECV_BUF_SIZE as i32;
        RECV_REQ.func = Some(recv_complete);

        // Flush cache before submitting to USB driver
        flush_dcache(&raw const RECV_REQ as *const u8, core::mem::size_of::<UsbdDeviceReq>());
        flush_dcache((&raw const RECV_BUF) as *const u8, RECV_BUF_SIZE);

        usbd::req_recv(&raw mut RECV_REQ)
    }
}

/// Check if receive completed. Returns (done, bytes_received, status).
pub fn recv_poll() -> (bool, i32, i32) {
    unsafe {
        let done = core::ptr::read_volatile(&raw const RECV_DONE);
        let size = core::ptr::read_volatile(&raw const RECV_SIZE);
        let status = core::ptr::read_volatile(&raw const RECV_STATUS);
        (done, size, status)
    }
}

/// Get a slice of the received data (valid after recv_poll returns done=true).
pub fn recv_data(len: usize) -> &'static [u8] {
    let n = len.min(RECV_BUF_SIZE);
    // SAFETY: RECV_BUF is only written by the USB controller before RECV_DONE is set,
    // and we only read after RECV_DONE is true.
    unsafe { &RECV_BUF[..n] }
}

/// Send data on EP1 (bulk IN, PSP→host).
/// Copies data into the send buffer and queues the transfer.
/// Returns 0 on success, negative on error.
pub unsafe fn start_send(ep1: *mut UsbEndpoint, data: &[u8]) -> i32 {
    let len = data.len().min(SEND_BUF_SIZE);

    unsafe {
        core::ptr::write_volatile(&raw mut SEND_DONE, false);
        core::ptr::write_volatile(&raw mut SEND_STATUS, 0);

        // Copy data to send buffer (cached — flush before submit)
        let dst = (&raw mut SEND_BUF) as *mut u8;
        core::ptr::copy_nonoverlapping(data.as_ptr(), dst, len);

        // Fill the request struct
        core::ptr::write_bytes(&raw mut SEND_REQ, 0, 1);

        SEND_REQ.endp = ep1;
        SEND_REQ.data = dst;
        SEND_REQ.size = len as i32;
        SEND_REQ.func = Some(send_complete);

        // Flush cache before submitting to USB driver
        flush_dcache(&raw const SEND_REQ as *const u8, core::mem::size_of::<UsbdDeviceReq>());
        flush_dcache((&raw const SEND_BUF) as *const u8, len);

        usbd::req_send(&raw mut SEND_REQ)
    }
}

/// Check if send completed. Returns (done, status).
pub fn send_poll() -> (bool, i32) {
    unsafe {
        let done = core::ptr::read_volatile(&raw const SEND_DONE);
        let status = core::ptr::read_volatile(&raw const SEND_STATUS);
        (done, status)
    }
}
