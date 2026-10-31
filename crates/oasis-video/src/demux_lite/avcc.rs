//! AVCC to Annex B NAL conversion logic.

use super::AvccConfig;
use super::LiteError;

/// Annex B start code (4-byte variant).
pub(crate) const ANNEX_B_START: [u8; 4] = [0x00, 0x00, 0x00, 0x01];

/// Convert AVCC-formatted NAL units to Annex B format with start codes.
/// Prepends SPS + PPS on keyframes.
pub fn avcc_to_annex_b(
    data: &[u8],
    avcc: &AvccConfig,
    is_keyframe: bool,
) -> Result<Vec<u8>, LiteError> {
    let mut out = Vec::with_capacity(data.len() + 64);

    // Prepend SPS + PPS on keyframes.
    if is_keyframe {
        for set in [&avcc.sps, &avcc.pps] {
            if !set.is_empty() {
                out.extend_from_slice(&ANNEX_B_START);
                out.extend_from_slice(set);
            }
        }
    }

    append_annex_b(data, avcc.nal_length_size, &mut out)?;
    Ok(out)
}

/// Append the NAL units of an AVCC packet (each prefixed by a big-endian
/// length of `nal_length_size` bytes, 1-4) to `out` as Annex B (each
/// prefixed by a start code).
///
/// Errors on an invalid `nal_length_size` or a NAL that runs past the
/// packet; trailing bytes too short for a length prefix are ignored.
pub(crate) fn append_annex_b(
    data: &[u8],
    nal_length_size: usize,
    out: &mut Vec<u8>,
) -> Result<(), LiteError> {
    if !(1..=4).contains(&nal_length_size) {
        return Err(LiteError::Parse("invalid NAL length size".into()));
    }
    let mut rest = data;
    while let Some((prefix, body)) = rest.split_at_checked(nal_length_size) {
        let nal_len = prefix
            .iter()
            .fold(0usize, |len, &b| (len << 8) | usize::from(b));
        let Some((nal, tail)) = body.split_at_checked(nal_len) else {
            return Err(LiteError::Parse("NAL unit exceeds sample bounds".into()));
        };
        out.extend_from_slice(&ANNEX_B_START);
        out.extend_from_slice(nal);
        rest = tail;
    }
    Ok(())
}
