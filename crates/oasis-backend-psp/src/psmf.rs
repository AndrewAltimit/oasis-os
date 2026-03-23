//! PSMF (PSP Movie Format) header generator and MPEG-PS stream wrapper.
//!
//! Format derived from hex-dumping a real PMF file created by UMD Stream
//! Composer via ffmpeg (MPEG-2 PS with H.264) + mps2pmf.
//!
//! Each 2048-byte sector contains exactly:
//!   - Pack header (14 bytes): 00 00 01 BA + SCR + mux_rate + stuffing
//!   - PES video packet filling the rest of the sector
//!
//! First AU sector: PES with PTS (flags=0x80, hdr_len includes PTS)
//! Continuation sectors: PES with NO PTS/DTS (flags=0x00, hdr_len=1, pad=0xFF)
//!
//! PES length ALWAYS covers the entire remaining sector space. No padding
//! stream — the PES payload includes any trailing bytes.

/// PSMF packet size (matches ringbuffer packet size).
pub const PACKET_SIZE: usize = 2048;

const PACK_START_CODE: [u8; 4] = [0x00, 0x00, 0x01, 0xBA];
const SYSTEM_HEADER_CODE: [u8; 4] = [0x00, 0x00, 0x01, 0xBB];
const PES_VIDEO_STREAM_ID: u8 = 0xE0;

/// Generate a PSMF header matching the format from pmftools/mps2pmf.
pub fn generate_psmf_header(width: u16, height: u16, stream_size: u32) -> [u8; PACKET_SIZE] {
    let mut h = [0u8; PACKET_SIZE];

    h[0x000] = b'P';
    h[0x001] = b'S';
    h[0x002] = b'M';
    h[0x003] = b'F';
    // Version "0012" (matches pmftools template)
    h[0x004] = b'0';
    h[0x005] = b'0';
    h[0x006] = b'1';
    h[0x007] = b'2';
    // Header size = 0x800
    h[0x00A] = 0x08;
    // Stream data size (big-endian)
    h[0x00C] = (stream_size >> 24) as u8;
    h[0x00D] = (stream_size >> 16) as u8;
    h[0x00E] = (stream_size >> 8) as u8;
    h[0x00F] = stream_size as u8;
    // Stream info table offset → 0x50
    h[0x047] = 0x50;
    // Number of streams = 1
    h[0x051] = 0x01;
    // Stream 0: AVC video
    h[0x052] = 0x00; // type
    h[0x053] = 0x00; // channel
    h[0x054] = 0x42; // Baseline profile
    h[0x055] = 0x1F; // Level 3.1
    h[0x056] = (width >> 8) as u8;
    h[0x057] = width as u8;
    h[0x058] = (height >> 8) as u8;
    h[0x059] = height as u8;
    // Last timestamp (90kHz, ~10 min)
    let last_ts: u32 = 54_000_000;
    h[0x05C] = (last_ts >> 24) as u8;
    h[0x05D] = (last_ts >> 16) as u8;
    h[0x05E] = (last_ts >> 8) as u8;
    h[0x05F] = last_ts as u8;
    // EPMap offset → 0x60, 1 entry
    h[0x04B] = 0x60;
    h[0x061] = 0x01;
    // Dimensions in macroblock units at 0x8E/0x8F
    h[0x08E] = ((width as u32 + 15) / 16) as u8;
    h[0x08F] = ((height as u32 + 15) / 16) as u8;

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

    /// Write MPEG-2 PS pack header (14 bytes) into pkt at pos.
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
        pkt[p + 13] = 0xF8; // no stuffing

        *pos = p + 14;
    }

    /// Write system header matching the real PMF format.
    /// Only included in the very first MPEG-PS sector.
    fn write_system_header(&self, pkt: &mut [u8; PACKET_SIZE], pos: &mut usize) {
        let p = *pos;
        pkt[p..p + 4].copy_from_slice(&SYSTEM_HEADER_CODE);

        // Header length = 12 (matches real PMF)
        pkt[p + 4] = 0x00;
        pkt[p + 5] = 0x0C;

        // Rate bound (3 bytes, with markers)
        let mr = self.mux_rate;
        pkt[p + 6] = ((mr >> 15) & 0x7F) as u8 | 0x80;
        pkt[p + 7] = ((mr >> 7) & 0xFF) as u8;
        pkt[p + 8] = (((mr & 0x7F) << 1) as u8) | 0x01;

        // audio_bound=0, fixed=0, CSPS=0
        pkt[p + 9] = 0x00;
        // audio_lock=0, video_lock=0, marker=1, video_bound=1
        pkt[p + 10] = 0x21;
        // Marker byte (matches real PMF pattern)
        pkt[p + 11] = 0xFF;

        // Video stream entry: stream_id + P-STD buffer
        pkt[p + 12] = PES_VIDEO_STREAM_ID;
        pkt[p + 13] = 0xE0; // '11' + scale=1 + buf_bound[12:8]=0
        pkt[p + 14] = 0xE6; // buf_bound[7:0]=0xE6

        // Audio stream entry: stream_id 0xC0 + P-STD buffer
        pkt[p + 15] = 0xC0;
        pkt[p + 16] = 0xC0; // '11' + scale=0 + buf_bound
        pkt[p + 17] = 0x20;

        *pos = p + 18; // 4 + 2 + 12 = 18
    }

    /// Wrap a single H.264 AU into 2048-byte PSMF sectors.
    ///
    /// Format matches real PMF files:
    /// - First AU sector: pack + system_header(first ever) + PES(PTS) + data
    /// - Continuation sectors: pack + PES(no PTS, hdr_len=1) + data
    /// - PES length always fills the entire remaining sector
    pub fn wrap_au(&mut self, au_data: &[u8], pts_90khz: u64) -> Vec<[u8; PACKET_SIZE]> {
        let mut packets = Vec::new();
        let mut au_offset = 0;
        let mut is_first_sector_of_au = true;

        while au_offset < au_data.len() {
            let mut pkt = [0u8; PACKET_SIZE];
            let mut pos = 0;

            // Pack header (14 bytes)
            self.write_pack_header(&mut pkt, &mut pos);

            // System header (first sector ever only)
            if self.first_au_sector && is_first_sector_of_au && packets.is_empty() {
                self.write_system_header(&mut pkt, &mut pos);
                self.first_au_sector = false;
            }

            // PES header
            pkt[pos] = 0x00;
            pkt[pos + 1] = 0x00;
            pkt[pos + 2] = 0x01;
            pkt[pos + 3] = PES_VIDEO_STREAM_ID;
            pos += 4;

            // PES length fills entire remaining sector
            // Length = sector_remaining - 2 (length field itself)
            let pes_len = PACKET_SIZE - pos - 2;
            pkt[pos] = (pes_len >> 8) as u8;
            pkt[pos + 1] = pes_len as u8;
            pos += 2;

            if is_first_sector_of_au {
                // First sector: PES with PTS
                // Flags byte 1: marker=10, no scrambling, no priority, no alignment
                pkt[pos] = 0x80;
                pos += 1;
                // Flags byte 2: PTS=1, DTS=0, no other flags
                pkt[pos] = 0x80;
                pos += 1;
                // PES header data length: 5 (PTS only)
                pkt[pos] = 0x05;
                pos += 1;
                // PTS (5 bytes, marker=0x20 for PTS-only)
                write_pts_dts(&mut pkt[pos..], 0x20, pts_90khz);
                pos += 5;

                is_first_sector_of_au = false;
            } else {
                // Continuation sector: PES with NO PTS/DTS
                // Flags byte 1: marker=10
                pkt[pos] = 0x80;
                pos += 1;
                // Flags byte 2: no PTS, no DTS, no flags
                pkt[pos] = 0x00;
                pos += 1;
                // PES header data length: 1 (just a pad byte)
                pkt[pos] = 0x01;
                pos += 1;
                // Pad byte (matches real PMF)
                pkt[pos] = 0xFF;
                pos += 1;
            }

            // Copy AU data into remaining sector space
            let space = PACKET_SIZE - pos;
            let remaining = au_data.len() - au_offset;
            let copy_len = remaining.min(space);
            pkt[pos..pos + copy_len]
                .copy_from_slice(&au_data[au_offset..au_offset + copy_len]);
            au_offset += copy_len;
            // Remaining bytes stay zero (part of PES payload per the length)

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

/// Write a PTS or DTS value in MPEG-2 PES format (5 bytes).
fn write_pts_dts(buf: &mut [u8], marker: u8, ts: u64) {
    buf[0] = marker | (((ts >> 29) & 0x0E) as u8) | 0x01;
    buf[1] = ((ts >> 22) & 0xFF) as u8;
    buf[2] = (((ts >> 14) & 0xFE) as u8) | 0x01;
    buf[3] = ((ts >> 7) & 0xFF) as u8;
    buf[4] = (((ts & 0x7F) << 1) as u8) | 0x01;
}
