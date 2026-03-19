//! USB descriptor definitions matching the PSP UsbData layout.
//!
//! CRITICAL: The PSP USB driver expects descriptors in a specific nested
//! struct format with pointer fields. Passing raw descriptor bytes crashes.
//! Layout reverse-engineered from USBHostFS (psplinkusb/usbhostfs/main.c).

/// Standard USB device descriptor (18 bytes)
#[repr(C, packed)]
#[derive(Copy, Clone)]
pub struct DeviceDescriptor {
    pub b_length: u8,
    pub b_descriptor_type: u8,
    pub bcd_usb: u16,
    pub b_device_class: u8,
    pub b_device_sub_class: u8,
    pub b_device_protocol: u8,
    pub b_max_packet_size0: u8,
    pub id_vendor: u16,
    pub id_product: u16,
    pub bcd_device: u16,
    pub i_manufacturer: u8,
    pub i_product: u8,
    pub i_serial_number: u8,
    pub b_num_configurations: u8,
}

/// Standard USB config descriptor (9 bytes)
#[repr(C, packed)]
#[derive(Copy, Clone)]
pub struct ConfigDescriptor {
    pub b_length: u8,
    pub b_descriptor_type: u8,
    pub w_total_length: u16,
    pub b_num_interfaces: u8,
    pub b_configuration_value: u8,
    pub i_configuration: u8,
    pub bm_attributes: u8,
    pub b_max_power: u8,
}

/// Standard USB interface descriptor (9 bytes)
#[repr(C, packed)]
#[derive(Copy, Clone)]
pub struct InterfaceDescriptor {
    pub b_length: u8,
    pub b_descriptor_type: u8,
    pub b_interface_number: u8,
    pub b_alternate_setting: u8,
    pub b_num_endpoints: u8,
    pub b_interface_class: u8,
    pub b_interface_sub_class: u8,
    pub b_interface_protocol: u8,
    pub i_interface: u8,
}

/// Standard USB endpoint descriptor (7 bytes)
#[repr(C, packed)]
#[derive(Copy, Clone)]
pub struct EndpointDescriptor {
    pub b_length: u8,
    pub b_descriptor_type: u8,
    pub b_endpoint_address: u8,
    pub bm_attributes: u8,
    pub w_max_packet_size: u16,
    pub b_interval: u8,
}

/// String descriptor
#[repr(C, packed)]
pub struct StringDescriptor {
    pub b_length: u8,
    pub b_descriptor_type: u8,
    pub data: [u8; 30],
}

// ---------------------------------------------------------------------------
// PSP UsbData nested struct (matches kernel expectations)
// ---------------------------------------------------------------------------

/// Endpoint wrapper within UsbData
#[repr(C)]
pub struct UsbDataEndpoint {
    pub desc: [u8; 16], // endpoint descriptor bytes (7 used, rest padding)
}

/// Interface descriptor wrapper within UsbData
#[repr(C)]
pub struct UsbDataInterDesc {
    pub desc: [u8; 12],         // interface descriptor bytes (9 used)
    pub pendp: *mut UsbDataEndpoint, // pointer to endpoint array
    pub pad: [u8; 32],          // required padding
}

/// Interface list within UsbData
#[repr(C)]
pub struct UsbDataInterfaces {
    pub pinterdesc: [*mut UsbDataInterDesc; 2], // pointers to interface descriptors
    pub intcount: u32,
}

/// Config descriptor wrapper within UsbData
#[repr(C)]
pub struct UsbDataConfDesc {
    pub desc: [u8; 12],               // config descriptor bytes (9 used)
    pub pinterfaces: *mut UsbDataInterfaces,
}

/// Config pointer struct (what confp_hi/confp actually point to)
#[repr(C)]
pub struct UsbDataConfig {
    pub pconfdesc: *mut UsbDataConfDesc,
    pub pinterfaces: *mut UsbDataInterfaces,
    pub pinterdesc: *mut UsbDataInterDesc,
    pub pendp: *mut UsbDataEndpoint,
}

/// Complete UsbData structure — one per speed (hi-speed + full-speed)
#[repr(C)]
pub struct UsbData {
    pub devdesc: *mut DeviceDescriptor,
    pub config: UsbDataConfig,
    pub confdesc: UsbDataConfDesc,
    pub _pad1: [u8; 8],
    pub interfaces: UsbDataInterfaces,
    pub interdesc: UsbDataInterDesc,
    pub endp: [UsbDataEndpoint; 3],
}

// SAFETY: UsbData is only accessed from init thread and USB interrupt context
unsafe impl Sync for UsbData {}
unsafe impl Sync for StringDescriptor {}

// ---------------------------------------------------------------------------
// Static descriptor instances
// ---------------------------------------------------------------------------

// wTotalLength = config(9) + interface(9) + 2 endpoints(7*2) = 32
const TOTAL_LEN: u16 = 9 + 9 + 7 + 7;

pub static mut DEVDESC_HI: DeviceDescriptor = DeviceDescriptor {
    b_length: 18,
    b_descriptor_type: 1,
    bcd_usb: 0x0200,
    b_device_class: 0,
    b_device_sub_class: 0,
    b_device_protocol: 0,
    b_max_packet_size0: 64,
    id_vendor: 0,   // USBHostFS uses 0
    id_product: 0,
    bcd_device: 0x0100,
    i_manufacturer: 0,
    i_product: 0,
    i_serial_number: 0,
    b_num_configurations: 1,
};

pub static mut DEVDESC_FULL: DeviceDescriptor = DeviceDescriptor {
    b_length: 18,
    b_descriptor_type: 1,
    bcd_usb: 0x0200,
    b_device_class: 0,
    b_device_sub_class: 0,
    b_device_protocol: 0,
    b_max_packet_size0: 64,
    id_vendor: 0,
    id_product: 0,
    bcd_device: 0x0100,
    i_manufacturer: 0,
    i_product: 0,
    i_serial_number: 0,
    b_num_configurations: 1,
};

pub static CONFDESC_HI: ConfigDescriptor = ConfigDescriptor {
    b_length: 9,
    b_descriptor_type: 2,
    w_total_length: TOTAL_LEN,
    b_num_interfaces: 1,
    b_configuration_value: 1,
    i_configuration: 0,
    bm_attributes: 0xC0,
    b_max_power: 0,
};

pub static CONFDESC_FULL: ConfigDescriptor = ConfigDescriptor {
    b_length: 9,
    b_descriptor_type: 2,
    w_total_length: TOTAL_LEN,
    b_num_interfaces: 1,
    b_configuration_value: 1,
    i_configuration: 0,
    bm_attributes: 0xC0,
    b_max_power: 0,
};

pub static INTERDESC_HI: InterfaceDescriptor = InterfaceDescriptor {
    b_length: 9,
    b_descriptor_type: 4,
    b_interface_number: 0,
    b_alternate_setting: 0,
    b_num_endpoints: 2,
    b_interface_class: 0xFF,
    b_interface_sub_class: 0x01,
    b_interface_protocol: 0xFF,
    i_interface: 1,
};

pub static INTERDESC_FULL: InterfaceDescriptor = InterfaceDescriptor {
    b_length: 9,
    b_descriptor_type: 4,
    b_interface_number: 0,
    b_alternate_setting: 0,
    b_num_endpoints: 2,
    b_interface_class: 0xFF,
    b_interface_sub_class: 0x01,
    b_interface_protocol: 0xFF,
    i_interface: 1,
};

pub static ENDPDESC_HI: [EndpointDescriptor; 2] = [
    EndpointDescriptor {
        b_length: 7, b_descriptor_type: 5,
        b_endpoint_address: 0x81, bm_attributes: 2,
        w_max_packet_size: 512, b_interval: 0,
    },
    EndpointDescriptor {
        b_length: 7, b_descriptor_type: 5,
        b_endpoint_address: 0x02, bm_attributes: 2,
        w_max_packet_size: 512, b_interval: 0,
    },
];

pub static ENDPDESC_FULL: [EndpointDescriptor; 2] = [
    EndpointDescriptor {
        b_length: 7, b_descriptor_type: 5,
        b_endpoint_address: 0x81, bm_attributes: 2,
        w_max_packet_size: 64, b_interval: 0,
    },
    EndpointDescriptor {
        b_length: 7, b_descriptor_type: 5,
        b_endpoint_address: 0x02, bm_attributes: 2,
        w_max_packet_size: 64, b_interval: 0,
    },
];

/// The two UsbData instances (hi-speed and full-speed)
pub static mut USB_DATA: [UsbData; 2] = [
    // [0] = hi-speed
    UsbData {
        devdesc: core::ptr::null_mut(),
        config: UsbDataConfig {
            pconfdesc: core::ptr::null_mut(),
            pinterfaces: core::ptr::null_mut(),
            pinterdesc: core::ptr::null_mut(),
            pendp: core::ptr::null_mut(),
        },
        confdesc: UsbDataConfDesc {
            desc: [0; 12],
            pinterfaces: core::ptr::null_mut(),
        },
        _pad1: [0; 8],
        interfaces: UsbDataInterfaces {
            pinterdesc: [core::ptr::null_mut(); 2],
            intcount: 0,
        },
        interdesc: UsbDataInterDesc {
            desc: [0; 12],
            pendp: core::ptr::null_mut(),
            pad: [0; 32],
        },
        endp: [
            UsbDataEndpoint { desc: [0; 16] },
            UsbDataEndpoint { desc: [0; 16] },
            UsbDataEndpoint { desc: [0; 16] },
        ],
    },
    // [1] = full-speed
    UsbData {
        devdesc: core::ptr::null_mut(),
        config: UsbDataConfig {
            pconfdesc: core::ptr::null_mut(),
            pinterfaces: core::ptr::null_mut(),
            pinterdesc: core::ptr::null_mut(),
            pendp: core::ptr::null_mut(),
        },
        confdesc: UsbDataConfDesc {
            desc: [0; 12],
            pinterfaces: core::ptr::null_mut(),
        },
        _pad1: [0; 8],
        interfaces: UsbDataInterfaces {
            pinterdesc: [core::ptr::null_mut(); 2],
            intcount: 0,
        },
        interdesc: UsbDataInterDesc {
            desc: [0; 12],
            pendp: core::ptr::null_mut(),
            pad: [0; 32],
        },
        endp: [
            UsbDataEndpoint { desc: [0; 16] },
            UsbDataEndpoint { desc: [0; 16] },
            UsbDataEndpoint { desc: [0; 16] },
        ],
    },
];

/// Initialize UsbData structures — must be called before driver registration.
/// Copies descriptors and wires up all internal pointers.
pub unsafe fn init_usb_data() {
    unsafe {
        // Helper: copy bytes
        fn copy(dst: &mut [u8], src: &[u8]) {
            let n = src.len().min(dst.len());
            dst[..n].copy_from_slice(&src[..n]);
        }

        // Byte-cast helper
        fn as_bytes<T>(v: &T) -> &[u8] {
            unsafe {
                core::slice::from_raw_parts(v as *const T as *const u8, core::mem::size_of::<T>())
            }
        }

        // Hi-speed [0]
        USB_DATA[0].devdesc = &raw mut DEVDESC_HI;
        USB_DATA[0].config.pconfdesc = &raw mut USB_DATA[0].confdesc;
        USB_DATA[0].config.pinterfaces = &raw mut USB_DATA[0].interfaces;
        USB_DATA[0].config.pinterdesc = &raw mut USB_DATA[0].interdesc;
        USB_DATA[0].config.pendp = &raw mut USB_DATA[0].endp[0];
        copy(&mut USB_DATA[0].confdesc.desc, as_bytes(&CONFDESC_HI));
        USB_DATA[0].confdesc.pinterfaces = &raw mut USB_DATA[0].interfaces;
        USB_DATA[0].interfaces.pinterdesc[0] = &raw mut USB_DATA[0].interdesc;
        USB_DATA[0].interfaces.intcount = 1;
        copy(&mut USB_DATA[0].interdesc.desc, as_bytes(&INTERDESC_HI));
        USB_DATA[0].interdesc.pendp = &raw mut USB_DATA[0].endp[0];
        copy(&mut USB_DATA[0].endp[0].desc, as_bytes(&ENDPDESC_HI[0]));
        copy(&mut USB_DATA[0].endp[1].desc, as_bytes(&ENDPDESC_HI[1]));

        // Full-speed [1]
        USB_DATA[1].devdesc = &raw mut DEVDESC_FULL;
        USB_DATA[1].config.pconfdesc = &raw mut USB_DATA[1].confdesc;
        USB_DATA[1].config.pinterfaces = &raw mut USB_DATA[1].interfaces;
        USB_DATA[1].config.pinterdesc = &raw mut USB_DATA[1].interdesc;
        USB_DATA[1].config.pendp = &raw mut USB_DATA[1].endp[0];
        copy(&mut USB_DATA[1].confdesc.desc, as_bytes(&CONFDESC_FULL));
        USB_DATA[1].confdesc.pinterfaces = &raw mut USB_DATA[1].interfaces;
        USB_DATA[1].interfaces.pinterdesc[0] = &raw mut USB_DATA[1].interdesc;
        USB_DATA[1].interfaces.intcount = 1;
        copy(&mut USB_DATA[1].interdesc.desc, as_bytes(&INTERDESC_FULL));
        USB_DATA[1].interdesc.pendp = &raw mut USB_DATA[1].endp[0];
        copy(&mut USB_DATA[1].endp[0].desc, as_bytes(&ENDPDESC_FULL[0]));
        copy(&mut USB_DATA[1].endp[1].desc, as_bytes(&ENDPDESC_FULL[1]));
    }
}
