//! Internet Archive source for WASM: hands the URL to the audio
//! backend so the `<audio>` element can stream it natively. The
//! `RadioSource` interface is satisfied with a one-shot dummy chunk
//! that satisfies `RadioManager`'s buffering logic; no MP3 bytes flow
//! through Rust.

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

use oasis_audio::radio::BUFFER_THRESHOLD;
use oasis_audio::radio::archive::ArchiveCatalog;
use oasis_audio::radio::icy::StreamMetadata;
use oasis_audio::radio::source::{AudioChunk, RadioSource, SourceState};
use oasis_types::error::Result;

/// Size of each synthetic primer chunk: enough that two of them put
/// `RadioManager`'s `audio_buf` comfortably above [`BUFFER_THRESHOLD`].
/// Tied to that constant so the state-machine walk stays correct if the
/// threshold ever changes.
const PRIMER_CHUNK_BYTES: usize = BUFFER_THRESHOLD * 2;

// ---------------------------------------------------------------------------
// WasmArchiveSource -- direct-URL playback handed to the audio backend.
// ---------------------------------------------------------------------------
//
// Earlier revisions fetched the MP3 ourselves and pushed bytes through
// `MediaSource` / `SourceBuffer("audio/mpeg")`. Firefox doesn't decode
// `audio/mpeg` through MSE ("Cannot play media. No decoders for
// requested formats: audio/mpeg") and even Chrome adds latency from the
// extra hop. We now hand the URL straight to a `<audio>` element via
// `RadioSource::streaming_url`; the audio backend points the element at
// `archive.org`'s download URL and lets the browser do native HTTP
// streaming + decoding. No fetch is spawned here.

struct WasmFetchState {
    source_state: SourceState,
    track_title: String,
    track_creator: String,
    metadata_sent: bool,
    /// Number of primer chunks already emitted. We need at least
    /// **two**: the first lands while `RadioManager` is in
    /// `Connecting` (it transitions to `Buffering` but doesn't start
    /// playback yet), the second lands while in `Buffering` and
    /// triggers `start_playback`. Once playback has begun the
    /// `<audio>` element handles the real bytes — further chunks
    /// from us would just be discarded by the audio backend's
    /// direct-URL feed_data shortcut, so we stop emitting.
    primers_emitted: u8,
}

/// A `RadioSource` that delegates streaming to the audio backend by
/// exposing the source URL via [`RadioSource::streaming_url`].
pub struct WasmArchiveSource {
    url: String,
    shared: Rc<RefCell<WasmFetchState>>,
}

impl WasmArchiveSource {
    /// Create a source that hands `url` to the audio backend.
    pub fn new(url: &str, title: &str, creator: &str) -> Self {
        let shared = Rc::new(RefCell::new(WasmFetchState {
            // Active immediately — the `<audio>` element is responsible
            // for the actual network connection.
            source_state: SourceState::Active,
            track_title: title.to_string(),
            track_creator: creator.to_string(),
            metadata_sent: false,
            primers_emitted: 0,
        }));
        Self {
            url: url.to_string(),
            shared,
        }
    }
}

impl RadioSource for WasmArchiveSource {
    fn poll(&mut self) -> Result<Option<AudioChunk>> {
        let mut state = self.shared.borrow_mut();

        if state.source_state == SourceState::Ended || state.source_state == SourceState::Error {
            return Ok(None);
        }

        // Emit synthetic primer chunks for the first two polls so
        // `RadioManager` walks through Connecting → Buffering →
        // Playing. The audio backend's `feed_data` ignores these
        // bytes in direct-URL mode — actual audio comes from the
        // `<audio>` element streaming the URL on its own.
        if state.primers_emitted < 2 {
            state.primers_emitted += 1;
            let metadata = if !state.metadata_sent {
                state.metadata_sent = true;
                let title = if state.track_creator.is_empty() {
                    state.track_title.clone()
                } else {
                    format!("{} - {}", state.track_creator, state.track_title)
                };
                Some(StreamMetadata { title })
            } else {
                None
            };
            // Each primer chunk is `PRIMER_CHUNK_BYTES` (= 2 ×
            // `BUFFER_THRESHOLD`); two of them put the buffer well
            // above threshold so playback can transition.
            return Ok(Some(AudioChunk {
                data: vec![0u8; PRIMER_CHUNK_BYTES],
                metadata,
            }));
        }
        Ok(None)
    }

    fn disconnect(&mut self) {
        self.shared.borrow_mut().source_state = SourceState::Ended;
    }

    fn state(&self) -> SourceState {
        self.shared.borrow().source_state
    }

    fn source_type(&self) -> &str {
        "wasm-archive"
    }

    fn streaming_url(&self) -> Option<&str> {
        Some(&self.url)
    }
}

// ---------------------------------------------------------------------------
// WasmArchiveCatalogFetcher -- async catalog discovery
// ---------------------------------------------------------------------------

struct CatalogFetchState {
    catalog: Option<ArchiveCatalog>,
    first_track_source: Option<Box<dyn RadioSource>>,
    error: Option<String>,
    done: bool,
}

/// Asynchronously fetches an IA collection's catalog and creates the first
/// track source. Synchronous code polls `is_ready()` each frame.
pub struct WasmArchiveCatalogFetcher {
    shared: Rc<RefCell<CatalogFetchState>>,
}

impl WasmArchiveCatalogFetcher {
    /// Start fetching catalog for the given collection.
    pub fn new(collection: &str, seed: u64) -> Self {
        let shared = Rc::new(RefCell::new(CatalogFetchState {
            catalog: None,
            first_track_source: None,
            error: None,
            done: false,
        }));

        let shared_clone = Rc::clone(&shared);
        let collection = collection.to_string();
        wasm_bindgen_futures::spawn_local(async move {
            match fetch_catalog(&collection, seed).await {
                Ok((catalog, source)) => {
                    let mut state = shared_clone.borrow_mut();
                    state.catalog = Some(catalog);
                    state.first_track_source = Some(source);
                    state.done = true;
                },
                Err(e) => {
                    let mut state = shared_clone.borrow_mut();
                    state.error = Some(format!("{e:?}"));
                    state.done = true;
                },
            }
        });

        Self { shared }
    }

    /// Check if the catalog fetch is complete.
    pub fn is_ready(&self) -> bool {
        self.shared.borrow().done
    }

    /// Extract the catalog and first track source (consumes the results).
    pub fn take_results(
        &self,
    ) -> std::result::Result<(ArchiveCatalog, Box<dyn RadioSource>), String> {
        let mut state = self.shared.borrow_mut();
        if let Some(ref e) = state.error {
            return Err(e.clone());
        }
        let catalog = state.catalog.take().ok_or("no catalog")?;
        let source = state.first_track_source.take().ok_or("no source")?;
        Ok((catalog, source))
    }
}

async fn fetch_catalog(
    collection: &str,
    seed: u64,
) -> std::result::Result<(ArchiveCatalog, Box<dyn RadioSource>), JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("no window"))?;

    // 1. Search for items in the collection.
    let search_url = ArchiveCatalog::search_url(collection);
    let resp_val = JsFuture::from(window.fetch_with_str(&search_url)).await?;
    let resp: web_sys::Response = resp_val.dyn_into()?;
    if !resp.ok() {
        return Err(JsValue::from_str(&format!(
            "search API: HTTP {}",
            resp.status()
        )));
    }
    let body_val = JsFuture::from(resp.text()?).await?;
    let body: String = body_val.as_string().unwrap_or_default();

    let items = ArchiveCatalog::parse_search_response(&body);

    // 2. Fetch files for first few items found by the collection
    // search.
    let mut catalog = ArchiveCatalog::new(collection);
    let max_items = items.len().min(5);
    for (item_id, _title, creator) in &items[..max_items] {
        let files_url = format!("https://archive.org/metadata/{item_id}/files");
        let freq = JsFuture::from(window.fetch_with_str(&files_url)).await?;
        let fresp: web_sys::Response = freq.dyn_into()?;
        if !fresp.ok() {
            continue; // Skip this item, try next.
        }
        let fbody_val = JsFuture::from(fresp.text()?).await?;
        let fbody: String = fbody_val.as_string().unwrap_or_default();
        let tracks = ArchiveCatalog::parse_files_response(&fbody, item_id, creator);
        catalog.tracks.extend(tracks);
    }

    // 3. Single-item fallback. Some "stations" in the registry are
    // actually IA item ids that hold many MP3 files directly (e.g.
    // `OTRR_This_Is_Your_FBI_Singles`, ~382 episodes). The collection
    // search returns zero hits for those, so try treating the
    // identifier itself as an item id and pull files from
    // `/metadata/<id>/files`. This mirrors the SDL backend's behaviour
    // in `oasis-app/src/main.rs::fetch_catalog_blocking`.
    if catalog.tracks.is_empty() {
        let files_url = format!("https://archive.org/metadata/{collection}/files");
        let freq = JsFuture::from(window.fetch_with_str(&files_url)).await?;
        let fresp: web_sys::Response = freq.dyn_into()?;
        if fresp.ok() {
            let fbody_val = JsFuture::from(fresp.text()?).await?;
            let fbody: String = fbody_val.as_string().unwrap_or_default();
            let tracks = ArchiveCatalog::parse_files_response(&fbody, collection, "Unknown");
            catalog.tracks.extend(tracks);
        }
    }

    if catalog.tracks.is_empty() {
        return Err(JsValue::from_str(&format!(
            "no MP3 files for '{collection}' (collection or item id)",
        )));
    }

    catalog.shuffle(seed);

    // 3. Create source for first track.
    let track = catalog
        .current_track()
        .cloned()
        .ok_or_else(|| JsValue::from_str("empty catalog after shuffle"))?;
    let url = ArchiveCatalog::download_url(&track);
    let source: Box<dyn RadioSource> =
        Box::new(WasmArchiveSource::new(&url, &track.title, &track.creator));

    Ok((catalog, source))
}
