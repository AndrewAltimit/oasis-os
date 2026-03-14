//! AVCC to Annex B NAL conversion logic.

use super::AvccConfig;
use super::LiteError;

/// Annex B start code (4-byte variant).
pub(super) const ANNEX_B_START: [u8; 4] = [0x00, 0x00, 0x00, 0x01];

/// Convert AVCC-formatted NAL units to Annex B format with start codes.
/// Prepends SPS + PPS on keyframes.
pub fn avcc_to_annex_b(
    data: &[u8],
    avcc: &AvccConfig,
    is_keyframe: bool,
) -> Result<Vec<u8>, LiteError> {
    let nls = avcc.nal_length_size;
    let mut out = Vec::with_capacity(data.len() + 64);

    // Prepend SPS + PPS on keyframes.
    if is_keyframe {
        if !avcc.sps.is_empty() {
            out.extend_from_slice(&ANNEX_B_START);
            out.extend_from_slice(&avcc.sps);
        }
        if !avcc.pps.is_empty() {
            out.extend_from_slice(&ANNEX_B_START);
            out.extend_from_slice(&avcc.pps);
        }
    }

    let mut offset = 0;
    while offset + nls <= data.len() {
        let nal_len = match nls {
            1 => usize::from(data[offset]),
            2 => u16::from_be_bytes([data[offset], data[offset + 1]]) as usize,
            3 => {
                ((data[offset] as usize) << 16)
                    | ((data[offset + 1] as usize) << 8)
                    | (data[offset + 2] as usize)
            },
            4 => u32::from_be_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]) as usize,
            _ => return Err(LiteError::Parse("invalid NAL length size".into())),
        };
        offset += nls;

        match offset.checked_add(nal_len) {
            Some(end) if end <= data.len() => {
                out.extend_from_slice(&ANNEX_B_START);
                out.extend_from_slice(&data[offset..end]);
                offset = end;
            },
            _ => {
                return Err(LiteError::Parse("NAL unit exceeds sample bounds".into()));
            },
        }
    }

    Ok(out)
}
