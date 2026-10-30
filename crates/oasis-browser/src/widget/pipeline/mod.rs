//! Navigation, resource loading, and content processing for [`BrowserWidget`].

use std::collections::HashMap;

use oasis_vfs::Vfs;

use crate::css;
use crate::gemini;
use crate::html;
use crate::html::dom::NodeId;
use crate::html::escape::push_escaped;
#[cfg(feature = "javascript")]
use crate::js_dom;
use crate::layout;
use crate::loader::cache::CacheEntry;
use crate::loader::csp::{CspPolicy, CspResourceType, InlineKind};
use crate::loader::{
    self, ContentType, ResourceRequest, ResourceResponse, ResourceSource, load_resource,
};
use crate::reader;
use crate::{BrowserWidget, LoadingState, SimpleTextMeasurer};

#[cfg(native_net)]
use crate::loader::io_thread::{IoRequestKind, IoThread};

mod document;
mod gemini_page;
mod navigation;
mod scripts;
mod stylesheets;

#[cfg(test)]
pub(crate) use stylesheets::css_import_urls;

/// Whether a stylesheet's source contains media-dependent rules.
fn css_uses_media_queries(css_text: &str) -> bool {
    css_text
        .as_bytes()
        .windows(6)
        .any(|w| w.eq_ignore_ascii_case(b"@media"))
}

/// HTML source to render for a text-like response body. HTML (and
/// untyped bodies, which are sniffed as HTML) is used as-is; `text/plain`
/// is escaped into a `<pre>` so markup in it is displayed verbatim rather
/// than parsed — and any `<script>` in it never runs.
fn html_for_text_body(content_type: ContentType, body: &[u8]) -> String {
    let text = String::from_utf8_lossy(body);
    if content_type != ContentType::PlainText {
        return text.into_owned();
    }
    let mut html = String::with_capacity(text.len() + 96);
    html.push_str("<!DOCTYPE html><html><body><pre style=\"white-space:pre-wrap\">");
    push_escaped(&mut html, &text);
    html.push_str("</pre></body></html>");
    html
}
