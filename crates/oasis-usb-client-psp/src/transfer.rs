//! Bulk transfer management — send/receive data over USB endpoints.
//!
//! Uses sceUsbbdReqSend (bulk IN, device→host) and sceUsbbdReqRecv
//! (bulk OUT, host→device) with completion callbacks for async I/O.
//!
//! Two modes:
//! - **Echo mode**: recv_complete echoes data back, send_complete re-queues recv.
//! - **Thin-client mode**: recv_complete parses protocol messages (FRAME_CHUNK,
//!   FRAME_DONE, GET_INPUT), writes pixels to VRAM, responds with InputState.

use crate::driver::{UsbEndpoint, UsbdDeviceReq};
use crate::framebuffer;
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

/// Thin-client mode: when true, recv callback parses protocol messages
/// and responds with InputState instead of echoing.
static mut THIN_CLIENT_MODE: bool = false;

/// Counters for main loop to monitor (read-only from main)
static mut ECHO_COUNT: u32 = 0;
static mut LAST_RECV_SIZE: i32 = 0;
static mut LAST_RECV_STATUS: i32 = 0;
static mut LAST_SEND_STATUS: i32 = 0;

/// Flag for main loop to detect new completions
static mut ECHO_UPDATED: bool = false;


// ---------------------------------------------------------------------------
// Thin-client protocol constants (must match host protocol.rs)
// ---------------------------------------------------------------------------

/// Message header size
const MSG_HEADER_SIZE: usize = 4;

/// Host → PSP message types
const CMD_FRAME_CHUNK: u8 = 0x11;
const CMD_FRAME_DONE: u8 = 0x12;
const CMD_GET_INPUT: u8 = 0x20;

/// PSP → Host message types
const RSP_INPUT_STATE: u8 = 0x21;

/// InputState: current controller state, updated by main loop.
/// 8 bytes: [buttons: u32, analog_x: u8, analog_y: u8, battery: u8, pad: u8]
#[repr(C)]
struct InputStateData {
    buttons: u32,
    analog_x: u8,
    analog_y: u8,
    battery: u8,
    _pad: u8,
}

static mut CURRENT_INPUT: InputStateData = InputStateData {
    buttons: 0,
    analog_x: 128,
    analog_y: 128,
    battery: 0,
    _pad: 0,
};

// ---------------------------------------------------------------------------
// Completion callbacks (USB interrupt context — NO file I/O)
// ---------------------------------------------------------------------------

unsafe extern "C" fn recv_complete(
    req: *mut UsbdDeviceReq,
    _arg1: i32,
    _arg2: i32,
) -> i32 {
    // SAFETY: volatile read of a fixed-address register or module-static.
    unsafe {
        let size = (*req).recvsize;
        let status = (*req).retcode;
        core::ptr::write_volatile(&raw mut LAST_RECV_SIZE, size);
        core::ptr::write_volatile(&raw mut LAST_RECV_STATUS, status);

        if size <= 0 || status != 0 {
            return 0;
        }

        // Thin-client mode: parse protocol message, respond with InputState
        if core::ptr::read_volatile(&raw const THIN_CLIENT_MODE) {
            handle_thin_client_recv(size as usize);
            return 0;
        }

        // Echo mode: immediately send the received data back
        if core::ptr::read_volatile(&raw const ECHO_MODE) {
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

/// Handle a received message in thin-client mode.
///
/// Parses the MsgHeader, dispatches by type, and sends an InputState response.
///
/// # Safety
/// Called from USB interrupt context — no syscalls, no file I/O.
unsafe fn handle_thin_client_recv(size: usize) {
    // SAFETY: PSP-specific unsafe op (kernel-mode hardware / syscall access).
    unsafe {
        if size < MSG_HEADER_SIZE {
            // Too short for a header — send InputState anyway to keep chain alive
            send_input_response();
            return;
        }

        let buf = (&raw const RECV_BUF.0) as *const u8;

        // Parse header: [type, flags, len_lo, len_hi]
        let msg_type = core::ptr::read(buf);
        let flags = core::ptr::read(buf.add(1));
        let payload = buf.add(MSG_HEADER_SIZE);
        // Use the header's payload_len (not raw USB transfer size) to
        // exclude any ZLP-avoidance padding byte the host may have added.
        let payload_len = core::ptr::read(buf.add(2)) as usize
            | ((core::ptr::read(buf.add(3)) as usize) << 8);
        let payload_size = payload_len.min(size - MSG_HEADER_SIZE);

        match msg_type {
            CMD_FRAME_CHUNK => {
                // flags = chunk_index (0-17)
                framebuffer::write_chunk(
                    flags,
                    core::slice::from_raw_parts(payload, payload_size),
                );
                send_input_response();
            }
            CMD_FRAME_DONE => {
                // flags = frame_seq (ignored for now)
                framebuffer::swap();
                send_input_response();
            }
            CMD_GET_INPUT => {
                send_input_response();
            }
            _ => {
                // Unknown message type — respond with InputState to keep chain alive
                send_input_response();
            }
        }
    }
}

/// Build and send a 12-byte InputState response: 4-byte header + 8-byte InputState.
///
/// # Safety
/// Called from USB interrupt context.
unsafe fn send_input_response() {
    // SAFETY: volatile read of a fixed-address register or module-static.
    unsafe {
        let dst = (&raw mut SEND_BUF.0) as *mut u8;

        // Header: [RSP_INPUT_STATE, 0, 8, 0] (payload_len = 8)
        core::ptr::write(dst, RSP_INPUT_STATE);
        core::ptr::write(dst.add(1), 0); // flags
        core::ptr::write(dst.add(2), 8); // payload_len low byte
        core::ptr::write(dst.add(3), 0); // payload_len high byte

        // InputState (8 bytes)
        let input = &raw const CURRENT_INPUT;
        let buttons = core::ptr::read_volatile(&(*input).buttons);
        let b = buttons.to_le_bytes();
        core::ptr::write(dst.add(4), b[0]);
        core::ptr::write(dst.add(5), b[1]);
        core::ptr::write(dst.add(6), b[2]);
        core::ptr::write(dst.add(7), b[3]);
        core::ptr::write(dst.add(8), core::ptr::read_volatile(&(*input).analog_x));
        core::ptr::write(dst.add(9), core::ptr::read_volatile(&(*input).analog_y));
        core::ptr::write(dst.add(10), core::ptr::read_volatile(&(*input).battery));
        core::ptr::write(dst.add(11), 0); // pad

        // Queue 12-byte send
        core::ptr::write_bytes(&raw mut SEND_REQ.0, 0, 1);
        SEND_REQ.0.endp = core::ptr::read_volatile(&raw const EP1_PTR);
        SEND_REQ.0.data = dst;
        SEND_REQ.0.size = 12;
        SEND_REQ.0.func = Some(send_complete);

        flush_dcache();
        usbd::req_send(&raw mut SEND_REQ.0);
    }
}

unsafe extern "C" fn send_complete(
    _req: *mut UsbdDeviceReq,
    _arg1: i32,
    _arg2: i32,
) -> i32 {
    // SAFETY: volatile read of a fixed-address register or module-static.
    unsafe {
        let status = (*_req).retcode;
        core::ptr::write_volatile(&raw mut LAST_SEND_STATUS, status);

        let count = core::ptr::read_volatile(&raw const ECHO_COUNT);
        core::ptr::write_volatile(&raw mut ECHO_COUNT, count + 1);
        core::ptr::write_volatile(&raw mut ECHO_UPDATED, true);

        // In echo or thin-client mode: immediately re-queue recv for next packet
        if core::ptr::read_volatile(&raw const ECHO_MODE)
            || core::ptr::read_volatile(&raw const THIN_CLIENT_MODE)
        {
            // Invalidate recv buffer cache before re-queuing (USBHostFS pattern)
            let buf_ptr = (&raw mut RECV_BUF.0) as *mut u8;
            invalidate_dcache_range(buf_ptr, RECV_BUF_SIZE);

            core::ptr::write_bytes(&raw mut RECV_REQ.0, 0, 1);
            RECV_REQ.0.endp = core::ptr::read_volatile(&raw const EP2_PTR);
            RECV_REQ.0.data = buf_ptr;
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
    // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
    unsafe { sceKernelDcacheWritebackInvalidateAll() };
}

/// Invalidate dcache for a recv buffer (like USBHostFS does before recv).
/// This ensures the CPU reads DMA-written data, not stale cache.
unsafe fn invalidate_dcache_range(ptr: *const u8, size: usize) {
    unsafe extern "C" {
        fn sceKernelDcacheInvalidateRange(p: *const u8, size: u32) -> i32;
    }
    // Align to 64-byte boundary
    let addr = ptr as u32;
    let block = addr & !63;
    let top = (addr + size as u32 + 63) & !63;
    // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
    unsafe { sceKernelDcacheInvalidateRange(block as *const u8, top - block) };
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Initialize endpoint pointers for callback-driven transfers.
/// Must be called before any transfers.
pub unsafe fn init_endpoints(ep1: *mut UsbEndpoint, ep2: *mut UsbEndpoint) {
    // SAFETY: volatile write of a module-static; only mutated under exclusive control of this experiment.
    unsafe {
        core::ptr::write_volatile(&raw mut EP1_PTR, ep1);
        core::ptr::write_volatile(&raw mut EP2_PTR, ep2);
    }
}

/// Enable callback-driven echo mode.
pub fn enable_echo_mode() {
    // SAFETY: volatile write of a module-static; only mutated under exclusive control of this experiment.
    unsafe { core::ptr::write_volatile(&raw mut ECHO_MODE, true) };
}

/// Enable thin-client mode (replaces echo mode).
/// recv_complete parses protocol messages and responds with InputState.
pub fn enable_thin_client_mode() {
    // SAFETY: volatile write of a module-static; only mutated under exclusive control of this experiment.
    unsafe {
        core::ptr::write_volatile(&raw mut ECHO_MODE, false);
        core::ptr::write_volatile(&raw mut THIN_CLIENT_MODE, true);
    }
}

/// Update the current input state from the main loop.
/// Called every tick with fresh controller data.
pub fn update_input(buttons: u32, analog_x: u8, analog_y: u8, battery: u8) {
    // SAFETY: volatile write of a module-static; only mutated under exclusive control of this experiment.
    unsafe {
        core::ptr::write_volatile(&raw mut CURRENT_INPUT.buttons, buttons);
        core::ptr::write_volatile(&raw mut CURRENT_INPUT.analog_x, analog_x);
        core::ptr::write_volatile(&raw mut CURRENT_INPUT.analog_y, analog_y);
        core::ptr::write_volatile(&raw mut CURRENT_INPUT.battery, battery);
    }
}

/// Blocking recv: queue a recv, poll until data arrives.
/// Re-queues automatically if cancelled (e.g. by host claim_interface).
/// Used for the initial handshake before enabling callback-driven mode.
/// Returns true if data was received, false on timeout.
pub unsafe fn blocking_recv(ep2: *mut UsbEndpoint) -> bool {
    // SAFETY: PSP firmware syscall — kernel-mode binary; signature is documented in pspsdk.
    unsafe {
        core::ptr::write_volatile(&raw mut HANDSHAKE_DONE, false);
        core::ptr::write_volatile(&raw mut HANDSHAKE_CANCELLED, false);

        queue_blocking_recv(ep2);

        // Poll for completion (up to 30 seconds)
        for _ in 0..300 {
            psp::sys::sceKernelDelayThread(100_000); // 100ms
            if core::ptr::read_volatile(&raw const HANDSHAKE_DONE) {
                return true;
            }
            // Re-queue if cancelled (host claim_interface resets endpoints)
            if core::ptr::read_volatile(&raw const HANDSHAKE_CANCELLED) {
                core::ptr::write_volatile(&raw mut HANDSHAKE_CANCELLED, false);
                queue_blocking_recv(ep2);
            }
        }
        false
    }
}

unsafe fn queue_blocking_recv(ep2: *mut UsbEndpoint) {
    // SAFETY: PSP-specific unsafe op (kernel-mode hardware / syscall access).
    unsafe {
        let buf_ptr = (&raw mut RECV_BUF.0) as *mut u8;
        invalidate_dcache_range(buf_ptr, RECV_BUF_SIZE);

        core::ptr::write_bytes(&raw mut RECV_REQ.0, 0, 1);
        RECV_REQ.0.endp = ep2;
        RECV_REQ.0.data = buf_ptr;
        RECV_REQ.0.size = RECV_BUF_SIZE as i32;
        RECV_REQ.0.func = Some(blocking_recv_complete);

        flush_dcache();
        usbd::req_recv(&raw mut RECV_REQ.0);
    }
}

/// Process the handshake message already in RECV_BUF and send InputState response.
/// Then queue the first callback-driven recv for the thin-client chain.
pub fn process_and_respond() {
    // SAFETY: volatile read of a fixed-address register or module-static.
    unsafe {
        let size = core::ptr::read_volatile(&raw const LAST_RECV_SIZE) as usize;
        if size > 0 {
            handle_thin_client_recv(size);
        }
        // send_complete will queue the next recv (callback chain starts)
    }
}

static mut HANDSHAKE_DONE: bool = false;
static mut HANDSHAKE_CANCELLED: bool = false;

unsafe extern "C" fn blocking_recv_complete(
    req: *mut UsbdDeviceReq,
    _arg1: i32,
    _arg2: i32,
) -> i32 {
    // SAFETY: volatile write of a module-static; only mutated under exclusive control of this experiment.
    unsafe {
        let size = (*req).recvsize;
        let status = (*req).retcode;
        core::ptr::write_volatile(&raw mut LAST_RECV_SIZE, size);
        core::ptr::write_volatile(&raw mut LAST_RECV_STATUS, status);
        if size > 0 && status == 0 {
            core::ptr::write_volatile(&raw mut HANDSHAKE_DONE, true);
        } else {
            // Cancelled (e.g. by host claim_interface) — signal re-queue
            core::ptr::write_volatile(&raw mut HANDSHAKE_CANCELLED, true);
        }
    }
    0
}

/// Start an async receive on EP2 (bulk OUT, host→PSP).
/// Matches USBHostFS pattern: invalidate recv buffer cache, then queue.
pub unsafe fn start_recv(ep2: *mut UsbEndpoint) -> i32 {
    // SAFETY: PSP-specific unsafe op (kernel-mode hardware / syscall access).
    unsafe {
        // Invalidate recv buffer cache (USBHostFS pattern — DMA will write here)
        let buf_ptr = (&raw mut RECV_BUF.0) as *mut u8;
        invalidate_dcache_range(buf_ptr, RECV_BUF_SIZE);

        core::ptr::write_bytes(&raw mut RECV_REQ.0, 0, 1);

        RECV_REQ.0.endp = ep2;
        RECV_REQ.0.data = buf_ptr;
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

    // SAFETY: dst is SEND_BUF (module-static) sized SEND_BUF_SIZE >= len; src is `data`,
    // a caller-supplied shared reference that cannot alias a uniquely-borrowed module-static,
    // so copy_nonoverlapping's non-overlap requirement is satisfied.
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

/// Check if there's a new completion. Returns (updated, count).
/// Clears the updated flag.
pub fn poll_echo() -> (bool, u32) {
    // SAFETY: volatile read of a fixed-address register or module-static.
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
    // SAFETY: volatile read of a fixed-address register or module-static.
    unsafe {
        let size = core::ptr::read_volatile(&raw const LAST_RECV_SIZE);
        let status = core::ptr::read_volatile(&raw const LAST_RECV_STATUS);
        (size, status)
    }
}
