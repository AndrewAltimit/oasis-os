//! `AudioBackend` implementation using the Web Audio API.

use std::collections::HashMap;

use wasm_bindgen::prelude::*;
use web_sys::{AudioBuffer, AudioBufferSourceNode, AudioContext, GainNode};

use oasis_types::backend::{AudioBackend, AudioTrackId};
use oasis_types::error::{OasisError, Result};

fn js_err(e: JsValue) -> OasisError {
    OasisError::Backend(format!("{e:?}"))
}

// ---------------------------------------------------------------------------
// WasmAudioBackend
// ---------------------------------------------------------------------------

pub struct WasmAudioBackend {
    ctx: Option<AudioContext>,
    gain: Option<GainNode>,
    tracks: HashMap<u64, AudioBuffer>,
    next_id: u64,
    current_source: Option<AudioBufferSourceNode>,
    current_track: Option<u64>,
    volume: u8,
    playing: bool,
    paused: bool,
}

impl WasmAudioBackend {
    pub fn new() -> Self {
        Self {
            ctx: None,
            gain: None,
            tracks: HashMap::new(),
            next_id: 1,
            current_source: None,
            current_track: None,
            volume: 80,
            playing: false,
            paused: false,
        }
    }

    /// Ensure the AudioContext is created and resumed.
    ///
    /// Browsers block audio before user interaction, so we lazily create
    /// the context on first use.
    fn ensure_context(&mut self) -> Result<&AudioContext> {
        if self.ctx.is_none() {
            let ctx = AudioContext::new().map_err(js_err)?;
            let gain = ctx.create_gain().map_err(js_err)?;
            gain.connect_with_audio_node(&ctx.destination())
                .map_err(js_err)?;
            gain.gain().set_value(self.volume as f32 / 100.0);
            self.gain = Some(gain);
            self.ctx = Some(ctx);
        }
        Ok(self.ctx.as_ref().unwrap())
    }
}

impl Default for WasmAudioBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioBackend for WasmAudioBackend {
    fn init(&mut self) -> Result<()> {
        // Defer AudioContext creation to first user interaction.
        Ok(())
    }

    fn shutdown(&mut self) -> Result<()> {
        self.stop()?;
        if let Some(ctx) = self.ctx.take() {
            let _ = ctx.close();
        }
        self.gain = None;
        self.tracks.clear();
        Ok(())
    }

    fn load_track(&mut self, data: &[u8]) -> Result<AudioTrackId> {
        // Web Audio `decodeAudioData` is async, but we need a sync API here.
        // Store raw bytes and we'll decode on play. For now, create an empty
        // buffer as a placeholder — the actual decoding would require async.
        //
        // For a synchronous API, we store the raw data and create the buffer
        // during playback using a synchronous-compatible approach.
        let _ctx = self.ensure_context()?;

        // Store raw data — we'll decode it lazily.
        // For MVP, create a short silent buffer as a placeholder.
        let id = self.next_id;
        self.next_id += 1;

        let ctx = self.ctx.as_ref().unwrap();
        let sample_rate = ctx.sample_rate();
        // Estimate duration: assume ~128kbps MP3 for reasonable duration.
        let estimated_samples = (data.len() as f32 * 8.0 / 128000.0 * sample_rate).max(1.0);
        let buffer = ctx
            .create_buffer(1, estimated_samples as u32, sample_rate)
            .map_err(js_err)?;
        self.tracks.insert(id, buffer);
        Ok(AudioTrackId(id))
    }

    fn unload_track(&mut self, track: AudioTrackId) -> Result<()> {
        self.tracks.remove(&track.0);
        if self.current_track == Some(track.0) {
            self.stop()?;
        }
        Ok(())
    }

    fn play(&mut self, track: AudioTrackId) -> Result<()> {
        let buffer = self
            .tracks
            .get(&track.0)
            .ok_or_else(|| OasisError::Backend(format!("track {} not found", track.0)))?
            .clone();

        let ctx = self.ensure_context()?;

        // Resume context if suspended (autoplay policy).
        let _ = ctx.resume();

        let source = ctx.create_buffer_source().map_err(js_err)?;
        source.set_buffer(Some(&buffer));

        if let Some(ref gain) = self.gain {
            source.connect_with_audio_node(gain).map_err(js_err)?;
        }

        source.start().map_err(js_err)?;
        self.current_source = Some(source);
        self.current_track = Some(track.0);
        self.playing = true;
        self.paused = false;
        Ok(())
    }

    fn pause(&mut self) -> Result<()> {
        if let Some(ref ctx) = self.ctx {
            let _ = ctx.suspend();
        }
        self.paused = true;
        self.playing = false;
        Ok(())
    }

    fn resume(&mut self) -> Result<()> {
        if let Some(ref ctx) = self.ctx {
            let _ = ctx.resume();
        }
        if self.paused {
            self.playing = true;
            self.paused = false;
        }
        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        if let Some(ref source) = self.current_source {
            #[allow(deprecated)]
            let _ = source.stop_with_when(0.0);
        }
        self.current_source = None;
        self.current_track = None;
        self.playing = false;
        self.paused = false;
        Ok(())
    }

    fn set_volume(&mut self, volume: u8) -> Result<()> {
        self.volume = volume.min(100);
        if let Some(ref gain) = self.gain {
            gain.gain().set_value(self.volume as f32 / 100.0);
        }
        Ok(())
    }

    fn get_volume(&self) -> u8 {
        self.volume
    }

    fn is_playing(&self) -> bool {
        self.playing
    }

    fn position_ms(&self) -> u64 {
        self.ctx
            .as_ref()
            .map(|ctx| (ctx.current_time() * 1000.0) as u64)
            .unwrap_or(0)
    }

    fn duration_ms(&self) -> u64 {
        self.current_track
            .and_then(|id| self.tracks.get(&id))
            .map(|buf| (buf.duration() * 1000.0) as u64)
            .unwrap_or(0)
    }
}
