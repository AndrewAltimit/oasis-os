//! PSMF (PSP Movie Format) header generator and MPEG-PS stream wrapper.
//!
//! The PSP's `sceMpeg` API expects data in PSMF format: a 2048-byte header
//! followed by MPEG-2 Program Stream packs containing H.264 AUs in PES
//! packets. The ringbuffer callback receives data as sequential 2048-byte
//! packets — first packet is the PSMF header, subsequent packets are MPEG-PS.
//!
//! Each 2048-byte sector contains:
//!   - Pack header (14 bytes): 00 00 01 BA + SCR + mux_rate + stuffing
//!   - PES video packet: 00 00 01 E0 + length + flags + PTS/DTS + AU data
//!   - Padding stream (remaining space): 00 00 01 BE + length + 0xFF fill
//!
//! The PES length MUST be correct (not 0) because the firmware's MPEG-PS
//! demuxer uses it to skip video data. With length=0, the scanner enters
//! the H.264 payload and finds NAL start codes (00 00 01) that look like
//! MPEG-PS start codes, causing an infinite loop.

/// PSMF packet size (matches ringbuffer packet size).
pub const PACKET_SIZE: usize = 2048;

/// MPEG-PS start codes.
const PACK_START_CODE: [u8; 4] = [0x00, 0x00, 0x01, 0xBA];
const PES_VIDEO_STREAM_ID: u8 = 0xE0;

/// Generate a minimal PSMF header for H.264 video.
///
/// The header is exactly 2048 bytes (one ringbuffer packet). It declares
/// a single AVC video stream with the given parameters.
pub fn generate_psmf_header(width: u16, height: u16, stream_size: u32) -> [u8; PACKET_SIZE] {
    let mut h = [0u8; PACKET_SIZE];

    // Magic: "PSMF"
    h[0x000] = b'P';
    h[0x001] = b'S';
    h[0x002] = b'M';
    h[0x003] = b'F';

    // Version: "0015" (FW 1.5+ format)
    h[0x004] = b'0';
    h[0x005] = b'0';
    h[0x006] = b'1';
    h[0x007] = b'5';

    // Header size (big-endian): 0x00000800 = 2048
    h[0x00A] = 0x08;

    // Stream data size (big-endian)
    h[0x00C] = (stream_size >> 24) as u8;
    h[0x00D] = (stream_size >> 16) as u8;
    h[0x00E] = (stream_size >> 8) as u8;
    h[0x00F] = stream_size as u8;

    // Stream info table offset (big-endian, relative to header start).
    h[0x047] = 0x50;

    // Number of streams (big-endian u16 at offset 0x50)
    h[0x051] = 0x01;

    // Stream 0 descriptor (starts at 0x52):
    h[0x052] = 0x00; // type: AVC video
    h[0x053] = 0x00; // channel: 0
    h[0x054] = 0x42; // AVC profile: Baseline
    h[0x055] = 0x1F; // AVC level: 3.1

    // Video width/height (big-endian)
    h[0x056] = (width >> 8) as u8;
    h[0x057] = width as u8;
    h[0x058] = (height >> 8) as u8;
    h[0x059] = height as u8;

    // PPSSPP reads dimensions from buffer[142]*16, buffer[143]*16
    h[0x08E] = ((width as u32 + 15) / 16) as u8;
    h[0x08F] = ((height as u32 + 15) / 16) as u8;

    // Last timestamp (big-endian u32 at 0x5C, 90kHz clock)
    let last_ts: u32 = 54_000_000; // ~10 minutes
    h[0x05C] = (last_ts >> 24) as u8;
    h[0x05D] = (last_ts >> 16) as u8;
    h[0x05E] = (last_ts >> 8) as u8;
    h[0x05F] = last_ts as u8;

    // EPMap offset (big-endian at 0x48) → minimal EPMap at 0x60
    h[0x04B] = 0x60;

    // EP map: 1 entry
    h[0x061] = 0x01;

    h
}

/// State for wrapping H.264 AUs into PSMF-compatible MPEG-PS packets.
pub struct PsmfMuxer {
    scr_base: u64,
    packet_count: u32,
    header_sent: bool,
    header: [u8; PACKET_SIZE],
}

impl PsmfMuxer {
    pub fn new(width: u16, height: u16) -> Self {
        let stream_size = 64 * 1024 * 1024;
        Self {
            scr_base: 0,
            packet_count: 0,
            header_sent: false,
            header: generate_psmf_header(width, height, stream_size),
        }
    }

    pub fn peek_header(&self) -> &[u8; PACKET_SIZE] {
        &self.header
    }

    pub fn take_header_packet(&mut self) -> Option<[u8; PACKET_SIZE]> {
        if self.header_sent {
            return None;
        }
        self.header_sent = true;
        Some(self.header)
    }

    /// Wrap a single H.264 access unit into 2048-byte PSMF packets.
    ///
    /// Each packet contains a pack header + PES video packet with correct
    /// length + AU data chunk + padding stream. PTS/DTS is included in
    /// every PES (only the first matters, but including it is simpler).
    pub fn wrap_au(&mut self, au_data: &[u8], pts_90khz: u64) -> Vec<[u8; PACKET_SIZE]> {
        let mut packets = Vec::new();
        let mut au_offset = 0;

        while au_offset < au_data.len() {
            let mut pkt = [0u8; PACKET_SIZE];
            let mut pos = 0;

            // --- Pack header (14 bytes) ---
            pkt[pos..pos + 4].copy_from_slice(&PACK_START_CODE);
            pos += 4;

            // SCR (MPEG-2 PS format, 6 bytes)
            let scr = self.scr_base;
            pkt[pos] = 0x44
                | (((scr >> 27) & 0x38) as u8)
                | (((scr >> 28) & 0x03) as u8);
            pos += 1;
            pkt[pos] = ((scr >> 20) & 0xFF) as u8;
            pos += 1;
            pkt[pos] = (((scr >> 12) & 0xF8) as u8)
                | 0x04
                | (((scr >> 13) & 0x03) as u8);
            pos += 1;
            pkt[pos] = ((scr >> 5) & 0xFF) as u8;
            pos += 1;
            pkt[pos] = (((scr & 0x1F) << 3) as u8) | 0x04;
            pos += 1;
            pkt[pos] = 0x01;
            pos += 1;

            // Mux rate (3 bytes)
            let mux_rate: u32 = 10080;
            pkt[pos] = ((mux_rate >> 14) & 0xFF) as u8;
            pos += 1;
            pkt[pos] = ((mux_rate >> 6) & 0xFF) as u8;
            pos += 1;
            pkt[pos] = (((mux_rate & 0x3F) << 2) as u8) | 0x03;
            pos += 1;

            // Stuffing
            pkt[pos] = 0xF8;
            pos += 1;
            // pos = 14

            // --- PES video packet ---
            pkt[pos] = 0x00;
            pkt[pos + 1] = 0x00;
            pkt[pos + 2] = 0x01;
            pkt[pos + 3] = PES_VIDEO_STREAM_ID;
            pos += 4;
            // pos = 18

            // PES header: flags(1) + PTS/DTS flag(1) + header_len(1) + PTS(5) + DTS(5) = 13
            let pes_hdr_data = 3 + 10; // 13 bytes of PES header after length field

            // How much AU data fits: sector - pack(14) - PES start(4) - PES len(2) - PES hdr(13)
            let payload_space = PACKET_SIZE - 14 - 4 - 2 - pes_hdr_data;
            // = 2048 - 33 = 2015
            let remaining = au_data.len() - au_offset;
            let au_in_sector = remaining.min(payload_space);

            // PES packet length = PES header data + AU payload
            // This is the number of bytes AFTER the 2-byte length field.
            let pes_len = pes_hdr_data + au_in_sector;
            pkt[pos] = (pes_len >> 8) as u8;
            pkt[pos + 1] = pes_len as u8;
            pos += 2;
            // pos = 20

            // PES flags: 10_00_0_0_0_1 (marker=10, original=1)
            pkt[pos] = 0x81;
            pos += 1;

            // PTS/DTS flags: 11 = both present
            pkt[pos] = 0xC0;
            pos += 1;

            // PES header data length: 10 bytes (5 PTS + 5 DTS)
            pkt[pos] = 0x0A;
            pos += 1;

            // PTS
            write_pts_dts(&mut pkt[pos..], 0x30, pts_90khz);
            pos += 5;

            // DTS (same as PTS for streaming)
            write_pts_dts(&mut pkt[pos..], 0x10, pts_90khz);
            pos += 5;
            // pos = 33

            // Copy AU data for this sector
            pkt[pos..pos + au_in_sector]
                .copy_from_slice(&au_data[au_offset..au_offset + au_in_sector]);
            pos += au_in_sector;
            au_offset += au_in_sector;

            // Fill remaining space with padding stream (00 00 01 BE)
            let remaining_space = PACKET_SIZE - pos;
            if remaining_space >= 6 {
                pkt[pos] = 0x00;
                pkt[pos + 1] = 0x00;
                pkt[pos + 2] = 0x01;
                pkt[pos + 3] = 0xBE; // PADDING_STREAM
                let pad_payload = remaining_space - 6;
                pkt[pos + 4] = (pad_payload >> 8) as u8;
                pkt[pos + 5] = pad_payload as u8;
                for i in (pos + 6)..PACKET_SIZE {
                    pkt[i] = 0xFF;
                }
            } else if remaining_space > 0 {
                for i in pos..PACKET_SIZE {
                    pkt[i] = 0xFF;
                }
            }

            packets.push(pkt);
            self.packet_count += 1;
            self.scr_base += 366;
        }

        packets
    }

    pub fn reset(&mut self, width: u16, height: u16) {
        let stream_size = 64 * 1024 * 1024;
        self.scr_base = 0;
        self.packet_count = 0;
        self.header_sent = false;
        self.header = generate_psmf_header(width, height, stream_size);
    }
}

/// Write a PTS or DTS value in MPEG-2 PES format (5 bytes).
fn write_pts_dts(buf: &mut [u8], marker: u8, ts: u64) {
    buf[0] = marker | (((ts >> 29) & 0x0E) as u8) | 0x01;
    buf[1] = ((ts >> 22) & 0xFF) as u8;
    buf[2] = (((ts >> 14) & 0xFE) as u8) | 0x01;
    buf[3] = ((ts >> 7) & 0xFF) as u8;
    buf[4] = (((ts & 0x7F) << 1) as u8) | 0x01;
}
