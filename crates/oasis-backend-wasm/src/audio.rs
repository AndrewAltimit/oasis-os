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
    OasisError::Backend(format!("{e:?}").into())
}

/// Append a streaming `<audio>` element to `document.body` so the
/// browser will actually route playback to the speakers.
///
/// A detached element will fail to fire `canplaythrough` on Firefox
/// and silently drop autoplay output on Chrome, so when the body is
/// unavailable (e.g. `load_streaming` racing the DOM during early
/// init) we surface a console warning rather than skipping silently
/// and stranding the radio in `Buffering`. `mode` is included in the
/// log so direct-URL vs. MSE failures can be distinguished.
fn attach_audio_to_body(audio_el: &web_sys::HtmlAudioElement, mode: &str) {
    match web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.body())
    {
        Some(body) => {
            let _ = body.append_child(audio_el);
        },
        None => {
            web_sys::console::warn_1(&JsValue::from_str(&format!(
                "oasis radio ({mode}): document.body unavailable, audio element \
                 not attached — playback may stall in Buffering",
            )));
        },
    }
}

/// Maximum pending chunks before dropping oldest (prevents unbounded growth
/// when MSE SourceBuffer can't keep up, e.g. QuotaExceededError).
const MAX_PENDING_CHUNKS: usize = 50;

// ---------------------------------------------------------------------------
// MSE streaming track state
// ---------------------------------------------------------------------------

/// Shared optional JS closure slot for event handler lifecycle management.
type SharedClosure = Rc<RefCell<Option<Closure<dyn FnMut()>>>>;

/// How a [`StreamingTrack`] is fed audio.
///
/// Direct-URL hands the URL to a `<audio>` element and lets the browser
/// stream + decode natively (used on Firefox, which doesn't decode
/// `audio/mpeg` through MSE, and as a simpler path on Chrome). MSE feeds
/// chunks through `MediaSource` + `SourceBuffer("audio/mpeg")`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamingMode {
    DirectUrl,
    Mse,
}

struct StreamingTrack {
    audio_el: web_sys::HtmlAudioElement,
    mode: StreamingMode,
    /// `Some` only in MSE mode; `None` in direct-URL mode (the
    /// `<audio>` element handles streaming/decoding itself, and
    /// instantiating a `MediaSource` is unnecessary and would fail on
    /// browsers without MSE support).
    #[allow(dead_code)]
    media_source: Option<web_sys::MediaSource>,
    source_buffer: Rc<RefCell<Option<web_sys::SourceBuffer>>>,
    pending_chunks: Rc<RefCell<VecDeque<Vec<u8>>>>,
    #[allow(dead_code)]
    updating: Rc<Cell<bool>>,
    ready: Rc<Cell<bool>>,
    /// Object URL backing the MSE `<audio>.src`; empty in direct-URL
    /// mode. Stored so it can be revoked when the track is dropped.
    object_url: String,
    /// Latched true by this track's `<audio>` element `ended` event.
    /// Per-track so a delayed event from a previously-detached element
    /// can never mutate a newer track's state, even in the unlikely
    /// case wasm-bindgen's Closure-drop invalidation races with a
    /// queued JS callback.
    streaming_ended: Rc<Cell<bool>>,
    /// Latched by this track's `<audio>` element `error` event.
    /// Per-track for the same isolation reason as `streaming_ended`.
    streaming_error: Rc<RefCell<Option<String>>>,
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
    /// URL queued by [`set_streaming_url`]; consumed on the next
    /// `load_streaming` call to wire `<audio>.src` directly to the
    /// resource. When `None`, `load_streaming` falls back to the MSE
    /// pipeline (used by browsers that decode `audio/mpeg` through
    /// MediaSource — Chrome does, Firefox doesn't).
    pending_streaming_url: Option<String>,
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
            pending_streaming_url: None,
        }
    }

    /// Tell the next `load_streaming` call to point the `<audio>`
    /// element at `url` and let the browser stream it natively, instead
    /// of feeding MSE chunks. Required on Firefox (no `audio/mpeg` MSE
    /// support); also a simpler path on Chrome.
    pub fn set_streaming_url(&mut self, url: &str) {
        self.pending_streaming_url = Some(url.to_string());
    }

    /// Consume the latched "audio element ended" flag for the current
    /// streaming track. Returns true once per `ended` event; subsequent
    /// calls return false until the next track ends. Returns false if
    /// no streaming track is loaded. Used by `RadioManager` driver
    /// code to advance to the next track in an archive playlist.
    pub fn take_streaming_ended(&self) -> bool {
        self.streaming_track
            .as_ref()
            .map(|st| st.streaming_ended.replace(false))
            .unwrap_or(false)
    }

    /// Consume any latched audio-element error from the current
    /// streaming track. Returns the message once and clears the slot;
    /// returns `None` if no streaming track is loaded.
    pub fn take_streaming_error(&self) -> Option<String> {
        self.streaming_track
            .as_ref()
            .and_then(|st| st.streaming_error.borrow_mut().take())
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
        // If this is the streaming track, kick the audio element.
        if let Some(ref st) = self.streaming_track
            && self.current_track == Some(track.0)
        {
            self.playing = true;
            self.paused = false;
            // Direct-URL mode: the `canplaythrough` listener in
            // `load_streaming` is responsible for the actual `play()`
            // call once the browser has buffered enough to play
            // through. We just record the playing state here so the
            // radio manager doesn't think we're stuck.
            if st.mode == StreamingMode::DirectUrl {
                return Ok(());
            }
            // MSE path: start playback immediately. Same muted-then-
            // unmute trick as `video.rs::open_url` so autoplay isn't
            // blocked by the user-gesture rules.
            let promise = st.audio_el.play().map_err(js_err)?;
            let audio_clone = st.audio_el.clone();
            wasm_bindgen_futures::spawn_local(async move {
                if wasm_bindgen_futures::JsFuture::from(promise).await.is_ok() {
                    audio_clone.set_muted(false);
                }
            });
            return Ok(());
        }

        let buffer = self
            .tracks
            .get(&track.0)
            .ok_or_else(|| OasisError::Backend(format!("track {} not found", track.0).into()))?
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
            st.audio_el.remove();
            if !st.object_url.is_empty() {
                let _ = web_sys::Url::revoke_object_url(&st.object_url);
            }
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

        // Clean up any previous streaming track. Dropping `st` also
        // drops its closures (invalidating their JS counterparts) and
        // its per-track `streaming_ended`/`streaming_error` Rcs — so a
        // delayed event fired by the detached element after we've
        // installed the new track cannot mutate the new track's
        // latched flags.
        if let Some(st) = self.streaming_track.take() {
            st.audio_el.pause().ok();
            // Detach the previous audio element so we don't leak it
            // across station changes. `remove()` is a no-op if it's
            // already detached.
            st.audio_el.remove();
            if !st.object_url.is_empty() {
                let _ = web_sys::Url::revoke_object_url(&st.object_url);
            }
        }

        // Direct-URL mode: when `set_streaming_url` was called before
        // `load_streaming`, point a `<audio>` element straight at the
        // URL and let the browser stream + decode natively. Bypasses
        // MediaSource entirely so it works on Firefox (no
        // `audio/mpeg` SourceBuffer support).
        if let Some(url) = self.pending_streaming_url.take() {
            let audio_el = web_sys::HtmlAudioElement::new().map_err(js_err)?;
            // Start muted: muted autoplay is unconditionally allowed
            // by every browser, so `play()` resolves cleanly. Once
            // it's playing we unmute (mirrors what
            // `video.rs::open_url` does for TV Guide). Without this
            // the radio audio "starts" but the browser silently
            // refuses to route it to the speakers because the
            // user-gesture token from the keypress doesn't propagate
            // through the wasm tick → setTimeout chain that
            // eventually calls `play()`.
            audio_el.set_muted(true);
            audio_el.set_preload("auto");
            audio_el.set_src(&url);
            audio_el
                .style()
                .set_property("display", "none")
                .map_err(js_err)?;
            attach_audio_to_body(&audio_el, "direct-URL");
            audio_el.set_volume(self.volume as f64 / 100.0);

            // Per-track latched event flags. Old tracks own their own
            // Rcs (dropped with the track); new tracks get fresh Rcs.
            let streaming_ended: Rc<Cell<bool>> = Rc::new(Cell::new(false));
            let streaming_error: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));

            let mut closures_ev: Vec<Closure<dyn FnMut(web_sys::Event)>> = Vec::new();

            // Defer the actual `play()` call until the element fires
            // `canplaythrough` — at that point the browser believes
            // it has enough buffered data to play to the end without
            // re-buffering. Calling `play()` earlier (the moment the
            // element exists) drains the tiny initial network buffer
            // and the audio stutters as the decoder waits for more
            // data. We still start muted so autoplay isn't blocked,
            // and unmute once `play()`'s promise resolves.
            {
                let audio_clone = audio_el.clone();
                let played = Rc::new(Cell::new(false));
                let played_ref = Rc::clone(&played);
                let on_canplaythrough = Closure::wrap(Box::new(move |_ev: web_sys::Event| {
                    if played_ref.get() {
                        return;
                    }
                    played_ref.set(true);
                    if let Ok(promise) = audio_clone.play() {
                        let inner = audio_clone.clone();
                        wasm_bindgen_futures::spawn_local(async move {
                            if wasm_bindgen_futures::JsFuture::from(promise).await.is_ok() {
                                inner.set_muted(false);
                            }
                        });
                    }
                })
                    as Box<dyn FnMut(web_sys::Event)>);
                let _ = audio_el.add_event_listener_with_callback(
                    "canplaythrough",
                    on_canplaythrough.as_ref().unchecked_ref(),
                );
                closures_ev.push(on_canplaythrough);
            }

            // `ended` → latch flag for tick loop to advance track.
            {
                let ended_ref = Rc::clone(&streaming_ended);
                let on_ended = Closure::wrap(Box::new(move |_ev: web_sys::Event| {
                    ended_ref.set(true);
                }) as Box<dyn FnMut(web_sys::Event)>);
                let _ = audio_el
                    .add_event_listener_with_callback("ended", on_ended.as_ref().unchecked_ref());
                closures_ev.push(on_ended);
            }
            // `error` → latch a generic message; also flag ended so
            // the outer state machine releases the source rather than
            // wedging in Buffering forever. (We don't read
            // `audio_el.error()` here because that requires the
            // `MediaError` web-sys feature; the radio status text
            // already includes "audio decode failed" which is enough
            // for the user to distinguish from a network error.)
            {
                let err_ref = Rc::clone(&streaming_error);
                let ended_ref = Rc::clone(&streaming_ended);
                let on_error = Closure::wrap(Box::new(move |_ev: web_sys::Event| {
                    *err_ref.borrow_mut() = Some("audio decode/network failure".to_string());
                    ended_ref.set(true);
                }) as Box<dyn FnMut(web_sys::Event)>);
                let _ = audio_el
                    .add_event_listener_with_callback("error", on_error.as_ref().unchecked_ref());
                closures_ev.push(on_error);
            }

            let id = self.next_id;
            self.next_id += 1;
            self.streaming_track = Some(StreamingTrack {
                audio_el,
                mode: StreamingMode::DirectUrl,
                // No MediaSource needed in direct-URL mode — the
                // `<audio>` element streams + decodes natively.
                // Avoids `MediaSource::new()`, which fails on browsers
                // without MSE support and would break the one path
                // that explicitly does not need it.
                media_source: None,
                source_buffer: Rc::new(RefCell::new(None)),
                pending_chunks: Rc::new(RefCell::new(VecDeque::new())),
                updating: Rc::new(Cell::new(false)),
                ready: Rc::new(Cell::new(false)),
                object_url: String::new(),
                streaming_ended,
                streaming_error,
                closures: Vec::new(),
                closures_ev,
                update_end_closure: Rc::new(RefCell::new(None)),
            });
            self.current_track = Some(id);
            self.playing = true;
            return Ok(AudioTrackId(id));
        }

        let media_source = web_sys::MediaSource::new().map_err(js_err)?;
        let object_url =
            web_sys::Url::create_object_url_with_source(&media_source).map_err(js_err)?;

        let audio_el = web_sys::HtmlAudioElement::new().map_err(js_err)?;
        audio_el.set_src(&object_url);
        // Some browsers (notably Chrome) refuse to actually output sound
        // from a detached `HTMLAudioElement` even after the user
        // gesture has unlocked autoplay — the element has to be in the
        // document tree. Hide it visually but keep it attached. Without
        // this, the radio "streams" (network bytes flow, MSE buffers
        // append) but never produces audible output.
        audio_el
            .style()
            .set_property("display", "none")
            .map_err(js_err)?;
        attach_audio_to_body(&audio_el, "MSE");

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

        // Per-track latched event flags; MSE mode currently doesn't
        // attach `ended`/`error` listeners to the `<audio>` element
        // (no auto-advance pipeline through MSE), but the fields are
        // present so `take_streaming_*` can read them uniformly.
        let streaming_ended: Rc<Cell<bool>> = Rc::new(Cell::new(false));
        let streaming_error: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));

        self.streaming_track = Some(StreamingTrack {
            audio_el,
            mode: StreamingMode::Mse,
            media_source: Some(media_source),
            source_buffer,
            pending_chunks,
            updating,
            ready,
            object_url,
            streaming_ended,
            streaming_error,
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
            // Direct-URL mode: the `<audio>` element handles its own
            // network streaming, so any data the radio source produces
            // here is just the synthetic primer chunk and should be
            // discarded.
            if st.mode == StreamingMode::DirectUrl {
                return Ok(());
            }
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
