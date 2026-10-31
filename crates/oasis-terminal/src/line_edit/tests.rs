//! Unit tests for the line editor.

use super::*;

/// Shorthand: build a simple history vector.
fn hist(entries: &[&str]) -> Vec<String> {
    entries.iter().map(|s| (*s).to_string()).collect()
}

// -- Basic insertion --------------------------------------------------

#[test]
fn insert_chars() {
    let mut ed = LineEditor::new();
    ed.apply(EditAction::InsertChar('h'), &[]);
    ed.apply(EditAction::InsertChar('i'), &[]);
    assert_eq!(ed.buffer(), "hi");
    assert_eq!(ed.cursor(), 2);
}

#[test]
fn insert_at_middle() {
    let mut ed = LineEditor::new();
    ed.set_buffer("ac");
    ed.cursor = 1; // between 'a' and 'c'
    ed.apply(EditAction::InsertChar('b'), &[]);
    assert_eq!(ed.buffer(), "abc");
    assert_eq!(ed.cursor(), 2);
}

// -- Cursor movement --------------------------------------------------

#[test]
fn move_to_start_and_end() {
    let mut ed = LineEditor::new();
    ed.set_buffer("hello");
    assert_eq!(ed.cursor(), 5);
    ed.apply(EditAction::MoveToStart, &[]);
    assert_eq!(ed.cursor(), 0);
    ed.apply(EditAction::MoveToEnd, &[]);
    assert_eq!(ed.cursor(), 5);
}

#[test]
fn move_left_right() {
    let mut ed = LineEditor::new();
    ed.set_buffer("abc");
    ed.apply(EditAction::MoveLeft, &[]);
    assert_eq!(ed.cursor(), 2);
    ed.apply(EditAction::MoveRight, &[]);
    assert_eq!(ed.cursor(), 3);
}

#[test]
fn move_left_at_start_is_noop() {
    let mut ed = LineEditor::new();
    ed.set_buffer("x");
    ed.cursor = 0;
    ed.apply(EditAction::MoveLeft, &[]);
    assert_eq!(ed.cursor(), 0);
}

#[test]
fn move_right_at_end_is_noop() {
    let mut ed = LineEditor::new();
    ed.set_buffer("x");
    ed.apply(EditAction::MoveRight, &[]);
    assert_eq!(ed.cursor(), 1);
}

// -- Word movement ----------------------------------------------------

#[test]
fn move_word_left() {
    let mut ed = LineEditor::new();
    ed.set_buffer("hello world");
    // cursor at end
    ed.apply(EditAction::MoveWordLeft, &[]);
    assert_eq!(ed.display_cursor(), 6); // before 'w'
    ed.apply(EditAction::MoveWordLeft, &[]);
    assert_eq!(ed.cursor(), 0);
}

#[test]
fn move_word_right() {
    let mut ed = LineEditor::new();
    ed.set_buffer("hello world");
    ed.cursor = 0;
    ed.apply(EditAction::MoveWordRight, &[]);
    assert_eq!(ed.display_cursor(), 5); // after 'hello'
    ed.apply(EditAction::MoveWordRight, &[]);
    assert_eq!(ed.cursor(), ed.buffer().len());
}

#[test]
fn move_word_with_punctuation() {
    let mut ed = LineEditor::new();
    ed.set_buffer("foo.bar baz");
    // cursor at end
    ed.apply(EditAction::MoveWordLeft, &[]);
    // should be at 'b' of baz (position 8)
    assert_eq!(ed.display_cursor(), 8);
    ed.apply(EditAction::MoveWordLeft, &[]);
    // should be at 'b' of bar (position 4)
    assert_eq!(ed.display_cursor(), 4);
}

// -- Deletion ---------------------------------------------------------

#[test]
fn delete_char_back() {
    let mut ed = LineEditor::new();
    ed.set_buffer("abc");
    ed.apply(EditAction::DeleteCharBack, &[]);
    assert_eq!(ed.buffer(), "ab");
    assert_eq!(ed.cursor(), 2);
}

#[test]
fn delete_char_back_at_start_is_noop() {
    let mut ed = LineEditor::new();
    ed.set_buffer("x");
    ed.cursor = 0;
    ed.apply(EditAction::DeleteCharBack, &[]);
    assert_eq!(ed.buffer(), "x");
}

#[test]
fn delete_char_forward() {
    let mut ed = LineEditor::new();
    ed.set_buffer("abc");
    ed.cursor = 1;
    ed.apply(EditAction::DeleteCharForward, &[]);
    assert_eq!(ed.buffer(), "ac");
    assert_eq!(ed.cursor(), 1);
}

#[test]
fn delete_char_forward_at_end_is_noop() {
    let mut ed = LineEditor::new();
    ed.set_buffer("x");
    ed.apply(EditAction::DeleteCharForward, &[]);
    assert_eq!(ed.buffer(), "x");
}

#[test]
fn delete_word_back() {
    let mut ed = LineEditor::new();
    ed.set_buffer("hello world");
    ed.apply(EditAction::DeleteWordBack, &[]);
    assert_eq!(ed.buffer(), "hello ");
    assert_eq!(ed.cursor(), 6);
    assert_eq!(ed.kill_ring, "world");
}

#[test]
fn delete_word_back_at_start() {
    let mut ed = LineEditor::new();
    ed.set_buffer("test");
    ed.cursor = 0;
    ed.apply(EditAction::DeleteWordBack, &[]);
    assert_eq!(ed.buffer(), "test");
}

// -- Kill / yank ------------------------------------------------------

#[test]
fn kill_to_end() {
    let mut ed = LineEditor::new();
    ed.set_buffer("hello world");
    ed.cursor = 5;
    ed.apply(EditAction::KillToEnd, &[]);
    assert_eq!(ed.buffer(), "hello");
    assert_eq!(ed.kill_ring, " world");
}

#[test]
fn kill_to_start() {
    let mut ed = LineEditor::new();
    ed.set_buffer("hello world");
    ed.cursor = 5;
    ed.apply(EditAction::KillToStart, &[]);
    assert_eq!(ed.buffer(), " world");
    assert_eq!(ed.cursor(), 0);
    assert_eq!(ed.kill_ring, "hello");
}

#[test]
fn kill_to_end_at_end_is_noop() {
    let mut ed = LineEditor::new();
    ed.set_buffer("test");
    ed.apply(EditAction::KillToEnd, &[]);
    assert_eq!(ed.buffer(), "test");
}

#[test]
fn yank_pastes_kill_ring() {
    let mut ed = LineEditor::new();
    ed.set_buffer("hello world");
    ed.cursor = 5;
    ed.apply(EditAction::KillToEnd, &[]);
    assert_eq!(ed.buffer(), "hello");

    // Move to end and yank.
    ed.apply(EditAction::MoveToEnd, &[]);
    ed.yank();
    assert_eq!(ed.buffer(), "hello world");
}

#[test]
fn yank_empty_kill_ring_is_noop() {
    let mut ed = LineEditor::new();
    ed.set_buffer("abc");
    ed.yank();
    assert_eq!(ed.buffer(), "abc");
}

// -- SwapChars --------------------------------------------------------

#[test]
fn swap_chars_at_end() {
    let mut ed = LineEditor::new();
    ed.set_buffer("ab");
    // cursor at end
    ed.apply(EditAction::SwapChars, &[]);
    assert_eq!(ed.buffer(), "ba");
    assert_eq!(ed.cursor(), 2);
}

#[test]
fn swap_chars_in_middle() {
    let mut ed = LineEditor::new();
    ed.set_buffer("abc");
    ed.cursor = 2; // between 'b' and 'c'
    ed.apply(EditAction::SwapChars, &[]);
    assert_eq!(ed.buffer(), "acb");
}

#[test]
fn swap_chars_too_short() {
    let mut ed = LineEditor::new();
    ed.set_buffer("a");
    ed.apply(EditAction::SwapChars, &[]);
    assert_eq!(ed.buffer(), "a");
}

#[test]
fn swap_chars_cursor_at_zero() {
    let mut ed = LineEditor::new();
    ed.set_buffer("ab");
    ed.cursor = 0;
    ed.apply(EditAction::SwapChars, &[]);
    assert_eq!(ed.buffer(), "ab"); // no-op
}

// -- History ----------------------------------------------------------

#[test]
fn history_prev_next() {
    let mut ed = LineEditor::new();
    let h = hist(&["first", "second", "third"]);

    ed.set_buffer("current");
    ed.apply(EditAction::HistoryPrev, &h);
    assert_eq!(ed.buffer(), "third");
    ed.apply(EditAction::HistoryPrev, &h);
    assert_eq!(ed.buffer(), "second");
    ed.apply(EditAction::HistoryNext, &h);
    assert_eq!(ed.buffer(), "third");
    ed.apply(EditAction::HistoryNext, &h);
    assert_eq!(ed.buffer(), "current");
}

#[test]
fn history_prev_empty() {
    let mut ed = LineEditor::new();
    ed.set_buffer("x");
    let result = ed.apply(EditAction::HistoryPrev, &[]);
    assert_eq!(result, EditResult::Continue);
    assert_eq!(ed.buffer(), "x");
}

#[test]
fn history_next_without_prev() {
    let mut ed = LineEditor::new();
    ed.set_buffer("x");
    ed.apply(EditAction::HistoryNext, &hist(&["a"]));
    assert_eq!(ed.buffer(), "x"); // no-op
}

#[test]
fn history_prev_at_oldest_is_noop() {
    let mut ed = LineEditor::new();
    let h = hist(&["only"]);
    ed.apply(EditAction::HistoryPrev, &h);
    assert_eq!(ed.buffer(), "only");
    ed.apply(EditAction::HistoryPrev, &h);
    assert_eq!(ed.buffer(), "only"); // still at oldest
}

#[test]
fn history_saves_current_line() {
    let mut ed = LineEditor::new();
    let h = hist(&["old"]);
    ed.set_buffer("typed");
    ed.apply(EditAction::HistoryPrev, &h);
    assert_eq!(ed.buffer(), "old");
    ed.apply(EditAction::HistoryNext, &h);
    assert_eq!(ed.buffer(), "typed");
}

// -- Reverse search ---------------------------------------------------

#[test]
fn search_basic() {
    let mut ed = LineEditor::new();
    let h = hist(&["ls -la", "echo hello", "ls /tmp"]);

    ed.apply(EditAction::StartSearch, &h);
    assert!(ed.is_searching());

    ed.apply(EditAction::SearchChar('l'), &h);
    ed.apply(EditAction::SearchChar('s'), &h);
    // Should match "ls /tmp" (most recent containing "ls")
    assert_eq!(ed.buffer(), "ls /tmp");

    ed.apply(EditAction::AcceptSearch, &h);
    assert!(!ed.is_searching());
    assert_eq!(ed.buffer(), "ls /tmp");
}

#[test]
fn search_next_match() {
    let mut ed = LineEditor::new();
    let h = hist(&["ls -la", "echo hello", "ls /tmp"]);

    ed.apply(EditAction::StartSearch, &h);
    ed.apply(EditAction::SearchChar('l'), &h);
    ed.apply(EditAction::SearchChar('s'), &h);
    assert_eq!(ed.buffer(), "ls /tmp");

    // Ctrl+R again -> next older match
    ed.apply(EditAction::SearchNext, &h);
    assert_eq!(ed.buffer(), "ls -la");
}

#[test]
fn search_cancel_restores() {
    let mut ed = LineEditor::new();
    let h = hist(&["old command"]);

    ed.set_buffer("my input");
    ed.apply(EditAction::StartSearch, &h);
    ed.apply(EditAction::SearchChar('o'), &h);
    assert_eq!(ed.buffer(), "old command");

    ed.apply(EditAction::CancelSearch, &h);
    assert!(!ed.is_searching());
    assert_eq!(ed.buffer(), "my input");
}

#[test]
fn search_display_returns_query_and_match() {
    let mut ed = LineEditor::new();
    let h = hist(&["cargo test"]);

    ed.apply(EditAction::StartSearch, &h);
    ed.apply(EditAction::SearchChar('c'), &h);

    let (query, matched) = ed.search_display().expect("in search");
    assert_eq!(query, "c");
    assert_eq!(matched, "cargo test");
}

#[test]
fn search_display_none_when_not_searching() {
    let ed = LineEditor::new();
    assert!(ed.search_display().is_none());
}

// -- Unicode ----------------------------------------------------------

#[test]
fn unicode_insertion_and_movement() {
    let mut ed = LineEditor::new();
    ed.apply(EditAction::InsertChar('a'), &[]);
    ed.apply(EditAction::InsertChar('\u{00e9}'), &[]); // e-acute
    ed.apply(EditAction::InsertChar('b'), &[]);
    assert_eq!(ed.buffer(), "a\u{00e9}b");
    assert_eq!(ed.display_cursor(), 3);

    ed.apply(EditAction::MoveLeft, &[]);
    assert_eq!(ed.display_cursor(), 2);
    ed.apply(EditAction::MoveLeft, &[]);
    assert_eq!(ed.display_cursor(), 1);
}

#[test]
fn unicode_delete_back() {
    let mut ed = LineEditor::new();
    // Japanese character (3 bytes in UTF-8)
    ed.set_buffer("a\u{3042}b"); // a + hiragana-a + b
    ed.cursor = 4; // after the hiragana
    ed.apply(EditAction::DeleteCharBack, &[]);
    assert_eq!(ed.buffer(), "ab");
}

#[test]
fn unicode_delete_forward() {
    let mut ed = LineEditor::new();
    ed.set_buffer("\u{1f600}end"); // emoji (4 bytes) + "end"
    ed.cursor = 0;
    ed.apply(EditAction::DeleteCharForward, &[]);
    assert_eq!(ed.buffer(), "end");
}

// -- AcceptLine / Cancel ----------------------------------------------

#[test]
fn accept_line_returns_content() {
    let mut ed = LineEditor::new();
    ed.set_buffer("echo hi");
    let result = ed.apply(EditAction::AcceptLine, &[]);
    assert_eq!(result, EditResult::Accept("echo hi".into()));
}

#[test]
fn cancel_returns_cancel() {
    let mut ed = LineEditor::new();
    ed.set_buffer("partial");
    let result = ed.apply(EditAction::Cancel, &[]);
    assert_eq!(result, EditResult::Cancel);
}

#[test]
fn clear_screen_returns_clear() {
    let mut ed = LineEditor::new();
    let result = ed.apply(EditAction::ClearScreen, &[]);
    assert_eq!(result, EditResult::ClearScreen);
}

// -- Empty buffer edge cases ------------------------------------------

#[test]
fn empty_buffer_operations() {
    let mut ed = LineEditor::new();
    // None of these should panic.
    ed.apply(EditAction::DeleteCharBack, &[]);
    ed.apply(EditAction::DeleteCharForward, &[]);
    ed.apply(EditAction::DeleteWordBack, &[]);
    ed.apply(EditAction::KillToEnd, &[]);
    ed.apply(EditAction::KillToStart, &[]);
    ed.apply(EditAction::MoveLeft, &[]);
    ed.apply(EditAction::MoveRight, &[]);
    ed.apply(EditAction::MoveWordLeft, &[]);
    ed.apply(EditAction::MoveWordRight, &[]);
    ed.apply(EditAction::SwapChars, &[]);
    assert_eq!(ed.buffer(), "");
}

// -- set_buffer / clear -----------------------------------------------

#[test]
fn set_buffer_places_cursor_at_end() {
    let mut ed = LineEditor::new();
    ed.set_buffer("hello");
    assert_eq!(ed.cursor(), 5);
    assert_eq!(ed.buffer(), "hello");
}

#[test]
fn clear_resets_everything() {
    let mut ed = LineEditor::new();
    ed.set_buffer("data");
    ed.history_index = Some(2);
    ed.search_mode = true;
    ed.clear();
    assert_eq!(ed.buffer(), "");
    assert_eq!(ed.cursor(), 0);
    assert!(!ed.is_searching());
    assert!(ed.history_index.is_none());
}

// -- Completion -------------------------------------------------------

#[test]
fn first_tab_returns_empty_candidates() {
    let mut ed = LineEditor::new();
    ed.set_buffer("ec");
    let result = ed.apply(EditAction::Complete, &[]);
    assert_eq!(result, EditResult::Complete(vec![]));
}

#[test]
fn set_completions_single_fills() {
    let mut ed = LineEditor::new();
    ed.set_buffer("ec");
    ed.apply(EditAction::Complete, &[]);
    ed.set_completions(vec!["echo".into()]);
    assert_eq!(ed.buffer(), "echo ");
}

#[test]
fn set_completions_multiple_fills_prefix() {
    let mut ed = LineEditor::new();
    ed.set_buffer("h");
    ed.apply(EditAction::Complete, &[]);
    ed.set_completions(vec!["help".into(), "history".into()]);
    // Common prefix is "h" which equals the base -- no change
    // but candidates are stored for cycling.
    assert!(!ed.completion_candidates.is_empty());
}

#[test]
fn tab_cycling() {
    let mut ed = LineEditor::new();
    ed.set_buffer("h");
    ed.apply(EditAction::Complete, &[]);
    ed.set_completions(vec!["help".into(), "history".into()]);
    // Second tab should cycle to first candidate.
    ed.apply(EditAction::Complete, &[]);
    let buf1 = ed.buffer().trim().to_string();
    // Third tab should cycle to second candidate.
    ed.apply(EditAction::Complete, &[]);
    let buf2 = ed.buffer().trim().to_string();
    assert_ne!(buf1, buf2);
    // Candidates should be help and history.
    assert!((buf1 == "history" && buf2 == "help") || (buf1 == "help" && buf2 == "history"));
}

// -- display_cursor ---------------------------------------------------

#[test]
fn display_cursor_counts_chars_not_bytes() {
    let mut ed = LineEditor::new();
    ed.set_buffer("\u{00e9}\u{00e9}"); // two 2-byte chars
    assert_eq!(ed.cursor(), 4); // 4 bytes
    assert_eq!(ed.display_cursor(), 2); // 2 chars
}

// -- Default trait ----------------------------------------------------

#[test]
fn default_creates_empty() {
    let ed = LineEditor::default();
    assert_eq!(ed.buffer(), "");
    assert_eq!(ed.cursor(), 0);
}
