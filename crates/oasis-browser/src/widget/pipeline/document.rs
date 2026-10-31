//! Building a page from HTML source: parse, cascade, layout, form
//! state and the link map (`load_html`).

use super::*;

impl BrowserWidget {
    /// Parse HTML, run the CSS cascade, build layout, and prepare
    /// for painting.
    /// Maximum HTML source size accepted for parsing (10 MB).
    pub(super) const MAX_HTML_SOURCE_BYTES: usize = 10 * 1024 * 1024;

    pub fn load_html(&mut self, html_source: &str, url: &str) {
        self.diag(&format!(
            "[BR] load_html start: {} bytes",
            html_source.len()
        ));
        // Guard against oversized HTML input.
        let source = if html_source.len() > Self::MAX_HTML_SOURCE_BYTES {
            log::warn!(
                "HTML source too large ({} bytes), truncating to {} bytes",
                html_source.len(),
                Self::MAX_HTML_SOURCE_BYTES,
            );
            &html_source[..html_source.floor_char_boundary(Self::MAX_HTML_SOURCE_BYTES)]
        } else {
            html_source
        };

        // 1. Tokenize and build DOM, reusing the previous document's
        //    arena allocation when available to avoid reallocations.
        //    The tree builder drives the tokenizer (raw text only for HTML
        //    elements, CDATA in SVG); `<noscript>` is markup without JS.
        self.diag("[BR] html parse start");
        let options = html::tree_builder::ParseOptions {
            scripting: cfg!(feature = "javascript"),
        };
        let doc = if let Some(old_doc) = self.document.take() {
            html::tree_builder::TreeBuilder::parse_reuse(source, options, old_doc)
        } else {
            html::tree_builder::TreeBuilder::parse_with(source, options)
        };
        self.diag(&format!("[BR] tree build done: {} nodes", doc.nodes.len()));
        // The cascade below reads `hover_node`, so clear the previous
        // page's node references before it runs.
        self.clear_node_references();

        // 1b. Execute inline <script> blocks (if JS enabled).
        #[cfg(feature = "javascript")]
        let doc = self.run_page_scripts(doc, url);

        // 2. Extract page title.
        let title = doc.title().unwrap_or_else(|| url.to_string());

        // 3. Collect <style> blocks and inline style="" attributes from DOM.
        //    Cache them so hover restyles don't re-parse.
        self.diag("[BR] collect stylesheets start");
        let media_viewport = css::parser::MediaViewport {
            width: self.window_w as f32,
            height: self.window_h as f32,
            dark_mode: false,
            prefers_reduced_motion: false,
            hover: true,
            pointer: "fine",
        };
        let (author_sheets, author_sheet_positions) =
            Self::collect_style_sheets(&doc, media_viewport, self.page_csp.as_ref());
        let inline_styles = Self::collect_parser_inline_styles(&doc, self.page_csp.as_ref());
        self.diag(&format!(
            "[BR] collect stylesheets done: {} sheets, {} inline",
            author_sheets.len(),
            inline_styles.len()
        ));

        // 3b. Collect `<link rel="stylesheet" href>` URLs and dispatch
        //     async fetches. The initial render applies only inline
        //     `<style>` sheets so the page appears immediately; external
        //     sheets are re-cascaded and relaid out as they arrive. This
        //     is the primary reason old.reddit.com renders as unstyled
        //     bullets without this pass — the site ships essentially no
        //     inline CSS.
        let (linked_urls, linked_positions) =
            Self::collect_linked_stylesheet_urls(&doc, url, self.page_csp.as_ref());
        self.external_stylesheets = vec![None; linked_urls.len()];
        self.external_stylesheet_sources = vec![None; linked_urls.len()];
        self.styled_viewport = (self.window_w, self.window_h);
        self.media_dependent_css = author_sheet_positions
            .iter()
            .any(|&nid| css_uses_media_queries(&doc.text_content(nid)));
        self.external_stylesheet_positions = linked_positions;
        #[cfg(native_net)]
        {
            self.pending_io_stylesheets.clear();
            self.pending_vfs_stylesheets.clear();
            self.imported_stylesheet_urls.clear();
        }
        self.pending_external_css_apply = false;
        if !linked_urls.is_empty() {
            self.submit_external_stylesheets(linked_urls, url);
        }

        // 4. CSS cascade: user-agent + author stylesheets + inline styles.
        self.diag("[BR] cascade start");
        let ua_sheet = css::default::default_stylesheet();
        let mut all_sheets: Vec<&css::parser::Stylesheet> = vec![ua_sheet];
        for sheet in &author_sheets {
            all_sheets.push(sheet);
        }
        let mut styles = {
            let ctx = css::cascade::CascadeContext {
                hover_node: self.hover_node,
                visited_urls: Some(&self.visited_urls),
                focused_node: None,
                containers: None,
                global_layers: None,
            };
            css::cascade::style_tree(&doc, &all_sheets, &inline_styles, &ctx)
        };
        self.diag("[BR] selector index start");
        let selector_index = css::cascade::SelectorIndex::build(&all_sheets);
        self.diag(&format!("[BR] cascade done: {} styles", styles.len()));

        // 4b. Web fonts: collect @font-face rules from stylesheets.
        //     Actual font data loading happens in `load_web_fonts()`
        //     which is called with VFS access before the first paint.
        #[cfg(feature = "web-fonts")]
        {
            let mut font_reg = self.font_registry.borrow_mut();
            // Clear previous page's fonts, and the glyph textures keyed
            // by their (now reused) font ids.
            *font_reg = crate::font::FontRegistry::new();
            self.glyph_tex_cache.clear_into(&mut self.retired_textures);
            self.fonts_load_attempted = false;

            // Collect @font-face rules from all stylesheets.
            for sheet in &all_sheets {
                font_reg.collect_font_faces(&sheet.font_faces);
            }
        }

        // 5. Build link href map from DOM.
        let href_map = Self::build_link_map(&doc);

        // 6. Build layout tree.
        self.diag("[BR] layout start");
        let content_h = self.config.content_height(self.window_h);
        self.refresh_image_info();
        layout_measurer!(self, measurer);
        let viewport_w = self.window_w as f32 / self.config.zoom_level;
        let viewport_h = content_h as f32 / self.config.zoom_level;
        let mut layout_root = layout::block::build_layout_tree(
            &doc,
            &styles,
            measurer,
            viewport_w,
            viewport_h,
            Some(url),
            &self.cached_image_info,
        );
        self.diag("[BR] layout done");

        // 6b. `@container` queries: if any author/UA rule is gated behind
        //     a container condition, do a second cascade+layout pass with
        //     the post-layout container sizes plumbed into cascade. Pages
        //     without container queries skip the work entirely.
        let container_lookup = if css::cascade::stylesheets_use_container_queries(&all_sheets) {
            let first_lookup = crate::layout::container_query::build_container_lookup(&layout_root);
            if !first_lookup.is_empty() {
                self.diag("[BR] container-query restyle");
                let ctx = css::cascade::CascadeContext {
                    hover_node: self.hover_node,
                    visited_urls: Some(&self.visited_urls),
                    focused_node: None,
                    containers: Some(&first_lookup),
                    global_layers: None,
                };
                styles = css::cascade::style_tree(&doc, &all_sheets, &inline_styles, &ctx);
                layout_root = layout::block::build_layout_tree(
                    &doc,
                    &styles,
                    measurer,
                    viewport_w,
                    viewport_h,
                    Some(url),
                    &self.cached_image_info,
                );
                // Rebuild from the post-second-layout tree so the
                // cached lookup reflects the *final* container sizes.
                // Hover/focus restyles reuse this and would otherwise
                // see stale pre-restyle dimensions for nested
                // container-query cases.
                Some(crate::layout::container_query::build_container_lookup(
                    &layout_root,
                ))
            } else {
                Some(first_lookup)
            }
        } else {
            None
        };

        // Update shared computed styles for JS getComputedStyle().
        #[cfg(feature = "javascript")]
        {
            *self.js_styles.borrow_mut() = styles.clone();
        }

        // 4b. Register CSS animations with the animation engine using
        //     the final styles (after any container-query restyle).
        self.start_page_animations(ua_sheet, &author_sheets, &styles);
        drop(all_sheets);

        // Cache parsed sheets and selector index for hover restyles.
        self.cached_author_sheets = author_sheets;
        self.cached_author_sheet_positions = author_sheet_positions;
        self.cached_inline_styles = inline_styles;
        self.cached_selector_index = Some(selector_index);
        self.container_lookup = container_lookup;

        // 7a. Collect canvas states from layout tree.
        #[cfg(feature = "javascript")]
        {
            self.canvas_states.borrow_mut().clear();
            crate::canvas::collect_canvas_states(&layout_root, &self.canvas_states);
        }

        // 7. Store results.
        self.body_node_id = doc.body();
        self.document = Some(doc);
        self.styles = styles;
        self.href_map = href_map;
        self.layout_root = Some(layout_root);
        self.link_map.clear();

        // 7b. Rebuild form state from the fresh DOM. Without this
        // `form_manager.forms` stays empty and every click on a
        // `<form>`'s inputs silently fails to find an owning form,
        // which means keystrokes never land in the search box.
        self.form_manager.clear();
        if let Some(doc) = &self.document {
            Self::populate_forms_from_dom(doc, &mut self.form_manager);
        }
        self.scroll.reset();
        // Seed the scrollable extent from the fresh layout right away:
        // history restores (`go_back`) and fragment scrolls position the
        // viewport before the first paint records the display list, and
        // `ScrollState` clamps against this height.
        if let Some(layout) = &self.layout_root {
            self.scroll
                .set_content_height(layout.scrollable_height() as i32);
        }
        self.nested_scroll_offsets.clear();
        self.state = LoadingState::Idle;
        self.layout_dirty = false;
        self.last_layout_w = self.window_w;
        // Invalidate the cached display list so it gets rebuilt on next paint.
        self.display_list.clear();
        self.display_list_stale = true;

        // 8. Update navigation (skip if restoring from history).
        if !self.skip_nav_push {
            self.nav.navigate(url, &title);
        }
        self.skip_nav_push = false;

        // 9. `page#fragment` loads land on the fragment target.
        if let Some((_, fragment)) = url.split_once('#') {
            let fragment = fragment.to_string();
            self.scroll_to_fragment(&fragment);
        }
        self.diag("[BR] load_html done");
    }

    /// Run the page's inline scripts against `doc` and keep the engine
    /// (and its shared document) for event dispatch; returns the
    /// document as the scripts left it.
    ///
    /// The document is wrapped in `Rc<RefCell<>>` so JS closures can
    /// mutate it. After the engine is dropped (freeing all JS-side `Rc`
    /// clones), the `Rc` unwraps to an owned `Document` again.
    #[cfg(feature = "javascript")]
    fn run_page_scripts(&mut self, doc: html::dom::Document, url: &str) -> html::dom::Document {
        self.console_output.clear();
        // Drop any previously retained engine before loading a new page.
        self.js_engine = None;
        self.js_doc = None;
        let (scripts, deferred) = Self::collect_scripts(&doc, self.page_csp.as_ref());
        self.deferred_scripts = deferred;
        let shared: js_dom::SharedDoc = std::rc::Rc::new(std::cell::RefCell::new(doc));
        match oasis_js::JsEngine::new(self.config.js_memory_limit_bytes) {
            Ok(mut engine) => {
                engine.set_max_exec_ms(self.config.js_max_exec_ms);
                let s = std::rc::Rc::clone(&shared);
                let nav = std::rc::Rc::clone(&self.js_nav_actions);
                let js_sty = std::rc::Rc::clone(&self.js_styles);
                let ls = std::rc::Rc::clone(&self.js_local_storage);
                let dirty = std::rc::Rc::clone(&self.js_dom_dirty);
                // Reset between page loads so a previous page's
                // mutation doesn't force an extra relayout here.
                dirty.set(false);
                if let Err(e) = engine.with_context(|ctx| {
                    js_dom::install_document_global_with_csp(
                        &ctx,
                        &s,
                        url,
                        &nav,
                        &js_sty,
                        self.page_csp.as_ref(),
                        self.tls.clone(),
                        Some(&ls),
                        Some(&dirty),
                    )
                }) {
                    log::warn!("JS DOM install failed: {}", e.message);
                    self.record_error(
                        crate::BrowserErrorKind::Script,
                        format!("JS DOM install: {}", e.message),
                    );
                }
                // Install canvas 2D context bindings.
                #[cfg(feature = "canvas")]
                {
                    let cm = std::rc::Rc::clone(&self.canvas_states);
                    if let Err(e) =
                        engine.with_context(|ctx| js_dom::install_canvas_bindings(&ctx, &cm))
                    {
                        log::warn!("Canvas bindings install failed: {}", e.message);
                    }
                }
                if !scripts.is_empty() {
                    let script_refs: Vec<&str> = scripts.iter().map(String::as_str).collect();
                    engine.eval_all(&script_refs);
                }
                // Install site-compat shims (togglecomment, etc.)
                // before wiring inline handlers, so onclick bodies
                // that reference them find defined globals. User
                // scripts already ran and can override if they
                // defined their own versions.
                js_dom::install_site_compat_shims(&engine);
                // Register inline event handlers (onclick, etc.)
                // after scripts so Element class is available. CSP
                // `script-src-attr` judges each handler's own text.
                {
                    let csp = self.page_csp.as_ref();
                    let allowed = |body: &str| {
                        csp.is_none_or(|p| p.allows_inline(InlineKind::ScriptAttribute, None, body))
                    };
                    let doc_borrow = shared.borrow();
                    js_dom::register_inline_handlers(&engine, &doc_borrow, &allowed);
                }
                // readystatechange -> DOMContentLoaded -> window load.
                js_dom::fire_document_lifecycle(&engine);
                self.console_output = engine.console_output();
                // Retain engine + shared doc for event dispatch.
                self.js_engine = Some(engine);
                self.js_doc = Some(std::rc::Rc::clone(&shared));
            },
            Err(e) => {
                log::warn!("JS engine init failed: {}", e.message);
                self.record_error(
                    crate::BrowserErrorKind::Script,
                    format!("JS engine init: {}", e.message),
                );
            },
        }
        // Mutations made while the page loaded are already in the
        // document we are about to lay out; don't replay them on the
        // first tick.
        self.js_dom_dirty.set(false);
        // Try to take ownership without cloning. This succeeds when
        // no JS engine was retained (init failure or no scripts).
        // When the engine holds a clone via js_doc, fall back to clone.
        match std::rc::Rc::try_unwrap(shared) {
            Ok(cell) => cell.into_inner(),
            Err(shared) => shared.borrow().clone(),
        }
    }

    /// Restart the animation engine with every `animation` the final
    /// styles declare, resolved against the UA and author `@keyframes`.
    fn start_page_animations(
        &mut self,
        ua_sheet: &css::parser::Stylesheet,
        author_sheets: &[css::parser::Stylesheet],
        styles: &[Option<css::values::ComputedStyle>],
    ) {
        self.animation_engine = css::animation::AnimationEngine::new();
        // Cloned once per page, not once per animated node.
        let mut keyframes: Vec<css::parser::KeyframesRule> = Vec::new();
        for sheet in std::iter::once(ua_sheet).chain(author_sheets) {
            keyframes.extend(sheet.keyframes.iter().cloned());
        }
        for (node_id, maybe_style) in styles.iter().enumerate() {
            if let Some(computed) = maybe_style
                && !computed.animations.is_empty()
            {
                self.animation_engine
                    .start_animations(node_id, &computed.animations, &keyframes);
            }
        }
    }

    /// Walk the DOM and register every `<form>` (plus its descendant
    /// inputs / selects / textareas / buttons) with `form_manager`.
    ///
    /// This is what makes a newly-loaded page actually interactive:
    /// the click handler and keyboard router both look up whether an
    /// element belongs to a registered form, and without this pass
    /// they always miss.
    pub(super) fn populate_forms_from_dom(
        doc: &html::dom::Document,
        form_manager: &mut crate::forms::FormManager,
    ) {
        use crate::forms::{FormElement, FormMethod, InputType, SelectOption};
        use html::dom::{NodeKind, TagName};

        // Find every <form>. Nested <form> elements are disallowed in
        // HTML5, so a flat walk is sufficient.
        for (form_nid, form_node) in doc.nodes.iter().enumerate() {
            let NodeKind::Element(form_elem) = &form_node.kind else {
                continue;
            };
            if form_elem.tag != TagName::Form {
                continue;
            }

            let action = form_elem.get_attribute("action").unwrap_or("").to_string();
            let method = match form_elem
                .get_attribute("method")
                .unwrap_or("get")
                .to_ascii_lowercase()
                .as_str()
            {
                "post" => FormMethod::Post,
                _ => FormMethod::Get,
            };
            let form_id = form_manager.add_form(&action, method);

            // DFS the form's subtree collecting form-element descendants.
            // Reverse the initial children so `pop()` yields them in
            // document order (matches the inner `iter().rev()` push).
            // Without this, direct <form> children are visited in reverse,
            // making submitted form data appear in reverse document order.
            let mut stack: Vec<usize> = form_node.children.iter().rev().copied().collect();
            while let Some(nid) = stack.pop() {
                let node = &doc.nodes[nid];
                for &c in node.children.iter().rev() {
                    stack.push(c);
                }
                let NodeKind::Element(elem) = &node.kind else {
                    continue;
                };
                match elem.tag {
                    TagName::Input => {
                        let input_type = elem
                            .get_attribute("type")
                            .unwrap_or("text")
                            .to_ascii_lowercase();
                        let name = elem.get_attribute("name").unwrap_or("").to_string();
                        let value = elem.get_attribute("value").unwrap_or("").to_string();
                        let placeholder =
                            elem.get_attribute("placeholder").unwrap_or("").to_string();
                        let maxlength = elem
                            .get_attribute("maxlength")
                            .and_then(|v| v.parse::<usize>().ok());
                        let required = elem.get_attribute("required").is_some();

                        match input_type.as_str() {
                            "hidden" => {
                                form_manager
                                    .add_element(form_id, FormElement::HiddenInput { name, value });
                            },
                            "submit" => {
                                let label = if value.is_empty() {
                                    "Submit".to_string()
                                } else {
                                    value.clone()
                                };
                                form_manager.add_element(
                                    form_id,
                                    FormElement::SubmitButton { name, value, label },
                                );
                            },
                            "reset" => {
                                let label = if value.is_empty() {
                                    "Reset".to_string()
                                } else {
                                    value.clone()
                                };
                                form_manager
                                    .add_element(form_id, FormElement::ResetButton { label });
                            },
                            "button" => {
                                // Plain push button — no submission. Treat
                                // like SubmitButton for focus purposes but
                                // don't submit on click.
                            },
                            "checkbox" => {
                                let checked = elem.get_attribute("checked").is_some();
                                let label = String::new();
                                // A checkbox without `value` submits "on".
                                let value = elem.get_attribute("value").unwrap_or("on").to_string();
                                form_manager.add_element(
                                    form_id,
                                    FormElement::Checkbox {
                                        name,
                                        value,
                                        checked,
                                        label,
                                    },
                                );
                            },
                            "radio" => {
                                let checked = elem.get_attribute("checked").is_some();
                                let value = elem.get_attribute("value").unwrap_or("on").to_string();
                                form_manager.add_element(
                                    form_id,
                                    FormElement::RadioButton {
                                        name: name.clone(),
                                        value,
                                        checked,
                                        group: name,
                                    },
                                );
                            },
                            _ => {
                                // text / search / email / password / number
                                // and unknown types collapse to TextInput so
                                // the user can at least type into them.
                                let it = match input_type.as_str() {
                                    "password" => InputType::Password,
                                    "email" => InputType::Email,
                                    "number" => InputType::Number,
                                    _ => InputType::Text,
                                };
                                form_manager.add_element(
                                    form_id,
                                    FormElement::TextInput {
                                        name,
                                        value,
                                        placeholder,
                                        maxlength,
                                        input_type: it,
                                        required,
                                        minlength: elem
                                            .get_attribute("minlength")
                                            .and_then(|v| v.parse::<usize>().ok()),
                                        pattern: elem
                                            .get_attribute("pattern")
                                            .map(|s| s.to_string()),
                                        min: elem
                                            .get_attribute("min")
                                            .and_then(|v| v.parse::<f64>().ok()),
                                        max: elem
                                            .get_attribute("max")
                                            .and_then(|v| v.parse::<f64>().ok()),
                                    },
                                );
                            },
                        }
                    },
                    TagName::Textarea => {
                        let name = elem.get_attribute("name").unwrap_or("").to_string();
                        // `<textarea>`'s initial value is its text children.
                        let value = doc.text_content(nid);
                        let placeholder =
                            elem.get_attribute("placeholder").unwrap_or("").to_string();
                        let rows = elem
                            .get_attribute("rows")
                            .and_then(|v| v.parse::<u32>().ok())
                            .unwrap_or(3);
                        let cols = elem
                            .get_attribute("cols")
                            .and_then(|v| v.parse::<u32>().ok())
                            .unwrap_or(40);
                        let required = elem.get_attribute("required").is_some();
                        form_manager.add_element(
                            form_id,
                            FormElement::TextArea {
                                name,
                                value,
                                rows,
                                cols,
                                placeholder,
                                required,
                                minlength: elem
                                    .get_attribute("minlength")
                                    .and_then(|v| v.parse::<usize>().ok()),
                                maxlength: elem
                                    .get_attribute("maxlength")
                                    .and_then(|v| v.parse::<usize>().ok()),
                            },
                        );
                    },
                    TagName::Select => {
                        let name = elem.get_attribute("name").unwrap_or("").to_string();
                        let mut options = Vec::new();
                        let mut selected_index = None;
                        // Walk every descendant of the <select> so
                        // <option> elements wrapped in <optgroup> are
                        // collected too. `<optgroup>` is parsed as
                        // `TagName::Unknown("optgroup")` since the
                        // enum has no dedicated variant. We push
                        // children in reverse onto the stack so the
                        // DFS yields them back in document order,
                        // which keeps `selected_index` stable.
                        let mut opt_stack: Vec<usize> =
                            node.children.iter().rev().copied().collect();
                        let mut ordered: Vec<usize> = Vec::new();
                        while let Some(nid) = opt_stack.pop() {
                            ordered.push(nid);
                            for &c in doc.nodes[nid].children.iter().rev() {
                                opt_stack.push(c);
                            }
                        }
                        for opt_id in ordered {
                            if let NodeKind::Element(ref opt) = doc.nodes[opt_id].kind
                                && opt.tag == TagName::Option
                            {
                                let value = opt
                                    .get_attribute("value")
                                    .map(|s| s.to_string())
                                    .unwrap_or_else(|| doc.text_content(opt_id));
                                let label = doc.text_content(opt_id);
                                let disabled = opt.get_attribute("disabled").is_some();
                                if opt.get_attribute("selected").is_some() {
                                    selected_index = Some(options.len());
                                }
                                options.push(SelectOption {
                                    value,
                                    label,
                                    disabled,
                                });
                            }
                        }
                        if selected_index.is_none() && !options.is_empty() {
                            selected_index = Some(0);
                        }
                        form_manager.add_element(
                            form_id,
                            FormElement::SelectBox {
                                name,
                                options,
                                selected_index,
                                open: false,
                            },
                        );
                    },
                    TagName::Button => {
                        let btype = elem
                            .get_attribute("type")
                            .unwrap_or("submit")
                            .to_ascii_lowercase();
                        match btype.as_str() {
                            "submit" => {
                                let name = elem.get_attribute("name").unwrap_or("").to_string();
                                let value = elem.get_attribute("value").unwrap_or("").to_string();
                                let label = doc.text_content(nid);
                                form_manager.add_element(
                                    form_id,
                                    FormElement::SubmitButton { name, value, label },
                                );
                            },
                            "reset" => {
                                let text = doc.text_content(nid);
                                let label = if text.is_empty() {
                                    "Reset".to_string()
                                } else {
                                    text
                                };
                                form_manager
                                    .add_element(form_id, FormElement::ResetButton { label });
                            },
                            _ => {},
                        }
                    },
                    _ => {},
                }
            }
            let _ = form_nid;
        }
    }

    /// Walk the DOM to build a map of `<a>` element NodeIds to their
    /// `href` attribute values.
    pub fn build_link_map(doc: &html::dom::Document) -> HashMap<NodeId, String> {
        let mut map = HashMap::new();
        for (id, node) in doc.nodes.iter().enumerate() {
            if let html::dom::NodeKind::Element(elem) = &node.kind
                && elem.tag == html::dom::TagName::A
                && let Some(href) = elem.get_attribute("href")
            {
                map.insert(id, href.to_string());
            }
        }
        map
    }
}
