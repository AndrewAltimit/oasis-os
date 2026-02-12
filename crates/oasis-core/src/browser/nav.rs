//! Navigation controller: history stack, bookmarks, URL bar state.

/// A single entry in the navigation history.
#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub url: String,
    pub title: String,
    pub scroll_y: i32,
    pub reader_mode: bool,
    pub timestamp: u64,
}

/// A bookmark.
#[derive(Debug, Clone, PartialEq)]
pub struct Bookmark {
    pub url: String,
    pub title: String,
}

/// Navigation controller managing history and bookmarks.
pub struct NavigationController {
    back_stack: Vec<HistoryEntry>,
    forward_stack: Vec<HistoryEntry>,
    current: Option<HistoryEntry>,
    home_url: String,
    bookmarks: Vec<Bookmark>,
}

impl NavigationController {
    pub fn new(home_url: &str) -> Self {
        Self {
            back_stack: Vec::new(),
            forward_stack: Vec::new(),
            current: None,
            home_url: home_url.to_string(),
            bookmarks: Vec::new(),
        }
    }

    /// Navigate to a new URL. Pushes current page to back stack,
    /// clears forward stack.
    pub fn navigate(&mut self, url: &str, title: &str) {
        if let Some(entry) = self.current.take() {
            self.back_stack.push(entry);
        }
        self.forward_stack.clear();
        self.current = Some(HistoryEntry {
            url: url.to_string(),
            title: title.to_string(),
            scroll_y: 0,
            reader_mode: false,
            timestamp: 0,
        });
    }

    /// Go back in history. Returns the URL to load, or None.
    pub fn go_back(&mut self) -> Option<HistoryEntry> {
        let prev = self.back_stack.pop()?;
        if let Some(current) = self.current.take() {
            self.forward_stack.push(current);
        }
        self.current = Some(prev.clone());
        Some(prev)
    }

    /// Go forward in history. Returns the URL to load, or None.
    pub fn go_forward(&mut self) -> Option<HistoryEntry> {
        let next = self.forward_stack.pop()?;
        if let Some(current) = self.current.take() {
            self.back_stack.push(current);
        }
        self.current = Some(next.clone());
        Some(next)
    }

    /// Navigate to the home page.
    pub fn go_home(&mut self) -> String {
        let url = self.home_url.clone();
        self.navigate(&url, "");
        url
    }

    /// Get the current URL, if any.
    pub fn current_url(&self) -> Option<&str> {
        self.current.as_ref().map(|e| e.url.as_str())
    }

    /// Get the current page title, if any.
    pub fn current_title(&self) -> Option<&str> {
        self.current.as_ref().map(|e| e.title.as_str())
    }

    /// Update the current page's scroll position (for restoring
    /// on back-nav).
    pub fn update_scroll(&mut self, scroll_y: i32) {
        if let Some(entry) = self.current.as_mut() {
            entry.scroll_y = scroll_y;
        }
    }

    /// Update the current page's reader mode state.
    pub fn update_reader_mode(&mut self, reader_mode: bool) {
        if let Some(entry) = self.current.as_mut() {
            entry.reader_mode = reader_mode;
        }
    }

    /// Update the current page's title (after page load completes).
    pub fn update_title(&mut self, title: &str) {
        if let Some(entry) = self.current.as_mut() {
            entry.title = title.to_string();
        }
    }

    /// Check if back navigation is possible.
    pub fn can_go_back(&self) -> bool {
        !self.back_stack.is_empty()
    }

    /// Check if forward navigation is possible.
    pub fn can_go_forward(&self) -> bool {
        !self.forward_stack.is_empty()
    }

    /// Add a bookmark for the current page.
    pub fn add_bookmark(&mut self) {
        if let Some(entry) = &self.current {
            let bm = Bookmark {
                url: entry.url.clone(),
                title: entry.title.clone(),
            };
            if !self.bookmarks.contains(&bm) {
                self.bookmarks.push(bm);
            }
        }
    }

    /// Remove a bookmark by URL.
    pub fn remove_bookmark(&mut self, url: &str) {
        self.bookmarks.retain(|bm| bm.url != url);
    }

    /// Check if the current URL is bookmarked.
    pub fn is_bookmarked(&self) -> bool {
        let Some(entry) = &self.current else {
            return false;
        };
        self.bookmarks.iter().any(|bm| bm.url == entry.url)
    }

    /// Get all bookmarks.
    pub fn bookmarks(&self) -> &[Bookmark] {
        &self.bookmarks
    }

    /// Get history entries (most recent first).
    ///
    /// Returns the current page followed by back-stack entries in
    /// reverse chronological order.
    pub fn history(&self) -> Vec<&HistoryEntry> {
        let mut entries: Vec<&HistoryEntry> = Vec::new();
        if let Some(current) = &self.current {
            entries.push(current);
        }
        for entry in self.back_stack.iter().rev() {
            entries.push(entry);
        }
        entries
    }

    /// Set the home URL.
    pub fn set_home(&mut self, url: &str) {
        self.home_url = url.to_string();
    }

    /// Get the home URL.
    pub fn home_url(&self) -> &str {
        &self.home_url
    }

    /// Generate HTML for the bookmarks page.
    pub fn bookmarks_page_html(&self) -> String {
        let mut html = String::from(
            "<html><head><title>Bookmarks</title></head><body>\
             <h1>Bookmarks</h1><ul>",
        );
        for bm in &self.bookmarks {
            html.push_str(&format!(
                "<li><a href=\"{}\">{}</a></li>",
                bm.url,
                if bm.title.is_empty() {
                    &bm.url
                } else {
                    &bm.title
                }
            ));
        }
        html.push_str("</ul></body></html>");
        html
    }

    /// Generate HTML for the history page.
    pub fn history_page_html(&self) -> String {
        let mut html = String::from(
            "<html><head><title>History</title></head><body>\
             <h1>History</h1><ul>",
        );
        for entry in self.history() {
            html.push_str(&format!(
                "<li><a href=\"{}\">{}</a></li>",
                entry.url,
                if entry.title.is_empty() {
                    &entry.url
                } else {
                    &entry.title
                }
            ));
        }
        html.push_str("</ul></body></html>");
        html
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigate_pushes_to_back_stack() {
        let mut nav = NavigationController::new("about:home");
        nav.navigate("https://a.com", "A");
        nav.navigate("https://b.com", "B");

        assert!(nav.can_go_back());
        assert_eq!(nav.current_url(), Some("https://b.com"));
    }

    #[test]
    fn go_back_restores_previous_entry() {
        let mut nav = NavigationController::new("about:home");
        nav.navigate("https://a.com", "A");
        nav.navigate("https://b.com", "B");

        let entry = nav.go_back().unwrap();
        assert_eq!(entry.url, "https://a.com");
        assert_eq!(entry.title, "A");
        assert_eq!(nav.current_url(), Some("https://a.com"));
    }

    #[test]
    fn go_forward_after_go_back() {
        let mut nav = NavigationController::new("about:home");
        nav.navigate("https://a.com", "A");
        nav.navigate("https://b.com", "B");
        nav.go_back();

        assert!(nav.can_go_forward());
        let entry = nav.go_forward().unwrap();
        assert_eq!(entry.url, "https://b.com");
        assert_eq!(nav.current_url(), Some("https://b.com"));
    }

    #[test]
    fn forward_stack_cleared_on_new_navigation() {
        let mut nav = NavigationController::new("about:home");
        nav.navigate("https://a.com", "A");
        nav.navigate("https://b.com", "B");
        nav.go_back();
        assert!(nav.can_go_forward());

        nav.navigate("https://c.com", "C");
        assert!(!nav.can_go_forward());
    }

    #[test]
    fn can_go_back_and_forward() {
        let mut nav = NavigationController::new("about:home");
        assert!(!nav.can_go_back());
        assert!(!nav.can_go_forward());

        nav.navigate("https://a.com", "A");
        assert!(!nav.can_go_back());

        nav.navigate("https://b.com", "B");
        assert!(nav.can_go_back());
        assert!(!nav.can_go_forward());

        nav.go_back();
        assert!(!nav.can_go_back());
        assert!(nav.can_go_forward());
    }

    #[test]
    fn add_remove_bookmarks() {
        let mut nav = NavigationController::new("about:home");
        nav.navigate("https://a.com", "A");
        nav.add_bookmark();

        assert_eq!(nav.bookmarks().len(), 1);
        assert_eq!(nav.bookmarks()[0].url, "https://a.com");
        assert_eq!(nav.bookmarks()[0].title, "A");

        nav.remove_bookmark("https://a.com");
        assert!(nav.bookmarks().is_empty());
    }

    #[test]
    fn is_bookmarked_check() {
        let mut nav = NavigationController::new("about:home");
        nav.navigate("https://a.com", "A");
        assert!(!nav.is_bookmarked());

        nav.add_bookmark();
        assert!(nav.is_bookmarked());

        // Duplicate add is a no-op.
        nav.add_bookmark();
        assert_eq!(nav.bookmarks().len(), 1);
    }

    #[test]
    fn update_scroll_position_on_current_entry() {
        let mut nav = NavigationController::new("about:home");
        nav.navigate("https://a.com", "A");
        nav.update_scroll(150);

        nav.navigate("https://b.com", "B");
        let entry = nav.go_back().unwrap();
        assert_eq!(entry.scroll_y, 150);
    }

    #[test]
    fn home_navigation() {
        let mut nav = NavigationController::new("about:home");
        nav.navigate("https://a.com", "A");

        let url = nav.go_home();
        assert_eq!(url, "about:home");
        assert_eq!(nav.current_url(), Some("about:home"));
        assert!(nav.can_go_back());
    }

    #[test]
    fn history_listing() {
        let mut nav = NavigationController::new("about:home");
        nav.navigate("https://a.com", "A");
        nav.navigate("https://b.com", "B");
        nav.navigate("https://c.com", "C");

        let history = nav.history();
        assert_eq!(history.len(), 3);
        // Most recent first.
        assert_eq!(history[0].url, "https://c.com");
        assert_eq!(history[1].url, "https://b.com");
        assert_eq!(history[2].url, "https://a.com");
    }

    mod prop {
        use super::*;
        use proptest::prelude::*;

        fn arb_url() -> impl Strategy<Value = String> {
            "[a-z]{3,10}".prop_map(|s| format!("https://{s}.com"))
        }

        fn arb_urls(min: usize, max: usize) -> impl Strategy<Value = Vec<String>> {
            proptest::collection::vec(arb_url(), min..max)
        }

        proptest! {
            #[test]
            fn current_url_equals_last_navigated(urls in arb_urls(1, 20)) {
                let mut nav = NavigationController::new("about:home");
                for url in &urls {
                    nav.navigate(url, "");
                }
                prop_assert_eq!(nav.current_url(), Some(urls.last().unwrap().as_str()));
            }

            #[test]
            fn back_then_forward_returns_to_same(urls in arb_urls(2, 10)) {
                let mut nav = NavigationController::new("about:home");
                for url in &urls {
                    nav.navigate(url, "");
                }
                let before_back = nav.current_url().unwrap().to_string();
                nav.go_back().unwrap();
                nav.go_forward().unwrap();
                prop_assert_eq!(nav.current_url().unwrap(), before_back.as_str());
            }

            #[test]
            fn history_length_equals_navigations(urls in arb_urls(1, 20)) {
                let mut nav = NavigationController::new("about:home");
                for url in &urls {
                    nav.navigate(url, "");
                }
                let history = nav.history();
                prop_assert_eq!(history.len(), urls.len());
            }

            #[test]
            fn navigate_clears_forward_stack(urls in arb_urls(3, 10)) {
                let mut nav = NavigationController::new("about:home");
                for url in &urls {
                    nav.navigate(url, "");
                }
                nav.go_back();
                prop_assert!(nav.can_go_forward());
                nav.navigate("https://new.com", "New");
                prop_assert!(!nav.can_go_forward());
            }

            #[test]
            fn can_go_back_all_the_way(urls in arb_urls(1, 20)) {
                let mut nav = NavigationController::new("about:home");
                for url in &urls {
                    nav.navigate(url, "");
                }
                // Go back urls.len()-1 times (first navigate sets current, rest push to back).
                let mut back_count = 0;
                while nav.can_go_back() {
                    nav.go_back();
                    back_count += 1;
                }
                prop_assert_eq!(back_count, urls.len() - 1);
                // Now current should be the first URL.
                prop_assert_eq!(nav.current_url().unwrap(), urls[0].as_str());
            }

            #[test]
            fn can_go_forward_all_the_way(urls in arb_urls(2, 10)) {
                let mut nav = NavigationController::new("about:home");
                for url in &urls {
                    nav.navigate(url, "");
                }
                // Go all the way back.
                while nav.can_go_back() {
                    nav.go_back();
                }
                // Now go all the way forward.
                let mut fwd_count = 0;
                while nav.can_go_forward() {
                    nav.go_forward();
                    fwd_count += 1;
                }
                prop_assert_eq!(fwd_count, urls.len() - 1);
                prop_assert_eq!(nav.current_url().unwrap(), urls.last().unwrap().as_str());
            }

            #[test]
            fn bookmark_add_is_idempotent(url in arb_url()) {
                let mut nav = NavigationController::new("about:home");
                nav.navigate(&url, "Title");
                nav.add_bookmark();
                nav.add_bookmark();
                nav.add_bookmark();
                prop_assert_eq!(nav.bookmarks().len(), 1);
            }

            #[test]
            fn scroll_position_preserved_on_back(
                urls in arb_urls(2, 5),
                scroll in 0i32..10000,
            ) {
                let mut nav = NavigationController::new("about:home");
                nav.navigate(&urls[0], "A");
                nav.update_scroll(scroll);
                nav.navigate(&urls[1], "B");
                let entry = nav.go_back().unwrap();
                prop_assert_eq!(entry.scroll_y, scroll);
            }
        }
    }
}
