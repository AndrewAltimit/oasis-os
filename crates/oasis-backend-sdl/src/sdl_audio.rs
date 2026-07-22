//! SDL3 audio backend for OASIS_OS.
//!
//! Implements `AudioBackend` using SDL3's audio stream API and the `rmp3` MP3
//! decoder.  Incoming MP3 data (from `feed_data` or `load_track`) is decoded
//! to PCM i16 samples via `rmp3::RawDecoder`, volume-scaled, and pushed to an
//! `sdl3::audio::AudioStream` bound to the default playback device.
//!
//! The SDL3 audio device and stream are opened lazily on the first decoded
//! frame so that the sample rate and channel count come from the actual stream.
//!
//! Note: Real audio output requires hardware. In CI (Docker without audio
//! devices), `init()` may fail gracefully. The `NullAudioBackend` in
//! `oasis-core` is used for testing without hardware.

use std::collections::HashMap;

use oasis_core::backend::{AudioBackend, AudioTrackId};
use oasis_core::error::{OasisError, Result};

/// High-water mark: stop accepting PCM once the SDL3 stream holds this much.
/// ~9 seconds at 44.1 kHz stereo i16.
const MAX_QUEUE_BYTES: u32 = 1_600_000;

/// Low-water mark: resume accepting PCM only after the queue drains below
/// this level. The wide hysteresis (5 seconds of playback between refills)
/// avoids the old "pull a chunk, back-pressure, pull a chunk, back-pressure"
/// oscillation at ~4 Hz — that tight cycle interacted badly with SDL3 /
/// PulseAudio's timing and produced an audible pause-and-catch-up artifact.
const REFILL_QUEUE_BYTES: u32 = 700_000;

/// Fixed output sample rate we resample every MP3 to before handing it
/// to SDL. 48 kHz is the native rate on essentially all modern Linux
/// audio hardware (USB/HDMI/onboard), so opening the device here means
/// the ALSA/PipeWire chain performs no rate conversion at all —
/// removing the last periodic stutter source that surfaced once we
/// fixed the mono-upmix one.
const OUTPUT_SAMPLE_RATE: u32 = 48_000;

/// Stateful linear resampler for stereo i16 audio. Internal state
/// (the last input frame and fractional read position) survives
/// across `process` calls so chunk boundaries don't introduce
/// audible artifacts. Intended use: one instance per streaming
/// session, reset when the source rate changes.
struct LinearResampler {
    out_rate: u32,
    in_rate: u32,
    prev_l: i16,
    prev_r: i16,
    /// Position of the next output sample, in input-frame units,
    /// measured from the start of the current input chunk. Can be
    /// negative to indicate interpolation with `prev_*` from the
    /// previous chunk.
    phase: f64,
    has_prev: bool,
}

impl LinearResampler {
    fn new(out_rate: u32) -> Self {
        Self {
            out_rate,
            in_rate: 0,
            prev_l: 0,
            prev_r: 0,
            phase: 0.0,
            has_prev: false,
        }
    }

    /// Re-seat the resampler for a new source rate. Always clears
    /// `has_prev` so we don't interpolate between unrelated streams.
    fn set_input_rate(&mut self, in_rate: u32) {
        self.in_rate = in_rate;
        self.prev_l = 0;
        self.prev_r = 0;
        self.phase = 0.0;
        self.has_prev = false;
    }

    /// Resample `input` (stereo i16 interleaved) and append the
    /// stereo i16 result at `out_rate` to `dst`.
    fn process(&mut self, input: &[i16], dst: &mut Vec<i16>) {
        let in_frames = input.len() / 2;
        if in_frames == 0 || self.in_rate == 0 {
            return;
        }
        // Pass-through when rates match.
        if self.in_rate == self.out_rate {
            dst.extend_from_slice(input);
            self.prev_l = input[input.len() - 2];
            self.prev_r = input[input.len() - 1];
            self.has_prev = true;
            return;
        }

        let step = self.in_rate as f64 / self.out_rate as f64;
        let mut p = self.phase;
        // Emit outputs while we have TWO input samples straddling p.
        // That means we need floor(p)+1 to be a valid in-chunk index,
        // i.e. p < in_frames - 1.
        let limit = in_frames as f64 - 1.0;
        while p < limit {
            let idx0 = p.floor() as i32;
            let frac = p - idx0 as f64;
            let (l0, r0) = if idx0 == -1 && self.has_prev {
                (self.prev_l, self.prev_r)
            } else if idx0 >= 0 {
                let i = idx0 as usize;
                (input[2 * i], input[2 * i + 1])
            } else {
                // Phase walked past what we have context for — stop
                // and wait for more input.
                break;
            };
            let idx1 = idx0 + 1;
            let (l1, r1) = if idx1 == -1 && self.has_prev {
                (self.prev_l, self.prev_r)
            } else if idx1 >= 0 && (idx1 as usize) < in_frames {
                let i = idx1 as usize;
                (input[2 * i], input[2 * i + 1])
            } else {
                break;
            };
            let l = l0 as f64 + frac * (l1 as f64 - l0 as f64);
            let r = r0 as f64 + frac * (r1 as f64 - r0 as f64);
            dst.push(l.clamp(-32768.0, 32767.0) as i16);
            dst.push(r.clamp(-32768.0, 32767.0) as i16);
            p += step;
        }

        // Normalize phase so it's relative to the START of the next
        // chunk. The last frame of this chunk becomes index -1 there.
        self.phase = p - in_frames as f64;
        self.prev_l = input[input.len() - 2];
        self.prev_r = input[input.len() - 1];
        self.has_prev = true;
    }
}

/// SDL3-based audio backend with real MP3 decoding.
pub struct SdlAudioBackend {
    /// Whether the audio subsystem has been initialized.
    initialized: bool,
    /// SDL3 audio stream owner (owns both device and stream).
    stream_owner: Option<sdl3::audio::AudioStreamOwner>,
    /// SDL3 audio subsystem (kept alive for device lifetime).
    audio_subsystem: Option<sdl3::AudioSubsystem>,
    /// MP3 decoder state.
    decoder: rmp3::RawDecoder,
    /// Pending MP3 bytes not yet decoded (streaming track).
    mp3_buffer: Vec<u8>,
    /// Reusable staging buffer for decoded PCM (avoids per-call allocation).
    pcm_staging: Vec<i16>,
    /// Reusable output buffer for resampled PCM at OUTPUT_SAMPLE_RATE.
    pcm_resampled: Vec<i16>,
    /// Keeps the streaming session's resampler state across feed
    /// boundaries, so the device always sees a single 48 kHz stream
    /// even when the source rate (22.05/44.1/48 kHz …) varies.
    resampler: LinearResampler,
    /// Dedicated stream for one-shot UI sound effects. SDL mixes it with
    /// the music stream at the device level, so short samples play over
    /// whatever is queued without touching the music pipeline.
    sfx_stream: Option<sdl3::audio::AudioStreamOwner>,
    /// Reusable staging buffer for volume-scaled SFX PCM.
    sfx_scratch: Vec<i16>,
    /// Loaded non-streaming tracks (raw MP3 data).
    tracks: HashMap<u64, Vec<u8>>,
    /// Next track ID to assign.
    next_id: u64,
    /// Currently playing track ID (if any).
    current_track: Option<u64>,
    /// Active streaming track ID (if any).
    stream_track: Option<u64>,
    /// Current volume (0-100).
    volume: u8,
    /// Whether playback is active.
    playing: bool,
    /// Whether playback is paused.
    paused: bool,
    /// Sample rate detected from first decoded frame.
    sample_rate: i32,
    /// Channel count detected from first decoded frame.
    channels: usize,
    /// Total PCM samples queued (for position_ms calculation).
    samples_queued: u64,
    /// Back-pressure hysteresis. `true` while we want more PCM (queue is
    /// being filled toward MAX_QUEUE_BYTES); flips to `false` at the high
    /// mark and back to `true` at REFILL_QUEUE_BYTES. Interior mutability
    /// so `streaming_can_accept` can update it through `&self`.
    accepting: std::cell::Cell<bool>,
}

impl SdlAudioBackend {
    /// Create a new SDL3 audio backend (not yet initialized).
    pub fn new() -> Self {
        Self {
            initialized: false,
            stream_owner: None,
            audio_subsystem: None,
            decoder: rmp3::RawDecoder::new(),
            mp3_buffer: Vec::new(),
            pcm_staging: Vec::new(),
            pcm_resampled: Vec::new(),
            resampler: LinearResampler::new(OUTPUT_SAMPLE_RATE),
            sfx_stream: None,
            sfx_scratch: Vec::new(),
            tracks: HashMap::new(),
            next_id: 0,
            current_track: None,
            stream_track: None,
            volume: 80,
            playing: false,
            paused: false,
            sample_rate: 0,
            channels: 0,
            samples_queued: 0,
            accepting: std::cell::Cell::new(true),
        }
    }

    /// Open a playback stream on the default device with the given format.
    /// Used by both the music stream (`open_device`) and the SFX stream.
    fn create_stream(
        &self,
        sample_rate: i32,
        channels: u8,
    ) -> Result<sdl3::audio::AudioStreamOwner> {
        let audio = self
            .audio_subsystem
            .as_ref()
            .ok_or_else(|| OasisError::Backend("audio subsystem not available".into()))?;

        let spec = sdl3::audio::AudioSpec::new(
            Some(sample_rate),
            Some(channels as i32),
            Some(sdl3::audio::AudioFormat::S16LE),
        );
        let device = audio
            .open_playback_device(&spec)
            .map_err(|e| OasisError::Backend(e.to_string().into()))?;
        let stream = device
            .open_device_stream(Some(&spec))
            .map_err(|e| OasisError::Backend(e.to_string().into()))?;
        stream
            .resume()
            .map_err(|e| OasisError::Backend(e.to_string().into()))?;
        Ok(stream)
    }

    /// Open an SDL3 audio device and stream with the given sample rate and
    /// channels.
    fn open_device(&mut self, sample_rate: i32, channels: u8) -> Result<()> {
        let stream = self.create_stream(sample_rate, channels)?;
        self.sample_rate = sample_rate;
        self.channels = channels as usize;
        log::info!(
            "SDL3 audio device opened: {}Hz, {} channels",
            self.sample_rate,
            self.channels,
        );
        self.stream_owner = Some(stream);
        Ok(())
    }

    /// Queue PCM i16 samples to the SDL3 audio stream.
    fn queue_pcm(&mut self, pcm: &[i16]) -> Result<()> {
        if let Some(ref stream) = self.stream_owner {
            stream
                .put_data_i16(pcm)
                .map_err(|e| OasisError::Backend(e.to_string().into()))?;
            self.samples_queued += pcm.len() as u64;
        }
        Ok(())
    }

    /// Get the number of bytes currently queued in the audio stream.
    fn queued_bytes(&self) -> u32 {
        self.stream_owner
            .as_ref()
            .and_then(|s| s.queued_bytes().ok())
            .unwrap_or(0) as u32
    }

    /// Queue one-shot UI sound PCM (interleaved stereo i16 at 48 kHz —
    /// the mix `oasis_audio::sfx::SfxPlayer::render` produces).
    ///
    /// Opens a dedicated SFX stream lazily; SDL mixes it with the music
    /// stream at the device level. The global volume (0-100) is applied
    /// here, matching the music paths, so a muted system stays silent.
    /// No-op in headless environments (no audio subsystem).
    pub fn queue_sfx(&mut self, pcm: &[i16]) -> Result<()> {
        if pcm.is_empty() || self.audio_subsystem.is_none() {
            return Ok(());
        }
        if self.sfx_stream.is_none() {
            let stream = self.create_stream(OUTPUT_SAMPLE_RATE as i32, 2)?;
            log::info!("SDL3 SFX stream opened: {OUTPUT_SAMPLE_RATE}Hz, 2 channels");
            self.sfx_stream = Some(stream);
        }
        let vol = self.volume as i32;
        self.sfx_scratch.clear();
        self.sfx_scratch
            .extend(pcm.iter().map(|&s| ((s as i32 * vol) / 100) as i16));
        if let Some(ref stream) = self.sfx_stream {
            stream
                .put_data_i16(&self.sfx_scratch)
                .map_err(|e| OasisError::Backend(e.to_string().into()))?;
        }
        Ok(())
    }

    /// Bytes currently queued in the music/streaming SDL stream.
    /// Diagnostic accessor: 0 means the device is starving (underrun).
    pub fn music_queued_bytes(&self) -> u32 {
        self.queued_bytes()
    }

    /// Bytes currently queued in the SFX stream (0 when it isn't open).
    /// The shell uses this to keep a small fixed backlog of mixed SFX.
    pub fn sfx_queued_bytes(&self) -> u32 {
        self.sfx_stream
            .as_ref()
            .and_then(|s| s.queued_bytes().ok())
            .unwrap_or(0) as u32
    }

    /// Decode available MP3 frames from `mp3_buffer` and queue PCM to the
    /// SDL3 audio stream.
    ///
    /// Throttle is applied **before** decoding: if the SDL3 stream already has
    /// enough audio we leave the MP3 bytes in the buffer for next time,
    /// avoiding the old bug where decoded PCM was silently dropped.
    fn decode_buffered(&mut self) -> Result<()> {
        // If the SDL3 stream already has plenty of audio, skip decoding and
        // keep the MP3 bytes for later. This bounds latency without losing
        // any decoded audio.
        if self.queued_bytes() > MAX_QUEUE_BYTES && self.mp3_buffer.len() >= 16 * 1024 {
            return Ok(());
        }
        // Max MP3 Layer 3 frame is 1440 bytes, and minimp3 also peeks at
        // the next frame's sync header before committing to decoding the
        // current one. We need BOTH in the buffer simultaneously;
        // otherwise `decoder.next` silently returns None even when there
        // IS a decodable frame right at offset, because the look-ahead
        // check fails. 2 KB leaves headroom for any bitrate/rate. This
        // leaves up to ~2 KB of trailing MP3 undecoded at end of source;
        // `finalize_streaming` drains the tail with the smaller
        // `MIN_DECODE_BYTES_TAIL` threshold so the last ~1 s of audio
        // isn't lost.
        const MIN_DECODE_BYTES: usize = 2048;
        self.decode_mp3_buffer(MIN_DECODE_BYTES)
    }

    /// Shared decode-and-queue path used by both `decode_buffered` (big
    /// threshold, mid-stream) and `finalize_streaming` (small threshold,
    /// end-of-stream tail drain).
    fn decode_mp3_buffer(&mut self, min_decode_bytes: usize) -> Result<()> {
        let mut pcm_out = [0i16; 2304];
        let mut offset = 0;
        self.pcm_staging.clear();
        // Lock detected format to the *first* audio frame in this batch.
        // Later frames should report the same rate/channels; using the
        // first avoids being misled by a spurious resync frame near the
        // end of the batch.
        let mut detected_rate = 0u32;
        let mut detected_channels = 0u16;

        loop {
            let remaining = self.mp3_buffer.len() - offset;
            if remaining < min_decode_bytes {
                break;
            }
            match self.decoder.next(&self.mp3_buffer[offset..], &mut pcm_out) {
                Some((frame, consumed)) => {
                    offset += consumed;
                    if let rmp3::Frame::Audio(audio) = frame {
                        if detected_rate == 0 {
                            detected_rate = audio.sample_rate();
                            detected_channels = audio.channels();
                        }
                        self.pcm_staging.extend_from_slice(audio.samples());
                    }
                },
                None => break,
            }
        }

        self.mp3_buffer.drain(..offset);

        // The SDL device is always opened at OUTPUT_SAMPLE_RATE /
        // stereo — we do both upmixing and rate conversion in
        // userspace. A rate change in the source therefore just
        // re-seats the resampler; the device stays open and
        // PipeWire/PulseAudio never sees a reconfigure.
        if detected_rate > 0 && self.resampler.in_rate != detected_rate {
            log::debug!("SDL audio: source MP3 format = {detected_rate}Hz {detected_channels}ch");
            self.resampler.set_input_rate(detected_rate);
        }

        // Open device once, at the fixed output format.
        if self.stream_owner.is_none() && detected_rate > 0 && self.audio_subsystem.is_some() {
            self.open_device(OUTPUT_SAMPLE_RATE as i32, 2)?;
        }

        if !self.pcm_staging.is_empty() {
            // Record the output format for `position_ms` / status.
            if detected_rate > 0 {
                self.sample_rate = OUTPUT_SAMPLE_RATE as i32;
                self.channels = 2;
            }

            // Upmix mono → stereo in-place via a quick rewrite so we
            // feed the resampler a stereo-interleaved buffer.
            if detected_channels == 1 {
                let mono_len = self.pcm_staging.len();
                self.pcm_staging.resize(mono_len * 2, 0);
                // Walk from the end inward so we don't overwrite
                // samples we haven't duplicated yet.
                for i in (0..mono_len).rev() {
                    let s = self.pcm_staging[i];
                    self.pcm_staging[2 * i] = s;
                    self.pcm_staging[2 * i + 1] = s;
                }
            }

            // Apply volume scaling before resampling so the interpolator
            // sees the already-scaled signal (avoids tiny quantisation
            // differences versus scaling after).
            let vol = self.volume as i32;
            for s in &mut self.pcm_staging {
                *s = ((*s as i32 * vol) / 100) as i16;
            }

            // Resample to OUTPUT_SAMPLE_RATE stereo and queue.
            self.pcm_resampled.clear();
            self.resampler
                .process(&self.pcm_staging, &mut self.pcm_resampled);
            if !self.pcm_resampled.is_empty() {
                let out = std::mem::take(&mut self.pcm_resampled);
                self.queue_pcm(&out)?;
                self.pcm_resampled = out;
                self.pcm_resampled.clear();
            }
        }

        Ok(())
    }

    /// Decode an entire audio buffer and queue all PCM at once (for static
    /// tracks loaded via `load_track`).
    fn decode_and_queue_all(&mut self, audio_data: &[u8]) -> Result<()> {
        // Try WAV first (cheap header check), then Ogg Vorbis, then MP3.
        if oasis_audio::wav::is_wav(audio_data) {
            return self.decode_wav_and_queue(audio_data);
        }
        if oasis_audio::ogg::is_ogg(audio_data) {
            return self.decode_ogg_and_queue(audio_data);
        }
        self.decode_mp3_and_queue(audio_data)
    }

    fn decode_ogg_and_queue(&mut self, ogg_data: &[u8]) -> Result<()> {
        let ogg = oasis_audio::ogg::decode_ogg(ogg_data)
            .ok_or_else(|| OasisError::Backend("invalid Ogg Vorbis data".into()))?;

        if self.stream_owner.is_none() && self.audio_subsystem.is_some() {
            self.open_device(ogg.sample_rate as i32, ogg.channels as u8)?;
        }

        self.sample_rate = ogg.sample_rate as i32;
        self.channels = ogg.channels as usize;

        let mut pending_pcm = ogg.samples;
        let vol = self.volume as i32;
        for s in &mut pending_pcm {
            *s = ((*s as i32 * vol) / 100) as i16;
        }

        self.queue_pcm(&pending_pcm)?;
        Ok(())
    }

    fn decode_wav_and_queue(&mut self, wav_data: &[u8]) -> Result<()> {
        let wav = oasis_audio::wav::decode_wav(wav_data)
            .ok_or_else(|| OasisError::Backend("invalid WAV data".into()))?;

        if self.stream_owner.is_none() && self.audio_subsystem.is_some() {
            self.open_device(wav.sample_rate as i32, wav.channels as u8)?;
        }

        self.sample_rate = wav.sample_rate as i32;
        self.channels = wav.channels as usize;

        let mut pending_pcm = wav.samples;
        let vol = self.volume as i32;
        for s in &mut pending_pcm {
            *s = ((*s as i32 * vol) / 100) as i16;
        }

        self.queue_pcm(&pending_pcm)?;
        Ok(())
    }

    fn decode_mp3_and_queue(&mut self, mp3_data: &[u8]) -> Result<()> {
        // Static MP3 playback (load_track) goes through the *same*
        // pipeline as streaming: decode → upmix mono to stereo →
        // resample to OUTPUT_SAMPLE_RATE → queue. That way the
        // `play_mp3` example binary exercises exactly what the radio
        // app does, which keeps it useful as a diagnostic tool.
        let mut decoder = rmp3::RawDecoder::new();
        let mut pcm_out = [0i16; 2304];
        let mut offset = 0;
        let mut decoded: Vec<i16> = Vec::new();
        let mut detected_rate = 0u32;
        let mut detected_channels = 0u16;

        loop {
            let remaining = mp3_data.len() - offset;
            if remaining < 16 {
                break;
            }
            match decoder.next(&mp3_data[offset..], &mut pcm_out) {
                Some((frame, consumed)) => {
                    offset += consumed;
                    if let rmp3::Frame::Audio(audio) = frame {
                        if detected_rate == 0 {
                            detected_rate = audio.sample_rate();
                            detected_channels = audio.channels();
                        }
                        decoded.extend_from_slice(audio.samples());
                    }
                },
                None => break,
            }
        }

        if decoded.is_empty() || detected_rate == 0 {
            return Ok(());
        }

        // Upmix mono → stereo.
        if detected_channels == 1 {
            let mono_len = decoded.len();
            decoded.resize(mono_len * 2, 0);
            for i in (0..mono_len).rev() {
                let s = decoded[i];
                decoded[2 * i] = s;
                decoded[2 * i + 1] = s;
            }
        }

        // Apply volume scaling.
        let vol = self.volume as i32;
        for s in &mut decoded {
            *s = ((*s as i32 * vol) / 100) as i16;
        }

        // Open device at the fixed output format if it isn't already.
        if self.stream_owner.is_none() && self.audio_subsystem.is_some() {
            self.open_device(OUTPUT_SAMPLE_RATE as i32, 2)?;
        }
        self.sample_rate = OUTPUT_SAMPLE_RATE as i32;
        self.channels = 2;

        // Route through the streaming resampler so the static path and
        // streaming path produce bit-identical output for the same
        // input.
        let mut local_resampler = LinearResampler::new(OUTPUT_SAMPLE_RATE);
        local_resampler.set_input_rate(detected_rate);
        // Size the output buffer for the expected sample count: when
        // upsampling (e.g. 22.05 kHz → 48 kHz, ~2.17×) a raw
        // `Vec::with_capacity(decoded.len())` would immediately
        // reallocate.
        let expected_out = (decoded.len() as u64).saturating_mul(OUTPUT_SAMPLE_RATE as u64)
            / detected_rate.max(1) as u64;
        let mut resampled = Vec::with_capacity(expected_out as usize + 16);
        local_resampler.process(&decoded, &mut resampled);

        self.queue_pcm(&resampled)?;

        Ok(())
    }
}

impl Default for SdlAudioBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioBackend for SdlAudioBackend {
    fn init(&mut self) -> Result<()> {
        // Ask SDL3 for a generously large audio device buffer. The
        // default on Linux (PulseAudio / PipeWire) is typically
        // ~1024 frames (~23 ms at 44.1 kHz), which is tight — a brief
        // scheduling hitch in our feed path can starve the device and
        // produce ~10 ms "skip"-like stutters a few times a second.
        // 16384 frames (~370 ms at 44.1 kHz) gives the driver enough
        // runway that even a very preempted main thread won't
        // underrun. Radio is a buffered, non-interactive playback
        // context so the extra latency is imperceptible.
        //
        // Must be set *before* the audio device is opened — SDL
        // reads hints at device-open time.
        sdl3::hint::set("SDL_AUDIO_DEVICE_SAMPLE_FRAMES", "16384");

        // Try to initialize SDL3 audio subsystem.
        // In CI/headless environments this may fail — we log a warning
        // but still mark as initialized so non-audio functionality works.
        match sdl3::init() {
            Ok(sdl) => match sdl.audio() {
                Ok(audio) => {
                    self.audio_subsystem = Some(audio);
                    log::info!("SDL3 audio subsystem initialized");
                },
                Err(e) => {
                    log::warn!("SDL3 audio unavailable: {e}");
                },
            },
            Err(e) => {
                log::warn!("SDL3 init failed (headless?): {e}");
            },
        }
        self.initialized = true;
        Ok(())
    }

    fn load_track(&mut self, data: &[u8]) -> Result<AudioTrackId> {
        if !self.initialized {
            return Err(OasisError::Backend("audio not initialized".into()));
        }
        let id = self.next_id;
        self.next_id += 1;
        self.tracks.insert(id, data.to_vec());
        log::debug!("Loaded audio track {id} ({} bytes)", data.len());
        Ok(AudioTrackId(id))
    }

    fn play(&mut self, track: AudioTrackId) -> Result<()> {
        if !self.initialized {
            return Err(OasisError::Backend("audio not initialized".into()));
        }

        let is_stream = self.stream_track == Some(track.0);
        if !is_stream && !self.tracks.contains_key(&track.0) {
            return Err(OasisError::Backend(
                format!("track {} not loaded", track.0).into(),
            ));
        }

        self.current_track = Some(track.0);
        self.playing = true;
        self.paused = false;

        // For static tracks, decode the full MP3 and queue all PCM.
        if !is_stream && let Some(data) = self.tracks.get(&track.0).cloned() {
            self.decode_and_queue_all(&data)?;
        }

        // Resume the SDL3 audio device so queued samples play.
        if let Some(ref stream) = self.stream_owner {
            let _ = stream.resume();
        }

        Ok(())
    }

    fn pause(&mut self) -> Result<()> {
        if !self.playing {
            return Err(OasisError::Backend("not playing".into()));
        }
        if let Some(ref stream) = self.stream_owner {
            let _ = stream.pause();
        }
        self.playing = false;
        self.paused = true;
        Ok(())
    }

    fn resume(&mut self) -> Result<()> {
        if !self.paused {
            return Err(OasisError::Backend("not paused".into()));
        }
        if let Some(ref stream) = self.stream_owner {
            let _ = stream.resume();
        }
        self.playing = true;
        self.paused = false;
        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        if let Some(ref stream) = self.stream_owner {
            let _ = stream.pause();
        }
        if let Some(ref stream) = self.stream_owner {
            let _ = stream.clear();
        }
        self.mp3_buffer.clear();
        self.playing = false;
        self.paused = false;
        self.samples_queued = 0;
        Ok(())
    }

    fn set_volume(&mut self, volume: u8) -> Result<()> {
        self.volume = volume.min(100);
        Ok(())
    }

    fn get_volume(&self) -> u8 {
        self.volume
    }

    fn is_playing(&self) -> bool {
        self.playing
    }

    fn position_ms(&self) -> u64 {
        if self.sample_rate > 0 && self.channels > 0 {
            // Subtract unplayed samples still sitting in the SDL audio stream
            // so the position reflects what the user actually hears, not what
            // has been decoded and queued.  `stream.available()` returns queued
            // bytes; each sample is i16 = 2 bytes.
            let unplayed = self.queued_bytes() as u64 / 2;
            let played = self.samples_queued.saturating_sub(unplayed);
            (played / self.channels as u64) * 1000 / self.sample_rate as u64
        } else {
            0
        }
    }

    fn duration_ms(&self) -> u64 {
        // For streaming tracks, duration is unknown.
        if self.stream_track.is_some() {
            return 0;
        }
        // Rough estimate from MP3 data size (assume 128kbps).
        if let Some(id) = self.current_track
            && let Some(data) = self.tracks.get(&id)
        {
            return (data.len() as u64 * 8) / 128;
        }
        0
    }

    fn unload_track(&mut self, track: AudioTrackId) -> Result<()> {
        if self.current_track == Some(track.0) {
            self.stop()?;
            self.current_track = None;
        }
        if self.stream_track == Some(track.0) {
            self.stream_track = None;
            self.mp3_buffer.clear();
            self.decoder = rmp3::RawDecoder::new();
        }
        self.tracks.remove(&track.0);
        Ok(())
    }

    fn shutdown(&mut self) -> Result<()> {
        self.stop()?;
        self.stream_owner = None;
        self.sfx_stream = None;
        self.sfx_scratch.clear();
        self.audio_subsystem = None;
        self.tracks.clear();
        self.stream_track = None;
        self.mp3_buffer.clear();
        self.pcm_staging.clear();
        self.decoder = rmp3::RawDecoder::new();
        self.initialized = false;
        log::info!("SDL3 audio backend shut down");
        Ok(())
    }

    fn load_streaming(&mut self) -> Result<AudioTrackId> {
        if !self.initialized {
            return Err(OasisError::Backend("audio not initialized".into()));
        }
        let id = self.next_id;
        self.next_id += 1;
        self.mp3_buffer.clear();
        self.decoder = rmp3::RawDecoder::new();
        self.stream_track = Some(id);
        self.samples_queued = 0;
        // Drop the SDL stream so the next decoded frame opens a fresh
        // one. Tried the lighter `stream.clear()` first, but a stream
        // reused across station switches consistently produced
        // periodic stutters on the *second* station while the first
        // (cold-opened) one was clean — something in the reused
        // stream's internal state carried forward. The device spec
        // itself doesn't change (always 48 kHz stereo) so PipeWire
        // only sees a quick re-bind, not a full reconfigure.
        self.stream_owner = None;
        // Reset resampler state — the new track might not share the
        // previous one's sample rate, and any interpolation across
        // that boundary would be meaningless.
        self.resampler.set_input_rate(0);
        // Start a new track in the "accept PCM" phase of the hysteresis
        // cycle so we fill the queue quickly from empty.
        self.accepting.set(true);
        log::debug!("Created streaming audio track {id}");
        Ok(AudioTrackId(id))
    }

    fn feed_data(&mut self, track: AudioTrackId, data: &[u8]) -> Result<()> {
        if self.stream_track != Some(track.0) {
            return Err(OasisError::Backend(
                format!("streaming track {} not found", track.0).into(),
            ));
        }

        // Accept the new bytes unconditionally. Callers should use
        // `streaming_can_accept()` to apply back-pressure — dropping bytes
        // from the middle of the MP3 stream here (which the old 256 KB cap
        // did) breaks frame sync and produces periodic stutter. A very
        // large safety cap catches pathological cases without affecting
        // normal playback.
        self.mp3_buffer.extend_from_slice(data);
        const MP3_BUF_SAFETY_CAP: usize = 32 * 1024 * 1024;
        if self.mp3_buffer.len() > MP3_BUF_SAFETY_CAP {
            log::warn!(
                "SDL audio: MP3 side-buffer exceeded {MP3_BUF_SAFETY_CAP} bytes, \
                 clearing (network faster than playback + back-pressure broken?)"
            );
            self.mp3_buffer.clear();
            self.decoder = rmp3::RawDecoder::new();
        }

        self.decode_buffered()
    }

    fn finalize_streaming(&mut self, track: AudioTrackId) -> Result<()> {
        if self.stream_track != Some(track.0) {
            return Ok(());
        }
        // Drain the last bit of `mp3_buffer` with a relaxed threshold.
        // Normal `decode_buffered` needs ~2 KB of look-ahead for
        // minimp3, so up to 2 KB trails undecoded at end-of-source.
        // Sixteen bytes is the minimum rmp3 needs to avoid a
        // bounds-check panic.
        self.decode_mp3_buffer(16)
    }

    fn streaming_queued_ms(&self, track: AudioTrackId) -> Option<u32> {
        if self.stream_track != Some(track.0) {
            return None;
        }
        // Queue holds interleaved stereo i16 at OUTPUT_SAMPLE_RATE:
        // 48 kHz * 2 ch * 2 bytes = 192 bytes/ms.
        Some(self.queued_bytes() / 192)
    }

    fn streaming_can_accept(&self, track: AudioTrackId) -> bool {
        if self.stream_track != Some(track.0) {
            return false;
        }
        // Hysteresis: fill the queue to MAX_QUEUE_BYTES, then don't accept
        // more PCM until it has drained below REFILL_QUEUE_BYTES. This
        // replaces the old "threshold just below the cap" check which
        // caused tight back-pressure cycles of ~250 ms — one poll every
        // ~1/4 s produced an audible pause-and-catch-up artifact on
        // Linux/PulseAudio. With the wide band we now pull in longer
        // bursts separated by multiple seconds of steady playout.
        let queued = self.queued_bytes();
        let accepting = self.accepting.get();
        let next = if accepting {
            queued < MAX_QUEUE_BYTES
        } else {
            queued < REFILL_QUEUE_BYTES
        };
        if next != accepting {
            self.accepting.set(next);
        }
        next
    }

    fn feed_pcm_f32(
        &mut self,
        track: AudioTrackId,
        samples: &[f32],
        channels: u16,
        sample_rate: u32,
    ) -> Result<()> {
        if self.stream_track != Some(track.0) {
            return Err(OasisError::Backend(
                format!("streaming track {} not found", track.0).into(),
            ));
        }

        if samples.is_empty() {
            return Ok(());
        }

        // Open device lazily with the stream's format, or reopen if format changed.
        let format_changed = self.stream_owner.is_some()
            && (self.sample_rate != sample_rate as i32 || self.channels != channels as usize);
        if format_changed {
            log::info!(
                "SDL audio: format change {}Hz/{}ch -> {}Hz/{}ch, reopening device",
                self.sample_rate,
                self.channels,
                sample_rate,
                channels,
            );
            self.stream_owner = None;
        }
        if self.stream_owner.is_none() && self.audio_subsystem.is_some() {
            self.open_device(sample_rate as i32, channels as u8)?;
            // Resume immediately — play() was called before the device existed.
            if self.playing
                && let Some(ref stream) = self.stream_owner
            {
                let _ = stream.resume();
            }
        }

        self.sample_rate = sample_rate as i32;
        self.channels = channels as usize;

        // Convert f32 → i16 with volume scaling.
        let vol = self.volume as f32 / 100.0;
        self.pcm_staging.clear();
        self.pcm_staging.reserve(samples.len());
        for &s in samples {
            let scaled = s * vol * 32767.0;
            self.pcm_staging
                .push(scaled.clamp(-32768.0, 32767.0) as i16);
        }

        // Apply backpressure: skip if queue is already full to prevent unbounded growth.
        let queued = self.queued_bytes();
        if queued < MAX_QUEUE_BYTES {
            let staging = std::mem::take(&mut self.pcm_staging);
            self.queue_pcm(&staging)?;
            self.pcm_staging = staging;
        } else {
            log::debug!(
                "SDL audio: queue full ({queued} bytes), dropping {} samples",
                self.pcm_staging.len(),
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init_backend() -> SdlAudioBackend {
        let mut backend = SdlAudioBackend::new();
        backend.init().unwrap();
        backend
    }

    #[test]
    fn init_and_shutdown() {
        let mut backend = SdlAudioBackend::new();
        assert!(!backend.initialized);
        backend.init().unwrap();
        assert!(backend.initialized);
        backend.shutdown().unwrap();
        assert!(!backend.initialized);
    }

    #[test]
    fn load_and_play_track() {
        let mut backend = init_backend();
        let track = backend.load_track(b"fake mp3 data here").unwrap();
        backend.play(track).unwrap();
        assert!(backend.is_playing());
        assert_eq!(backend.current_track, Some(track.0));
    }

    #[test]
    fn load_without_init_fails() {
        let mut backend = SdlAudioBackend::new();
        assert!(backend.load_track(b"data").is_err());
    }

    #[test]
    fn play_missing_track_fails() {
        let mut backend = init_backend();
        assert!(backend.play(AudioTrackId(999)).is_err());
    }

    #[test]
    fn pause_and_resume() {
        let mut backend = init_backend();
        let track = backend.load_track(b"data").unwrap();
        backend.play(track).unwrap();

        backend.pause().unwrap();
        assert!(!backend.is_playing());
        assert!(backend.paused);

        backend.resume().unwrap();
        assert!(backend.is_playing());
        assert!(!backend.paused);
    }

    #[test]
    fn pause_when_not_playing_fails() {
        let mut backend = init_backend();
        assert!(backend.pause().is_err());
    }

    #[test]
    fn resume_when_not_paused_fails() {
        let mut backend = init_backend();
        assert!(backend.resume().is_err());
    }

    #[test]
    fn stop_playback() {
        let mut backend = init_backend();
        let track = backend.load_track(b"data").unwrap();
        backend.play(track).unwrap();
        backend.stop().unwrap();
        assert!(!backend.is_playing());
        assert_eq!(backend.position_ms(), 0);
    }

    #[test]
    fn volume_control() {
        let mut backend = init_backend();
        backend.set_volume(42).unwrap();
        assert_eq!(backend.get_volume(), 42);

        // Clamp to 100.
        backend.set_volume(200).unwrap();
        assert_eq!(backend.get_volume(), 100);
    }

    #[test]
    fn unload_track() {
        let mut backend = init_backend();
        let track = backend.load_track(b"data").unwrap();
        backend.play(track).unwrap();

        backend.unload_track(track).unwrap();
        assert!(!backend.is_playing());
        assert!(!backend.tracks.contains_key(&track.0));
    }

    #[test]
    fn multiple_tracks() {
        let mut backend = init_backend();
        let t1 = backend.load_track(b"track 1").unwrap();
        let t2 = backend.load_track(b"track 2").unwrap();
        assert_ne!(t1, t2);
        assert_eq!(backend.tracks.len(), 2);

        backend.play(t1).unwrap();
        assert!(backend.is_playing());

        backend.stop().unwrap();
        backend.play(t2).unwrap();
        assert!(backend.is_playing());
        assert_eq!(backend.current_track, Some(t2.0));
    }

    #[test]
    fn shutdown_clears_tracks() {
        let mut backend = init_backend();
        backend.load_track(b"track 1").unwrap();
        backend.load_track(b"track 2").unwrap();
        backend.shutdown().unwrap();
        assert!(backend.tracks.is_empty());
    }

    #[test]
    fn volume_clamps_to_zero() {
        let mut backend = init_backend();
        backend.set_volume(0).unwrap();
        assert_eq!(backend.get_volume(), 0);
    }

    #[test]
    fn volume_clamps_to_max() {
        let mut backend = init_backend();
        backend.set_volume(255).unwrap();
        assert_eq!(backend.get_volume(), 100);
    }

    #[test]
    fn rapid_play_pause_cycle() {
        let mut backend = init_backend();
        let track = backend.load_track(b"data").unwrap();

        for _ in 0..10 {
            backend.play(track).unwrap();
            assert!(backend.is_playing());
            backend.pause().unwrap();
            assert!(!backend.is_playing());
            backend.resume().unwrap();
            assert!(backend.is_playing());
            backend.stop().unwrap();
            assert!(!backend.is_playing());
        }
    }

    #[test]
    fn double_stop_is_idempotent() {
        let mut backend = init_backend();
        let track = backend.load_track(b"data").unwrap();
        backend.play(track).unwrap();
        backend.stop().unwrap();
        // Second stop should not error.
        backend.stop().unwrap();
        assert!(!backend.is_playing());
    }

    #[test]
    fn unload_nonexistent_track_is_silent() {
        let mut backend = init_backend();
        // Unloading a track that doesn't exist just removes nothing.
        backend.unload_track(AudioTrackId(999)).unwrap();
    }

    #[test]
    fn play_after_unload_fails() {
        let mut backend = init_backend();
        let track = backend.load_track(b"data").unwrap();
        backend.unload_track(track).unwrap();
        assert!(backend.play(track).is_err());
    }

    #[test]
    fn duration_estimate_scales_with_data() {
        let mut backend = init_backend();
        let small = backend.load_track(&[0u8; 1_000]).unwrap();
        backend.play(small).unwrap();
        let dur_small = backend.duration_ms();

        backend.stop().unwrap();
        let large = backend.load_track(&[0u8; 10_000]).unwrap();
        backend.play(large).unwrap();
        let dur_large = backend.duration_ms();

        assert!(
            dur_large > dur_small,
            "larger track should have longer duration: {dur_large} > {dur_small}"
        );
    }

    // ---------------------------------------------------------------
    // Streaming tests
    // ---------------------------------------------------------------

    #[test]
    fn streaming_lifecycle() {
        let mut backend = init_backend();
        let track = backend.load_streaming().unwrap();
        assert_eq!(backend.stream_track, Some(track.0));
        backend.feed_data(track, b"chunk 1").unwrap();
        backend.feed_data(track, b"chunk 2").unwrap();
        // Decoder consumes garbage bytes while scanning for sync words,
        // so mp3_buffer may be smaller than total fed bytes.
        backend.unload_track(track).unwrap();
        assert_eq!(backend.stream_track, None);
        assert!(backend.mp3_buffer.is_empty());
    }

    #[test]
    fn streaming_buffer_is_bounded() {
        let mut backend = init_backend();
        let track = backend.load_streaming().unwrap();
        // Feed more than 256KB to verify the buffer is capped.
        for _ in 0..80 {
            backend.feed_data(track, &[0xAA; 4096]).unwrap();
        }
        // 80 * 4096 = 320KB, should be capped to 256KB.
        assert!(backend.mp3_buffer.len() <= 256 * 1024);
    }

    #[test]
    fn streaming_without_init_fails() {
        let mut backend = SdlAudioBackend::new();
        assert!(backend.load_streaming().is_err());
    }

    #[test]
    fn feed_data_invalid_track_fails() {
        let mut backend = init_backend();
        assert!(backend.feed_data(AudioTrackId(999), b"data").is_err());
    }

    #[test]
    fn reinit_after_shutdown() {
        let mut backend = init_backend();
        backend.load_track(b"data").unwrap();
        backend.shutdown().unwrap();
        assert!(!backend.initialized);

        // Re-init should work.
        backend.init().unwrap();
        assert!(backend.initialized);
        let track = backend.load_track(b"new data").unwrap();
        backend.play(track).unwrap();
        assert!(backend.is_playing());
    }

    #[test]
    fn streaming_starts_empty() {
        let mut backend = init_backend();
        let _track = backend.load_streaming().unwrap();
        assert!(backend.mp3_buffer.is_empty());
    }

    #[test]
    fn streaming_play_after_feed() {
        let mut backend = init_backend();
        let track = backend.load_streaming().unwrap();
        backend.feed_data(track, b"audio data").unwrap();
        // Should be able to play a streaming track.
        backend.play(track).unwrap();
        assert!(backend.is_playing());
    }

    #[test]
    fn streaming_unload_stops_playback() {
        let mut backend = init_backend();
        let track = backend.load_streaming().unwrap();
        backend.feed_data(track, b"data").unwrap();
        backend.play(track).unwrap();
        assert!(backend.is_playing());
        backend.unload_track(track).unwrap();
        assert!(!backend.is_playing());
        assert_eq!(backend.stream_track, None);
    }

    #[test]
    fn streaming_feed_empty_data() {
        let mut backend = init_backend();
        let track = backend.load_streaming().unwrap();
        backend.feed_data(track, b"").unwrap();
        assert!(backend.mp3_buffer.is_empty());
    }

    #[test]
    fn streaming_buffer_cap_prevents_oom() {
        let mut backend = init_backend();
        let track = backend.load_streaming().unwrap();
        // Feed a large chunk; the decoder will consume garbage data as it
        // scans, and the 256KB cap prevents unbounded growth in between.
        let chunk = vec![0xBBu8; 256 * 1024];
        backend.feed_data(track, &chunk).unwrap();
        // After decoding, buffer should not exceed 256KB.
        assert!(backend.mp3_buffer.len() <= 256 * 1024);
    }

    #[test]
    fn streaming_overflow_does_not_error() {
        let mut backend = init_backend();
        let track = backend.load_streaming().unwrap();
        // Feed >256KB to verify the cap + decode cycle stays stable.
        let first = vec![0xAAu8; 256 * 1024];
        backend.feed_data(track, &first).unwrap();
        backend.feed_data(track, &[0xBBu8; 100]).unwrap();
        // Buffer should never exceed cap (decoder consumes garbage).
        assert!(backend.mp3_buffer.len() <= 256 * 1024);
    }

    #[test]
    fn streaming_shutdown_clears_streaming() {
        let mut backend = init_backend();
        let t1 = backend.load_streaming().unwrap();
        backend.feed_data(t1, b"data").unwrap();
        backend.shutdown().unwrap();
        assert_eq!(backend.stream_track, None);
        assert!(backend.mp3_buffer.is_empty());
    }

    #[test]
    fn streaming_feed_after_unload_fails() {
        let mut backend = init_backend();
        let track = backend.load_streaming().unwrap();
        backend.unload_track(track).unwrap();
        assert!(backend.feed_data(track, b"data").is_err());
    }

    #[test]
    fn streaming_incremental_feed() {
        let mut backend = init_backend();
        let track = backend.load_streaming().unwrap();
        // Feed many small chunks; decoder consumes garbage bytes, so the
        // buffer may not grow monotonically, but no errors should occur.
        for i in 0..10 {
            backend.feed_data(track, &[i as u8; 100]).unwrap();
        }
        // Buffer stays bounded (decoder consumed garbage).
        assert!(backend.mp3_buffer.len() <= 1000);
    }

    #[test]
    fn position_ms_is_zero_before_playback() {
        let backend = init_backend();
        assert_eq!(backend.position_ms(), 0);
    }

    // ---------------------------------------------------------------
    // SFX one-shot tests
    // ---------------------------------------------------------------

    #[test]
    fn queue_sfx_never_errors_headless() {
        let mut backend = init_backend();
        // With no audio hardware (CI) this is a silent no-op; with
        // hardware it opens the SFX stream. Either way: no error.
        backend.queue_sfx(&[100, -100, 200, -200]).unwrap();
        backend.queue_sfx(&[]).unwrap();
        let _ = backend.sfx_queued_bytes();
    }

    #[test]
    fn sfx_queued_bytes_zero_when_closed() {
        let backend = SdlAudioBackend::new();
        assert_eq!(backend.sfx_queued_bytes(), 0);
    }

    #[test]
    fn shutdown_closes_sfx_stream() {
        let mut backend = init_backend();
        backend.queue_sfx(&[1, 2, 3, 4]).unwrap();
        backend.shutdown().unwrap();
        assert!(backend.sfx_stream.is_none());
        assert_eq!(backend.sfx_queued_bytes(), 0);
    }
}
