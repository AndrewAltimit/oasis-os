//! USB device driver registration — UsbDriver struct + callbacks.
//!
//! Matches USBHostFS layout from psplinkusb/usbhostfs/main.c exactly.

use crate::descriptors::{self, StringDescriptor, USB_DATA};
use crate::usbd;

pub const DRIVER_NAME: &[u8] = b"OasisUSBClient\0";

// ---------------------------------------------------------------------------
// USB structures matching PSP SDK layout
// ---------------------------------------------------------------------------

#[repr(C)]
pub struct UsbEndpoint {
    pub endpnum: i32,
    pub unk2: i32,
    pub unk3: i32,
}

#[repr(C)]
pub struct UsbInterface {
    pub expect_interface: i32,
    pub unk8: i32,
    pub num_interface: i32,
}

#[repr(C)]
pub struct UsbdDeviceReq {
    pub endp: *mut UsbEndpoint,
    pub data: *mut u8,
    pub size: i32,
    pub unkc: i32,
    pub func: Option<unsafe extern "C" fn(*mut UsbdDeviceReq, i32, i32) -> i32>,
    pub recvsize: i32,
    pub retcode: i32,
    pub unk1c: i32,
    pub arg: *mut u8,
    pub link: *mut UsbdDeviceReq,
}

#[repr(C)]
pub struct UsbDriver {
    pub name: *const u8,
    pub endpoints: i32,
    pub endp: *mut UsbEndpoint,
    pub intp: *mut UsbInterface,
    pub devp_hi: *mut u8,
    pub confp_hi: *mut u8,
    pub devp: *mut u8,
    pub confp: *mut u8,
    pub str_desc: *mut StringDescriptor,
    pub recvctl: Option<unsafe extern "C" fn(i32, i32, *mut u8) -> i32>,
    pub func28: Option<unsafe extern "C" fn(i32, i32, i32) -> i32>,
    pub attach: Option<unsafe extern "C" fn(i32, *mut u8, *mut u8) -> i32>,
    pub detach: Option<unsafe extern "C" fn(i32, i32, i32) -> i32>,
    pub unk34: i32,
    pub start_func: Option<unsafe extern "C" fn(i32, *mut u8) -> i32>,
    pub stop_func: Option<unsafe extern "C" fn(i32, *mut u8) -> i32>,
    pub link: *mut UsbDriver,
}

unsafe impl Sync for UsbDriver {}
unsafe impl Send for UsbDriver {}
unsafe impl Sync for UsbEndpoint {}
unsafe impl Send for UsbEndpoint {}
unsafe impl Sync for UsbInterface {}
unsafe impl Send for UsbInterface {}
unsafe impl Sync for UsbdDeviceReq {}
unsafe impl Send for UsbdDeviceReq {}

// ---------------------------------------------------------------------------
// Static driver state (matches USBHostFS exactly)
// ---------------------------------------------------------------------------

/// 4 endpoints: EP0 (control) + EP1 (bulk IN) + EP2 (bulk OUT) + EP3 (spare)
/// Must match USBHostFS layout — kernel expects endpoint count to match array
static mut ENDPOINTS: [UsbEndpoint; 4] = [
    UsbEndpoint { endpnum: 0, unk2: 0, unk3: 0 },
    UsbEndpoint { endpnum: 1, unk2: 0, unk3: 0 },
    UsbEndpoint { endpnum: 2, unk2: 0, unk3: 0 },
    UsbEndpoint { endpnum: 3, unk2: 0, unk3: 0 },
];

static mut INTERFACE: UsbInterface = UsbInterface {
    expect_interface: -1i32,  // 0xFFFFFFFF
    unk8: 0,
    num_interface: 1,
};

/// String descriptor: "<>" in UTF-16LE (matches USBHostFS)
static mut STRING_DESC: [u8; 8] = [0x08, 0x03, b'<', 0, b'>', 0, 0, 0];

pub static mut DRIVER_STATIC: UsbDriver = UsbDriver {
    name: DRIVER_NAME.as_ptr(),
    endpoints: 4,
    endp: unsafe { &raw mut ENDPOINTS[0] },
    intp: &raw mut INTERFACE,
    devp_hi: core::ptr::null_mut(),  // filled by start_func
    confp_hi: core::ptr::null_mut(),
    devp: core::ptr::null_mut(),
    confp: core::ptr::null_mut(),
    str_desc: (&raw mut STRING_DESC) as *mut StringDescriptor,
    recvctl: Some(usb_recvctl),
    func28: Some(usb_func28),
    attach: Some(usb_attach),
    detach: Some(usb_detach),
    unk34: 0,
    start_func: Some(usb_start),
    stop_func: Some(usb_stop),
    link: core::ptr::null_mut(),
};

static mut ATTACHED: bool = false;

// ---------------------------------------------------------------------------
// Driver callbacks
// ---------------------------------------------------------------------------

unsafe extern "C" fn usb_recvctl(arg1: i32, _arg2: i32, req: *mut u8) -> i32 {
    if !req.is_null() {
        let bm = unsafe { *req };
        let breq = unsafe { *req.add(1) };
        psp::dprintln!("[CB] ctl bm={:02X} req={:02X} a1={}", bm, breq, arg1);
    }
    -1 // let bus driver handle
}

unsafe extern "C" fn usb_func28(arg1: i32, _arg2: i32, _arg3: i32) -> i32 {
    // NOTE: interrupt context — NO file I/O (log_str/log_hex), only dprintln
    psp::dprintln!("[CB] func28 a1={:08X}", arg1);
    0
}

/// Stores the speed reported by the attach callback.
/// 2 = hi-speed (480 Mbps), 1 = full-speed (12 Mbps).
static mut ATTACH_SPEED: i32 = 0;

unsafe extern "C" fn usb_attach(speed: i32, _arg2: *mut u8, _arg3: *mut u8) -> i32 {
    // NOTE: interrupt context — NO file I/O, only dprintln + volatile writes
    // Do NOT queue transfers here — echo mode handles the chain from main loop
    psp::dprintln!("[CB] ATTACH speed={}", speed);
    unsafe {
        core::ptr::write_volatile(&raw mut ATTACH_SPEED, speed);
        core::ptr::write_volatile(&raw mut ATTACHED, true);
    }
    0
}

unsafe extern "C" fn usb_detach(_arg1: i32, _arg2: i32, _arg3: i32) -> i32 {
    // NOTE: interrupt context — NO file I/O
    psp::dprintln!("[CB] DETACH");
    unsafe { core::ptr::write_volatile(&raw mut ATTACHED, false) };
    0
}

unsafe extern "C" fn usb_start(_size: i32, _args: *mut u8) -> i32 {
    psp::dprintln!("[USB] start_func ENTER");
    // Log to file too — this runs inside sceUsbStart callback context
    crate::log_str("[CB] start_func entered");

    unsafe {
        descriptors::init_usb_data();
        crate::log_str("[CB] init_usb_data done");

        DRIVER_STATIC.devp_hi = USB_DATA[0].devdesc as *mut u8;
        DRIVER_STATIC.confp_hi = &raw mut USB_DATA[0].config as *mut u8;
        DRIVER_STATIC.devp = USB_DATA[1].devdesc as *mut u8;
        DRIVER_STATIC.confp = &raw mut USB_DATA[1].config as *mut u8;
        crate::log_hex("[CB] devp_hi=", DRIVER_STATIC.devp_hi as u32);
        crate::log_hex("[CB] confp_hi=", DRIVER_STATIC.confp_hi as u32);
        crate::log_hex("[CB] devp=", DRIVER_STATIC.devp as u32);
        crate::log_hex("[CB] confp=", DRIVER_STATIC.confp as u32);
    }

    // Log the full descriptor pointer chain
    unsafe {
        crate::log_hex("[CB] ud0.devdesc=", USB_DATA[0].devdesc as u32);
        crate::log_hex("[CB] ud0.cfg.pconf=", USB_DATA[0].config.pconfdesc as u32);
        crate::log_hex("[CB] ud0.cfg.pintf=", USB_DATA[0].config.pinterfaces as u32);
        crate::log_hex("[CB] ud0.cfg.pintd=", USB_DATA[0].config.pinterdesc as u32);
        crate::log_hex("[CB] ud0.cfg.pendp=", USB_DATA[0].config.pendp as u32);
        crate::log_hex("[CB] ud0.confd.pintf=", USB_DATA[0].confdesc.pinterfaces as u32);
        crate::log_hex("[CB] ud0.intf.pd0=", USB_DATA[0].interfaces.pinterdesc[0] as u32);
        crate::log_hex("[CB] ud0.intf.cnt=", USB_DATA[0].interfaces.intcount);
        crate::log_hex("[CB] ud0.intd.pendp=", USB_DATA[0].interdesc.pendp as u32);
        // Endpoint descriptor bytes
        crate::log_hex("[CB] ep0.b0123=", u32::from_le_bytes([
            USB_DATA[0].endp[0].desc[0], USB_DATA[0].endp[0].desc[1],
            USB_DATA[0].endp[0].desc[2], USB_DATA[0].endp[0].desc[3],
        ]));
        crate::log_hex("[CB] ep1.b0123=", u32::from_le_bytes([
            USB_DATA[0].endp[1].desc[0], USB_DATA[0].endp[1].desc[1],
            USB_DATA[0].endp[1].desc[2], USB_DATA[0].endp[1].desc[3],
        ]));
    }

    crate::log_str("[CB] start_func done");
    0
}

unsafe extern "C" fn usb_stop(_size: i32, _args: *mut u8) -> i32 {
    psp::dprintln!("[USB] stop_func");
    0
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

pub unsafe fn register() -> i32 {
    unsafe {
        DRIVER_STATIC.str_desc = (&raw mut STRING_DESC) as *mut StringDescriptor;
        let ptr = &raw mut DRIVER_STATIC;
        psp::dprintln!("[reg] ptr={:08X}", ptr as u32);
        usbd::register_driver(ptr)
    }
}

pub fn is_attached() -> bool {
    unsafe { core::ptr::read_volatile(&raw const ATTACHED) }
}

/// Get the connection speed from the attach callback.
/// 2 = hi-speed (480 Mbps), 1 = full-speed (12 Mbps), 0 = not yet attached.
pub fn attach_speed() -> i32 {
    unsafe { core::ptr::read_volatile(&raw const ATTACH_SPEED) }
}

pub unsafe fn get_endpoint(index: usize) -> *mut UsbEndpoint {
    unsafe { &raw mut ENDPOINTS[index] }
}
