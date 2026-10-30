//! Stylesheets and web fonts: `<link>` / `<style>` / `@import`
//! collection, CSP checks, viewport-dependent restyling and author-sheet
//! merging.

use super::*;

impl BrowserWidget {
    /// Load pending web fonts and resolve `web_font_id` on styles.
    ///
    /// Called from `tick()` when VFS access is available. This triggers
    /// the actual network/VFS fetch of font data, parses it with fontdue,
    /// and then resolves `web_font_id` on each `ComputedStyle`. If any
    /// fonts are loaded, the layout and display list are rebuilt.
    #[cfg(feature = "web-fonts")]
    pub(crate) fn load_web_fonts(&mut self, vfs: &dyn oasis_vfs::Vfs) {
        // Only attempt font loading once per page. After the first call,
        // `pending` is drained inside `load_fonts`, so repeated calls
        // would be harmless but wasteful no-ops.
        if self.fonts_load_attempted || self.font_registry.borrow().has_fonts() {
            return;
        }
        self.fonts_load_attempted = true;

        let base_url = self.nav.current_url().map(|s| s.to_string());
        {
            let mut reg = self.font_registry.borrow_mut();
            reg.load_fonts(base_url.as_deref(), vfs, self.tls.as_deref());
        }

        // If fonts were loaded, resolve web_font_id on styles and
        // trigger relayout.
        let font_count = self.font_registry.borrow().font_count();
        if font_count > 0 {
            self.diag(&format!("[BR] web fonts loaded: {} faces", font_count));
            // Resolve web_font_id on cached styles.
            let reg = self.font_registry.borrow();
            for style in self.styles.iter_mut().flatten() {
                let italic = style.font_style == crate::css::values::FontStyle::Italic;
                let weight = style.font_weight.0;
                if let Some(font_id) = reg.resolve_font(&style.font_family.families, weight, italic)
                {
                    style.web_font_id = Some(font_id.as_raw());
                }
            }
            drop(reg);
            // Force display list rebuild on next paint.
            self.layout_dirty = true;
        }
    }

    /// Walk the DOM to collect text from `<style>` elements and parse
    /// each into a `Stylesheet`. Both `<head>` and `<body>` style blocks
    /// are included.
    ///
    /// `viewport` is threaded into `Stylesheet::parse_with_viewport` so
    /// `@media (min/max-width)` and `@media (prefers-color-scheme)` gate
    /// against the actual window size rather than the 480x272 default —
    /// without this, desktop-sized windows silently get the mobile
    /// breakpoints of any page whose author CSS uses `max-width`.
    ///
    /// Blocks the page's CSP does not allow (`style-src-elem` /
    /// `style-src`: nonce, `'sha256-…'` hash or `'unsafe-inline'`) are
    /// skipped.
    pub(super) fn collect_style_sheets(
        doc: &html::dom::Document,
        viewport: css::parser::MediaViewport,
        csp: Option<&CspPolicy>,
    ) -> (Vec<css::parser::Stylesheet>, Vec<html::dom::NodeId>) {
        let mut sheets = Vec::new();
        let mut positions = Vec::new();
        for (id, node) in doc.nodes.iter().enumerate() {
            if let html::dom::NodeKind::Element(elem) = &node.kind
                && elem.tag == html::dom::TagName::Style
            {
                let css_text = doc.text_content(id);
                let nonce = elem.get_attribute("nonce");
                if csp.is_some_and(|p| !p.allows_inline(InlineKind::StyleElement, nonce, &css_text))
                {
                    log::warn!("CSP style-src blocked an inline <style> block");
                    continue;
                }
                if !css_text.is_empty() {
                    sheets.push(css::parser::Stylesheet::parse_with_viewport(
                        &css_text, viewport,
                    ));
                    positions.push(id);
                }
            }
        }
        (sheets, positions)
    }

    /// Return `true` if a single comma-separated media-query token
    /// (e.g. `print`, `only print`, `print and (color)`) targets the
    /// print medium exclusively. Used by `<link media="…">` filtering
    /// to drop print-only sheets without swallowing mixed lists like
    /// `print, screen` or queries like `(min-width: 500px)`.
    pub(crate) fn is_print_only_media_query(token: &str) -> bool {
        let trimmed = token.trim();
        if trimmed.is_empty() {
            return false;
        }
        // Drop a leading `only` / `not` modifier before inspecting the
        // media type. `not print` is *not* print-only — it matches
        // everything except print — so treat it as non-print.
        let rest = if let Some(r) = trimmed.strip_prefix("only ") {
            r.trim_start()
        } else if let Some(r) = trimmed.strip_prefix("not ") {
            // Only bare `not all` matches zero media types. A query of the
            // form `not all and (feature)` matches media where `(feature)`
            // is false (e.g. `not all and (color)` matches monochrome
            // screens) so must not be filtered here. Any other `not <type>`
            // (e.g. `not screen`, `not print`) still matches *some* media.
            return r.trim() == "all";
        } else {
            trimmed
        };
        // The media type is the first whitespace-delimited word.
        let media_type = rest.split_whitespace().next().unwrap_or("");
        media_type == "print"
    }

    /// Walk the DOM and collect `<link rel="stylesheet" href="...">`
    /// URLs, resolved against `base_url`.
    ///
    /// Skips `rel="alternate stylesheet"` and links whose `media` attr
    /// is a `print`-only query (the layout pipeline renders with
    /// `@media screen` semantics). Data URIs are rejected here because
    /// the async loader expects an HTTP/VFS fetchable resource; inline
    /// `data:` stylesheets should use a `<style>` block anyway.
    ///
    /// The returned vector preserves DOM order so the cascade applies
    /// external sheets in the same sequence the author wrote them. The
    /// parallel `Vec<NodeId>` gives each retained URL its originating
    /// DOM node index so callers can interleave external sheets with
    /// inline `<style>` blocks by DOM order in the cascade.
    ///
    /// URLs the page's CSP (`style-src-elem` / `style-src`) refuses, and
    /// `vfs://` URLs on a page with a web origin
    /// ([`loader::subresource_allowed`]), are dropped.
    pub(super) fn collect_linked_stylesheet_urls(
        doc: &html::dom::Document,
        base_url: &str,
        csp: Option<&CspPolicy>,
    ) -> (Vec<String>, Vec<html::dom::NodeId>) {
        const MAX_LINKED_STYLESHEETS: usize = 16;
        let mut urls = Vec::new();
        let mut positions = Vec::new();
        let base_parsed = loader::Url::parse(base_url);
        for (node_id, node) in doc.nodes.iter().enumerate() {
            if urls.len() >= MAX_LINKED_STYLESHEETS {
                log::warn!(
                    "external stylesheet limit ({MAX_LINKED_STYLESHEETS}) \
                     reached; remaining <link rel=\"stylesheet\"> tags dropped"
                );
                break;
            }
            let html::dom::NodeKind::Element(elem) = &node.kind else {
                continue;
            };
            if elem.tag != html::dom::TagName::Link {
                continue;
            }
            let rel = elem.get_attribute("rel").unwrap_or("");
            let is_stylesheet = rel
                .split_ascii_whitespace()
                .any(|t| t.eq_ignore_ascii_case("stylesheet"));
            let is_alternate = rel
                .split_ascii_whitespace()
                .any(|t| t.eq_ignore_ascii_case("alternate"));
            if !is_stylesheet || is_alternate {
                continue;
            }
            // Skip print-only stylesheets. `media` is a comma-separated
            // list of media queries; if *every* entry targets `print`
            // exclusively, the sheet is irrelevant to screen rendering.
            // A mixed list like `print, screen` still matches screen and
            // must not be skipped.
            if let Some(media) = elem.get_attribute("media") {
                let m = media.trim().to_ascii_lowercase();
                if !m.is_empty() && m.split(',').all(Self::is_print_only_media_query) {
                    continue;
                }
            }
            let Some(href) = elem.get_attribute("href") else {
                continue;
            };
            let href = href.trim();
            if href.is_empty() || href.starts_with("data:") {
                continue;
            }
            let resolved = match &base_parsed {
                Some(base) => base
                    .resolve(href)
                    .map(|u| u.to_string())
                    .unwrap_or_else(|| href.to_string()),
                None => href.to_string(),
            };
            // Reject resolved URLs the loader can't actually fetch.
            // `file://` (used as the base for local fixture pages) can't
            // follow protocol-relative `//cdn/...` references since the
            // network loader rejects the `file` scheme. Silently drop
            // them rather than spamming the I/O thread with certain-fail
            // requests.
            let scheme = loader::Url::parse(&resolved)
                .map(|u| u.scheme.clone())
                .unwrap_or_default();
            match scheme.as_str() {
                "http" | "https" | "vfs" => {},
                _ => continue,
            }
            if urls.iter().any(|u| u == &resolved) {
                continue;
            }
            if !Self::stylesheet_url_allowed(base_url, &resolved, csp) {
                continue;
            }
            urls.push(resolved);
            positions.push(node_id);
        }
        (urls, positions)
    }

    /// Submit each linked stylesheet URL to the I/O thread for fetch.
    /// Each slot in `external_stylesheets` is keyed by its index in the
    /// submitted vec; the `poll_io_thread` handler fills the slot when
    /// the CSS bytes arrive.
    ///
    /// VFS-scheme stylesheets (`vfs://…/foo.css`) are loaded synchronously
    /// here rather than round-tripped through the I/O thread — that thread
    /// doesn't hold a VFS handle, so routing a VFS request through it
    /// would always return "unsupported network scheme: vfs" and the
    /// slot would never fill.
    #[cfg(native_net)]
    pub(super) fn submit_external_stylesheets(&mut self, urls: Vec<String>, base_url: &str) {
        let source = if self.config.features.sandbox_only {
            ResourceSource::Vfs
        } else {
            ResourceSource::VfsThenNetwork
        };
        let referrer = loader::strip_referrer(base_url);
        let base = Some(base_url.to_string());
        let mut pending_network: Vec<(usize, ResourceRequest)> = Vec::new();
        for (idx, url) in urls.into_iter().enumerate() {
            let request = ResourceRequest {
                url: url.clone(),
                base_url: base.clone(),
                source,
                method: crate::loader::HttpMethod::Get,
                body: None,
                referrer: referrer.clone(),
                initiator: base.clone(),
            };
            if url.starts_with("vfs://") {
                // VFS lookups require the caller's `Vfs` handle, which
                // we don't hold during `load_html`. Stash them for the
                // next `tick()` to resolve synchronously against the
                // passed-in `vfs`.
                self.pending_vfs_stylesheets.push((idx, request));
                continue;
            }
            pending_network.push((idx, request));
        }

        if !pending_network.is_empty() {
            self.ensure_io_thread();
            if self.io_thread.is_none() {
                log::warn!(
                    "I/O thread unavailable; dropping {} external stylesheet fetch(es). \
                     Page will render without linked CSS.",
                    pending_network.len()
                );
                return;
            }
            // Safe: the `is_none()` check above early-returned, so the
            // I/O thread is guaranteed present for the rest of this call.
            let io = self
                .io_thread
                .as_mut()
                .expect("io_thread checked non-None above");
            for (idx, request) in pending_network {
                let validators = self.cache.peek_validators(&request.url);
                let id = io.send(IoRequestKind::Stylesheet, request, validators, None);
                self.pending_io_stylesheets.insert(id, idx);
            }
        }
    }

    /// WASM/PSP stub: no async I/O path, skip external stylesheet loading.
    #[cfg(not(native_net))]
    pub(super) fn submit_external_stylesheets(&mut self, _urls: Vec<String>, _base_url: &str) {}

    /// Drain any VFS-scheme stylesheets queued during `load_html` and
    /// parse them against the caller-supplied VFS. Called from `tick()`
    /// so the initial paint can land without waiting.
    #[cfg(native_net)]
    pub(crate) fn load_pending_vfs_stylesheets(&mut self, vfs: &dyn Vfs) {
        if self.pending_vfs_stylesheets.is_empty() {
            return;
        }
        let viewport = css::parser::MediaViewport {
            width: self.window_w as f32,
            height: self.window_h as f32,
            dark_mode: false,
            prefers_reduced_motion: false,
            hover: true,
            pointer: "fine",
        };
        let drained: Vec<(usize, ResourceRequest)> =
            std::mem::take(&mut self.pending_vfs_stylesheets);
        let mut arrived: Vec<(usize, String, String)> = Vec::new();
        for (idx, request) in drained {
            match loader::vfs::load_from_vfs(vfs, &request) {
                Ok(resp) => {
                    let css_text = String::from_utf8_lossy(&resp.body).into_owned();
                    let sheet = css::parser::Stylesheet::parse_with_viewport(&css_text, viewport);
                    if idx < self.external_stylesheets.len() {
                        self.external_stylesheets[idx] = Some(sheet);
                        arrived.push((idx, request.url.clone(), css_text.clone()));
                        self.record_external_source(idx, css_text);
                        self.pending_external_css_apply = true;
                    }
                },
                Err(e) => {
                    log::debug!("vfs stylesheet fetch failed: {e}");
                },
            }
        }
        self.queue_stylesheet_imports(arrived);
    }

    /// Follow the `@import` rules of freshly arrived linked sheets
    /// (`(slot, sheet url, css)`): each import gets its own slot right
    /// before its importer (imports cascade before the importing sheet's
    /// own rules) and is fetched like a `<link>`ed sheet — including its
    /// own imports once it arrives. At most [`MAX_STYLESHEET_IMPORTS`]
    /// distinct URLs per page, which also ends import cycles.
    #[cfg(native_net)]
    pub(super) fn queue_stylesheet_imports(&mut self, mut arrived: Vec<(usize, String, String)>) {
        // Highest slot first: inserting slots shifts only the slots at
        // or after the insertion point, never a lower one still to do.
        arrived.sort_by_key(|(idx, _, _)| std::cmp::Reverse(*idx));
        let source = if self.config.features.sandbox_only {
            ResourceSource::Vfs
        } else {
            ResourceSource::VfsThenNetwork
        };
        let page_url = self.nav.current_url().unwrap_or("").to_string();
        for (idx, sheet_url, css_text) in arrived {
            let urls: Vec<String> = css_import_urls(&css_text, &sheet_url)
                .into_iter()
                .filter(|u| Self::stylesheet_url_allowed(&page_url, u, self.page_csp.as_ref()))
                .filter(|u| {
                    self.imported_stylesheet_urls.len() < MAX_STYLESHEET_IMPORTS
                        && self.imported_stylesheet_urls.insert(u.clone())
                })
                .collect();
            if urls.is_empty() || idx >= self.external_stylesheets.len() {
                continue;
            }
            let n = urls.len();
            let pos = self.external_stylesheet_positions[idx];
            for k in 0..n {
                self.external_stylesheets.insert(idx + k, None);
                self.external_stylesheet_positions.insert(idx + k, pos);
                self.external_stylesheet_sources.insert(idx + k, None);
            }
            for slot in self.pending_io_stylesheets.values_mut() {
                if *slot >= idx {
                    *slot += n;
                }
            }
            for (slot, _) in &mut self.pending_vfs_stylesheets {
                if *slot >= idx {
                    *slot += n;
                }
            }
            let referrer = loader::strip_referrer(&sheet_url);
            // The page, not the importing sheet, decides same-site.
            let initiator = self.nav.current_url().map(String::from);
            for (k, url) in urls.into_iter().enumerate() {
                let request = ResourceRequest {
                    url: url.clone(),
                    base_url: Some(sheet_url.clone()),
                    source,
                    method: crate::loader::HttpMethod::Get,
                    body: None,
                    referrer: referrer.clone(),
                    initiator: initiator.clone(),
                };
                if url.starts_with("vfs://") {
                    self.pending_vfs_stylesheets.push((idx + k, request));
                } else if self.ensure_io_thread()
                    && let Some(io) = self.io_thread.as_mut()
                {
                    let validators = self.cache.peek_validators(&request.url);
                    let id = io.send(IoRequestKind::Stylesheet, request, validators, None);
                    self.pending_io_stylesheets.insert(id, idx + k);
                }
            }
        }
    }

    /// Remember the source text of external sheet `idx` (for re-parsing
    /// on viewport changes) and note whether it is media-dependent.
    #[cfg_attr(not(native_net), allow(dead_code))]
    pub(super) fn record_external_source(&mut self, idx: usize, css_text: String) {
        if css_uses_media_queries(&css_text) {
            self.media_dependent_css = true;
        }
        if let Some(slot) = self.external_stylesheet_sources.get_mut(idx) {
            *slot = Some(css_text);
        }
    }

    /// Re-evaluate `@media` rules after the window size changed.
    ///
    /// Media queries are resolved when a sheet is parsed, so the author
    /// sheets are re-parsed for the new viewport (inline `<style>` text
    /// from the DOM, linked sheets from their saved source) and the page
    /// is re-cascaded. No-op when the size is unchanged or no sheet uses
    /// `@media`.
    pub(crate) fn restyle_for_viewport_if_needed(&mut self) {
        let size = (self.window_w, self.window_h);
        if !self.media_dependent_css || size == self.styled_viewport {
            return;
        }
        let Some(doc) = self.document.as_ref() else {
            return;
        };
        self.styled_viewport = size;
        let viewport = css::parser::MediaViewport {
            width: self.window_w as f32,
            height: self.window_h as f32,
            dark_mode: false,
            prefers_reduced_motion: false,
            hover: true,
            pointer: "fine",
        };
        let (sheets, positions) = Self::collect_style_sheets(doc, viewport, self.page_csp.as_ref());
        self.cached_author_sheets = sheets;
        self.cached_author_sheet_positions = positions;
        for (slot, source) in self
            .external_stylesheets
            .iter_mut()
            .zip(&self.external_stylesheet_sources)
        {
            if let Some(source) = source {
                *slot = Some(css::parser::Stylesheet::parse_with_viewport(
                    source, viewport,
                ));
            }
        }
        self.pending_external_css_apply = true;
        self.apply_external_stylesheets_if_pending();
        #[cfg(feature = "javascript")]
        {
            *self.js_styles.borrow_mut() = self.styles.clone();
        }
    }

    /// Interleave cached inline `<style>` sheets and fetched external
    /// `<link>` sheets back into DOM order. Cascade precedence depends
    /// on source position within the author origin, so a page with a
    /// `<link>` before a `<style>` in `<head>` must apply the link's
    /// rules first — grouping all inline sheets before all external
    /// sheets (or vice-versa) would flip the winner for any rule of
    /// equal specificity.
    pub(super) fn author_sheets_in_dom_order(&self) -> Vec<&css::parser::Stylesheet> {
        Self::merge_author_sheets(
            &self.cached_author_sheets,
            &self.cached_author_sheet_positions,
            &self.external_stylesheets,
            &self.external_stylesheet_positions,
        )
    }

    /// Field-level form of [`Self::author_sheets_in_dom_order`], for
    /// callers that need to mutate other fields (e.g. `styles`) while
    /// the returned sheet references are alive. Every re-cascade must
    /// use the full inline + linked list: the cached selector index is
    /// built over it, so a shorter list both drops `<link>` rules and
    /// indexes out of bounds.
    pub(crate) fn merge_author_sheets<'a>(
        inline: &'a [css::parser::Stylesheet],
        inline_positions: &[NodeId],
        external: &'a [Option<css::parser::Stylesheet>],
        external_positions: &[NodeId],
    ) -> Vec<&'a css::parser::Stylesheet> {
        let inline_n = inline.len();
        let external_n = external.len();
        let mut out: Vec<&css::parser::Stylesheet> = Vec::with_capacity(inline_n + external_n);
        let mut i = 0; // inline cursor
        let mut e = 0; // external cursor
        while i < inline_n || e < external_n {
            let inline_pos = inline_positions.get(i).copied().unwrap_or(usize::MAX);
            let external_pos = external_positions.get(e).copied().unwrap_or(usize::MAX);
            if inline_pos <= external_pos {
                if i < inline_n {
                    out.push(&inline[i]);
                }
                i += 1;
            } else {
                if let Some(Some(sheet)) = external.get(e) {
                    out.push(sheet);
                }
                e += 1;
            }
        }
        out
    }

    /// Apply any external stylesheets that have arrived since the last
    /// cascade. Rebuilds `self.styles` from UA + cached inline sheets +
    /// arrived external sheets and marks the layout dirty so the next
    /// paint picks up the new visual state.
    ///
    /// Re-runs cascade only — keyframes, @font-face, animations, and
    /// the scripts pipeline are left alone; style changes alone are
    /// sufficient for the visual delta that matters for real-world
    /// sites (old.reddit, MediaWiki) whose external CSS is declarative.
    ///
    /// `@import`s inside linked sheets get their own slots (see
    /// `queue_stylesheet_imports`) and land here like any linked sheet.
    /// `@import` inside inline `<style>` blocks is not followed.
    pub(crate) fn apply_external_stylesheets_if_pending(&mut self) {
        if !self.pending_external_css_apply {
            return;
        }
        self.pending_external_css_apply = false;
        let Some(doc) = self.document.as_ref() else {
            return;
        };
        let ua_sheet = css::default::default_stylesheet();
        let mut all_sheets: Vec<&css::parser::Stylesheet> = vec![ua_sheet];
        for sheet in self.author_sheets_in_dom_order() {
            all_sheets.push(sheet);
        }
        // Preserve the container-query lookup captured during `load_html`'s
        // post-layout restyle pass; without it, `@container` rules would
        // silently stop matching on every re-cascade driven by a late-
        // arriving external sheet. `global_layers` is intentionally left
        // `None` — layer ordering is recomputed fresh from the merged
        // sheet list below, matching the initial cascade path.
        let ctx = css::cascade::CascadeContext {
            hover_node: self.hover_node,
            visited_urls: Some(&self.visited_urls),
            focused_node: self.focused_node,
            containers: self.container_lookup.as_ref(),
            global_layers: None,
        };
        let styles = css::cascade::style_tree(doc, &all_sheets, &self.cached_inline_styles, &ctx);
        let selector_index = css::cascade::SelectorIndex::build(&all_sheets);

        #[cfg(feature = "web-fonts")]
        {
            let mut font_reg = self.font_registry.borrow_mut();
            for sheet in &all_sheets {
                font_reg.collect_font_faces(&sheet.font_faces);
            }
            self.fonts_load_attempted = false;
        }

        self.styles = styles;
        self.cached_selector_index = Some(selector_index);
        self.layout_dirty = true;
        self.full_repaint_needed = true;
        self.display_list.clear();
        self.display_list_stale = true;
    }

    /// Whether a page at `page_url` may load the stylesheet `url`: CSP
    /// `style-src-elem` / `style-src` and the `vfs://` subresource gate.
    pub(super) fn stylesheet_url_allowed(
        page_url: &str,
        url: &str,
        csp: Option<&CspPolicy>,
    ) -> bool {
        if !loader::subresource_allowed(Some(page_url), url) {
            log::warn!("blocked vfs:// stylesheet {url} on web page {page_url}");
            return false;
        }
        if let Some(policy) = csp
            && !policy.allows(url, page_url, CspResourceType::Style)
        {
            log::warn!("CSP style-src blocked stylesheet {url}");
            return false;
        }
        true
    }

    /// [`Self::collect_inline_styles`] for the parser-inserted
    /// attributes of a freshly loaded page: empty when the page's CSP
    /// (`style-src-attr` / `style-src`) lacks `'unsafe-inline'`.
    ///
    /// Re-collection after script DOM mutations
    /// (`apply_js_dom_mutations`) is not filtered: script `element.style`
    /// writes land in the `style` attribute, and CSSOM writes are exempt
    /// from `style-src` in browsers too.
    pub(super) fn collect_parser_inline_styles(
        doc: &html::dom::Document,
        csp: Option<&CspPolicy>,
    ) -> Vec<(NodeId, Vec<css::parser::Declaration>)> {
        if csp.is_some_and(|p| !p.allows_inline(InlineKind::StyleAttribute, None, "")) {
            log::warn!("CSP style-src blocked inline style=\"\" attributes");
            return Vec::new();
        }
        Self::collect_inline_styles(doc)
    }

    /// Walk the DOM to collect inline `style=""` attributes and parse
    /// each into a list of declarations keyed by NodeId.
    pub(crate) fn collect_inline_styles(
        doc: &html::dom::Document,
    ) -> Vec<(NodeId, Vec<css::parser::Declaration>)> {
        let mut result = Vec::new();
        for (id, node) in doc.nodes.iter().enumerate() {
            if let html::dom::NodeKind::Element(elem) = &node.kind
                && let Some(style_attr) = elem.get_attribute("style")
            {
                let decls = css::parser::parse_inline_style(style_attr);
                if !decls.is_empty() {
                    result.push((id, decls));
                }
            }
        }
        result
    }
}

/// Most `@import`ed stylesheets followed per page.
#[cfg(native_net)]
const MAX_STYLESHEET_IMPORTS: usize = 16;

/// URLs of the `@import` rules at the head of `css` (imports must come
/// before any other rule but `@charset` / `@layer` statements), resolved
/// against `sheet_url`. Print-only imports and schemes the loader can't
/// fetch are skipped; other media / `layer()` / `supports()` conditions
/// are not evaluated (the sheet is imported unconditionally).
#[cfg_attr(not(native_net), allow(dead_code))]
pub(crate) fn css_import_urls(css: &str, sheet_url: &str) -> Vec<String> {
    let base = loader::Url::parse(sheet_url);
    let mut urls = Vec::new();
    let mut rest = css;
    loop {
        // Skip whitespace and comments.
        rest = rest.trim_start();
        if let Some(after) = rest.strip_prefix("/*") {
            rest = after.split_once("*/").map_or("", |(_, r)| r);
            continue;
        }
        let lower: String = rest
            .chars()
            .take(8)
            .collect::<String>()
            .to_ascii_lowercase();
        let is_import = lower.starts_with("@import");
        if !(is_import || lower.starts_with("@charset") || lower.starts_with("@layer")) {
            break;
        }
        // The statement runs to the first `;` outside quotes / parens; a
        // `{` first means a block (`@layer x { ... }`): imports are over.
        let (mut quote, mut depth, mut end) = (None::<char>, 0i32, None);
        for (i, c) in rest.char_indices() {
            match (quote, c) {
                (Some(q), c) if c == q => quote = None,
                (Some(_), _) => {},
                (None, '"' | '\'') => quote = Some(c),
                (None, '(') => depth += 1,
                (None, ')') => depth -= 1,
                (None, '{') if depth == 0 => break,
                (None, ';') if depth == 0 => {
                    end = Some(i);
                    break;
                },
                _ => {},
            }
        }
        let Some(end) = end else { break };
        let statement = &rest[..end];
        rest = &rest[end + 1..];
        if !is_import {
            continue;
        }
        let prelude = statement["@import".len()..].trim_start();
        let (target, media) = if let Some(inner) = prelude
            .get(..4)
            .filter(|p| p.eq_ignore_ascii_case("url("))
            .and_then(|_| prelude[4..].split_once(')'))
        {
            (inner.0.trim().trim_matches(['"', '\'']), inner.1)
        } else if let Some(q) = prelude.chars().next().filter(|c| *c == '"' || *c == '\'') {
            match prelude[1..].split_once(q) {
                Some((target, media)) => (target, media),
                None => continue,
            }
        } else {
            continue;
        };
        let media = media.trim().to_ascii_lowercase();
        if !media.is_empty()
            && media
                .split(',')
                .all(BrowserWidget::is_print_only_media_query)
        {
            continue;
        }
        let resolved = match &base {
            Some(b) => b.resolve(target).map(|u| u.to_string()),
            None => Some(target.to_string()),
        };
        let Some(resolved) = resolved else { continue };
        let scheme = loader::Url::parse(&resolved)
            .map(|u| u.scheme.clone())
            .unwrap_or_default();
        if matches!(scheme.as_str(), "http" | "https" | "vfs") && !urls.contains(&resolved) {
            urls.push(resolved);
        }
    }
    urls
}
