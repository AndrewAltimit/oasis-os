//! In-page text search (Find in Page).
//!
//! Provides case-sensitive and case-insensitive text searching across
//! the rendered page.  [`SearchState`] tracks the current query, all
//! matches, and the currently highlighted match.  Helper functions
//! handle the low-level substring search and formatting.

// -------------------------------------------------------------------
// Public types
// -------------------------------------------------------------------

/// Bounding rectangle for a search match highlight.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchRect {
    /// X coordinate of the top-left corner.
    pub x: f32,
    /// Y coordinate of the top-left corner.
    pub y: f32,
    /// Width of the rectangle.
    pub width: f32,
    /// Height of the rectangle.
    pub height: f32,
}

/// A single search match within the page.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchMatch {
    /// DOM node id containing the matched text.
    pub node: usize,
    /// Character offset within the text node where the match starts.
    pub text_offset: usize,
    /// Length of the match in characters.
    pub match_len: usize,
    /// Bounding box for rendering the highlight overlay.
    pub rect: SearchRect,
}

/// Persistent state for in-page text search.
///
/// Manages the search query, collected matches, navigation between
/// matches, and display options (case sensitivity).
pub struct SearchState {
    /// Current search query string.
    query: String,
    /// All matches found for the current query.
    matches: Vec<SearchMatch>,
    /// Index of the currently highlighted match within `matches`.
    current_index: usize,
    /// Whether the search is case-sensitive.
    case_sensitive: bool,
    /// Whether the search overlay is visible / active.
    active: bool,
}

// -------------------------------------------------------------------
// SearchState implementation
// -------------------------------------------------------------------

impl SearchState {
    /// Create a new inactive search state.
    pub fn new() -> Self {
        Self {
            query: String::new(),
            matches: Vec::new(),
            current_index: 0,
            case_sensitive: false,
            active: false,
        }
    }

    /// Activate search with an initial query.
    ///
    /// This opens the search overlay and sets the query string.
    /// Matches must be populated separately via [`set_matches`] or
    /// the `collect_text_nodes` helper after layout.
    pub fn open(&mut self, query: &str) {
        self.active = true;
        self.query = query.to_string();
        self.matches.clear();
        self.current_index = 0;
    }

    /// Deactivate search and clear all state.
    pub fn close(&mut self) {
        self.active = false;
        self.query.clear();
        self.matches.clear();
        self.current_index = 0;
    }

    /// Update the search query, clearing existing matches.
    ///
    /// The caller should re-run the search after changing the query.
    pub fn set_query(&mut self, query: &str) {
        self.query = query.to_string();
        self.matches.clear();
        self.current_index = 0;
    }

    /// Replace the current match list.
    pub fn set_matches(&mut self, matches: Vec<SearchMatch>) {
        self.matches = matches;
        self.current_index = 0;
    }

    /// Whether the search overlay is active.
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Toggle between case-sensitive and case-insensitive search.
    ///
    /// Clears existing matches so the caller can re-search.
    pub fn toggle_case_sensitive(&mut self) {
        self.case_sensitive = !self.case_sensitive;
        self.matches.clear();
        self.current_index = 0;
    }

    /// Number of matches found.
    pub fn match_count(&self) -> usize {
        self.matches.len()
    }

    /// Get a reference to the currently highlighted match, if any.
    pub fn current_match(&self) -> Option<&SearchMatch> {
        self.matches.get(self.current_index)
    }

    /// Advance to the next match, wrapping around to the first.
    pub fn next_match(&mut self) {
        if self.matches.is_empty() {
            return;
        }
        self.current_index = (self.current_index + 1) % self.matches.len();
    }

    /// Go to the previous match, wrapping around to the last.
    pub fn prev_match(&mut self) {
        if self.matches.is_empty() {
            return;
        }
        if self.current_index == 0 {
            self.current_index = self.matches.len() - 1;
        } else {
            self.current_index -= 1;
        }
    }

    /// Returns the Y coordinate to scroll to for the current match.
    ///
    /// Returns `None` when there are no matches.
    pub fn scroll_to_current(&self) -> Option<f32> {
        self.current_match().map(|m| m.rect.y)
    }

    /// RGBA color for regular match highlights (yellow, semi-transparent).
    pub fn highlight_color() -> (u8, u8, u8, u8) {
        (255, 255, 0, 128)
    }

    /// RGBA color for the current/active match highlight (orange, more opaque).
    pub fn current_highlight_color() -> (u8, u8, u8, u8) {
        (255, 165, 0, 160)
    }

    /// Current query string.
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Whether case-sensitive mode is enabled.
    pub fn is_case_sensitive(&self) -> bool {
        self.case_sensitive
    }

    /// Read-only access to all matches.
    pub fn matches(&self) -> &[SearchMatch] {
        &self.matches
    }

    /// Current match index (0-based).
    pub fn current_index(&self) -> usize {
        self.current_index
    }
}

impl Default for SearchState {
    fn default() -> Self {
        Self::new()
    }
}

// -------------------------------------------------------------------
// Free functions
// -------------------------------------------------------------------

/// Find all occurrences of `query` in `text`.
///
/// Returns a list of `(byte_offset, char_length)` pairs.  When
/// `case_sensitive` is false both strings are lowercased before
/// comparison.  An empty query always returns an empty list.
pub fn search_text(text: &str, query: &str, case_sensitive: bool) -> Vec<(usize, usize)> {
    if query.is_empty() {
        return Vec::new();
    }

    let query_len = query.chars().count();

    if case_sensitive {
        let mut results = Vec::new();
        let mut start = 0;
        while let Some(pos) = text[start..].find(query) {
            let abs_pos = start + pos;
            // Convert byte offset to character offset.
            let char_offset = text[..abs_pos].chars().count();
            results.push((char_offset, query_len));
            start = abs_pos + query.len();
        }
        results
    } else {
        let lower_text = text.to_lowercase();
        let lower_query = query.to_lowercase();
        let mut results = Vec::new();
        let mut start = 0;
        while let Some(pos) = lower_text[start..].find(&lower_query) {
            let abs_pos = start + pos;
            let char_offset = lower_text[..abs_pos].chars().count();
            results.push((char_offset, query_len));
            start = abs_pos + lower_query.len();
        }
        results
    }
}

/// Collect search matches from pre-extracted text nodes.
///
/// Each entry in `nodes` is `(node_id, text, x, y, width, height)`.
/// For each text node the function searches for `query` and produces
/// a [`SearchMatch`] with an approximate bounding rect derived from
/// the character position within the node.
pub fn collect_text_nodes(
    nodes: &[(usize, String, f32, f32, f32, f32)],
    query: &str,
    case_sensitive: bool,
) -> Vec<SearchMatch> {
    let mut matches = Vec::new();

    for (node_id, text, x, y, width, height) in nodes {
        let hits = search_text(text, query, case_sensitive);
        let total_chars = text.chars().count();
        if total_chars == 0 {
            continue;
        }
        let char_width = *width / total_chars as f32;

        for (char_offset, match_len) in hits {
            let match_x = *x + char_offset as f32 * char_width;
            let match_w = match_len as f32 * char_width;
            matches.push(SearchMatch {
                node: *node_id,
                text_offset: char_offset,
                match_len,
                rect: SearchRect {
                    x: match_x,
                    y: *y,
                    width: match_w,
                    height: *height,
                },
            });
        }
    }

    matches
}

/// Format a status string for the search bar.
///
/// Returns:
/// - `""` when search is inactive
/// - `"No matches"` when active with zero results
/// - `"1/5 matches"` (1-indexed current position) otherwise
pub fn format_status(state: &SearchState) -> String {
    if !state.is_active() {
        return String::new();
    }
    if state.matches.is_empty() {
        if state.query.is_empty() {
            return String::new();
        }
        return "No matches".to_string();
    }
    let current = state.current_index + 1;
    let total = state.matches.len();
    format!("{current}/{total} matches")
}

// -------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- SearchState lifecycle ------------------------------------

    #[test]
    fn new_state_is_inactive() {
        let state = SearchState::new();
        assert!(!state.is_active());
        assert_eq!(state.query(), "");
        assert_eq!(state.match_count(), 0);
        assert!(state.current_match().is_none());
    }

    #[test]
    fn default_trait_matches_new() {
        let state = SearchState::default();
        assert!(!state.is_active());
        assert_eq!(state.match_count(), 0);
    }

    #[test]
    fn open_activates_with_query() {
        let mut state = SearchState::new();
        state.open("hello");
        assert!(state.is_active());
        assert_eq!(state.query(), "hello");
        assert_eq!(state.match_count(), 0);
    }

    #[test]
    fn close_deactivates_and_clears() {
        let mut state = SearchState::new();
        state.open("test");
        state.set_matches(vec![SearchMatch {
            node: 0,
            text_offset: 0,
            match_len: 4,
            rect: SearchRect {
                x: 0.0,
                y: 0.0,
                width: 20.0,
                height: 10.0,
            },
        }]);
        assert_eq!(state.match_count(), 1);

        state.close();
        assert!(!state.is_active());
        assert_eq!(state.query(), "");
        assert_eq!(state.match_count(), 0);
        assert!(state.current_match().is_none());
    }

    #[test]
    fn open_close_open_cycle() {
        let mut state = SearchState::new();
        state.open("first");
        assert!(state.is_active());
        state.close();
        assert!(!state.is_active());
        state.open("second");
        assert!(state.is_active());
        assert_eq!(state.query(), "second");
    }

    // -- Query management -----------------------------------------

    #[test]
    fn set_query_updates_and_clears_matches() {
        let mut state = SearchState::new();
        state.open("old");
        state.set_matches(vec![SearchMatch {
            node: 1,
            text_offset: 0,
            match_len: 3,
            rect: SearchRect {
                x: 0.0,
                y: 0.0,
                width: 15.0,
                height: 10.0,
            },
        }]);
        assert_eq!(state.match_count(), 1);

        state.set_query("new");
        assert_eq!(state.query(), "new");
        assert_eq!(state.match_count(), 0);
        assert_eq!(state.current_index(), 0);
    }

    // -- Case sensitivity -----------------------------------------

    #[test]
    fn default_is_case_insensitive() {
        let state = SearchState::new();
        assert!(!state.is_case_sensitive());
    }

    #[test]
    fn toggle_case_sensitive() {
        let mut state = SearchState::new();
        state.open("test");
        state.set_matches(vec![SearchMatch {
            node: 0,
            text_offset: 0,
            match_len: 4,
            rect: SearchRect {
                x: 0.0,
                y: 0.0,
                width: 20.0,
                height: 10.0,
            },
        }]);

        state.toggle_case_sensitive();
        assert!(state.is_case_sensitive());
        assert_eq!(state.match_count(), 0); // cleared

        state.toggle_case_sensitive();
        assert!(!state.is_case_sensitive());
    }

    // -- Navigation -----------------------------------------------

    #[test]
    fn next_match_advances_and_wraps() {
        let mut state = SearchState::new();
        state.open("x");
        let m = |i: usize| SearchMatch {
            node: i,
            text_offset: 0,
            match_len: 1,
            rect: SearchRect {
                x: 0.0,
                y: i as f32 * 10.0,
                width: 5.0,
                height: 10.0,
            },
        };
        state.set_matches(vec![m(0), m(1), m(2)]);

        assert_eq!(state.current_index(), 0);
        state.next_match();
        assert_eq!(state.current_index(), 1);
        state.next_match();
        assert_eq!(state.current_index(), 2);
        state.next_match();
        assert_eq!(state.current_index(), 0); // wrapped
    }

    #[test]
    fn prev_match_goes_back_and_wraps() {
        let mut state = SearchState::new();
        state.open("x");
        let m = |i: usize| SearchMatch {
            node: i,
            text_offset: 0,
            match_len: 1,
            rect: SearchRect {
                x: 0.0,
                y: i as f32 * 10.0,
                width: 5.0,
                height: 10.0,
            },
        };
        state.set_matches(vec![m(0), m(1), m(2)]);

        assert_eq!(state.current_index(), 0);
        state.prev_match();
        assert_eq!(state.current_index(), 2); // wrapped to last
        state.prev_match();
        assert_eq!(state.current_index(), 1);
        state.prev_match();
        assert_eq!(state.current_index(), 0);
    }

    #[test]
    fn next_prev_on_empty_matches_is_noop() {
        let mut state = SearchState::new();
        state.open("nothing");
        state.next_match();
        assert_eq!(state.current_index(), 0);
        state.prev_match();
        assert_eq!(state.current_index(), 0);
    }

    #[test]
    fn next_match_single_element_stays() {
        let mut state = SearchState::new();
        state.open("x");
        state.set_matches(vec![SearchMatch {
            node: 0,
            text_offset: 0,
            match_len: 1,
            rect: SearchRect {
                x: 0.0,
                y: 50.0,
                width: 5.0,
                height: 10.0,
            },
        }]);
        state.next_match();
        assert_eq!(state.current_index(), 0);
        state.prev_match();
        assert_eq!(state.current_index(), 0);
    }

    // -- Scroll position ------------------------------------------

    #[test]
    fn scroll_to_current_returns_y() {
        let mut state = SearchState::new();
        state.open("x");
        state.set_matches(vec![SearchMatch {
            node: 0,
            text_offset: 0,
            match_len: 1,
            rect: SearchRect {
                x: 10.0,
                y: 200.0,
                width: 5.0,
                height: 10.0,
            },
        }]);
        assert_eq!(state.scroll_to_current(), Some(200.0));
    }

    #[test]
    fn scroll_to_current_none_when_empty() {
        let state = SearchState::new();
        assert_eq!(state.scroll_to_current(), None);
    }

    // -- Colors ---------------------------------------------------

    #[test]
    fn highlight_color_is_yellow_semitransparent() {
        let (r, g, b, a) = SearchState::highlight_color();
        assert_eq!((r, g, b), (255, 255, 0));
        assert_eq!(a, 128);
    }

    #[test]
    fn current_highlight_color_is_orange() {
        let (r, g, b, a) = SearchState::current_highlight_color();
        assert_eq!((r, g, b), (255, 165, 0));
        assert_eq!(a, 160);
    }

    // -- search_text function -------------------------------------

    #[test]
    fn search_text_basic() {
        let hits = search_text("hello world", "world", false);
        assert_eq!(hits, vec![(6, 5)]);
    }

    #[test]
    fn search_text_multiple_matches() {
        let hits = search_text("abcabcabc", "abc", false);
        assert_eq!(hits, vec![(0, 3), (3, 3), (6, 3)]);
    }

    #[test]
    fn search_text_case_insensitive() {
        let hits = search_text("Hello HELLO hello", "hello", false);
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0], (0, 5));
        assert_eq!(hits[1], (6, 5));
        assert_eq!(hits[2], (12, 5));
    }

    #[test]
    fn search_text_case_sensitive() {
        let hits = search_text("Hello HELLO hello", "Hello", true);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0], (0, 5));
    }

    #[test]
    fn search_text_empty_query() {
        let hits = search_text("some text", "", false);
        assert!(hits.is_empty());
    }

    #[test]
    fn search_text_no_matches() {
        let hits = search_text("hello world", "xyz", false);
        assert!(hits.is_empty());
    }

    #[test]
    fn search_text_empty_text() {
        let hits = search_text("", "abc", false);
        assert!(hits.is_empty());
    }

    #[test]
    fn search_text_single_char() {
        let hits = search_text("abacada", "a", false);
        assert_eq!(hits.len(), 4);
        assert_eq!(hits[0].0, 0);
        assert_eq!(hits[1].0, 2);
        assert_eq!(hits[2].0, 4);
        assert_eq!(hits[3].0, 6);
    }

    #[test]
    fn search_text_unicode() {
        let text = "cafe\u{0301} is nice";
        let hits = search_text(text, "caf", false);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0], (0, 3));
    }

    #[test]
    fn search_text_unicode_multibyte() {
        // CJK characters: each is 3 bytes in UTF-8 but 1 char.
        let text = "ab\u{4e16}\u{754c}cd";
        let hits = search_text(text, "cd", false);
        assert_eq!(hits.len(), 1);
        // "ab\u{4e16}\u{754c}" is 4 chars, so "cd" starts at char 4.
        assert_eq!(hits[0], (4, 2));
    }

    #[test]
    fn search_text_query_longer_than_text() {
        let hits = search_text("hi", "hello world", false);
        assert!(hits.is_empty());
    }

    #[test]
    fn search_text_exact_match() {
        let hits = search_text("abc", "abc", false);
        assert_eq!(hits, vec![(0, 3)]);
    }

    // -- collect_text_nodes function ------------------------------

    #[test]
    fn collect_text_nodes_basic() {
        let nodes = vec![(0, "hello world".to_string(), 10.0, 20.0, 110.0, 12.0)];
        let matches = collect_text_nodes(&nodes, "world", false);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].node, 0);
        assert_eq!(matches[0].text_offset, 6);
        assert_eq!(matches[0].match_len, 5);
        // 110 / 11 chars = 10.0 per char, offset 6 => x = 10 + 60 = 70
        assert!((matches[0].rect.x - 70.0).abs() < 0.01);
        assert!((matches[0].rect.y - 20.0).abs() < 0.01);
        assert!((matches[0].rect.width - 50.0).abs() < 0.01);
        assert!((matches[0].rect.height - 12.0).abs() < 0.01);
    }

    #[test]
    fn collect_text_nodes_across_multiple_nodes() {
        let nodes = vec![
            (0, "hello".to_string(), 0.0, 0.0, 50.0, 10.0),
            (1, "hello again".to_string(), 0.0, 10.0, 110.0, 10.0),
        ];
        let matches = collect_text_nodes(&nodes, "hello", false);
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].node, 0);
        assert_eq!(matches[1].node, 1);
    }

    #[test]
    fn collect_text_nodes_no_matches() {
        let nodes = vec![(0, "hello".to_string(), 0.0, 0.0, 50.0, 10.0)];
        let matches = collect_text_nodes(&nodes, "xyz", false);
        assert!(matches.is_empty());
    }

    #[test]
    fn collect_text_nodes_empty_text() {
        let nodes = vec![(0, "".to_string(), 0.0, 0.0, 0.0, 10.0)];
        let matches = collect_text_nodes(&nodes, "x", false);
        assert!(matches.is_empty());
    }

    #[test]
    fn collect_text_nodes_empty_list() {
        let matches = collect_text_nodes(&[], "test", false);
        assert!(matches.is_empty());
    }

    // -- format_status function -----------------------------------

    #[test]
    fn format_status_inactive() {
        let state = SearchState::new();
        assert_eq!(format_status(&state), "");
    }

    #[test]
    fn format_status_no_matches() {
        let mut state = SearchState::new();
        state.open("xyz");
        assert_eq!(format_status(&state), "No matches");
    }

    #[test]
    fn format_status_with_matches() {
        let mut state = SearchState::new();
        state.open("x");
        let m = |i: usize| SearchMatch {
            node: i,
            text_offset: 0,
            match_len: 1,
            rect: SearchRect {
                x: 0.0,
                y: 0.0,
                width: 5.0,
                height: 10.0,
            },
        };
        state.set_matches(vec![m(0), m(1), m(2), m(3), m(4)]);
        assert_eq!(format_status(&state), "1/5 matches");

        state.next_match();
        assert_eq!(format_status(&state), "2/5 matches");

        state.next_match();
        state.next_match();
        state.next_match();
        assert_eq!(format_status(&state), "5/5 matches");

        state.next_match();
        assert_eq!(format_status(&state), "1/5 matches");
    }

    #[test]
    fn format_status_empty_query_active() {
        let mut state = SearchState::new();
        state.open("");
        assert_eq!(format_status(&state), "");
    }

    #[test]
    fn format_status_single_match() {
        let mut state = SearchState::new();
        state.open("x");
        state.set_matches(vec![SearchMatch {
            node: 0,
            text_offset: 0,
            match_len: 1,
            rect: SearchRect {
                x: 0.0,
                y: 0.0,
                width: 5.0,
                height: 10.0,
            },
        }]);
        assert_eq!(format_status(&state), "1/1 matches");
    }
}
