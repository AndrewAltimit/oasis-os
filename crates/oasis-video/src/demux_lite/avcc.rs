//! AVCDecoderConfigurationRecord (`avcC`) parsing and AVCC to Annex B NAL
//! conversion, shared by `demux_lite` and the symphonia demuxer.

use super::AvccConfig;
use super::LiteError;

/// Annex B start code (4-byte variant).
pub(crate) const ANNEX_B_START: [u8; 4] = [0x00, 0x00, 0x00, 0x01];

/// The fields of an `avcC` box this crate uses, borrowed from its content.
pub(crate) struct AvccRecord<'a> {
    /// Bytes in each NAL length prefix (1-4).
    pub(crate) nal_length_size: usize,
    /// Every sequence parameter set, in record order.
    pub(crate) sps: Vec<&'a [u8]>,
    /// Every picture parameter set, in record order.
    pub(crate) pps: Vec<&'a [u8]>,
}

/// Parse the content of an `avcC` box (AVCDecoderConfigurationRecord,
/// ISO 14496-15): the NAL length size and the length-prefixed SPS / PPS
/// lists. Every length is checked against `data`, so a truncated or
/// hostile record is an error, never a panic or an over-read.
pub(crate) fn parse_avcc_record(data: &[u8]) -> Result<AvccRecord<'_>, LiteError> {
    if data.len() < 7 {
        return Err(LiteError::Parse("avcC too short".into()));
    }
    let nal_length_size = (usize::from(data[4]) & 0x03) + 1;
    let mut rest = &data[5..];
    // The SPS count is the low 5 bits of its byte; the PPS count a full byte.
    let sps = param_sets(&mut rest, 0x1F, "SPS")?;
    let pps = param_sets(&mut rest, 0xFF, "PPS")?;
    Ok(AvccRecord {
        nal_length_size,
        sps,
        pps,
    })
}

/// Read a count byte (masked by `count_mask`) and that many 16-bit
/// length-prefixed parameter sets from the front of `*data`.
fn param_sets<'a>(
    data: &mut &'a [u8],
    count_mask: u8,
    what: &str,
) -> Result<Vec<&'a [u8]>, LiteError> {
    let (&count, mut rest) = data
        .split_first()
        .ok_or_else(|| LiteError::Parse(format!("avcC {what} count missing")))?;
    let count = count & count_mask;
    let mut sets = Vec::with_capacity(usize::from(count));
    for _ in 0..count {
        let (len, tail) = rest
            .split_first_chunk::<2>()
            .ok_or_else(|| LiteError::Parse(format!("avcC {what} truncated")))?;
        let (set, tail) = tail
            .split_at_checked(usize::from(u16::from_be_bytes(*len)))
            .ok_or_else(|| LiteError::Parse(format!("avcC {what} data truncated")))?;
        sets.push(set);
        rest = tail;
    }
    *data = rest;
    Ok(sets)
}

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
