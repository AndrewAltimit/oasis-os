//! Audio sample mixing utilities.
//!
//! Provides pure functions for mixing PCM audio samples together, applying
//! volume scaling, converting between mono/stereo, and performing sample-rate
//! conversion. These operations are used by audio backends (SDL, PSP, WASM)
//! when combining multiple sources or adjusting audio properties.

/// Mix two i16 PCM sample buffers together with saturation (clipping).
///
/// The output length equals the longer of the two inputs. Where both inputs
/// have samples, they are summed with saturation at `i16::MIN`/`i16::MAX`.
/// Where only one input has samples, those samples pass through unchanged.
pub fn mix_i16(a: &[i16], b: &[i16]) -> Vec<i16> {
    let len = a.len().max(b.len());
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        let sa = a.get(i).copied().unwrap_or(0);
        let sb = b.get(i).copied().unwrap_or(0);
        out.push((sa as i32 + sb as i32).clamp(i16::MIN as i32, i16::MAX as i32) as i16);
    }
    out
}

/// Mix two f32 PCM sample buffers together with saturation at +/- 1.0.
///
/// Same semantics as [`mix_i16`] but for floating-point samples.
pub fn mix_f32(a: &[f32], b: &[f32]) -> Vec<f32> {
    let len = a.len().max(b.len());
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        let sa = a.get(i).copied().unwrap_or(0.0);
        let sb = b.get(i).copied().unwrap_or(0.0);
        out.push((sa + sb).clamp(-1.0, 1.0));
    }
    out
}

/// Mix multiple i16 sources together with saturation.
pub fn mix_multiple_i16(sources: &[&[i16]]) -> Vec<i16> {
    if sources.is_empty() {
        return Vec::new();
    }
    let len = sources.iter().map(|s| s.len()).max().unwrap_or(0);
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        let sum: i32 = sources
            .iter()
            .map(|s| s.get(i).copied().unwrap_or(0) as i32)
            .sum();
        out.push(sum.clamp(i16::MIN as i32, i16::MAX as i32) as i16);
    }
    out
}

/// Apply volume scaling to i16 PCM samples.
///
/// `volume` is in the range 0-100 (0 = silent, 100 = full volume).
/// Values above 100 are clamped.
pub fn apply_volume_i16(samples: &mut [i16], volume: u8) {
    let vol = volume.min(100) as i32;
    for s in samples.iter_mut() {
        *s = ((*s as i32 * vol) / 100) as i16;
    }
}

/// Apply volume scaling to f32 PCM samples.
///
/// `volume` is in the range 0-100 (0 = silent, 100 = full volume).
pub fn apply_volume_f32(samples: &mut [f32], volume: u8) {
    let scale = volume.min(100) as f32 / 100.0;
    for s in samples.iter_mut() {
        *s *= scale;
    }
}

/// Convert mono i16 samples to interleaved stereo by duplicating each sample.
pub fn mono_to_stereo_i16(mono: &[i16]) -> Vec<i16> {
    let mut stereo = Vec::with_capacity(mono.len() * 2);
    for &s in mono {
        stereo.push(s);
        stereo.push(s);
    }
    stereo
}

/// Convert mono f32 samples to interleaved stereo by duplicating each sample.
pub fn mono_to_stereo_f32(mono: &[f32]) -> Vec<f32> {
    let mut stereo = Vec::with_capacity(mono.len() * 2);
    for &s in mono {
        stereo.push(s);
        stereo.push(s);
    }
    stereo
}

/// Convert interleaved stereo i16 samples to mono by averaging L+R channels.
pub fn stereo_to_mono_i16(stereo: &[i16]) -> Vec<i16> {
    stereo
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| ((pair[0] as i32 + pair[1] as i32) / 2) as i16)
        .collect()
}

/// Convert interleaved stereo f32 samples to mono by averaging L+R channels.
pub fn stereo_to_mono_f32(stereo: &[f32]) -> Vec<f32> {
    stereo
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| (pair[0] + pair[1]) * 0.5)
        .collect()
}

/// Perform nearest-neighbor sample rate conversion on i16 samples.
///
/// Converts from `src_rate` Hz to `dst_rate` Hz using simple linear
/// resampling. Suitable for low-quality preview or non-critical audio.
pub fn resample_nearest_i16(samples: &[i16], src_rate: u32, dst_rate: u32) -> Vec<i16> {
    if src_rate == 0 || dst_rate == 0 || samples.is_empty() {
        return Vec::new();
    }
    if src_rate == dst_rate {
        return samples.to_vec();
    }
    let out_len = (samples.len() as u64 * dst_rate as u64 / src_rate as u64) as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src_idx = (i as u64 * src_rate as u64 / dst_rate as u64) as usize;
        out.push(samples[src_idx.min(samples.len() - 1)]);
    }
    out
}

/// Convert i16 samples to f32 (normalized to -1.0..1.0).
pub fn i16_to_f32(samples: &[i16]) -> Vec<f32> {
    samples.iter().map(|&s| s as f32 / 32768.0).collect()
}

/// Convert f32 samples to i16 with saturation.
pub fn f32_to_i16(samples: &[f32]) -> Vec<i16> {
    samples
        .iter()
        .map(|&s| {
            let clamped = s.clamp(-1.0, 1.0);
            (clamped * 32767.0) as i16
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    // ---- mix_i16 tests ----

    #[test]
    fn mix_i16_equal_length() {
        let a = [100i16, 200, -300];
        let b = [50i16, -100, 400];
        let result = mix_i16(&a, &b);
        assert_eq!(result, vec![150, 100, 100]);
    }

    #[test]
    fn mix_i16_different_lengths() {
        let a = [100i16, 200];
        let b = [50i16, -100, 400, 500];
        let result = mix_i16(&a, &b);
        assert_eq!(result, vec![150, 100, 400, 500]);
    }

    #[test]
    fn mix_i16_overflow_saturates_max() {
        let a = [i16::MAX];
        let b = [1000i16];
        let result = mix_i16(&a, &b);
        assert_eq!(result[0], i16::MAX);
    }

    #[test]
    fn mix_i16_underflow_saturates_min() {
        let a = [i16::MIN];
        let b = [-1000i16];
        let result = mix_i16(&a, &b);
        assert_eq!(result[0], i16::MIN);
    }

    #[test]
    fn mix_i16_empty_inputs() {
        let result = mix_i16(&[], &[]);
        assert!(result.is_empty());
    }

    #[test]
    fn mix_i16_one_empty() {
        let a = [100i16, 200];
        let result = mix_i16(&a, &[]);
        assert_eq!(result, vec![100, 200]);
    }

    // ---- mix_f32 tests ----

    #[test]
    fn mix_f32_basic() {
        let a = [0.5f32, -0.3];
        let b = [0.2f32, 0.4];
        let result = mix_f32(&a, &b);
        assert!((result[0] - 0.7).abs() < 1e-6);
        assert!((result[1] - 0.1).abs() < 1e-6);
    }

    #[test]
    fn mix_f32_saturates_positive() {
        let a = [0.8f32];
        let b = [0.5f32];
        let result = mix_f32(&a, &b);
        assert_eq!(result[0], 1.0);
    }

    #[test]
    fn mix_f32_saturates_negative() {
        let a = [-0.8f32];
        let b = [-0.5f32];
        let result = mix_f32(&a, &b);
        assert_eq!(result[0], -1.0);
    }

    // ---- mix_multiple_i16 tests ----

    #[test]
    fn mix_multiple_three_sources() {
        let a: &[i16] = &[100, 200];
        let b: &[i16] = &[50, 50];
        let c: &[i16] = &[25, 25];
        let result = mix_multiple_i16(&[a, b, c]);
        assert_eq!(result, vec![175, 275]);
    }

    #[test]
    fn mix_multiple_empty_sources() {
        let result = mix_multiple_i16(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn mix_multiple_saturation() {
        let a: &[i16] = &[i16::MAX];
        let b: &[i16] = &[i16::MAX];
        let c: &[i16] = &[i16::MAX];
        let result = mix_multiple_i16(&[a, b, c]);
        assert_eq!(result[0], i16::MAX);
    }

    // ---- volume tests ----

    #[test]
    fn apply_volume_i16_full() {
        let mut samples = [1000i16, -2000, 0];
        apply_volume_i16(&mut samples, 100);
        assert_eq!(samples, [1000, -2000, 0]);
    }

    #[test]
    fn apply_volume_i16_half() {
        let mut samples = [1000i16, -2000, 100];
        apply_volume_i16(&mut samples, 50);
        assert_eq!(samples, [500, -1000, 50]);
    }

    #[test]
    fn apply_volume_i16_zero() {
        let mut samples = [1000i16, -2000, 32767];
        apply_volume_i16(&mut samples, 0);
        assert_eq!(samples, [0, 0, 0]);
    }

    #[test]
    fn apply_volume_i16_clamps_above_100() {
        let mut samples = [1000i16];
        apply_volume_i16(&mut samples, 200);
        // Volume clamped to 100, so output unchanged.
        assert_eq!(samples, [1000]);
    }

    #[test]
    fn apply_volume_f32_half() {
        let mut samples = [1.0f32, -0.5, 0.0];
        apply_volume_f32(&mut samples, 50);
        assert!((samples[0] - 0.5).abs() < 1e-6);
        assert!((samples[1] - (-0.25)).abs() < 1e-6);
        assert!((samples[2]).abs() < 1e-6);
    }

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
    fn mono_to_stereo_f32_basic() {
        let mono = [0.5f32, -0.5];
        let stereo = mono_to_stereo_f32(&mono);
        assert_eq!(stereo.len(), 4);
        assert!((stereo[0] - 0.5).abs() < 1e-6);
        assert!((stereo[1] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn stereo_to_mono_i16_basic() {
        let stereo = [100i16, 200, -100, 50];
        let mono = stereo_to_mono_i16(&stereo);
        assert_eq!(mono, vec![150, -25]);
    }

    #[test]
    fn stereo_to_mono_f32_basic() {
        let stereo = [0.5f32, 0.3, -0.4, 0.2];
        let mono = stereo_to_mono_f32(&stereo);
        assert!((mono[0] - 0.4).abs() < 1e-6);
        assert!((mono[1] - (-0.1)).abs() < 1e-6);
    }

    // ---- sample rate conversion tests ----

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
    fn resample_empty() {
        assert!(resample_nearest_i16(&[], 44100, 22050).is_empty());
    }

    // ---- format conversion tests ----

    #[test]
    fn i16_to_f32_conversion() {
        let samples = [0i16, i16::MAX, i16::MIN];
        let result = i16_to_f32(&samples);
        assert!((result[0]).abs() < 1e-6);
        assert!((result[1] - (32767.0 / 32768.0)).abs() < 1e-4);
        assert!((result[2] - (-1.0)).abs() < 1e-4);
    }

    #[test]
    fn f32_to_i16_conversion() {
        let samples = [0.0f32, 1.0, -1.0, 0.5];
        let result = f32_to_i16(&samples);
        assert_eq!(result[0], 0);
        assert_eq!(result[1], 32767);
        assert_eq!(result[2], -32767);
        assert!((result[3] - 16383).abs() <= 1);
    }

    #[test]
    fn f32_to_i16_saturates() {
        let samples = [2.0f32, -2.0];
        let result = f32_to_i16(&samples);
        assert_eq!(result[0], 32767);
        assert_eq!(result[1], -32767);
    }
}
