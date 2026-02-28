//! Async TV catalog fetcher for WASM — discovers video episodes from IA metadata.
//!
//! Fetches `/metadata/{item_id}/files` for each channel source, parses the
//! response for MP4/h.264 video files, and builds `ChannelCatalog`s.

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;

use oasis_core::apps::tv_guide::catalog::ChannelCatalog;
use oasis_core::apps::tv_guide::channel::Channel;

struct FetchState {
    catalogs: Vec<Option<ChannelCatalog>>,
    done: bool,
    error: Option<String>,
}

/// Asynchronously fetches video catalogs for all TV channels.
/// Polls `is_ready()` each frame; when ready, call `take_results()`.
pub struct WasmTvCatalogFetcher {
    shared: Rc<RefCell<FetchState>>,
}

impl WasmTvCatalogFetcher {
    /// Start fetching catalogs for the given channels.
    pub fn new(channels: &[Channel]) -> Self {
        let channel_count = channels.len();
        let shared = Rc::new(RefCell::new(FetchState {
            catalogs: vec![None; channel_count],
            done: false,
            error: None,
        }));

        let shared_clone = Rc::clone(&shared);
        let channels: Vec<Channel> = channels.to_vec();
        wasm_bindgen_futures::spawn_local(async move {
            if let Err(e) = fetch_all_catalogs(&shared_clone, &channels).await {
                let mut state = shared_clone.borrow_mut();
                state.error = Some(format!("{e:?}"));
                state.done = true;
            }
        });

        Self { shared }
    }

    /// Check if all catalog fetches are complete.
    pub fn is_ready(&self) -> bool {
        self.shared.borrow().done
    }

    /// Extract the catalogs (one per channel, `None` if fetch failed for that channel).
    pub fn take_results(&self) -> Result<Vec<Option<ChannelCatalog>>, String> {
        let mut state = self.shared.borrow_mut();
        if let Some(ref e) = state.error {
            return Err(e.clone());
        }
        Ok(std::mem::take(&mut state.catalogs))
    }
}

async fn fetch_all_catalogs(
    shared: &Rc<RefCell<FetchState>>,
    channels: &[Channel],
) -> Result<(), wasm_bindgen::JsValue> {
    let window = web_sys::window().ok_or_else(|| wasm_bindgen::JsValue::from_str("no window"))?;

    for (ch_idx, channel) in channels.iter().enumerate() {
        let mut catalog = ChannelCatalog::new(channel.number);

        for source in &channel.source {
            let files_url = format!("https://archive.org/metadata/{}/files", source.item_id);
            let resp_val = match JsFuture::from(window.fetch_with_str(&files_url)).await {
                Ok(v) => v,
                Err(_) => continue, // Network error, skip source.
            };
            let resp: web_sys::Response = match resp_val.dyn_into() {
                Ok(r) => r,
                Err(_) => continue,
            };

            if !resp.ok() {
                continue; // Skip failed sources, try next.
            }

            let text_promise = match resp.text() {
                Ok(p) => p,
                Err(_) => continue,
            };
            let body_val = match JsFuture::from(text_promise).await {
                Ok(v) => v,
                Err(_) => continue,
            };
            let body: String = body_val.as_string().unwrap_or_default();

            let episodes = ChannelCatalog::parse_files_response(
                &body,
                &source.item_id,
                source.subfolder.as_deref(),
            );
            catalog.add_episodes(episodes);
        }

        if !catalog.episodes.is_empty() {
            shared.borrow_mut().catalogs[ch_idx] = Some(catalog);
        }
    }

    shared.borrow_mut().done = true;
    Ok(())
}
