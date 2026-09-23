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
//! Max message = 16380 bytes (4-byte header + [`MAX_CHUNK_PAYLOAD`] = 16376
//! bytes of payload), which fits in one 16 KiB USB transfer.

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

// ---------------------------------------------------------------------------
// Frame streaming constants
// ---------------------------------------------------------------------------

/// VRAM stride in pixels (PSP hardware requires 512px stride for 480px display)
pub const FRAME_STRIDE: u32 = 512;

/// Frame size with stride padding: 512 * 272 * 2 = 278,528 bytes
pub const FRAME_SIZE_STRIDE: usize = (FRAME_STRIDE * DISPLAY_HEIGHT * BPP_RGB565) as usize;

/// Maximum chunk payload in bytes (16376 + 4 header = 16380, not a multiple of 512)
pub const MAX_CHUNK_PAYLOAD: usize = 16376;

/// Number of chunks per frame: ceil(278528 / 16376) = 18
pub const CHUNKS_PER_FRAME: usize = FRAME_SIZE_STRIDE.div_ceil(MAX_CHUNK_PAYLOAD);

/// Size of the host receive buffer (one USB transfer).
pub const MAX_TRANSFER: usize = 16384;

/// USB bulk max packet size. A transfer that is an exact multiple of this
/// needs a zero-length packet to terminate, which the PSP driver does not
/// handle -- so encoders append one padding byte instead.
pub const BULK_PACKET_SIZE: usize = 512;

/// Encode a framed message (header + payload) ready to write to EP2.
///
/// Rejects payloads larger than [`MAX_CHUNK_PAYLOAD`] (previously the
/// length was cast straight to `u16`, silently truncating the header's
/// `payload_len` for oversized payloads). If the packet length is an exact
/// multiple of [`BULK_PACKET_SIZE`] a single `0` padding byte is appended
/// (not counted in `payload_len`) to avoid needing a ZLP.
pub fn encode_msg(msg_type: u8, flags: u8, payload: &[u8]) -> Result<Vec<u8>, String> {
    if payload.len() > MAX_CHUNK_PAYLOAD {
        return Err(format!(
            "payload too large: {} bytes (max {MAX_CHUNK_PAYLOAD})",
            payload.len()
        ));
    }
    let header = MsgHeader {
        msg_type,
        flags,
        payload_len: payload.len() as u16,
    };
    let mut packet = Vec::with_capacity(MsgHeader::SIZE + payload.len() + 1);
    packet.extend_from_slice(&header.to_bytes());
    packet.extend_from_slice(payload);
    if packet.len().is_multiple_of(BULK_PACKET_SIZE) {
        packet.push(0);
    }
    Ok(packet)
}

/// Decode a framed message received on EP1.
///
/// Returns the header and the payload slice. Trailing bytes after the
/// declared payload (e.g. ZLP-avoidance padding) are ignored.
pub fn decode_msg(buf: &[u8]) -> Result<(MsgHeader, &[u8]), String> {
    let header =
        MsgHeader::from_bytes(buf).ok_or_else(|| format!("Short message: {} bytes", buf.len()))?;
    let total = MsgHeader::SIZE + header.payload_len as usize;
    if buf.len() < total {
        return Err(format!("Truncated: got {}, expected {total}", buf.len()));
    }
    Ok((header, &buf[MsgHeader::SIZE..total]))
}

/// Split a frame into `(chunk_index, bytes)` pieces of at most
/// [`MAX_CHUNK_PAYLOAD`] bytes.
///
/// Errors if the frame would need more chunks than fit in the `u8` chunk
/// index carried in the header's `flags` byte (previously `send_frame`
/// overflowed the index -- a panic in debug builds, a silent wrap to
/// chunk 0 in release).
pub fn frame_chunks(pixels: &[u8]) -> Result<Vec<(u8, &[u8])>, String> {
    let count = pixels.len().div_ceil(MAX_CHUNK_PAYLOAD);
    if count > usize::from(u8::MAX) + 1 {
        return Err(format!(
            "frame too large: {} bytes needs {count} chunks (max 256)",
            pixels.len()
        ));
    }
    Ok(pixels
        .chunks(MAX_CHUNK_PAYLOAD)
        .enumerate()
        .map(|(i, c)| (i as u8, c))
        .collect())
}

// ---------------------------------------------------------------------------
// PSP button bitmasks (matches psp::sys::CtrlButtons)
// ---------------------------------------------------------------------------

pub mod buttons {
    pub const SELECT: u32 = 0x000001;
    pub const START: u32 = 0x000008;
    pub const UP: u32 = 0x000010;
    pub const RIGHT: u32 = 0x000020;
    pub const DOWN: u32 = 0x000040;
    pub const LEFT: u32 = 0x000080;
    pub const L_TRIGGER: u32 = 0x000100;
    pub const R_TRIGGER: u32 = 0x000200;
    pub const TRIANGLE: u32 = 0x001000;
    pub const CIRCLE: u32 = 0x002000;
    pub const CROSS: u32 = 0x004000;
    pub const SQUARE: u32 = 0x008000;
    pub const HOME: u32 = 0x010000;
    pub const HOLD: u32 = 0x020000;
    pub const NOTE: u32 = 0x800000;

    /// Format button bitmask as a human-readable string.
    pub fn format(bits: u32) -> String {
        let names = [
            (SELECT, "SEL"),
            (START, "START"),
            (UP, "UP"),
            (RIGHT, "RIGHT"),
            (DOWN, "DOWN"),
            (LEFT, "LEFT"),
            (L_TRIGGER, "L"),
            (R_TRIGGER, "R"),
            (TRIANGLE, "△"),
            (CIRCLE, "○"),
            (CROSS, "×"),
            (SQUARE, "□"),
            (HOME, "HOME"),
            (HOLD, "HOLD"),
            (NOTE, "NOTE"),
        ];
        let mut result = String::new();
        for (mask, name) in names {
            if bits & mask != 0 {
                if !result.is_empty() {
                    result.push('+');
                }
                result.push_str(name);
            }
        }
        if result.is_empty() {
            result.push_str("(none)");
        }
        result
    }
}

// ---------------------------------------------------------------------------
// PSP input state
// ---------------------------------------------------------------------------

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

    /// Format as a human-readable string.
    pub fn display(&self) -> String {
        format!(
            "buttons={} analog=({},{}) bat={}%",
            buttons::format(self.buttons),
            self.analog_x,
            self.analog_y,
            self.battery,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn msg_header_round_trip_little_endian() {
        let h = MsgHeader {
            msg_type: cmd::FRAME_CHUNK,
            flags: 7,
            payload_len: 0x1234,
        };
        let bytes = h.to_bytes();
        assert_eq!(bytes, [cmd::FRAME_CHUNK, 7, 0x34, 0x12]);
        let back = MsgHeader::from_bytes(&bytes).expect("decodes");
        assert_eq!(back.msg_type, cmd::FRAME_CHUNK);
        assert_eq!(back.flags, 7);
        assert_eq!({ back.payload_len }, 0x1234);
    }

    #[test]
    fn msg_header_rejects_truncated_input() {
        for len in 0..MsgHeader::SIZE {
            assert!(
                MsgHeader::from_bytes(&[0xAA; 4][..len]).is_none(),
                "len {len}"
            );
        }
        // Extra bytes are fine (the header is a prefix).
        assert!(MsgHeader::from_bytes(&[1, 2, 3, 4, 5]).is_some());
    }

    #[test]
    fn input_state_round_trip() {
        let s = InputState {
            buttons: buttons::CROSS | buttons::UP | buttons::NOTE,
            analog_x: 12,
            analog_y: 250,
            battery: 87,
            _pad: 0,
        };
        let bytes = s.to_bytes();
        assert_eq!(&bytes[..4], &{ s.buttons }.to_le_bytes());
        let back = InputState::from_bytes(&bytes).expect("decodes");
        assert_eq!({ back.buttons }, { s.buttons });
        assert_eq!(back.analog_x, 12);
        assert_eq!(back.analog_y, 250);
        assert_eq!(back.battery, 87);
    }

    #[test]
    fn input_state_rejects_truncated_input() {
        let bytes = InputState::default().to_bytes();
        for len in 0..InputState::SIZE {
            assert!(InputState::from_bytes(&bytes[..len]).is_none(), "len {len}");
        }
    }

    #[test]
    fn encode_decode_round_trip() {
        let payload: Vec<u8> = (0..100u8).collect();
        let packet = encode_msg(rsp::ECHO, 3, &payload).expect("encodes");
        assert_eq!(packet.len(), MsgHeader::SIZE + payload.len());
        let (h, body) = decode_msg(&packet).expect("decodes");
        assert_eq!(h.msg_type, rsp::ECHO);
        assert_eq!(h.flags, 3);
        assert_eq!(body, &payload[..]);
    }

    #[test]
    fn encode_pads_exact_bulk_packet_multiples() {
        // 4-byte header + 508 payload = 512 -> one pad byte appended.
        let packet = encode_msg(cmd::FRAME_CHUNK, 0, &[0x55; 508]).expect("encodes");
        assert_eq!(packet.len(), 513);
        // The pad byte is not part of the payload.
        let (h, body) = decode_msg(&packet).expect("decodes");
        assert_eq!({ h.payload_len }, 508);
        assert_eq!(body.len(), 508);
        // Non-multiples are not padded.
        assert_eq!(encode_msg(cmd::PING, 0, &[]).expect("encodes").len(), 4);
    }

    #[test]
    fn encode_rejects_oversized_payload() {
        let max = vec![0u8; MAX_CHUNK_PAYLOAD];
        let packet = encode_msg(cmd::FRAME_CHUNK, 0, &max).expect("max fits");
        assert!(packet.len() <= MAX_TRANSFER);
        // Previously `len as u16` silently truncated e.g. 65540 -> 4.
        assert!(encode_msg(cmd::FRAME_CHUNK, 0, &[0u8; MAX_CHUNK_PAYLOAD + 1]).is_err());
        assert!(encode_msg(cmd::FRAME_CHUNK, 0, &vec![0u8; 65_540]).is_err());
    }

    #[test]
    fn decode_rejects_short_and_truncated_messages() {
        assert!(decode_msg(&[]).is_err());
        assert!(decode_msg(&[rsp::INPUT_STATE, 0, 8]).is_err());
        // Header claims 8 payload bytes but only 5 arrived.
        let mut packet = MsgHeader::new(rsp::INPUT_STATE, 8).to_bytes().to_vec();
        packet.extend_from_slice(&[1, 2, 3, 4, 5]);
        assert!(decode_msg(&packet).is_err());
        packet.extend_from_slice(&[6, 7, 8]);
        let (_, body) = decode_msg(&packet).expect("complete message");
        assert_eq!(InputState::from_bytes(body).expect("state").analog_x, 5);
    }

    #[test]
    fn frame_chunks_cover_full_frame() {
        let frame = vec![0xABu8; FRAME_SIZE_STRIDE];
        let chunks = frame_chunks(&frame).expect("frame fits");
        assert_eq!(chunks.len(), CHUNKS_PER_FRAME);
        assert_eq!(CHUNKS_PER_FRAME, 18);
        for (i, (idx, c)) in chunks.iter().enumerate() {
            assert_eq!(*idx as usize, i);
            assert!(c.len() <= MAX_CHUNK_PAYLOAD);
        }
        let total: usize = chunks.iter().map(|(_, c)| c.len()).sum();
        assert_eq!(total, FRAME_SIZE_STRIDE);
        assert!(frame_chunks(&[]).expect("empty ok").is_empty());
    }

    #[test]
    fn frame_chunks_rejects_index_overflow() {
        let ok = vec![0u8; MAX_CHUNK_PAYLOAD * 256];
        assert_eq!(frame_chunks(&ok).expect("256 chunks fit").len(), 256);
        let too_big = vec![0u8; MAX_CHUNK_PAYLOAD * 256 + 1];
        assert!(frame_chunks(&too_big).is_err());
    }

    #[test]
    fn button_format_names() {
        assert_eq!(buttons::format(0), "(none)");
        assert_eq!(
            buttons::format(buttons::START | buttons::L_TRIGGER),
            "START+L"
        );
    }
}
