//! Dedicated audio thread: MP3 playback, SFX mixing, radio streaming,
//! and video AAC hardware decode via `sceAudiocodec`.

use core::sync::atomic::{AtomicU32, Ordering};

use crate::audio::{AudioPlayer, RadioStreamer};
use crate::sfx::SfxEngine;

use super::{
    AUDIO_BITRATE, AUDIO_CHANNELS, AUDIO_DURATION_MS, AUDIO_PAUSED, AUDIO_PLAYING,
    AUDIO_POSITION_MS, AUDIO_QUEUE, AUDIO_SAMPLE_RATE, AudioCmd, RADIO_BUFFERING, RADIO_META_QUEUE,
    RADIO_STREAMING, io_log, io_log_verbose,
};

// ---------------------------------------------------------------------------
// PSP AAC hardware decoder
// ---------------------------------------------------------------------------

/// PSP sceAudiocodec codec type for AAC decoding.
const CODEC_TYPE_AAC: i32 = 0x1003;

/// Maximum number of AAC decoder initialization retries before giving up.
/// Failures may be transient (e.g., temporary EDRAM shortage).
const AAC_INIT_MAX_RETRIES: u32 = 3;

/// Raw PSP AAC hardware decoder using `sceAudiocodec*` syscalls directly.
///
/// Unlike the generic `AudiocodecDecoder`, this sets `buf[10] = sample_rate`
/// before `sceAudiocodecInit` (required for AAC) and does NOT overwrite it
/// during decode (which would break AAC by replacing the sample rate with
/// the source buffer length — an MP3-specific quirk).
struct PspAacDecoder {
    buf: Box<AacCodecBuf>,
    edram_allocated: bool,
}

/// 64-byte-aligned codec buffer for sceAudiocodec (65 words).
#[repr(C, align(64))]
struct AacCodecBuf {
    words: [u32; 65],
}

impl PspAacDecoder {
    /// Initialize the AAC hardware decoder with the given sample rate.
    fn init(sample_rate: u32) -> Result<Self, i32> {
        use psp::sys;

        crate::audio::load_av_modules_once_pub();

        let mut buf = Box::new(AacCodecBuf { words: [0u32; 65] });
        let ptr = buf.words.as_mut_ptr();

        // SAFETY: sceAudiocodec operates on the 64-byte-aligned buffer.
        // Flush cache before each codec call (DMA coherency).
        unsafe {
            sys::sceKernelDcacheWritebackInvalidateAll();
            let ret = sys::sceAudiocodecCheckNeedMem(ptr, CODEC_TYPE_AAC);
            if ret < 0 {
                io_log(&format!("[AUDIO] CheckNeedMem failed: {ret:#010x}"));
                return Err(ret);
            }

            sys::sceKernelDcacheWritebackInvalidateAll();
            let ret = sys::sceAudiocodecGetEDRAM(ptr, CODEC_TYPE_AAC);
            if ret < 0 {
                io_log(&format!("[AUDIO] GetEDRAM failed: {ret:#010x}"));
                return Err(ret);
            }

            // Set sample rate BEFORE init — required for AAC.
            buf.words[10] = sample_rate;

            sys::sceKernelDcacheWritebackInvalidateAll();
            let ret = sys::sceAudiocodecInit(ptr, CODEC_TYPE_AAC);
            if ret < 0 {
                io_log(&format!("[AUDIO] AudiocodecInit failed: {ret:#010x}"));
                sys::sceAudiocodecReleaseEDRAM(ptr);
                return Err(ret);
            }
        }

        Ok(Self {
            buf,
            edram_allocated: true,
        })
    }

    /// Decode one raw AAC frame into PCM. Returns number of bytes consumed.
    fn decode(&mut self, src: &[u8], dst: &mut [i16]) -> Result<usize, i32> {
        use psp::sys;

        let words = &mut self.buf.words;

        // Set source and destination pointers/sizes.
        words[6] = src.as_ptr() as u32;
        words[7] = src.len() as u32;
        words[8] = dst.as_mut_ptr() as u32;
        words[9] = (dst.len() * 2) as u32; // bytes
        // Do NOT touch words[10] — it holds the sample rate set during init.

        // Flush only the relevant D-cache ranges before DMA-based codec
        // decode. A full `sceKernelDcacheWritebackInvalidateAll` thrashes
        // the entire 16KB D-cache (~43x/sec during AAC streaming), hurting
        // all threads. Range-based flushes limit the impact to ~300 bytes.
        // SAFETY: Pointers and sizes are valid for the codec buffer,
        // source data, and destination PCM buffer.
        unsafe {
            let codec_ptr = words.as_ptr() as *const core::ffi::c_void;
            let codec_size = core::mem::size_of_val(words) as u32;
            sys::sceKernelDcacheWritebackInvalidateRange(codec_ptr, codec_size);
            sys::sceKernelDcacheWritebackRange(
                src.as_ptr() as *const core::ffi::c_void,
                src.len() as u32,
            );
        }

        // SAFETY: sceAudiocodecDecode operates on the aligned buffer.
        let ret = unsafe { sys::sceAudiocodecDecode(words.as_mut_ptr(), CODEC_TYPE_AAC) };
        if ret < 0 {
            return Err(ret);
        }

        Ok(words[7] as usize)
    }
}

impl Drop for PspAacDecoder {
    fn drop(&mut self) {
        if self.edram_allocated {
            // SAFETY: Release EDRAM allocated by sceAudiocodecGetEDRAM.
            unsafe {
                psp::sys::sceAudiocodecReleaseEDRAM(self.buf.words.as_mut_ptr());
            }
        }
    }
}

/// AAC: 1024 samples per frame, stereo = 2048 i16.
const AAC_FRAME_SAMPLES: i32 = 1024;

/// Decode a raw AAC frame via PSP hardware codec and output PCM.
///
/// `pcm_buf` is a pre-allocated buffer (at least 2048 i16) reused across
/// calls to avoid per-frame heap allocation on the PSP's slow allocator.
fn decode_aac_frame(
    data: &[u8],
    player: &mut AudioPlayer,
    aac_decoder: &mut Option<PspAacDecoder>,
    aac_sample_rate: u32,
    aac_init_failures: &mut u32,
    pcm_buf: &mut [i16],
) {
    use psp::audio::{AudioChannel, AudioFormat};

    if aac_sample_rate == 0 {
        // Config not received yet — drop frame silently.
        return;
    }

    // Lazily create AAC decoder with retry on transient failures.
    if aac_decoder.is_none() {
        if *aac_init_failures >= AAC_INIT_MAX_RETRIES {
            // Permanently failed after max retries — drop frames silently.
            return;
        }
        io_log(&format!(
            "[AUDIO] creating AAC decoder (rate={aac_sample_rate}, \
             attempt {}/{})",
            *aac_init_failures + 1,
            AAC_INIT_MAX_RETRIES,
        ));
        match PspAacDecoder::init(aac_sample_rate) {
            Ok(dec) => {
                io_log("[AUDIO] AAC decoder init OK");
                *aac_decoder = Some(dec);
                *aac_init_failures = 0;
            },
            Err(e) => {
                *aac_init_failures += 1;
                io_log(&format!(
                    "[AUDIO] AAC decoder init failed: {e:#010x} \
                     (attempt {}/{})",
                    *aac_init_failures, AAC_INIT_MAX_RETRIES,
                ));
                return;
            },
        }
    }

    // Ensure audio channel exists with the correct AAC sample count.
    if player.channel.is_none() {
        io_log("[AUDIO] reserving AAC audio channel...");
        player.channel = AudioChannel::reserve(AAC_FRAME_SAMPLES, AudioFormat::Stereo).ok();
        if player.channel.is_some() {
            io_log("[AUDIO] AAC audio channel reserved OK");
        } else {
            io_log("[AUDIO] AAC audio channel reserve FAILED");
        }
    }

    let Some(decoder) = aac_decoder.as_mut() else {
        // Unreachable: init block above guarantees Some or early-returns.
        return;
    };

    // Zero the pre-allocated PCM buffer before decode.
    let pcm = &mut pcm_buf[..AAC_FRAME_SAMPLES as usize * 2];
    pcm.fill(0);

    static DECODE_COUNT: AtomicU32 = AtomicU32::new(0);
    let count = DECODE_COUNT.fetch_add(1, Ordering::Relaxed);
    if count < 3 {
        io_log_verbose(&format!(
            "[AUDIO] decode #{count} src_len={} src_ptr={:#x}",
            data.len(),
            data.as_ptr() as u32,
        ));
    }

    match decoder.decode(data, pcm) {
        Ok(consumed) => {
            if count < 3 {
                io_log_verbose(&format!(
                    "[AUDIO] decode #{count} OK, consumed={consumed}"
                ));
            }
            if consumed == 0 {
                return;
            }
            if let Some(channel) = &player.channel {
                let _ = channel.output_blocking(0x8000, pcm);
            }
        },
        Err(e) => {
            static ERR_COUNT: AtomicU32 = AtomicU32::new(0);
            let c = ERR_COUNT.fetch_add(1, Ordering::Relaxed);
            if c < 10 {
                io_log(&format!("[AUDIO] AAC decode #{count} error: {e:#010x}"));
            }
        },
    }
}

// ---------------------------------------------------------------------------
// Audio thread main loop
// ---------------------------------------------------------------------------

/// Dedicated audio thread: MP3 playback + SFX mixing + radio streaming.
pub(super) fn audio_thread_fn() {
    let mut player = AudioPlayer::new();
    player.init();

    let mut sfx = SfxEngine::new();
    let mut radio: Option<RadioStreamer> = None;
    let mut aac_decoder: Option<PspAacDecoder> = None;
    let mut aac_sample_rate: u32 = 0;
    let mut aac_init_failures: u32 = 0;

    // Pre-allocate PCM decode buffer once to avoid per-frame heap
    // allocation (~43 allocs/sec at 44.1kHz/1024). Reused across all
    // AAC decode calls.
    let mut aac_pcm_buf = vec![0i16; AAC_FRAME_SAMPLES as usize * 2];

    loop {
        match AUDIO_QUEUE.pop() {
            Some(AudioCmd::LoadAndPlay(path)) => {
                // Stop radio if active.
                if let Some(mut r) = radio.take() {
                    r.stop();
                    RADIO_STREAMING.store(false, Ordering::Relaxed);
                    RADIO_BUFFERING.store(false, Ordering::Relaxed);
                }
                if player.load_and_play(&path) {
                    publish_audio_state(&player);
                } else {
                    AUDIO_PLAYING.store(false, Ordering::Relaxed);
                }
            },
            Some(AudioCmd::LoadAndPlayData(data)) => {
                if let Some(mut r) = radio.take() {
                    r.stop();
                    RADIO_STREAMING.store(false, Ordering::Relaxed);
                    RADIO_BUFFERING.store(false, Ordering::Relaxed);
                }
                if player.load_and_play_owned(data) {
                    publish_audio_state(&player);
                } else {
                    AUDIO_PLAYING.store(false, Ordering::Relaxed);
                }
            },
            Some(AudioCmd::Pause) => {
                if player.is_playing() && !player.is_paused() {
                    player.toggle_pause();
                    AUDIO_PAUSED.store(true, Ordering::Relaxed);
                }
            },
            Some(AudioCmd::Resume) => {
                if player.is_playing() && player.is_paused() {
                    player.toggle_pause();
                    AUDIO_PAUSED.store(false, Ordering::Relaxed);
                }
            },
            Some(AudioCmd::Stop) => {
                player.stop();
                AUDIO_PLAYING.store(false, Ordering::Relaxed);
                AUDIO_PAUSED.store(false, Ordering::Relaxed);
                // Also stop radio if active.
                if let Some(mut r) = radio.take() {
                    r.stop();
                    RADIO_STREAMING.store(false, Ordering::Relaxed);
                    RADIO_BUFFERING.store(false, Ordering::Relaxed);
                }
            },
            Some(AudioCmd::SetVolume(v)) => {
                player.set_volume(v);
                if let Some(r) = &mut radio {
                    r.set_volume(v);
                }
            },
            Some(AudioCmd::PlaySfx(id)) => {
                if let Some(sfx) = &sfx {
                    sfx.play(id);
                }
            },
            Some(AudioCmd::RadioStreamFromFd {
                fd,
                icy_metaint,
                initial_data,
            }) => {
                // Stop file player first.
                player.stop();
                AUDIO_PLAYING.store(false, Ordering::Relaxed);
                AUDIO_PAUSED.store(false, Ordering::Relaxed);
                // Stop any existing radio stream.
                if let Some(mut r) = radio.take() {
                    r.stop();
                }
                // Create new radio streamer with any leftover header data.
                let mut streamer = RadioStreamer::new(fd, icy_metaint);
                if !initial_data.is_empty() {
                    streamer.seed_buffer(&initial_data);
                }
                RADIO_BUFFERING.store(true, Ordering::Relaxed);
                RADIO_STREAMING.store(true, Ordering::Relaxed);
                radio = Some(streamer);
            },
            Some(AudioCmd::RadioStop) => {
                if let Some(mut r) = radio.take() {
                    r.stop();
                }
                RADIO_STREAMING.store(false, Ordering::Relaxed);
                RADIO_BUFFERING.store(false, Ordering::Relaxed);
            },
            Some(AudioCmd::VideoAudioData {
                pcm_i16,
                sample_rate: _,
                channels: _,
            }) => {
                // Stop radio/file playback if still active.
                if let Some(mut r) = radio.take() {
                    r.stop();
                    RADIO_STREAMING.store(false, Ordering::Relaxed);
                    RADIO_BUFFERING.store(false, Ordering::Relaxed);
                }
                if player.is_playing() {
                    player.stop();
                    AUDIO_PLAYING.store(false, Ordering::Relaxed);
                    AUDIO_PAUSED.store(false, Ordering::Relaxed);
                }
                // Output PCM directly to the hardware audio channel.
                player.output_video_pcm(&pcm_i16);
            },
            Some(AudioCmd::VideoAudioAacConfig {
                sample_rate,
                channels: _,
            }) => {
                // Store config for lazy decoder init.
                aac_sample_rate = sample_rate;
                // Reset decoder and retry counter if sample rate changed.
                aac_decoder = None;
                aac_init_failures = 0;
                // Release any existing audio channel so AAC can reserve one
                // with the correct sample count (1024 vs MP3's 1152).
                // Drop the old channel explicitly to free the hardware slot.
                player.channel = None;
                // Stop file player if still active (AAC takes over audio).
                if player.is_playing() {
                    player.stop();
                    AUDIO_PLAYING.store(false, Ordering::Relaxed);
                    AUDIO_PAUSED.store(false, Ordering::Relaxed);
                }
                io_log(&format!("[AUDIO] AAC config: rate={sample_rate}"));
            },
            Some(AudioCmd::VideoAudioAac { data }) => {
                // Decode raw AAC frame via sceAudiocodec and output PCM.
                decode_aac_frame(
                    &data,
                    &mut player,
                    &mut aac_decoder,
                    aac_sample_rate,
                    &mut aac_init_failures,
                    &mut aac_pcm_buf,
                );
            },
            Some(AudioCmd::VideoAudioStop) => {
                // Video playback ended -- flush AAC decoder state.
                aac_decoder = None;
                aac_sample_rate = 0;
                aac_init_failures = 0;
            },
            Some(AudioCmd::Shutdown) => {
                player.stop();
                AUDIO_PLAYING.store(false, Ordering::Relaxed);
                if let Some(mut r) = radio.take() {
                    r.stop();
                }
                RADIO_STREAMING.store(false, Ordering::Relaxed);
                RADIO_BUFFERING.store(false, Ordering::Relaxed);
                break;
            },
            None => {},
        }

        if player.is_playing() && !player.is_paused() {
            player.update();
            AUDIO_POSITION_MS.store(player.position_ms() as u32, Ordering::Relaxed);
            AUDIO_DURATION_MS.store(player.duration_ms() as u32, Ordering::Relaxed);
            if !player.is_playing() {
                AUDIO_PLAYING.store(false, Ordering::Relaxed);
            }
        } else if let Some(r) = &mut radio {
            // Radio streaming: recv data and decode.
            r.recv_data();
            if r.buffering && r.buf_valid >= RadioStreamer::BUFFER_THRESHOLD {
                r.buffering = false;
                RADIO_BUFFERING.store(false, Ordering::Relaxed);
            }
            if !r.buffering {
                r.update(&mut player);
                // Push ICY metadata to main thread.
                if let Some(title) = r.take_meta() {
                    let _ = RADIO_META_QUEUE.push(title);
                }
            }
            if r.is_error() {
                let _ = RADIO_META_QUEUE.push(String::from("[Stream error]"));
                radio = None;
                RADIO_STREAMING.store(false, Ordering::Relaxed);
                RADIO_BUFFERING.store(false, Ordering::Relaxed);
            }
        } else {
            // Sleep when idle. During AAC playback the audio thread must
            // wake frequently to pop frames, but a short sleep (1ms)
            // prevents a CPU-burning busy loop that can crash the PSP.
            let sleep_us = if aac_sample_rate > 0 { 1_000 } else { 10_000 };
            // SAFETY: sceKernelDelayThread sleeps the current thread.
            unsafe { psp::sys::sceKernelDelayThread(sleep_us) };
        }

        // Pump SFX mixer (separate hardware channel, short blocking).
        if let Some(sfx) = &mut sfx {
            sfx.pump();
        }
    }
}

/// Publish audio player state to shared atomics after a load_and_play.
fn publish_audio_state(player: &AudioPlayer) {
    AUDIO_SAMPLE_RATE.store(player.sample_rate, Ordering::Relaxed);
    AUDIO_BITRATE.store(player.bitrate, Ordering::Relaxed);
    AUDIO_CHANNELS.store(player.channels, Ordering::Relaxed);
    AUDIO_POSITION_MS.store(0, Ordering::Relaxed);
    AUDIO_DURATION_MS.store(0, Ordering::Relaxed);
    AUDIO_PAUSED.store(false, Ordering::Relaxed);
    // Set playing LAST so readers see consistent metadata first.
    AUDIO_PLAYING.store(true, Ordering::Relaxed);
}
