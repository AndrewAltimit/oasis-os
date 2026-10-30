//! In-page navigation: links, fragments, back / forward / home.

use super::*;

impl BrowserWidget {
    /// Navigate to a URL, resolving relative references against
    /// the current page.
    pub fn navigate_to(&mut self, href: &str, vfs: &dyn Vfs) {
        let resolved = if let Some(current) = self.nav.current_url() {
            if let Some(base) = Url::parse(current) {
                base.resolve(href)
                    .map(|u| u.to_string())
                    .unwrap_or_else(|| href.to_string())
            } else {
                href.to_string()
            }
        } else {
            href.to_string()
        };

        // Track this URL as visited for :visited pseudo-class.
        self.visited_urls.insert(resolved.clone());

        // Remember where the user was on the page being left, so Back
        // returns to the same spot.
        self.nav.update_scroll(self.scroll.scroll_y);

        // Same-document fragment navigation (`#section` links): push a
        // history entry and scroll to the target without refetching or
        // re-running the page's scripts.
        if let Some((_, fragment)) = resolved.split_once('#')
            && self.is_same_document(self.nav.current_url(), &resolved)
        {
            let fragment = fragment.to_string();
            let title = self.nav.current_title().unwrap_or("").to_string();
            self.nav.update_scroll(self.scroll.scroll_y);
            self.nav.navigate(&resolved, &title);
            self.scroll_to_fragment(&fragment);
            return;
        }

        self.navigate_vfs(&resolved, vfs);
    }

    /// Whether `url` addresses the currently loaded document (whose URL
    /// is `loaded`), ignoring any `#fragment` on either side.
    pub(super) fn is_same_document(&self, loaded: Option<&str>, url: &str) -> bool {
        if self.document.is_none() || self.state == crate::LoadingState::Loading {
            return false;
        }
        let strip = |u: &str| u.split('#').next().unwrap_or("").to_string();
        loaded.is_some_and(|current| strip(current) == strip(url))
    }

    /// Scroll the page so the element targeted by `fragment` (an `id`,
    /// or a legacy `<a name>`) sits at the top of the viewport. An empty
    /// fragment or `#top` scrolls to the top. Returns `false` when no
    /// target exists (the scroll position is then left alone, as
    /// browsers do).
    pub(crate) fn scroll_to_fragment(&mut self, fragment: &str) -> bool {
        use crate::html::dom::{NodeKind, TagName};

        if fragment.is_empty() || fragment.eq_ignore_ascii_case("top") {
            self.scroll.scroll_to(0);
            return true;
        }
        let (Some(doc), Some(layout)) = (&self.document, &self.layout_root) else {
            return false;
        };
        let target = doc.get_element_by_id(fragment).or_else(|| {
            doc.nodes.iter().position(|n| {
                matches!(&n.kind, NodeKind::Element(e)
                    if e.tag == TagName::A && e.get_attribute("name") == Some(fragment))
            })
        });
        let Some(rect) = target.and_then(|nid| Self::find_node_rect(layout, nid)) else {
            return false;
        };
        let content_h = layout.scrollable_height() as i32;
        self.scroll.set_content_height(content_h);
        self.scroll.scroll_to(rect.y as i32);
        true
    }

    /// Go back in history.
    ///
    /// Prefers loading from cache when available to avoid re-fetching.
    pub fn go_back(&mut self, vfs: &dyn Vfs) {
        // Save current scroll position.
        self.nav.update_scroll(self.scroll.scroll_y);
        let loaded = self.nav.current_url().map(str::to_string);

        if let Some(entry) = self.nav.go_back() {
            let url = entry.url.clone();
            let scroll_y = entry.scroll_y;
            // History traversal within one document (fragment entries)
            // only restores the scroll position.
            if !self.is_same_document(loaded.as_deref(), &url) {
                self.navigate_cached_or_fetch(&url, vfs);
            }
            self.scroll.scroll_to(scroll_y);
        }
    }

    /// Go forward in history.
    ///
    /// Prefers loading from cache when available to avoid re-fetching.
    pub fn go_forward(&mut self, vfs: &dyn Vfs) {
        self.nav.update_scroll(self.scroll.scroll_y);
        let loaded = self.nav.current_url().map(str::to_string);

        if let Some(entry) = self.nav.go_forward() {
            let url = entry.url.clone();
            let scroll_y = entry.scroll_y;
            if self.is_same_document(loaded.as_deref(), &url) {
                // A forward fragment entry has no saved offset yet (it
                // is recorded when leaving an entry): re-target it.
                match url.split_once('#') {
                    Some((_, frag)) if scroll_y == 0 => {
                        let frag = frag.to_string();
                        self.scroll_to_fragment(&frag);
                    },
                    _ => self.scroll.scroll_to(scroll_y),
                }
            } else {
                self.navigate_cached_or_fetch(&url, vfs);
                self.scroll.scroll_to(scroll_y);
            }
        }
    }

    /// Navigate to the home page.
    pub fn go_home(&mut self, vfs: &dyn Vfs) {
        let url = self.nav.go_home();
        self.navigate_vfs(&url, vfs);
    }
}
