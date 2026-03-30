//! PSMF container wrapper for PSP ringbuffer H.264 decode.
//!
//! Wraps raw H.264 Annex B NAL units into MPEG-PS packs that the PSP's
//! kernel-side demuxer can parse. Each pack is exactly 2048 bytes
//! (matching the ringbuffer packet size).
//!
//! Format derived from byte-level analysis of a real PMF file created
//! by Sony's tools (test.pmf on Memory Stick).

/// Ringbuffer packet size. Every pack must be exactly this many bytes.
pub const PACK_SIZE: usize = 2048;

/// PSMF header size (precedes the MPEG-PS data).
pub const PSMF_HEADER_SIZE: usize = 2048;

/// Pack header size (00 00 01 BA + SCR + mux_rate + stuff).
const PACK_HEADER_SIZE: usize = 14;

/// PES start code + stream ID + length = 6 bytes overhead.
const PES_OVERHEAD: usize = 6;

/// Max payload per video PES in a single pack.
pub const MAX_VIDEO_PES_PAYLOAD: usize = PACK_SIZE - PACK_HEADER_SIZE - PES_OVERHEAD;

/// Video stream ID.
const STREAM_VIDEO: u8 = 0xE0;

/// Padding stream ID.
const STREAM_PADDING: u8 = 0xBE;

/// System header stream ID.
#[allow(dead_code)]
const STREAM_SYSTEM: u8 = 0xBB;

/// Private stream 2 (index data).
const STREAM_PRIVATE2: u8 = 0xBF;

// -----------------------------------------------------------------------
// PSMF header (2048 bytes)
// -----------------------------------------------------------------------

/// Generate the 2048-byte PSMF header.
///
/// Contains stream descriptors for one H.264 video stream (0xE0).
/// The `data_size` is the total MPEG-PS data size that follows.
pub fn generate_psmf_header(
    width: u16,
    height: u16,
    data_size: u32,
) -> [u8; PSMF_HEADER_SIZE] {
    let mut hdr = [0u8; PSMF_HEADER_SIZE];

    // Magic + version
    hdr[0..4].copy_from_slice(b"PSMF");
    hdr[4..8].copy_from_slice(b"0015");

    // Header size (big-endian)
    hdr[8..12].copy_from_slice(&0x0800u32.to_be_bytes());

    // Data size (big-endian)
    hdr[12..16].copy_from_slice(&data_size.to_be_bytes());

    // Stream info at offset 0x50
    // Total duration placeholder (30fps, ~1000 frames)
    let num_frames: u32 = 90000; // placeholder
    hdr[0x50..0x54].copy_from_slice(&0x004Eu32.to_be_bytes());
    hdr[0x54..0x58].copy_from_slice(&1u32.to_be_bytes());
    hdr[0x58..0x5C].copy_from_slice(&num_frames.to_be_bytes());

    // Stream descriptor count + offset
    hdr[0x62] = 0x02; // 2 entries (1 video, 1 marker)
    hdr[0x63] = 0x01;

    // Stream entry 1: video (0xE0)
    hdr[0x82] = STREAM_VIDEO; // stream ID
    hdr[0x83] = 0x00;
    hdr[0x84] = 0x20; // codec: H.264
    hdr[0x85] = 0xFB; // flags

    // Video dimensions (big-endian, at 0x8E-0x8F area)
    // The exact position depends on PSMF version but 0x8E is common
    hdr[0x8E] = (height >> 4) as u8;
    hdr[0x8F] = (((height & 0xF) << 4) | ((width >> 8) & 0xF)) as u8;

    hdr
}

// -----------------------------------------------------------------------
// MPEG-2 SCR encoding
// -----------------------------------------------------------------------

/// Encode an MPEG-2 System Clock Reference (SCR) into 6 bytes.
///
/// SCR is a 42-bit value (33-bit base + 9-bit extension) encoded as:
/// `01xx_x0xx xxxx_xxxx xxxx_xxx0 xxxx_xxxx xxxx_xxx0 xxxx_xxxx`
/// where x bits hold the SCR value and 0/1 are marker bits.
fn encode_scr(scr: u64, out: &mut [u8]) {
    let base = (scr / 300) & 0x1_FFFF_FFFF; // 33-bit base
    let ext = (scr % 300) as u16; // 9-bit extension

    // Byte 0: 01_bbb_0_bb  (b = base bits 32-30, then 29-28)
    out[0] = 0x44
        | (((base >> 27) & 0x38) as u8) // bits 32-30 at positions 5-3
        | (((base >> 28) & 0x03) as u8); // bits 29-28 at positions 1-0

    // Byte 1: bbbb_bbbb (base bits 27-20)
    out[1] = ((base >> 20) & 0xFF) as u8;

    // Byte 2: bbbbb_0_bb (base bits 19-15, marker, bits 14-13)
    out[2] = (((base >> 12) & 0xF8) as u8)
        | 0x04 // marker bit
        | (((base >> 13) & 0x03) as u8);

    // Byte 3: bbbb_bbbb (base bits 12-5)
    out[3] = ((base >> 5) & 0xFF) as u8;

    // Byte 4: bbbbb_0_ee (base bits 4-0, marker, ext bits 8-7)
    out[4] = (((base & 0x1F) << 3) as u8)
        | 0x04 // marker bit
        | ((ext >> 7) as u8 & 0x03);

    // Byte 5: eeeeeee_1 (ext bits 6-0, marker)
    out[5] = ((ext & 0x7F) << 1) as u8 | 0x01;
}

// -----------------------------------------------------------------------
// Pack builder
// -----------------------------------------------------------------------

/// Write a 14-byte MPEG-2 pack header at the given offset.
///
/// `scr` is the System Clock Reference in 27MHz ticks.
/// Returns the offset after the header.
fn write_pack_header(buf: &mut [u8], offset: usize, scr: u64) -> usize {
    // Pack start code
    buf[offset] = 0x00;
    buf[offset + 1] = 0x00;
    buf[offset + 2] = 0x01;
    buf[offset + 3] = 0xBA;

    // SCR (6 bytes, MPEG-2 encoding)
    encode_scr(scr, &mut buf[offset + 4..offset + 10]);

    // Program mux rate (22 bits): use a standard value
    // 0x0186A3 = mux rate marker (100411 * 50 bytes/sec)
    buf[offset + 10] = 0x01;
    buf[offset + 11] = 0x86;
    buf[offset + 12] = 0xA3;

    // Stuff length: 0xF8 = 0 stuffing bytes (bits 2-0 = 0)
    buf[offset + 13] = 0xF8;

    offset + PACK_HEADER_SIZE
}

/// Write a system header (required in first pack only).
/// Returns offset after the system header.
fn write_system_header(buf: &mut [u8], offset: usize) -> usize {
    // System header start code
    buf[offset] = 0x00;
    buf[offset + 1] = 0x00;
    buf[offset + 2] = 0x01;
    buf[offset + 3] = 0xBB;

    // Header length (12 bytes of data after length field)
    buf[offset + 4] = 0x00;
    buf[offset + 5] = 0x0C;

    // Rate bound + flags (copied from real PMF)
    buf[offset + 6] = 0x80;
    buf[offset + 7] = 0xC3;
    buf[offset + 8] = 0x51;

    // Audio/video bound
    buf[offset + 9] = 0x80;
    buf[offset + 10] = 0xF0;
    buf[offset + 11] = 0x7F;

    // Video stream bound
    buf[offset + 12] = 0xB9;
    buf[offset + 13] = 0xE0;
    buf[offset + 14] = 0xFB;

    // Audio stream bound
    buf[offset + 15] = 0xBD;
    buf[offset + 16] = 0xE3;
    buf[offset + 17] = 0x08;

    offset + 18 // 4 (start code) + 2 (length) + 12 (data)
}

/// Write a private_stream_2 index entry (required in first pack).
/// Returns offset after the entry.
fn write_private_stream2_index(buf: &mut [u8], offset: usize) -> usize {
    // Private stream 2 start code
    buf[offset] = 0x00;
    buf[offset + 1] = 0x00;
    buf[offset + 2] = 0x01;
    buf[offset + 3] = STREAM_PRIVATE2;

    // Length: fill rest of pack with index data
    let remaining = PACK_SIZE - offset - PES_OVERHEAD;
    buf[offset + 4] = (remaining >> 8) as u8;
    buf[offset + 5] = remaining as u8;

    // Index data: video stream descriptor
    let data_start = offset + PES_OVERHEAD;
    buf[data_start] = 0x01;     // entry type: video
    buf[data_start + 1] = 0xE0; // stream ID
    // Rest is zero (padding)

    PACK_SIZE // fills to end of pack
}

/// Write a PES padding stream to fill remaining bytes in a pack.
fn write_padding(buf: &mut [u8], offset: usize) {
    let remaining = PACK_SIZE - offset;
    if remaining < PES_OVERHEAD {
        // Not enough room for padding PES, just zero-fill
        for b in &mut buf[offset..PACK_SIZE] {
            *b = 0x00;
        }
        return;
    }

    // Padding start code
    buf[offset] = 0x00;
    buf[offset + 1] = 0x00;
    buf[offset + 2] = 0x01;
    buf[offset + 3] = STREAM_PADDING;

    let pad_len = remaining - PES_OVERHEAD;
    buf[offset + 4] = (pad_len >> 8) as u8;
    buf[offset + 5] = pad_len as u8;

    // Fill with 0xFF (standard padding byte)
    for b in &mut buf[offset + PES_OVERHEAD..PACK_SIZE] {
        *b = 0xFF;
    }
}

// -----------------------------------------------------------------------
// Public API: pack generation
// -----------------------------------------------------------------------

/// Generate the first MPEG-PS pack (with system header + index).
///
/// This must be the first pack fed to the ringbuffer after the PSMF header.
pub fn generate_first_pack(scr: u64) -> [u8; PACK_SIZE] {
    let mut pack = [0u8; PACK_SIZE];
    let off = write_pack_header(&mut pack, 0, scr);
    let off = write_system_header(&mut pack, off);
    write_private_stream2_index(&mut pack, off);
    pack
}

/// Encode PTS (Presentation Time Stamp) into 5 bytes.
fn encode_pts(pts: u64, marker_nibble: u8, out: &mut [u8]) {
    let pts33 = pts & 0x1_FFFF_FFFF;
    out[0] = (marker_nibble << 4)
        | (((pts33 >> 29) & 0x0E) as u8)
        | 0x01; // marker
    out[1] = ((pts33 >> 22) & 0xFF) as u8;
    out[2] = (((pts33 >> 14) & 0xFE) as u8) | 0x01; // marker
    out[3] = ((pts33 >> 7) & 0xFF) as u8;
    out[4] = (((pts33 & 0x7F) << 1) as u8) | 0x01; // marker
}

/// Generate MPEG-PS packs containing one H.264 access unit.
///
/// The AU (in Annex B format) is split across multiple 2048-byte packs.
/// The first pack includes a PES header with PTS. Continuation packs
/// have `hdr_data_len=0` (no PTS).
///
/// Returns the number of packs written to `out_packs`.
pub fn wrap_video_au(
    annex_b: &[u8],
    pts: u64, // 90kHz PTS
    scr: &mut u64, // mutable SCR counter, advanced per pack
    out_packs: &mut Vec<[u8; PACK_SIZE]>,
) -> usize {
    let mut remaining = annex_b;
    let mut first = true;
    let mut count = 0;

    while !remaining.is_empty() || first {
        let mut pack = [0u8; PACK_SIZE];
        let off = write_pack_header(&mut pack, 0, *scr);
        *scr += 27_000_000 / 30; // advance SCR by ~1 frame at 30fps

        // PES header
        pack[off] = 0x00;
        pack[off + 1] = 0x00;
        pack[off + 2] = 0x01;
        pack[off + 3] = STREAM_VIDEO;

        if first {
            // First pack: include PTS (5 bytes) + flags (3 bytes) = 8 extra
            let hdr_data_len = 5u8; // PTS only
            let max_payload = PACK_SIZE - off - PES_OVERHEAD - 3 - hdr_data_len as usize;
            let take = remaining.len().min(max_payload);
            let pes_len = 3 + hdr_data_len as usize + take;

            pack[off + 4] = (pes_len >> 8) as u8;
            pack[off + 5] = pes_len as u8;
            pack[off + 6] = 0x81; // flags: original, PTS present
            pack[off + 7] = 0x80; // PTS flag
            pack[off + 8] = hdr_data_len;

            // PTS (5 bytes)
            encode_pts(pts, 0x02, &mut pack[off + 9..off + 14]);

            // Copy H.264 payload
            let payload_off = off + 9 + hdr_data_len as usize;
            pack[payload_off..payload_off + take]
                .copy_from_slice(&remaining[..take]);
            remaining = &remaining[take..];

            // Pad if needed
            let used = payload_off + take;
            if used < PACK_SIZE {
                write_padding(&mut pack, used);
            }

            first = false;
        } else {
            // Continuation pack: no PTS (hdr_data_len=0)
            let max_payload = PACK_SIZE - off - PES_OVERHEAD - 3;
            let take = remaining.len().min(max_payload);
            let pes_len = 3 + take;

            pack[off + 4] = (pes_len >> 8) as u8;
            pack[off + 5] = pes_len as u8;
            pack[off + 6] = 0x81; // flags
            pack[off + 7] = 0x00; // no PTS
            pack[off + 8] = 0x00; // hdr_data_len = 0

            let payload_off = off + 9;
            pack[payload_off..payload_off + take]
                .copy_from_slice(&remaining[..take]);
            remaining = &remaining[take..];

            let used = payload_off + take;
            if used < PACK_SIZE {
                write_padding(&mut pack, used);
            }
        }

        out_packs.push(pack);
        count += 1;
    }

    count
}
