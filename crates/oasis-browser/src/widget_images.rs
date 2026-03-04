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

impl BrowserWidget {
    // ---------------------------------------------------------------
    // Image loading
    // ---------------------------------------------------------------

    /// Walk the DOM to find `<img>` elements and collect their requests
    /// into `self.pending_images` for time-sliced loading. Does NOT
    /// fetch or decode — that happens in `load_next_image_batch()`.
    pub(crate) fn collect_page_image_requests(&mut self) {
        let doc = match &self.document {
            Some(d) => d,
            None => return,
        };
        let base_url = self.nav.current_url().map(String::from);

        let mut requests: Vec<(String, ResourceRequest)> = Vec::new();
        for node in &doc.nodes {
            if let NodeKind::Element(elem) = &node.kind
                && elem.tag == TagName::Img
                && let Some(src) = elem.src()
            {
                let resolved = Self::resolve_src(&base_url, src);
                if self.decoded_images.contains_key(&resolved) {
                    continue;
                }
                let source = if self.config.features.sandbox_only {
                    ResourceSource::Vfs
                } else {
                    ResourceSource::VfsThenNetwork
                };
                requests.push((
                    resolved.clone(),
                    ResourceRequest {
                        url: resolved,
                        base_url: base_url.clone(),
                        source,
                    },
                ));
            }
        }

        self.pending_images = requests;
    }

    /// Maximum decoded image memory budget (bytes of RGBA data).
    const IMAGE_MEMORY_BUDGET: usize = 8 * 1024 * 1024; // 8MB

    /// Process pending image requests within a time budget.
    ///
    /// Pops requests from `pending_images`, fetches and decodes each.
    /// Returns after the time budget is exhausted or all images are done.
    /// Each successful decode sets `layout_dirty` so the layout tree
    /// rebuilds with correct intrinsic dimensions.
    pub fn load_next_image_batch(&mut self, vfs: &dyn Vfs, budget_ms: u32) {
        if self.pending_images.is_empty() {
            return;
        }

        let start = std::time::Instant::now();
        let budget = std::time::Duration::from_millis(budget_ms as u64);
        let mut any_decoded = false;

        while let Some((resolved, request)) = self.pending_images.pop() {
            // Skip if already decoded (e.g. from cache).
            if self.decoded_images.contains_key(&resolved) {
                continue;
            }

            if let Ok(response) = load_resource(vfs, &request, self.tls.as_deref())
                && let Some(decoded) = image::decode_image(&response.body)
            {
                let img_bytes = (decoded.width * decoded.height * 4) as usize;

                // Evict oldest decoded images if over budget.
                while self.decoded_image_bytes + img_bytes > Self::IMAGE_MEMORY_BUDGET {
                    if let Some(evict_url) = self.decoded_image_lru.pop_back() {
                        if let Some(evicted) = self.decoded_images.remove(&evict_url) {
                            let evicted_bytes = (evicted.width * evicted.height * 4) as usize;
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

            // Check time budget after each image.
            if start.elapsed() >= budget {
                break;
            }
        }

        if any_decoded {
            self.layout_dirty = true;
            self.rebuild_layout_with_images();
        }

        if self.pending_images.is_empty() && self.state == LoadingState::Loading {
            self.state = LoadingState::Idle;
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
                && let Some(src) = elem.src()
            {
                let resolved = Self::resolve_src(&base_url, src);
                if !self.image_textures.contains_key(&resolved)
                    && self.decoded_images.contains_key(&resolved)
                {
                    pending.push(resolved);
                }
            }
        }

        // Create textures.
        for resolved in &pending {
            if let Some(decoded) = self.decoded_images.get(resolved)
                && let Ok(tex) =
                    backend.load_texture(decoded.width, decoded.height, &decoded.pixels)
            {
                self.image_textures.insert(resolved.clone(), tex);
            }
        }

        // Also collect background-image URLs from styles.
        for style_opt in &self.styles {
            if let Some(style) = style_opt
                && let css::values::BackgroundImage::Url(ref url) = style.background_image
            {
                let resolved = Self::resolve_src(&base_url, url);
                if !self.image_textures.contains_key(&resolved)
                    && self.decoded_images.contains_key(&resolved)
                    && !pending.contains(&resolved)
                {
                    pending.push(resolved);
                }
            }
        }

        // Create textures.
        for resolved in &pending {
            if let Some(decoded) = self.decoded_images.get(resolved)
                && let Ok(tex) =
                    backend.load_texture(decoded.width, decoded.height, &decoded.pixels)
            {
                self.image_textures.insert(resolved.clone(), tex);
            }
        }

        // Walk layout tree and assign textures.
        if let Some(layout) = &mut self.layout_root {
            Self::assign_textures_recursive(
                layout,
                &self.document,
                &base_url,
                &self.image_textures,
            );
        }
    }

    /// Recursively walk the layout tree and assign GPU textures to
    /// `ReplacedContent::Image` nodes and `background-image` styles.
    fn assign_textures_recursive(
        layout_box: &mut layout::box_model::LayoutBox,
        doc: &Option<html::dom::Document>,
        base_url: &Option<String>,
        textures: &HashMap<String, TextureId>,
    ) {
        if let layout::box_model::BoxType::Replaced(layout::box_model::ReplacedContent::Image {
            ref mut texture,
            ..
        }) = layout_box.box_type
            && texture.is_none()
            && let Some(node_id) = layout_box.node
            && let Some(doc) = doc
        {
            let node = doc.get(node_id);
            if let NodeKind::Element(elem) = &node.kind
                && let Some(src) = elem.src()
            {
                let resolved = Self::resolve_src(base_url, src);
                if let Some(&tex) = textures.get(&resolved) {
                    *texture = Some(tex);
                }
            }
        }

        // Assign background-image texture.
        if layout_box.background_texture.is_none()
            && let css::values::BackgroundImage::Url(ref url) = layout_box.style.background_image
        {
            let resolved = Self::resolve_src(base_url, url);
            if let Some(&tex) = textures.get(&resolved) {
                layout_box.background_texture = Some(tex);
            }
        }

        for child in &mut layout_box.children {
            Self::assign_textures_recursive(child, doc, base_url, textures);
        }
    }

    /// Rebuild the layout tree with image dimensions after images have
    /// been decoded (second layout pass).
    fn rebuild_layout_with_images(&mut self) {
        let image_info = self.build_image_info_map();
        if let Some(doc) = &self.document {
            let content_h = self.config.content_height(self.window_h);
            let base_url = self.nav.current_url().map(String::from);
            let measurer = layout::text_cache::CachingMeasurer::new(&SimpleTextMeasurer);
            let layout_root = layout::block::build_layout_tree(
                doc,
                &self.styles,
                &measurer,
                self.window_w as f32,
                content_h as f32,
                base_url.as_deref(),
                &image_info,
            );
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
