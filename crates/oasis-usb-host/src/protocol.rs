//! Wire protocol between PSP (device) and host (Luckfox Pico / desktop).
//!
//! All multi-byte values are little-endian (both sides are LE).
//!
//! ## Endpoints
//! - EP1 (0x81, bulk IN):  PSP → Host (input state)
//! - EP2 (0x02, bulk OUT): Host → PSP (frames, commands)
//!
//! ## Message framing
//! Each message starts with a 4-byte header: [type: u8, flags: u8, len: u16].
//! `len` is the payload length (excluding the header itself).
//! Max payload = 16380 bytes (fits in one USB transfer with header).

/// PSP display dimensions
pub const DISPLAY_WIDTH: u32 = 480;
pub const DISPLAY_HEIGHT: u32 = 272;

/// Bytes per pixel (RGB565 for bandwidth efficiency)
pub const BPP_RGB565: u32 = 2;

/// Full frame size in RGB565
pub const FRAME_SIZE_RGB565: usize = (DISPLAY_WIDTH * DISPLAY_HEIGHT * BPP_RGB565) as usize;

/// USB identifiers
pub const VENDOR_ID: u16 = 0x054C; // Sony
pub const PRODUCT_ID: u16 = 0x1337; // Custom

/// EP addresses
pub const EP_IN: u8 = 0x81; // PSP → Host (bulk IN)
pub const EP_OUT: u8 = 0x02; // Host → PSP (bulk OUT)

/// Message types (Host → PSP, on EP2)
pub mod cmd {
    /// Echo request — PSP echoes payload back on EP1.
    pub const ECHO: u8 = 0x01;

    /// Frame data — partial or full frame in RGB565.
    /// Payload: [x: u16, y: u16, w: u16, h: u16, pixels: [u8; w*h*2]]
    pub const FRAME: u8 = 0x10;

    /// Frame data (full screen, no rect header, just raw pixels).
    /// Multiple FRAME_CHUNK messages build up one frame.
    pub const FRAME_CHUNK: u8 = 0x11;

    /// Frame complete — PSP should swap the framebuffer.
    pub const FRAME_DONE: u8 = 0x12;

    /// Request input state — PSP responds with INPUT_STATE on EP1.
    pub const GET_INPUT: u8 = 0x20;

    /// Ping — PSP responds with PONG.
    pub const PING: u8 = 0xFE;
}

/// Message types (PSP → Host, on EP1)
pub mod rsp {
    /// Echo response — payload is the echoed data.
    pub const ECHO: u8 = 0x01;

    /// Input state report.
    /// Payload: InputState struct (8 bytes).
    pub const INPUT_STATE: u8 = 0x21;

    /// Pong response to PING.
    pub const PONG: u8 = 0xFE;

    /// Ready message (sent on connect).
    pub const READY: u8 = 0xFF;
}

/// Message header (4 bytes, little-endian)
#[repr(C, packed)]
#[derive(Copy, Clone, Debug)]
pub struct MsgHeader {
    pub msg_type: u8,
    pub flags: u8,
    pub payload_len: u16,
}

impl MsgHeader {
    pub const SIZE: usize = 4;

    pub fn new(msg_type: u8, payload_len: u16) -> Self {
        Self {
            msg_type,
            flags: 0,
            payload_len,
        }
    }

    pub fn to_bytes(self) -> [u8; 4] {
        let len = self.payload_len.to_le_bytes();
        [self.msg_type, self.flags, len[0], len[1]]
    }

    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        if b.len() < 4 {
            return None;
        }
        Some(Self {
            msg_type: b[0],
            flags: b[1],
            payload_len: u16::from_le_bytes([b[2], b[3]]),
        })
    }
}

/// PSP input state (8 bytes, sent PSP → Host)
#[repr(C, packed)]
#[derive(Copy, Clone, Debug, Default)]
pub struct InputState {
    pub buttons: u32,
    pub analog_x: u8,
    pub analog_y: u8,
    pub battery: u8,
    pub _pad: u8,
}

impl InputState {
    pub const SIZE: usize = 8;

    pub fn to_bytes(self) -> [u8; 8] {
        let b = self.buttons.to_le_bytes();
        [
            b[0],
            b[1],
            b[2],
            b[3],
            self.analog_x,
            self.analog_y,
            self.battery,
            self._pad,
        ]
    }

    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        if b.len() < 8 {
            return None;
        }
        Some(Self {
            buttons: u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            analog_x: b[4],
            analog_y: b[5],
            battery: b[6],
            _pad: b[7],
        })
    }
}
