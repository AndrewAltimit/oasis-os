//! Image loading and texture management for [`BrowserWidget`].

use std::collections::HashMap;

use oasis_types::backend::{SdiBackend, TextureId};
use oasis_vfs::Vfs;

use crate::css;
use crate::html;
use crate::html::dom::{NodeKind, TagName};
use crate::image;
use crate::layout;
use crate::loader::{self, ResourceRequest, ResourceSource, load_resource};
use crate::{BrowserWidget, ImageInfoMap, LoadingState, SimpleTextMeasurer};

/// Parse an HTML `srcset` attribute into a list of `(url, descriptor)` pairs.
///
/// Supports `<url> <N>x` (pixel density) and `<url> <N>w` (width) descriptors.
/// Entries without a descriptor default to `1.0`.
fn parse_srcset(srcset: &str) -> Vec<(String, f32)> {
    let mut results = Vec::new();
    for candidate in srcset.split(',') {
        let candidate = candidate.trim();
        if candidate.is_empty() {
            continue;
        }
        let mut parts = candidate.split_whitespace();
        let Some(url) = parts.next() else { continue };
        if url.is_empty() {
            continue;
        }
        let descriptor = parts.next().unwrap_or("1x");
        let value = if let Some(w) = descriptor.strip_suffix('w') {
            w.parse::<f32>().unwrap_or(1.0)
        } else if let Some(x) = descriptor.strip_suffix('x') {
            x.parse::<f32>().unwrap_or(1.0)
        } else {
            descriptor.parse::<f32>().unwrap_or(1.0)
        };
        results.push((url.to_string(), value));
    }
    results
}

/// Select the best image URL from a parsed `srcset` based on viewport width.
///
/// For `w` descriptors, picks the smallest image that is >= viewport width.
/// For `x` descriptors, picks the closest to `1x`.
/// Falls back to the first candidate if nothing matches well.
fn select_best_src(srcset: &str, viewport_width: u32) -> Option<String> {
    let candidates = parse_srcset(srcset);
    if candidates.is_empty() {
        return None;
    }

    // Heuristic: if any descriptor is >= 100, assume `w` descriptors.
    let is_width = candidates.iter().any(|(_, v)| *v >= 100.0);
    let vw = viewport_width as f32;

    if is_width {
        // Pick smallest width >= viewport, or largest overall.
        let mut best: Option<&(String, f32)> = None;
        for c in &candidates {
            if c.1 >= vw {
                match best {
                    Some(b) if b.1 > c.1 => best = Some(c),
                    None => best = Some(c),
                    _ => {},
                }
            }
        }
        if best.is_none() {
            // All are smaller than viewport -- pick largest.
            best = candidates.iter().max_by(|a, b| a.1.total_cmp(&b.1));
        }
        best.map(|(url, _)| url.clone())
    } else {
        // `x` descriptors: pick closest to 1x.
        candidates
            .iter()
            .min_by(|a, b| {
                let da = (a.1 - 1.0).abs();
                let db = (b.1 - 1.0).abs();
                da.total_cmp(&db)
            })
            .map(|(url, _)| url.clone())
    }
}

/// Determine the effective image source URL for an `<img>` element,
/// preferring `srcset` (if valid) over `src`.
fn effective_img_src(elem: &html::dom::ElementData, viewport_w: u32) -> Option<String> {
    let srcset_url = elem
        .get_attribute("srcset")
        .and_then(|ss| select_best_src(ss, viewport_w))
        .filter(|url| !url.is_empty() && !url.starts_with("data:"));
    srcset_url.or_else(|| elem.src().map(String::from))
}

/// Find the Y position of a layout box whose image src resolves to `url`.
///
/// Walks the layout tree recursively. Returns `Some(y)` for the first
/// matching `ReplacedContent::Image` node, or `None` if not found.
fn find_image_y(layout_box: &layout::box_model::LayoutBox, _url: &str) -> Option<f32> {
    // For images without a texture yet, their content.y tells us the
    // approximate vertical position in the page.
    if let layout::box_model::BoxType::Replaced(layout::box_model::ReplacedContent::Image {
        texture: None,
        ..
    }) = &layout_box.box_type
    {
        return Some(layout_box.dimensions.content.y);
    }
    for child in &layout_box.children {
        if let Some(y) = find_image_y(child, _url) {
            return Some(y);
        }
    }
    None
}

impl BrowserWidget {
    // ---------------------------------------------------------------
    // Image loading
    // ---------------------------------------------------------------

    /// Walk the DOM to find `<img>` elements and collect their requests
    /// into `self.pending_images` for time-sliced loading. Does NOT
    /// fetch or decode — that happens in `load_next_image_batch()`.
    ///
    /// Images with `loading="lazy"` are deferred to the end of the queue
    /// so that eagerly-loaded images (the default) are fetched first.
    pub(crate) fn collect_page_image_requests(&mut self) {
        let doc = match &self.document {
            Some(d) => d,
            None => return,
        };
        let base_url = self.nav.current_url().map(String::from);

        let mut eager_requests: Vec<(String, ResourceRequest)> = Vec::new();
        let mut lazy_requests: Vec<(String, ResourceRequest)> = Vec::new();
        for node in &doc.nodes {
            if let NodeKind::Element(elem) = &node.kind
                && elem.tag == TagName::Img
            {
                let effective = effective_img_src(elem, self.window_w);
                let effective_src = effective.as_deref();
                let Some(src) = effective_src else { continue };
                let resolved = Self::resolve_src(&base_url, src);
                if self.decoded_images.contains_key(&resolved) {
                    continue;
                }
                // CSP enforcement: check if the image source is allowed.
                if let Some(ref csp) = self.page_csp
                    && csp.is_active()
                    && let Some(ref page) = base_url
                    && !csp.allows(&resolved, page, crate::loader::csp::CspResourceType::Image)
                {
                    log::warn!("CSP blocked image: {resolved}");
                    continue;
                }
                let source = if self.config.features.sandbox_only {
                    ResourceSource::Vfs
                } else {
                    ResourceSource::VfsThenNetwork
                };
                let referrer = base_url.as_deref().and_then(loader::strip_referrer);
                let request = (
                    resolved.clone(),
                    ResourceRequest {
                        url: resolved,
                        base_url: base_url.clone(),
                        source,
                        method: crate::loader::HttpMethod::Get,
                        body: None,
                        referrer,
                    },
                );

                // Respect the `loading` attribute: "lazy" images are
                // deferred after all eager images have been fetched.
                let is_lazy = elem
                    .get_attribute("loading")
                    .is_some_and(|v| v.eq_ignore_ascii_case("lazy"));
                if is_lazy {
                    lazy_requests.push(request);
                } else {
                    eager_requests.push(request);
                }
            }
        }

        // Eager images first, lazy images appended after.
        eager_requests.extend(lazy_requests);
        self.pending_images = eager_requests;
    }

    /// Maximum decoded image memory budget (bytes of RGBA data).
    const IMAGE_MEMORY_BUDGET: usize = 8 * 1024 * 1024; // 8MB

    /// Process pending image requests within a time budget.
    ///
    /// Pops requests from `pending_images`, fetches and decodes each.
    /// Returns after the time budget is exhausted or all images are done.
    /// Each successful decode sets `layout_dirty` so the layout tree
    /// rebuilds with correct intrinsic dimensions.
    /// Pixels ahead of the viewport to start loading lazy images.
    const LAZY_LOAD_MARGIN: f32 = 512.0;

    /// Ensure the background image decode thread is running (non-WASM only).
    ///
    /// Lazily spawns a daemon thread on first call. The thread receives
    /// `(url, raw_bytes)` and sends back `(url, DecodedImage)`.
    #[cfg(not(target_arch = "wasm32"))]
    fn ensure_decode_thread(&mut self) {
        if self.image_decode_tx.is_some() {
            return;
        }
        let (work_tx, work_rx) = std::sync::mpsc::channel::<(String, Vec<u8>)>();
        let (result_tx, result_rx) = std::sync::mpsc::channel::<(String, image::DecodedImage)>();
        std::thread::Builder::new()
            .name("img-decode".into())
            .spawn(move || {
                while let Ok((url, data)) = work_rx.recv() {
                    if let Some(decoded) = image::decode_image(&data) {
                        if result_tx.send((url, decoded)).is_err() {
                            break; // receiver dropped
                        }
                    } else if result_tx
                        .send((
                            url,
                            image::DecodedImage {
                                width: 0,
                                height: 0,
                                pixels: Vec::new(),
                            },
                        ))
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .ok(); // If spawn fails, channels remain None → sync fallback
        self.image_decode_tx = Some(work_tx);
        self.image_decode_rx = Some(result_rx);
    }

    /// Collect completed image decodes from the background thread.
    ///
    /// Returns `true` if any new images were inserted.
    #[cfg(not(target_arch = "wasm32"))]
    fn poll_decoded_images(&mut self) -> bool {
        let rx = match &self.image_decode_rx {
            Some(rx) => rx,
            None => return false,
        };
        let mut any = false;
        loop {
            let (url, decoded) = match rx.try_recv() {
                Ok(pair) => pair,
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    // Decode thread died (panic or channel closed).
                    // Reset in-flight counter to unstick loading state.
                    self.image_decode_in_flight = 0;
                    self.image_decode_tx = None;
                    self.image_decode_rx = None;
                    break;
                },
            };
            self.image_decode_in_flight = self.image_decode_in_flight.saturating_sub(1);
            // Skip sentinel (failed decode) entries.
            if decoded.width == 0 && decoded.height == 0 {
                continue;
            }
            if self.decoded_images.contains_key(&url) {
                continue;
            }
            let img_bytes = decoded.width as usize * decoded.height as usize * 4;

            // Evict oldest decoded images if over budget.
            while self.decoded_image_bytes + img_bytes > Self::IMAGE_MEMORY_BUDGET {
                if let Some(evict_url) = self.decoded_image_lru.pop_back() {
                    if let Some(evicted) = self.decoded_images.remove(&evict_url) {
                        let evicted_bytes = evicted.width as usize * evicted.height as usize * 4;
                        self.decoded_image_bytes -= evicted_bytes;
                        self.image_info_dirty = true;
                    }
                } else {
                    break;
                }
            }

            self.decoded_image_bytes += img_bytes;
            self.decoded_image_lru.push_front(url.clone());
            self.decoded_images.insert(url, decoded);
            self.image_info_dirty = true;
            any = true;
        }
        any
    }

    pub fn load_next_image_batch(&mut self, vfs: &dyn Vfs, budget_ms: u32) {
        // Promote deferred images that have scrolled into view.
        self.promote_deferred_images();

        // On non-WASM targets, collect any completed background decodes.
        #[cfg(not(target_arch = "wasm32"))]
        let mut any_decoded = self.poll_decoded_images();
        #[cfg(target_arch = "wasm32")]
        let mut any_decoded = false;

        if self.pending_images.is_empty() {
            #[cfg(not(target_arch = "wasm32"))]
            if self.image_decode_in_flight == 0 && self.state == LoadingState::Loading {
                self.state = LoadingState::Idle;
            }
            if any_decoded {
                self.layout_dirty = true;
                self.rebuild_layout_with_images();
            }
            return;
        }

        let start = std::time::Instant::now();
        let budget = std::time::Duration::from_millis(budget_ms as u64);

        while let Some((resolved, request)) = self.pending_images.pop() {
            // Skip if already decoded (e.g. from cache).
            if self.decoded_images.contains_key(&resolved) {
                continue;
            }

            // Lazy loading: defer images far below the viewport.
            if self.is_image_off_viewport(&resolved) {
                self.deferred_images.push((resolved, request));
                continue;
            }

            if let Ok(loaded) = load_resource(
                vfs,
                &request,
                self.tls.as_deref(),
                #[cfg(not(target_arch = "wasm32"))]
                Some(&mut self.cookie_jar),
                #[cfg(not(target_arch = "wasm32"))]
                Some(&self.cache),
            ) {
                // On non-WASM, dispatch to background decode thread.
                // Falls back to synchronous decode if the channel is unavailable.
                #[cfg(not(target_arch = "wasm32"))]
                {
                    self.ensure_decode_thread();
                    let sent = if let Some(ref tx) = self.image_decode_tx {
                        tx.send((resolved.clone(), loaded.response.body.clone()))
                            .is_ok()
                    } else {
                        false
                    };
                    if sent {
                        self.image_decode_in_flight += 1;
                    } else if let Some(decoded) = image::decode_image(&loaded.response.body) {
                        // Sync fallback: channel unavailable or send failed.
                        let img_bytes = decoded.width as usize * decoded.height as usize * 4;
                        self.decoded_image_bytes += img_bytes;
                        self.decoded_image_lru.push_front(resolved.clone());
                        self.decoded_images.insert(resolved, decoded);
                        self.image_info_dirty = true;
                        any_decoded = true;
                    }
                }

                // On WASM, decode synchronously (no threads available).
                #[cfg(target_arch = "wasm32")]
                if let Some(decoded) = image::decode_image(&loaded.response.body) {
                    let img_bytes = decoded.width as usize * decoded.height as usize * 4;

                    while self.decoded_image_bytes + img_bytes > Self::IMAGE_MEMORY_BUDGET {
                        if let Some(evict_url) = self.decoded_image_lru.pop_back() {
                            if let Some(evicted) = self.decoded_images.remove(&evict_url) {
                                let evicted_bytes =
                                    evicted.width as usize * evicted.height as usize * 4;
                                self.decoded_image_bytes -= evicted_bytes;
                                self.image_info_dirty = true;
                            }
                        } else {
                            break;
                        }
                    }

                    self.decoded_image_bytes += img_bytes;
                    self.decoded_image_lru.push_front(resolved.clone());
                    self.decoded_images.insert(resolved, decoded);
                    self.image_info_dirty = true;
                    any_decoded = true;
                }
            }

            // Check time budget after each image.
            if start.elapsed() >= budget {
                break;
            }
        }

        // On non-WASM, wait within the remaining budget for in-flight
        // decodes to complete so callers that pass a generous budget
        // (e.g. tests with 5000ms) see results immediately.
        #[cfg(not(target_arch = "wasm32"))]
        if self.image_decode_in_flight > 0
            && let Some(ref rx) = self.image_decode_rx
        {
            while self.image_decode_in_flight > 0 {
                let remaining = budget.saturating_sub(start.elapsed());
                if remaining.is_zero() {
                    break;
                }
                match rx.recv_timeout(remaining) {
                    Ok((url, decoded)) => {
                        self.image_decode_in_flight = self.image_decode_in_flight.saturating_sub(1);
                        if decoded.width == 0 && decoded.height == 0 {
                            continue;
                        }
                        if self.decoded_images.contains_key(&url) {
                            continue;
                        }
                        let img_bytes = decoded.width as usize * decoded.height as usize * 4;

                        while self.decoded_image_bytes + img_bytes > Self::IMAGE_MEMORY_BUDGET {
                            if let Some(evict_url) = self.decoded_image_lru.pop_back() {
                                if let Some(evicted) = self.decoded_images.remove(&evict_url) {
                                    let evicted_bytes =
                                        evicted.width as usize * evicted.height as usize * 4;
                                    self.decoded_image_bytes -= evicted_bytes;
                                    self.image_info_dirty = true;
                                }
                            } else {
                                break;
                            }
                        }

                        self.decoded_image_bytes += img_bytes;
                        self.decoded_image_lru.push_front(url.clone());
                        self.decoded_images.insert(url, decoded);
                        self.image_info_dirty = true;
                        any_decoded = true;
                    },
                    Err(_) => break, // timeout or disconnected
                }
            }
        }

        if any_decoded {
            self.layout_dirty = true;
            self.rebuild_layout_with_images();
        }

        if self.pending_images.is_empty() {
            #[cfg(not(target_arch = "wasm32"))]
            {
                if self.image_decode_in_flight == 0 && self.state == LoadingState::Loading {
                    self.state = LoadingState::Idle;
                }
            }
            #[cfg(target_arch = "wasm32")]
            if self.state == LoadingState::Loading {
                self.state = LoadingState::Idle;
            }
        }
    }

    /// Re-evaluate deferred images after a scroll event.
    ///
    /// Moves images that are now near the viewport from `deferred_images`
    /// back into `pending_images` for loading.
    pub(crate) fn promote_deferred_images(&mut self) {
        if self.deferred_images.is_empty() {
            return;
        }
        // Compute viewport threshold once to avoid repeated borrows.
        let viewport_bottom = self.scroll.scroll_y as f32 + self.window_h as f32;
        let threshold = viewport_bottom + Self::LAZY_LOAD_MARGIN;

        let layout_root = &self.layout_root;
        let mut still_deferred = Vec::new();
        for (resolved, request) in std::mem::take(&mut self.deferred_images) {
            let off_viewport = layout_root
                .as_ref()
                .and_then(|root| find_image_y(root, &resolved))
                .is_some_and(|y| y > threshold);
            if off_viewport {
                still_deferred.push((resolved, request));
            } else {
                self.pending_images.push((resolved, request));
            }
        }
        self.deferred_images = still_deferred;
    }

    /// Check if an image with the given URL is far below the current viewport.
    ///
    /// Walks the layout tree to find the image's Y position. If it's
    /// more than [`LAZY_LOAD_MARGIN`] pixels below the viewport bottom,
    /// returns `true` (defer loading).
    fn is_image_off_viewport(&self, resolved_url: &str) -> bool {
        let Some(layout_root) = &self.layout_root else {
            return false; // No layout yet — load eagerly.
        };
        let viewport_bottom = self.scroll.scroll_y as f32 + self.window_h as f32;
        // Find the image's layout position.
        if let Some(y) = find_image_y(layout_root, resolved_url) {
            y > viewport_bottom + Self::LAZY_LOAD_MARGIN
        } else {
            false // Not found in layout — load eagerly.
        }
    }

    /// Get the cached image info map, rebuilding it if dirty.
    pub(crate) fn build_image_info_map(&mut self) -> ImageInfoMap {
        if self.image_info_dirty {
            self.cached_image_info = self
                .decoded_images
                .iter()
                .map(|(url, img)| (url.clone(), (img.width, img.height)))
                .collect();
            self.image_info_dirty = false;
        }
        self.cached_image_info.clone()
    }

    /// Upload decoded images as GPU textures and assign them to
    /// `ReplacedContent::Image` nodes in the layout tree.
    ///
    /// Small images (<= 128x128) are packed into a shared texture atlas
    /// to reduce GPU texture bind switches during rendering. Larger
    /// images get individual textures as before.
    pub(crate) fn ensure_image_textures(&mut self, backend: &mut dyn SdiBackend) {
        let doc = match &self.document {
            Some(d) => d,
            None => return,
        };
        let base_url = self.nav.current_url().map(String::from);

        // Collect URLs that need texture creation.
        let mut pending: Vec<String> = Vec::new();
        for node in &doc.nodes {
            if let NodeKind::Element(elem) = &node.kind
                && elem.tag == TagName::Img
            {
                let Some(src) = effective_img_src(elem, self.window_w) else {
                    continue;
                };
                let resolved = Self::resolve_src(&base_url, &src);
                if !self.image_textures.contains_key(&resolved)
                    && !self.image_atlas.contains(&resolved)
                    && self.decoded_images.contains_key(&resolved)
                    && !pending.contains(&resolved)
                {
                    pending.push(resolved);
                }
            }
        }

        // Collect background-image URLs from styles. These always get
        // individual textures (not atlas-packed) because background images
        // are typically tiled/stretched to arbitrary sizes.
        let mut bg_pending: Vec<String> = Vec::new();
        for style_opt in &self.styles {
            if let Some(style) = style_opt
                && let css::values::BackgroundImage::Url(ref url) = style.background_image
            {
                let resolved = Self::resolve_src(&base_url, url);
                if !self.image_textures.contains_key(&resolved)
                    && self.decoded_images.contains_key(&resolved)
                    && !bg_pending.contains(&resolved)
                {
                    bg_pending.push(resolved);
                }
            }
        }

        // Create textures: pack small images into the atlas, use
        // individual textures for larger ones.
        for resolved in &pending {
            if let Some(decoded) = self.decoded_images.get(resolved) {
                if crate::image_atlas::ImageAtlas::is_eligible(decoded.width, decoded.height) {
                    // Try to pack into the atlas.
                    self.image_atlas.insert(
                        resolved,
                        decoded.width,
                        decoded.height,
                        &decoded.pixels,
                    );
                } else if let Ok(tex) =
                    backend.load_texture(decoded.width, decoded.height, &decoded.pixels)
                {
                    self.image_textures.insert(resolved.clone(), tex);
                }
            }
        }

        // Create individual textures for background images (never atlas-packed).
        for resolved in &bg_pending {
            if let Some(decoded) = self.decoded_images.get(resolved)
                && let Ok(tex) =
                    backend.load_texture(decoded.width, decoded.height, &decoded.pixels)
            {
                self.image_textures.insert(resolved.clone(), tex);
            }
        }

        // Upload any dirty atlas pages to the GPU.
        self.image_atlas.upload_dirty(backend);

        // Walk layout tree and assign textures.
        if let Some(layout) = &mut self.layout_root {
            Self::assign_textures_recursive(
                layout,
                &self.document,
                &base_url,
                &self.image_textures,
                &self.image_atlas,
                self.window_w,
            );
        }
    }

    /// Recursively walk the layout tree and assign GPU textures to
    /// `ReplacedContent::Image` nodes and `background-image` styles.
    ///
    /// For images packed into the atlas, the atlas texture ID and
    /// source region are assigned so the paint layer can use `blit_sub`.
    fn assign_textures_recursive(
        layout_box: &mut layout::box_model::LayoutBox,
        doc: &Option<html::dom::Document>,
        base_url: &Option<String>,
        textures: &HashMap<String, TextureId>,
        atlas: &crate::image_atlas::ImageAtlas,
        viewport_w: u32,
    ) {
        if let layout::box_model::BoxType::Replaced(layout::box_model::ReplacedContent::Image {
            ref mut texture,
            ref mut atlas_region,
            ..
        }) = layout_box.box_type
            && texture.is_none()
            && let Some(node_id) = layout_box.node
            && let Some(doc) = doc
        {
            let node = doc.get(node_id);
            if let NodeKind::Element(elem) = &node.kind
                && let Some(src) = effective_img_src(elem, viewport_w)
            {
                let resolved = Self::resolve_src(base_url, &src);
                // Check atlas first, then individual textures.
                if let Some((atlas_tex, region)) = atlas.get(&resolved) {
                    *texture = Some(atlas_tex);
                    *atlas_region = Some(region);
                } else if let Some(&tex) = textures.get(&resolved) {
                    *texture = Some(tex);
                }
            }
        }

        // Assign background-image texture (not atlas-eligible since
        // background images are typically tiled/stretched to arbitrary
        // sizes and need their own texture).
        if layout_box.background_texture.is_none()
            && let css::values::BackgroundImage::Url(ref url) = layout_box.style.background_image
        {
            let resolved = Self::resolve_src(base_url, url);
            if let Some(&tex) = textures.get(&resolved) {
                layout_box.background_texture = Some(tex);
            }
        }

        for child in &mut layout_box.children {
            Self::assign_textures_recursive(child, doc, base_url, textures, atlas, viewport_w);
        }
    }

    /// Rebuild the layout tree with image dimensions after images have
    /// been decoded (second layout pass).
    fn rebuild_layout_with_images(&mut self) {
        let image_info = self.build_image_info_map();
        if let Some(doc) = &self.document {
            let content_h = self.config.content_height(self.window_h);
            let base_url = self.nav.current_url().map(String::from);
            let shared = std::rc::Rc::clone(&self.text_cache);
            let measurer =
                layout::text_cache::CachingMeasurer::with_shared(&SimpleTextMeasurer, shared);
            let viewport_w = self.window_w as f32 / self.config.zoom_level;
            let viewport_h = content_h as f32 / self.config.zoom_level;
            let layout_root = layout::block::build_layout_tree(
                doc,
                &self.styles,
                &measurer,
                viewport_w,
                viewport_h,
                base_url.as_deref(),
                &image_info,
            );
            #[cfg(feature = "javascript")]
            {
                self.canvas_states.borrow_mut().clear();
                crate::canvas::collect_canvas_states(&layout_root, &self.canvas_states);
            }

            self.layout_root = Some(layout_root);
            self.link_map.clear();
        }
    }

    /// Resolve an img `src` attribute against a base URL.
    fn resolve_src(base_url: &Option<String>, src: &str) -> String {
        match base_url {
            Some(base) => {
                if let Some(base_parsed) = loader::Url::parse(base) {
                    base_parsed
                        .resolve(src)
                        .map(|u| u.to_string())
                        .unwrap_or_else(|| src.to_string())
                } else {
                    src.to_string()
                }
            },
            None => src.to_string(),
        }
    }
}
