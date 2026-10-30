//! NAL-based H.264 decoder — a thin wrapper around `psp::mpeg::AvcDecoder`.

use core::sync::atomic::{AtomicU32, Ordering};

use psp::mpeg::{AvcDecoder, AvcNal};

use super::sps::parse_sps_info;
use super::{
    DecodedFrame, MPEG_INTERNAL_SEMA, StreamFrame, VIDEO_TEX_PTR, modules, now_us,
    video_texture_slice, vlog,
};

/// NAL-based H.264 decoder using the high-level `psp::mpeg::AvcDecoder`.
///
/// Adds OASIS-specific logging and SPS/PPS extraction on top of the
/// reusable rust-psp abstraction.
pub(super) struct NalDecoder {
    /// Wrapped in `Option` so `Drop` can move the decoder back into
    /// `PERSISTENT_DECODER` for reuse on the next channel tune. Always
    /// `Some` for the lifetime of a valid `NalDecoder`.
    decoder: Option<AvcDecoder>,
    sps: Vec<u8>,
    pps: Vec<u8>,
    first_frame: bool,
}

/// Persistent `AvcDecoder` kept alive across channel switches.
///
/// `rust-psp` skips `sceMpegDelete` in `AvcDecoder::Drop` to avoid an
/// intermittent firmware crash, which leaves the sceMpeg instance leaked.
/// On the next `sceMpegCreate`, mpeg_vsh370 returns 0x80628002
/// (`SCE_ERROR_MPEG_NO_MEMORY`) because the firmware still tracks the
/// prior instance. Reusing one decoder for every tune sidesteps the
/// Delete/Create cycle entirely.
///
/// On stream end, `Drop for NalDecoder` parks the decoder back here.
/// On the next tune, `NalDecoder::try_init` takes it back. The first
/// allocation is at the actual stream dimensions; subsequent streams
/// with ≤480p content reuse it (CSC stride is fixed at 512 for any
/// `width≤480`, so the same instance handles 320×240, 336×240, 480×272,
/// etc. — only `is_first_frame=true` and `flush()` are needed to reset
/// pic_num between streams).
///
/// Touched only on the video thread (`take_parked` / `park`), so there is
/// no cross-thread access.
static mut PERSISTENT_DECODER: Option<AvcDecoder> = None;

/// Take the parked decoder, if any.
fn take_parked() -> Option<AvcDecoder> {
    // SAFETY: only the video thread touches PERSISTENT_DECODER. Raw
    // pointer ops avoid creating a `&mut` to the `static mut`, which Rust
    // 2024 forbids (`static_mut_refs`).
    unsafe { core::ptr::replace(core::ptr::addr_of_mut!(PERSISTENT_DECODER), None) }
}

/// Park `dec` for the next tune. A previously parked decoder (there
/// should be none) is dropped.
fn park(dec: AvcDecoder) {
    // SAFETY: as in `take_parked`.
    let _ = unsafe { core::ptr::replace(core::ptr::addr_of_mut!(PERSISTENT_DECODER), Some(dec)) };
}

impl NalDecoder {
    /// Initialize the NAL decoder from the first keyframe.
    pub(super) fn try_init(first_frame: &StreamFrame) -> Result<Self, String> {
        vlog("[VIDEO] NalDecoder::try_init");
        crate::audio::load_av_modules_once_pub();
        modules::load_mpeg_vsh_module();

        // SPS/PPS from MP4 avcC atom (always present on keyframes).
        let (Some(sps), Some(pps)) = (&first_frame.avcc_sps, &first_frame.avcc_pps) else {
            return Err("no SPS/PPS on first keyframe".to_string());
        };
        let (sps, pps) = (sps.clone(), pps.clone());

        if sps.len() >= 4 {
            vlog(&format!(
                "[VIDEO] NAL: SPS={} PPS={} profile={:#x} level={:#x}",
                sps.len(),
                pps.len(),
                sps[1],
                sps[3]
            ));
        }

        // Parse dimensions and ref frame count from the raw SPS NAL.
        let sps_info = parse_sps_info(&sps);
        let (width, height) = sps_info
            .as_ref()
            .map_or((480, 272), |i| (i.width, i.height));
        let max_ref_frames = sps_info.as_ref().map_or(4, |i| i.max_ref_frames);
        let dpb_total = (width * height * 3 / 2) as usize * (max_ref_frames as usize + 1);
        vlog(&format!(
            "[VIDEO] NAL: {width}x{height} refs={max_ref_frames} \
             dpb={dpb_total}B (2MB workspace={})",
            if dpb_total > 0x20_0000 {
                "OVERFLOW"
            } else {
                "ok"
            }
        ));

        // Reuse the persistent decoder if available; otherwise allocate a
        // fresh one. See `PERSISTENT_DECODER` for why we never drop.
        let decoder = match take_parked() {
            Some(mut dec) => {
                vlog(&format!(
                    "[VIDEO] NAL: reusing persistent decoder ({}x{}) for {width}x{height} stream",
                    dec.width(),
                    dec.height(),
                ));
                // Reset stream-specific state so the ME treats this as a
                // fresh stream start. `flush()` zeroes pic_num.
                dec.flush();
                dec
            },
            None => {
                let dec =
                    AvcDecoder::new(width, height).map_err(|e| format!("AvcDecoder::new: {e}"))?;
                vlog(&format!(
                    "[VIDEO] NAL: created persistent decoder ({width}x{height}), ddr={:#x}",
                    dec.ddr_top(),
                ));
                dec
            },
        };

        // CSC writes directly into the pre-allocated GU video texture
        // (see `VIDEO_TEX_PTR`), sized for the largest ≤480p frame
        // (512 × 256 × 4 = 524288 bytes) and reused across streams. A
        // separate frame buffer would not fit: by this point the
        // persistent sceMpeg DDR workspace has eaten ~6.5 MB of partition
        // memory, leaving roughly 1 MB free.
        if VIDEO_TEX_PTR.load(Ordering::Acquire).is_null() {
            // Park the decoder before bailing out: `NalDecoder::Drop` is
            // the only other place that re-parks, and leaking it would make
            // the next tune's `AvcDecoder::new` fail with 0x80628002.
            park(decoder);
            return Err("GU video texture not pre-allocated (TV Guide tune-press \
                 path is responsible for calling PspBackend::alloc_video_texture)"
                .to_string());
        }

        // Publish the internal semaphore ID for the main-thread watchdog.
        if let Some(sema) = decoder.internal_sema_id() {
            MPEG_INTERNAL_SEMA.store(sema.0, Ordering::Release);
            vlog(&format!("[VIDEO] NAL: internal sema={:#x}", sema.0));
        }

        Ok(Self {
            decoder: Some(decoder),
            sps,
            pps,
            first_frame: true,
        })
    }

    /// Decode one H.264 access unit (raw AVCC format) straight into the GU
    /// video texture. Returns:
    /// - `Ok(Some(frame))` — frame decoded successfully
    /// - `Ok(None)` — no picture yet (B-frame reordering)
    /// - `Err(())` — decode error
    pub(super) fn decode(
        &mut self,
        avcc_data: &[u8],
        avcc_prefix: u8,
    ) -> Result<Option<DecodedFrame>, ()> {
        if avcc_data.is_empty() {
            return Ok(None);
        }

        static DECODE_COUNT: AtomicU32 = AtomicU32::new(0);
        let call_num = DECODE_COUNT.fetch_add(1, Ordering::Relaxed);
        let verbose = call_num < 10;

        let is_first = self.first_frame;
        self.first_frame = false;

        let nal = AvcNal {
            sps: &self.sps,
            pps: &self.pps,
            data: avcc_data,
            prefix_size: avcc_prefix as i32,
            is_first_frame: is_first,
        };

        // Zero-copy CSC: write directly into the GU video texture buffer.
        // The texture is pre-allocated by the main thread when the user
        // presses X to tune (`PspBackend::alloc_video_texture` →
        // `set_video_texture_buffer`). Single-buffered to fit memory
        // alongside the persistent sceMpeg DDR workspace.
        // SAFETY: video thread is the sole writer; main thread reads via
        // GU DMA, which fences on `sceGuFinish`.
        let Some(dst) = (unsafe { video_texture_slice() }) else {
            if verbose {
                vlog("[VIDEO] NAL: no texture buffer registered, skipping");
            }
            return Err(());
        };

        let t0 = now_us();
        let Some(dec) = self.decoder.as_mut() else {
            // Unreachable: only `Drop` takes the decoder.
            return Err(());
        };
        let result = dec.decode_csc_direct(&nal, dst);
        match result {
            Ok(true) => {
                if verbose {
                    let dt_ms = (now_us() - t0) / 1000;
                    vlog(&format!("[VIDEO] NAL: FRAME DECODED! ({dt_ms}ms)"));
                }
                Ok(Some(DecodedFrame {
                    width: dec.width(),
                    height: dec.height(),
                    stride: dec.stride(),
                }))
            },
            Ok(false) => {
                if verbose {
                    vlog("[VIDEO] NAL: no picture yet (reordering)");
                }
                Ok(None)
            },
            Err(e) => {
                if call_num < 50 {
                    vlog(&format!("[VIDEO] NAL: decode error: {e}"));
                }
                Err(())
            },
        }
    }
}

impl Drop for NalDecoder {
    fn drop(&mut self) {
        // Park the decoder for the next stream instead of letting
        // `AvcDecoder::Drop` run. See PERSISTENT_DECODER for the firmware
        // bug we're working around.
        if let Some(dec) = self.decoder.take() {
            park(dec);
        }
    }
}
