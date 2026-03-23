//! PSMF (PSP Movie Format) header generator and MPEG-PS stream wrapper.
//!
//! Format derived from libpmfplayer analysis + real PMF hex dumps.
//!
//! Critical: the PSMF header has TWO stream descriptor regions:
//!   - Offset 0x50: stream info table (used by scePsmf)
//!   - Offset 0x80: numStreams (u16 BE) + 16-byte entries at 0x82
//!     (used by sceMpeg's AnalyzeMpeg via sceMpegQueryStreamOffset)
//!
//! sceMpegQueryStreamOffset MUST be called before any RingbufferPut.
//! It parses the header and initializes the kernel's MPEG-PS demuxer.

/// PSMF packet size (matches ringbuffer packet size).
pub const PACKET_SIZE: usize = 2048;

const PACK_START_CODE: [u8; 4] = [0x00, 0x00, 0x01, 0xBA];
const SYSTEM_HEADER_CODE: [u8; 4] = [0x00, 0x00, 0x01, 0xBB];
const PES_VIDEO_STREAM_ID: u8 = 0xE0;

/// Write a 6-byte MPEG-2 PTS timestamp (33-bit value at 90kHz).
fn write_mpeg_ts_6(buf: &mut [u8], ts: u64) {
    // 6-byte format: same as 5-byte PES PTS but with leading zero byte
    // Actually the PSMF header uses a simpler big-endian encoding
    // at offsets 0x54 and 0x5A. Looking at PPSSPP getMpegTimeStamp:
    //   (buf[idx+1] & 0x01) << 32 | buf[idx+2]<<24 | ... | buf[idx+5]
    // So it's essentially a 33-bit BE value spread across bytes 1-5.
    buf[0] = 0x00;
    buf[1] = ((ts >> 32) & 0x01) as u8;
    buf[2] = ((ts >> 24) & 0xFF) as u8;
    buf[3] = ((ts >> 16) & 0xFF) as u8;
    buf[4] = ((ts >> 8) & 0xFF) as u8;
    buf[5] = (ts & 0xFF) as u8;
}

/// Generate a PSMF header with correct stream descriptors at offset 0x80+.
pub fn generate_psmf_header(width: u16, height: u16, stream_size: u32) -> [u8; PACKET_SIZE] {
    let mut h = [0u8; PACKET_SIZE];

    // Magic + version
    h[0x000] = b'P'; h[0x001] = b'S'; h[0x002] = b'M'; h[0x003] = b'F';
    h[0x004] = b'0'; h[0x005] = b'0'; h[0x006] = b'1'; h[0x007] = b'5';

    // Stream data offset (BE u32) = 0x800
    h[0x008] = 0x00; h[0x009] = 0x00; h[0x00A] = 0x08; h[0x00B] = 0x00;

    // Stream data size (BE u32)
    h[0x00C] = (stream_size >> 24) as u8;
    h[0x00D] = (stream_size >> 16) as u8;
    h[0x00E] = (stream_size >> 8) as u8;
    h[0x00F] = stream_size as u8;

    // First timestamp at 0x54 (6 bytes) = 90000 (1 sec at 90kHz)
    // This is REQUIRED — firmware checks mpegFirstTimestamp == 90000.
    write_mpeg_ts_6(&mut h[0x54..], 90000);

    // Last timestamp at 0x5A (6 bytes) = ~10 minutes
    let last_ts: u64 = 54_000_000;
    write_mpeg_ts_6(&mut h[0x5A..], last_ts);

    // Stream info table offset at 0x44 (BE u32) → points to 0x80
    h[0x044] = 0x00; h[0x045] = 0x00; h[0x046] = 0x00; h[0x047] = 0x80;

    // EPMap offset at 0x48 (BE u32) → point to after stream descriptors
    h[0x048] = 0x00; h[0x049] = 0x00; h[0x04A] = 0x00; h[0x04B] = 0xA0;

    // --- Stream descriptors at offset 0x80 (read by AnalyzeMpeg) ---
    // numStreams (BE u16 at 0x80)
    h[0x080] = 0x00;
    h[0x081] = 0x01; // 1 stream (video only)

    // Stream entry 0 at 0x82 (16 bytes):
    //   [0] stream_id: 0xE0 = video
    //   [1] private_stream_id: 0x00
    //   [2-3] reserved
    //   [4-7] EPMap offset (BE u32)
    //   [8-11] EPMap entries count (BE u32)
    //   [12] video width in macroblocks (×16)
    //   [13] video height in macroblocks (×16)
    //   [14-15] reserved
    h[0x082] = PES_VIDEO_STREAM_ID; // 0xE0
    h[0x083] = 0x00;
    // EPMap offset (relative) — point to 0xA0
    h[0x086] = 0x00; h[0x087] = 0xA0;
    // EPMap entries = 1
    h[0x08A] = 0x00; h[0x08B] = 0x01;
    // Video dimensions in macroblocks
    h[0x08E] = ((width as u32 + 15) / 16) as u8;
    h[0x08F] = ((height as u32 + 15) / 16) as u8;

    // --- EPMap at 0xA0 (1 entry, 10 bytes each) ---
    // Entry 0: EPIndex=0, EPPicOffset=0, EPPts=0, EPOffset=0
    // (all zeros is fine for streaming)

    h
}

pub struct PsmfMuxer {
    scr_base: u64,
    header_sent: bool,
    first_au_sector: bool,
    header: [u8; PACKET_SIZE],
    mux_rate: u32,
}

impl PsmfMuxer {
    pub fn new(width: u16, height: u16) -> Self {
        let stream_size = 64 * 1024 * 1024;
        Self {
            scr_base: 0,
            header_sent: false,
            first_au_sector: true,
            header: generate_psmf_header(width, height, stream_size),
            mux_rate: 10080,
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

    fn write_pack_header(&self, pkt: &mut [u8; PACKET_SIZE], pos: &mut usize) {
        let p = *pos;
        pkt[p..p + 4].copy_from_slice(&PACK_START_CODE);
        let scr = self.scr_base;
        pkt[p + 4] = 0x44
            | (((scr >> 27) & 0x38) as u8)
            | (((scr >> 28) & 0x03) as u8);
        pkt[p + 5] = ((scr >> 20) & 0xFF) as u8;
        pkt[p + 6] = (((scr >> 12) & 0xF8) as u8)
            | 0x04
            | (((scr >> 13) & 0x03) as u8);
        pkt[p + 7] = ((scr >> 5) & 0xFF) as u8;
        pkt[p + 8] = (((scr & 0x1F) << 3) as u8) | 0x04;
        pkt[p + 9] = 0x01;
        let mr = self.mux_rate;
        pkt[p + 10] = ((mr >> 14) & 0xFF) as u8;
        pkt[p + 11] = ((mr >> 6) & 0xFF) as u8;
        pkt[p + 12] = (((mr & 0x3F) << 2) as u8) | 0x03;
        pkt[p + 13] = 0xF8;
        *pos = p + 14;
    }

    fn write_system_header(&self, pkt: &mut [u8; PACKET_SIZE], pos: &mut usize) {
        let p = *pos;
        pkt[p..p + 4].copy_from_slice(&SYSTEM_HEADER_CODE);
        pkt[p + 4] = 0x00; pkt[p + 5] = 0x0C; // length=12
        let mr = self.mux_rate;
        pkt[p + 6] = ((mr >> 15) & 0x7F) as u8 | 0x80;
        pkt[p + 7] = ((mr >> 7) & 0xFF) as u8;
        pkt[p + 8] = (((mr & 0x7F) << 1) as u8) | 0x01;
        pkt[p + 9] = 0x00;  // audio_bound
        pkt[p + 10] = 0x21; // flags + video_bound=1
        pkt[p + 11] = 0xFF; // marker
        pkt[p + 12] = PES_VIDEO_STREAM_ID;
        pkt[p + 13] = 0xE0; pkt[p + 14] = 0xE6; // video P-STD
        pkt[p + 15] = 0xC0; // audio stream_id
        pkt[p + 16] = 0xC0; pkt[p + 17] = 0x20; // audio P-STD
        *pos = p + 18;
    }

    /// Write a Private Stream 2 (0xBF) navigation packet.
    ///
    /// Real PMF files include this between the system header and the first
    /// video PES packet. The firmware's MPEG-PS demuxer may require it.
    /// Content is stream info + padding (matches real PMF structure).
    fn write_private_stream2(&self, pkt: &mut [u8; PACKET_SIZE], pos: &mut usize) {
        let p = *pos;
        let remaining = PACKET_SIZE - p - 6; // 6 = start code + length
        // Cap at 254 bytes (like real PMF) or whatever fits.
        let data_len = remaining.min(254);

        pkt[p] = 0x00; pkt[p + 1] = 0x00;
        pkt[p + 2] = 0x01; pkt[p + 3] = 0xBF; // Private Stream 2
        pkt[p + 4] = (data_len >> 8) as u8;
        pkt[p + 5] = data_len as u8;

        // First byte: sub-stream ID (0x01 = video info in real PMFs).
        if data_len > 0 {
            pkt[p + 6] = 0x01;
        }
        // Second byte: stream_id reference (0xE0 = video).
        if data_len > 1 {
            pkt[p + 7] = PES_VIDEO_STREAM_ID;
        }
        // Rest is zero-filled (navigation/padding data).

        *pos = p + 6 + data_len;
    }

    /// Wrap H.264 AU into 2048-byte PSMF sectors matching real PMF format.
    pub fn wrap_au(&mut self, au_data: &[u8], pts_90khz: u64) -> Vec<[u8; PACKET_SIZE]> {
        let mut packets = Vec::new();
        let mut au_offset = 0;
        let mut is_first_sector_of_au = true;

        while au_offset < au_data.len() {
            let mut pkt = [0u8; PACKET_SIZE];
            let mut pos = 0;

            self.write_pack_header(&mut pkt, &mut pos);

            if self.first_au_sector && is_first_sector_of_au && packets.is_empty() {
                self.write_system_header(&mut pkt, &mut pos);
                self.write_private_stream2(&mut pkt, &mut pos);
                self.first_au_sector = false;
            }

            // PES header
            pkt[pos] = 0x00; pkt[pos + 1] = 0x00;
            pkt[pos + 2] = 0x01; pkt[pos + 3] = PES_VIDEO_STREAM_ID;
            pos += 4;

            let pes_len = PACKET_SIZE - pos - 2;
            pkt[pos] = (pes_len >> 8) as u8;
            pkt[pos + 1] = pes_len as u8;
            pos += 2;

            if is_first_sector_of_au {
                pkt[pos] = 0x80; pos += 1;     // flags1: marker=10
                pkt[pos] = 0x80; pos += 1;     // flags2: PTS only
                pkt[pos] = 0x05; pos += 1;     // hdr_len=5
                write_pts_dts(&mut pkt[pos..], 0x20, pts_90khz);
                pos += 5;
                is_first_sector_of_au = false;
            } else {
                pkt[pos] = 0x80; pos += 1;     // flags1
                pkt[pos] = 0x00; pos += 1;     // flags2: no PTS
                pkt[pos] = 0x01; pos += 1;     // hdr_len=1
                pkt[pos] = 0xFF; pos += 1;     // pad
            }

            let space = PACKET_SIZE - pos;
            let remaining = au_data.len() - au_offset;
            let copy_len = remaining.min(space);
            pkt[pos..pos + copy_len]
                .copy_from_slice(&au_data[au_offset..au_offset + copy_len]);
            au_offset += copy_len;

            packets.push(pkt);
            self.scr_base += 366;
        }

        packets
    }

    pub fn reset(&mut self, width: u16, height: u16) {
        let stream_size = 64 * 1024 * 1024;
        self.scr_base = 0;
        self.header_sent = false;
        self.first_au_sector = true;
        self.header = generate_psmf_header(width, height, stream_size);
    }
}

fn write_pts_dts(buf: &mut [u8], marker: u8, ts: u64) {
    buf[0] = marker | (((ts >> 29) & 0x0E) as u8) | 0x01;
    buf[1] = ((ts >> 22) & 0xFF) as u8;
    buf[2] = (((ts >> 14) & 0xFE) as u8) | 0x01;
    buf[3] = ((ts >> 7) & 0xFF) as u8;
    buf[4] = (((ts & 0x7F) << 1) as u8) | 0x01;
}
