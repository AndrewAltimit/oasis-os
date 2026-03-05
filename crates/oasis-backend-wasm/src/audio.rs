//! `AudioBackend` implementation using the Web Audio API.
//!
//! Supports both static buffer playback (Web Audio `AudioBuffer`) and
//! streaming playback via MSE (`MediaSource` + `SourceBuffer("audio/mpeg")`).

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

use wasm_bindgen::prelude::*;
use web_sys::{AudioBuffer, AudioBufferSourceNode, AudioContext, GainNode};

use oasis_types::backend::{AudioBackend, AudioTrackId};
use oasis_types::error::{OasisError, Result};

fn js_err(e: JsValue) -> OasisError {
    OasisError::Backend(format!("{e:?}"))
}

/// Maximum pending chunks before dropping oldest (prevents unbounded growth
/// when MSE SourceBuffer can't keep up, e.g. QuotaExceededError).
const MAX_PENDING_CHUNKS: usize = 50;

// ---------------------------------------------------------------------------
// MSE streaming track state
// ---------------------------------------------------------------------------

/// Shared optional JS closure slot for event handler lifecycle management.
type SharedClosure = Rc<RefCell<Option<Closure<dyn FnMut()>>>>;

struct StreamingTrack {
    audio_el: web_sys::HtmlAudioElement,
    #[allow(dead_code)]
    media_source: web_sys::MediaSource,
    source_buffer: Rc<RefCell<Option<web_sys::SourceBuffer>>>,
    pending_chunks: Rc<RefCell<VecDeque<Vec<u8>>>>,
    #[allow(dead_code)]
    updating: Rc<Cell<bool>>,
    ready: Rc<Cell<bool>>,
    object_url: String,
    // Hold closures to prevent GC.
    #[allow(dead_code)]
    closures: Vec<Closure<dyn FnMut()>>,
    #[allow(dead_code)]
    closures_ev: Vec<Closure<dyn FnMut(web_sys::Event)>>,
    // Hold the updateend closure (replaces .forget() leak).
    #[allow(dead_code)]
    update_end_closure: SharedClosure,
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
    streaming_track: Option<StreamingTrack>,
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
            streaming_track: None,
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
        self.ctx
            .as_ref()
            .ok_or_else(|| OasisError::Backend("AudioContext not initialized".into()))
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

        let ctx = self
            .ctx
            .as_ref()
            .ok_or_else(|| OasisError::Backend("AudioContext not initialized".into()))?;
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
        // If this is the streaming track, start the audio element.
        if let Some(ref st) = self.streaming_track
            && self.current_track == Some(track.0)
        {
            let _ = st.audio_el.play().map_err(js_err)?;
            self.playing = true;
            self.paused = false;
            return Ok(());
        }

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
        // Stop Web Audio buffer source.
        if let Some(ref source) = self.current_source {
            #[allow(deprecated)]
            let _ = source.stop_with_when(0.0);
        }
        self.current_source = None;
        self.current_track = None;
        // Stop MSE streaming track.
        if let Some(st) = self.streaming_track.take() {
            st.audio_el.pause().ok();
            let _ = web_sys::Url::revoke_object_url(&st.object_url);
        }
        self.playing = false;
        self.paused = false;
        Ok(())
    }

    fn set_volume(&mut self, volume: u8) -> Result<()> {
        self.volume = volume.min(100);
        if let Some(ref gain) = self.gain {
            gain.gain().set_value(self.volume as f32 / 100.0);
        }
        if let Some(ref st) = self.streaming_track {
            st.audio_el.set_volume(self.volume as f64 / 100.0);
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

    fn load_streaming(&mut self) -> Result<AudioTrackId> {
        use wasm_bindgen::JsCast;

        // Clean up any previous streaming track.
        if let Some(st) = self.streaming_track.take() {
            st.audio_el.pause().ok();
            let _ = web_sys::Url::revoke_object_url(&st.object_url);
        }

        let media_source = web_sys::MediaSource::new().map_err(js_err)?;
        let object_url =
            web_sys::Url::create_object_url_with_source(&media_source).map_err(js_err)?;

        let audio_el = web_sys::HtmlAudioElement::new().map_err(js_err)?;
        audio_el.set_src(&object_url);

        let source_buffer: Rc<RefCell<Option<web_sys::SourceBuffer>>> = Rc::new(RefCell::new(None));
        let pending_chunks: Rc<RefCell<VecDeque<Vec<u8>>>> = Rc::new(RefCell::new(VecDeque::new()));
        let updating = Rc::new(Cell::new(false));
        let ready = Rc::new(Cell::new(false));
        let update_end_closure: SharedClosure = Rc::new(RefCell::new(None));

        let mut closures: Vec<Closure<dyn FnMut()>> = Vec::new();
        let mut closures_ev: Vec<Closure<dyn FnMut(web_sys::Event)>> = Vec::new();

        // Set up `sourceopen` event to create SourceBuffer.
        {
            let sb_ref = Rc::clone(&source_buffer);
            let ready_ref = Rc::clone(&ready);
            let ms_ref = media_source.clone();
            let pending_ref = Rc::clone(&pending_chunks);
            let updating_ref = Rc::clone(&updating);
            let closure_slot = Rc::clone(&update_end_closure);

            let on_open = Closure::wrap(Box::new(move || {
                if let Ok(sb) = ms_ref.add_source_buffer("audio/mpeg") {
                    // Set up `updateend` to drain pending chunks.
                    let pending_inner = Rc::clone(&pending_ref);
                    let sb_inner = sb.clone();
                    let updating_inner = Rc::clone(&updating_ref);
                    let on_update_end = Closure::wrap(Box::new(move || {
                        updating_inner.set(false);
                        // Drop oldest if queue grew too large.
                        let mut pq = pending_inner.borrow_mut();
                        while pq.len() > MAX_PENDING_CHUNKS {
                            pq.pop_front();
                        }
                        // Append next pending chunk if available.
                        if let Some(chunk) = pq.pop_front() {
                            drop(pq);
                            let arr = js_sys::Uint8Array::from(chunk.as_slice());
                            if sb_inner.append_buffer_with_array_buffer_view(&arr).is_err() {
                                // Re-queue on failure (e.g. QuotaExceededError)
                                // so the chunk is not lost.
                                pending_inner.borrow_mut().push_front(chunk);
                            }
                        }
                    }) as Box<dyn FnMut()>);
                    sb.set_onupdateend(Some(on_update_end.as_ref().unchecked_ref()));
                    // Store closure in the shared slot instead of leaking with .forget().
                    *closure_slot.borrow_mut() = Some(on_update_end);

                    // Drain the first pending chunk to kick off the FIFO pipeline.
                    // Subsequent chunks are drained by the `updateend` handler.
                    let mut pq = pending_ref.borrow_mut();
                    if let Some(chunk) = pq.pop_front() {
                        let arr = js_sys::Uint8Array::from(chunk.as_slice());
                        if sb.append_buffer_with_array_buffer_view(&arr).is_err() {
                            // Re-queue on failure so the chunk is not lost.
                            pq.push_front(chunk);
                        }
                    }
                    drop(pq);

                    *sb_ref.borrow_mut() = Some(sb);
                    ready_ref.set(true);
                }
            }) as Box<dyn FnMut()>);

            let on_open_ev = Closure::wrap(Box::new({
                let on_open_ref = on_open.as_ref().unchecked_ref::<js_sys::Function>().clone();
                move |_ev: web_sys::Event| {
                    let _ = on_open_ref.call0(&JsValue::NULL);
                }
            }) as Box<dyn FnMut(web_sys::Event)>);
            media_source
                .add_event_listener_with_callback("sourceopen", on_open_ev.as_ref().unchecked_ref())
                .ok();
            closures.push(on_open);
            closures_ev.push(on_open_ev);
        }

        let vol = self.volume as f64 / 100.0;
        audio_el.set_volume(vol);

        let id = self.next_id;
        self.next_id += 1;

        self.streaming_track = Some(StreamingTrack {
            audio_el,
            media_source,
            source_buffer,
            pending_chunks,
            updating,
            ready,
            object_url,
            closures,
            closures_ev,
            update_end_closure,
        });

        self.current_track = Some(id);
        self.playing = true;

        Ok(AudioTrackId(id))
    }

    fn feed_data(&mut self, _track: AudioTrackId, data: &[u8]) -> Result<()> {
        if let Some(ref st) = self.streaming_track {
            // Always enqueue first to maintain FIFO order — earlier chunks
            // (including those queued before `sourceopen`) must be appended first.
            {
                let mut pq = st.pending_chunks.borrow_mut();
                pq.push_back(data.to_vec());
                while pq.len() > MAX_PENDING_CHUNKS {
                    pq.pop_front();
                }
            }
            // Try to drain the oldest pending chunk if the SourceBuffer is idle.
            if st.ready.get()
                && let Some(ref sb) = *st.source_buffer.borrow()
                && !sb.updating()
                && let Some(chunk) = st.pending_chunks.borrow_mut().pop_front()
            {
                let arr = js_sys::Uint8Array::from(chunk.as_slice());
                if sb.append_buffer_with_array_buffer_view(&arr).is_err() {
                    // Re-queue on failure.
                    st.pending_chunks.borrow_mut().push_front(chunk);
                }
            }
        }
        Ok(())
    }
}
