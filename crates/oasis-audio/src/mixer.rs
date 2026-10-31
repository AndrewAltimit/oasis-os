//! PCM sample conversion helpers used by the SFX engine (`sfx.rs`):
//! mono-to-stereo widening and nearest-neighbour resampling.

/// Convert mono i16 samples to interleaved stereo by duplicating each sample.
pub fn mono_to_stereo_i16(mono: &[i16]) -> Vec<i16> {
    let mut stereo = Vec::with_capacity(mono.len() * 2);
    for &s in mono {
        stereo.push(s);
        stereo.push(s);
    }
    stereo
}

/// Most samples [`resample_nearest_i16`] returns (2^27, as many as
/// [`crate::ogg::MAX_DECODED_SAMPLES`]).
pub const MAX_RESAMPLED_SAMPLES: usize = 1 << 27;

/// Perform nearest-neighbor sample rate conversion on i16 samples.
///
/// Converts from `src_rate` Hz to `dst_rate` Hz by picking the nearest
/// earlier source sample (no interpolation). Suitable for low-quality
/// preview or non-critical audio.
/// Output stops at [`MAX_RESAMPLED_SAMPLES`].
pub fn resample_nearest_i16(samples: &[i16], src_rate: u32, dst_rate: u32) -> Vec<i16> {
    resample_capped(samples, src_rate, dst_rate, MAX_RESAMPLED_SAMPLES)
}

/// [`resample_nearest_i16`] with an explicit output cap.
fn resample_capped(samples: &[i16], src_rate: u32, dst_rate: u32, cap: usize) -> Vec<i16> {
    if src_rate == 0 || dst_rate == 0 || samples.is_empty() {
        return Vec::new();
    }
    if src_rate == dst_rate {
        return samples.to_vec();
    }
    // Capped: a tiny source rate in a hostile file would otherwise ask for
    // billions of output samples (output is truncated past the cap).
    let out_len =
        (samples.len() as u64 * u64::from(dst_rate) / u64::from(src_rate)).min(cap as u64) as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src_idx = (i as u64 * src_rate as u64 / dst_rate as u64) as usize;
        out.push(samples[src_idx.min(samples.len() - 1)]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- mono/stereo conversion tests ----

    #[test]
    fn mono_to_stereo_i16_basic() {
        let mono = [100i16, -200, 300];
        let stereo = mono_to_stereo_i16(&mono);
        assert_eq!(stereo, vec![100, 100, -200, -200, 300, 300]);
    }

    #[test]
    fn mono_to_stereo_i16_empty() {
        assert!(mono_to_stereo_i16(&[]).is_empty());
    }

    #[test]
    fn resample_same_rate() {
        let samples = [100i16, 200, 300, 400];
        let result = resample_nearest_i16(&samples, 44100, 44100);
        assert_eq!(result, samples.to_vec());
    }

    #[test]
    fn resample_double_rate() {
        let samples = [100i16, 200];
        let result = resample_nearest_i16(&samples, 22050, 44100);
        // Each sample roughly doubled.
        assert_eq!(result.len(), 4);
    }

    #[test]
    fn resample_half_rate() {
        let samples = [100i16, 200, 300, 400];
        let result = resample_nearest_i16(&samples, 44100, 22050);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn resample_zero_rate() {
        let samples = [100i16];
        assert!(resample_nearest_i16(&samples, 0, 44100).is_empty());
        assert!(resample_nearest_i16(&samples, 44100, 0).is_empty());
    }

    #[test]
    fn resample_output_is_capped() {
        // 1 Hz -> 44.1 kHz would be 44100 output samples per input sample.
        let out = resample_capped(&[7i16; 4096], 1, 44_100, 1000);
        assert_eq!(out, vec![7; 1000]);
        assert_eq!(resample_capped(&[1, 2], 22_050, 44_100, 1000).len(), 4);
    }

    #[test]
    fn resample_empty() {
        assert!(resample_nearest_i16(&[], 44100, 22050).is_empty());
    }
}
