//! MP3 frame header parsing — bitrate/sample-rate tables and sync-word decoder.

/// MPEG version bitrate tables (kbps). Index: bitrate_index (1..14).
pub(super) const BITRATES_V1_L3: [u32; 15] = [
    0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
];
pub(super) const BITRATES_V2_L3: [u32; 15] =
    [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160];

/// Sample rates by MPEG version. [version_index][srate_index]
pub(super) const SAMPLE_RATES: [[u32; 3]; 4] = [
    [11025, 12000, 8000],  // MPEG 2.5
    [0, 0, 0],             // reserved
    [22050, 24000, 16000], // MPEG 2
    [44100, 48000, 32000], // MPEG 1
];

/// Parsed MP3 frame header.
pub(crate) struct Mp3FrameHeader {
    /// Sample rate in Hz.
    pub(crate) sample_rate: u32,
    /// Bitrate in kbps.
    pub(crate) bitrate: u32,
    /// Number of channels (1 or 2).
    pub(crate) channels: u32,
}

/// Parse an MP3 frame header from 4 bytes starting at a sync position.
pub(crate) fn parse_mp3_header(data: &[u8]) -> Option<Mp3FrameHeader> {
    if data.len() < 4 {
        return None;
    }
    let b1 = data[1];
    let b2 = data[2];
    let b3 = data[3];

    let version_bits = (b1 >> 3) & 0x03;
    let layer_bits = (b1 >> 1) & 0x03;
    let bitrate_idx = (b2 >> 4) & 0x0F;
    let srate_idx = (b2 >> 2) & 0x03;
    let channel_mode = (b3 >> 6) & 0x03;

    if version_bits == 1
        || layer_bits == 0
        || bitrate_idx == 0
        || bitrate_idx == 15
        || srate_idx == 3
    {
        return None;
    }
    // Only Layer III.
    if layer_bits != 1 {
        return None;
    }

    let is_v1 = version_bits == 3;
    let bitrate = if is_v1 {
        BITRATES_V1_L3[bitrate_idx as usize]
    } else {
        BITRATES_V2_L3[bitrate_idx as usize]
    };
    let sample_rate = SAMPLE_RATES[version_bits as usize][srate_idx as usize];
    if sample_rate == 0 || bitrate == 0 {
        return None;
    }
    let channels = if channel_mode == 3 { 1 } else { 2 };

    Some(Mp3FrameHeader {
        sample_rate,
        bitrate,
        channels,
    })
}
