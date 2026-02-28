//! Internet Archive source for WASM: fetch + ReadableStream for progressive download.
//!
//! Uses browser `fetch()` with `ReadableStream` for progressive MP3 download,
//! bridged to the synchronous `RadioSource` trait via shared state.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

use oasis_audio::radio::archive::ArchiveCatalog;
use oasis_audio::radio::icy::StreamMetadata;
use oasis_audio::radio::source::{AudioChunk, RadioSource, SourceState};
use oasis_types::error::{OasisError, Result};

// ---------------------------------------------------------------------------
// WasmArchiveSource -- streams a single MP3 file via fetch + ReadableStream
// ---------------------------------------------------------------------------

struct WasmFetchState {
    source_state: SourceState,
    audio_queue: VecDeque<Vec<u8>>,
    track_title: String,
    track_creator: String,
    metadata_sent: bool,
    error: Option<String>,
}

/// A `RadioSource` that downloads an MP3 file from the Internet Archive
/// using the browser's `fetch()` API with `ReadableStream` for progressive
/// streaming.
pub struct WasmArchiveSource {
    shared: Rc<RefCell<WasmFetchState>>,
}

impl WasmArchiveSource {
    /// Create a new source that begins fetching the given URL immediately.
    pub fn new(url: &str, title: &str, creator: &str) -> Self {
        let shared = Rc::new(RefCell::new(WasmFetchState {
            source_state: SourceState::Connecting,
            audio_queue: VecDeque::new(),
            track_title: title.to_string(),
            track_creator: creator.to_string(),
            metadata_sent: false,
            error: None,
        }));

        // Spawn the async fetch task.
        let shared_clone = Rc::clone(&shared);
        let url = url.to_string();
        wasm_bindgen_futures::spawn_local(async move {
            if let Err(e) = fetch_stream(&shared_clone, &url).await {
                let mut state = shared_clone.borrow_mut();
                state.error = Some(format!("{e:?}"));
                state.source_state = SourceState::Error;
            }
        });

        Self { shared }
    }
}

async fn fetch_stream(
    shared: &Rc<RefCell<WasmFetchState>>,
    url: &str,
) -> std::result::Result<(), JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("no window"))?;
    let resp_val = JsFuture::from(window.fetch_with_str(url)).await?;
    let resp: web_sys::Response = resp_val.dyn_into()?;

    if !resp.ok() {
        let status = resp.status();
        let mut state = shared.borrow_mut();
        state.error = Some(format!("HTTP {status}"));
        state.source_state = SourceState::Error;
        return Ok(());
    }

    {
        shared.borrow_mut().source_state = SourceState::Active;
    }

    let body = resp.body().ok_or_else(|| JsValue::from_str("no body"))?;
    let reader: web_sys::ReadableStreamDefaultReader = body.get_reader().dyn_into()?;

    loop {
        // Stop fetching if the source was disconnected externally.
        if shared.borrow().source_state == SourceState::Ended {
            let _ = reader.cancel();
            break;
        }
        let result = JsFuture::from(reader.read()).await?;
        let done = js_sys::Reflect::get(&result, &"done".into())?
            .as_bool()
            .unwrap_or(true);
        if done {
            shared.borrow_mut().source_state = SourceState::Ended;
            break;
        }
        let value = js_sys::Reflect::get(&result, &"value".into())?;
        let chunk: js_sys::Uint8Array = value.dyn_into()?;
        let data = chunk.to_vec();
        shared.borrow_mut().audio_queue.push_back(data);
    }

    Ok(())
}

impl RadioSource for WasmArchiveSource {
    fn poll(&mut self) -> Result<Option<AudioChunk>> {
        let mut state = self.shared.borrow_mut();

        if let Some(ref e) = state.error {
            let msg = e.clone();
            return Err(OasisError::Backend(msg));
        }

        if let Some(data) = state.audio_queue.pop_front() {
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
            Ok(Some(AudioChunk { data, metadata }))
        } else {
            Ok(None)
        }
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
    if items.is_empty() {
        return Err(JsValue::from_str("no items found"));
    }

    // 2. Fetch files for first few items.
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

    if catalog.tracks.is_empty() {
        return Err(JsValue::from_str("no MP3 files found"));
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
