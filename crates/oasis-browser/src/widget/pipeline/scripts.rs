//! Page scripts: collection under CSP, deferred execution and console
//! output.

use super::*;

impl BrowserWidget {
    /// Walk the DOM to collect inline `<script>` text in document order.
    /// External scripts (`<script src="...">`) and non-JavaScript types
    /// (e.g. `application/ld+json`) are skipped.
    ///
    /// Returns `(immediate, deferred)` where `deferred` contains scripts
    /// with the `defer` or `async` attribute, to be executed after the
    /// first paint.
    ///
    /// Scripts the page's CSP does not allow (`script-src-elem` /
    /// `script-src`: matching `nonce`, `'sha256-…'` hash, or
    /// `'unsafe-inline'` without nonces / hashes / `'strict-dynamic'`) are
    /// skipped.
    #[cfg(feature = "javascript")]
    pub(super) fn collect_scripts(
        doc: &html::dom::Document,
        csp: Option<&CspPolicy>,
    ) -> (Vec<String>, Vec<String>) {
        let mut immediate = Vec::new();
        let mut deferred = Vec::new();
        // Document order, skipping inert `<template>` contents.
        for id in js_dom::active_elements(doc) {
            if let html::dom::NodeKind::Element(elem) = &doc.nodes[id].kind
                && elem.tag == html::dom::TagName::Script
                && elem.get_attribute("src").is_none()
                && Self::is_js_script_type(elem.get_attribute("type"))
            {
                let text = doc.text_content(id);
                if let Some(policy) = csp
                    && !policy.allows_inline(
                        InlineKind::ScriptElement,
                        elem.get_attribute("nonce"),
                        &text,
                    )
                {
                    log::warn!("CSP script-src blocked an inline <script>");
                    continue;
                }
                if !text.is_empty() {
                    let is_deferred = elem.get_attribute("defer").is_some()
                        || elem.get_attribute("async").is_some();
                    if is_deferred {
                        deferred.push(text);
                    } else {
                        immediate.push(text);
                    }
                }
            }
        }
        (immediate, deferred)
    }

    /// Returns `true` if the script `type` attribute indicates JavaScript
    /// (or is absent/empty, which defaults to JS per the HTML spec).
    #[cfg(feature = "javascript")]
    pub(super) fn is_js_script_type(type_attr: Option<&str>) -> bool {
        match type_attr {
            None | Some("") => true,
            Some(t) => {
                let t = t.trim().to_ascii_lowercase();
                matches!(
                    t.as_str(),
                    "text/javascript"
                        | "application/javascript"
                        | "text/ecmascript"
                        | "application/ecmascript"
                        | "module"
                )
            },
        }
    }

    /// Execute deferred scripts (those with `defer` or `async` attributes).
    ///
    /// Should be called after the first paint so that initial rendering is
    /// not blocked by script execution.
    #[cfg(feature = "javascript")]
    pub fn execute_deferred_scripts(&mut self) {
        if self.deferred_scripts.is_empty() {
            return;
        }
        let scripts = std::mem::take(&mut self.deferred_scripts);
        if let Some(engine) = &self.js_engine {
            let refs: Vec<&str> = scripts.iter().map(String::as_str).collect();
            engine.eval_all(&refs);
            // Check if deferred scripts triggered any DOM mutations that
            // require relayout.
            let fired = engine.tick_timers(0.0);
            if fired > 0 {
                self.layout_dirty = true;
            }
        }
    }

    /// Console output from JavaScript execution on the current page.
    #[cfg(feature = "javascript")]
    pub fn console_output(&self) -> &[oasis_js::ConsoleEntry] {
        &self.console_output
    }
}
