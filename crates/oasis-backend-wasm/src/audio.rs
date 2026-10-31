//! `AudioBackend` implementation on HTML media elements.
//!
//! Two kinds of track:
//!
//! - **In-memory tracks** (`load_track`, the Music Player's files): the
//!   bytes go into a `Blob` whose object URL feeds an `<audio>` element,
//!   so the browser decodes MP3 / Ogg / WAV / FLAC natively. That keeps
//!   the trait's synchronous API (Web Audio's `decodeAudioData` is async)
//!   and gives real position / duration / pause for the progress bar.
//! - **Streaming tracks** (`load_streaming`, internet radio): an `<audio>`
//!   element pointed straight at the stream URL, or fed through MSE
//!   (`MediaSource` + `SourceBuffer("audio/mpeg")`).

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

use wasm_bindgen::prelude::*;

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
    /// Event listeners, removed again by [`StreamingTrack::release`].
    closures_ev: Vec<Listener>,
    // Hold the updateend closure (replaces .forget() leak).
    #[allow(dead_code)]
    update_end_closure: SharedClosure,
}

/// An event listener a track registered: the target, the event name and
/// the closure, so the listener can be removed before the closure drops.
type Listener = (
    web_sys::EventTarget,
    &'static str,
    Closure<dyn FnMut(web_sys::Event)>,
);

impl StreamingTrack {
    /// Stop the track and release its element, listeners and object URL.
    ///
    /// Listeners are removed before their closures drop: a late `error`
    /// (the stream connection dropping) or `sourceopen` would otherwise
    /// call a dropped wasm-bindgen closure and throw. The source goes with
    /// `removeAttribute("src")` + `load()`, which also ends the element's
    /// network fetch (removing the element alone does not).
    fn release(self) {
        for (target, name, closure) in &self.closures_ev {
            let _ =
                target.remove_event_listener_with_callback(name, closure.as_ref().unchecked_ref());
        }
        if let Some(sb) = self.source_buffer.borrow().as_ref() {
            sb.set_onupdateend(None);
        }
        let _ = self.audio_el.pause();
        let _ = self.audio_el.remove_attribute("src");
        self.audio_el.load();
        self.audio_el.remove();
        if !self.object_url.is_empty() {
            let _ = web_sys::Url::revoke_object_url(&self.object_url);
        }
    }
}

// ---------------------------------------------------------------------------
// WasmAudioBackend
// ---------------------------------------------------------------------------

/// An in-memory track: an `<audio>` element playing a `Blob` URL.
struct MemoryTrack {
    el: web_sys::HtmlAudioElement,
    /// Object URL of the track's `Blob`, revoked on unload.
    url: String,
}

impl MemoryTrack {
    /// Pause and release the element and its object URL.
    fn release(self) {
        let _ = self.el.pause();
        self.el.remove_attribute("src").ok();
        // Without `load()` the element keeps its media resource.
        self.el.load();
        let _ = web_sys::Url::revoke_object_url(&self.url);
    }
}

/// MIME type for an in-memory track, sniffed from its magic bytes so the
/// `Blob` is typed (browsers sniff untyped blobs too, but a type makes
/// `<audio>` pick the decoder without guessing). Unknown data is passed
/// as `audio/mpeg`, the bundled samples' format.
pub(crate) fn sniff_audio_mime(data: &[u8]) -> &'static str {
    match data {
        [b'O', b'g', b'g', b'S', ..] => "audio/ogg",
        [
            b'R',
            b'I',
            b'F',
            b'F',
            _,
            _,
            _,
            _,
            b'W',
            b'A',
            b'V',
            b'E',
            ..,
        ] => "audio/wav",
        [b'f', b'L', b'a', b'C', ..] => "audio/flac",
        [_, _, _, _, b'f', b't', b'y', b'p', ..] => "audio/mp4",
        _ => "audio/mpeg",
    }
}

pub struct WasmAudioBackend {
    /// Loaded in-memory tracks.
    tracks: HashMap<u64, MemoryTrack>,
    next_id: u64,
    /// The playing (or paused) track: an in-memory track id or the
    /// streaming track's id.
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
            tracks: HashMap::new(),
            next_id: 1,
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

    /// The element of the current track: the in-memory track's, else the
    /// streaming track's.
    fn current_element(&self) -> Option<&web_sys::HtmlAudioElement> {
        let id = self.current_track?;
        match self.tracks.get(&id) {
            Some(t) => Some(&t.el),
            None => self.streaming_track.as_ref().map(|st| &st.audio_el),
        }
    }
}

/// Start `el` playing; a rejected `play()` promise (autoplay policy,
/// undecodable data) is logged instead of left unhandled.
fn start_element(el: &web_sys::HtmlAudioElement) -> Result<()> {
    let promise = el.play().map_err(js_err)?;
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(e) = wasm_bindgen_futures::JsFuture::from(promise).await {
            web_sys::console::warn_1(&JsValue::from_str(&format!(
                "oasis audio: play() rejected: {e:?}"
            )));
        }
    });
    Ok(())
}

/// Media-element time (seconds, NaN / infinite before metadata or for a
/// live stream) as whole milliseconds, 0 when unknown.
fn element_ms(secs: f64) -> u64 {
    if secs.is_finite() && secs > 0.0 {
        (secs * 1000.0) as u64
    } else {
        0
    }
}

impl Default for WasmAudioBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioBackend for WasmAudioBackend {
    fn init(&mut self) -> Result<()> {
        Ok(())
    }

    fn shutdown(&mut self) -> Result<()> {
        self.stop()?;
        for (_, track) in self.tracks.drain() {
            track.release();
        }
        Ok(())
    }

    fn load_track(&mut self, data: &[u8]) -> Result<AudioTrackId> {
        let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(data));
        let bag = web_sys::BlobPropertyBag::new();
        bag.set_type(sniff_audio_mime(data));
        let blob =
            web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &bag).map_err(js_err)?;
        let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(js_err)?;
        let el = match web_sys::HtmlAudioElement::new_with_src(&url) {
            Ok(el) => el,
            Err(e) => {
                let _ = web_sys::Url::revoke_object_url(&url);
                return Err(js_err(e));
            },
        };
        el.set_preload("auto");
        el.set_volume(f64::from(self.volume) / 100.0);
        let id = self.next_id;
        self.next_id += 1;
        self.tracks.insert(id, MemoryTrack { el, url });
        Ok(AudioTrackId(id))
    }

    fn unload_track(&mut self, track: AudioTrackId) -> Result<()> {
        if self.current_track == Some(track.0) && self.tracks.contains_key(&track.0) {
            self.current_track = None;
            self.playing = false;
            self.paused = false;
        }
        if let Some(t) = self.tracks.remove(&track.0) {
            t.release();
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

        let el = self
            .tracks
            .get(&track.0)
            .map(|t| t.el.clone())
            .ok_or_else(|| OasisError::Backend(format!("track {} not found", track.0).into()))?;
        // One in-memory track plays at a time.
        if let Some(prev) = self.current_track.filter(|&id| id != track.0)
            && let Some(t) = self.tracks.get(&prev)
        {
            let _ = t.el.pause();
        }
        el.set_current_time(0.0);
        start_element(&el)?;
        self.current_track = Some(track.0);
        self.playing = true;
        self.paused = false;
        Ok(())
    }

    fn pause(&mut self) -> Result<()> {
        if let Some(el) = self.current_element() {
            let _ = el.pause();
        }
        self.paused = true;
        self.playing = false;
        Ok(())
    }

    fn resume(&mut self) -> Result<()> {
        if !self.paused {
            return Ok(());
        }
        if let Some(el) = self.current_element() {
            start_element(el)?;
        }
        self.playing = true;
        self.paused = false;
        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        // Stop the in-memory track (it stays loaded until unloaded).
        if let Some(t) = self.current_track.and_then(|id| self.tracks.get(&id)) {
            let _ = t.el.pause();
            t.el.set_current_time(0.0);
        }
        self.current_track = None;
        // Stop the streaming track.
        if let Some(st) = self.streaming_track.take() {
            st.release();
        }
        self.playing = false;
        self.paused = false;
        Ok(())
    }

    fn set_volume(&mut self, volume: u8) -> Result<()> {
        self.volume = volume.min(100);
        let v = f64::from(self.volume) / 100.0;
        for t in self.tracks.values() {
            t.el.set_volume(v);
        }
        if let Some(ref st) = self.streaming_track {
            st.audio_el.set_volume(v);
        }
        Ok(())
    }

    fn get_volume(&self) -> u8 {
        self.volume
    }

    fn is_playing(&self) -> bool {
        // An in-memory track that reached its end is no longer playing.
        self.playing
            && !self
                .current_track
                .and_then(|id| self.tracks.get(&id))
                .is_some_and(|t| t.el.ended())
    }

    fn position_ms(&self) -> u64 {
        self.current_element()
            .map_or(0, |el| element_ms(el.current_time()))
    }

    fn duration_ms(&self) -> u64 {
        self.current_element()
            .map_or(0, |el| element_ms(el.duration()))
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
            // Detaches the previous element so it doesn't leak across
            // station changes.
            st.release();
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

            let mut closures_ev: Vec<Listener> = Vec::new();

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
                closures_ev.push((audio_el.clone().into(), "canplaythrough", on_canplaythrough));
            }

            // `ended` → latch flag for tick loop to advance track.
            {
                let ended_ref = Rc::clone(&streaming_ended);
                let on_ended = Closure::wrap(Box::new(move |_ev: web_sys::Event| {
                    ended_ref.set(true);
                }) as Box<dyn FnMut(web_sys::Event)>);
                let _ = audio_el
                    .add_event_listener_with_callback("ended", on_ended.as_ref().unchecked_ref());
                closures_ev.push((audio_el.clone().into(), "ended", on_ended));
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
                closures_ev.push((audio_el.clone().into(), "error", on_error));
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
        let mut closures_ev: Vec<Listener> = Vec::new();

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
            closures_ev.push((media_source.clone().into(), "sourceopen", on_open_ev));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_common_audio_containers() {
        assert_eq!(sniff_audio_mime(b"OggS\0\x02"), "audio/ogg");
        assert_eq!(sniff_audio_mime(b"RIFF\x24\0\0\0WAVEfmt "), "audio/wav");
        assert_eq!(sniff_audio_mime(b"fLaC\0\0\0\x22"), "audio/flac");
        assert_eq!(sniff_audio_mime(b"\0\0\0\x20ftypM4A "), "audio/mp4");
        assert_eq!(sniff_audio_mime(b"ID3\x04\0"), "audio/mpeg");
        assert_eq!(sniff_audio_mime(&[0xFF, 0xFB, 0x90]), "audio/mpeg");
        // RIFF that is not WAVE (e.g. AVI) is not called WAV.
        assert_eq!(sniff_audio_mime(b"RIFF\x24\0\0\0AVI LIST"), "audio/mpeg");
        assert_eq!(sniff_audio_mime(b""), "audio/mpeg");
    }

    #[test]
    fn element_time_conversion() {
        assert_eq!(element_ms(1.5), 1500);
        assert_eq!(element_ms(f64::NAN), 0);
        assert_eq!(element_ms(f64::INFINITY), 0);
        assert_eq!(element_ms(-1.0), 0);
    }
}
