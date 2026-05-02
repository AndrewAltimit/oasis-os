//! Tests for `widget_input.rs` -- focus traversal, link cycling,
//! URL-bar editing, and the `styles_geometry_equal` invariant that
//! gates the hover-only-restyle optimization.
//!
//! Higher-level click + scroll behavior is exercised by
//! `browser_tests.rs` against a real `MemoryVfs`. This module covers
//! the small, branch-heavy helpers that are easy to break by edits to
//! `ComputedStyle` or by reorganizing the input dispatch table.
#![allow(clippy::unwrap_used)]

use oasis_types::backend::Color;
use oasis_types::input::{Button, InputEvent};
use oasis_vfs::MemoryVfs;

use crate::css::values::ComputedStyle;
use crate::css::values::types::BoxShadow;
use crate::layout::box_model::Rect;
use crate::paint::LinkRegion;
use crate::widget_input::styles_geometry_equal;
use crate::{BrowserConfig, BrowserWidget, Focus};

// -------------------------------------------------------------------
// `styles_geometry_equal` -- gates hover-only restyle (no relayout)
// -------------------------------------------------------------------

#[test]
fn geometry_equal_default_styles_match() {
    let a = ComputedStyle::default();
    let b = ComputedStyle::default();
    assert!(styles_geometry_equal(&a, &b));
}

#[test]
fn geometry_equal_color_change_only_is_equal() {
    // Color is a pure visual property -- changing it must NOT force
    // a relayout. This is the central invariant the hover-restyle
    // optimization depends on.
    let a = ComputedStyle::default();
    let mut b = ComputedStyle::default();
    b.color = Color::rgb(255, 0, 0);
    assert!(styles_geometry_equal(&a, &b));
}

#[test]
fn geometry_equal_background_color_change_is_equal() {
    let a = ComputedStyle::default();
    let mut b = ComputedStyle::default();
    b.background_color = Color::rgb(0, 0, 255);
    assert!(styles_geometry_equal(&a, &b));
}

#[test]
fn geometry_equal_opacity_change_is_equal() {
    let a = ComputedStyle::default();
    let mut b = ComputedStyle::default();
    b.opacity = 0.5;
    assert!(styles_geometry_equal(&a, &b));
}

#[test]
fn geometry_equal_box_shadow_change_is_equal() {
    let a = ComputedStyle::default();
    let mut b = ComputedStyle::default();
    b.box_shadow = vec![BoxShadow {
        offset_x: 2.0,
        offset_y: 2.0,
        blur: 4.0,
        spread: 0.0,
        color: Color::rgba(0, 0, 0, 64),
        inset: false,
    }];
    assert!(styles_geometry_equal(&a, &b));
}

#[test]
fn geometry_equal_outline_change_is_equal() {
    // Outline doesn't take space in the box model -- it's a visual
    // ring drawn outside the border.
    let a = ComputedStyle::default();
    let mut b = ComputedStyle::default();
    b.outline_width = 2.0;
    b.outline_color = Color::rgb(255, 200, 0);
    assert!(styles_geometry_equal(&a, &b));
}

#[test]
fn geometry_equal_margin_change_breaks_equality() {
    // Margins are pure box-model -- ANY change forces relayout.
    let a = ComputedStyle::default();
    let mut b = ComputedStyle::default();
    b.margin_top = 8.0;
    assert!(!styles_geometry_equal(&a, &b));
}

#[test]
fn geometry_equal_padding_change_breaks_equality() {
    let a = ComputedStyle::default();
    let mut b = ComputedStyle::default();
    b.padding_left = 4.0;
    assert!(!styles_geometry_equal(&a, &b));
}

#[test]
fn geometry_equal_border_width_change_breaks_equality() {
    let a = ComputedStyle::default();
    let mut b = ComputedStyle::default();
    b.border_top_width = 1.0;
    assert!(!styles_geometry_equal(&a, &b));
}

#[test]
fn geometry_equal_font_size_change_breaks_equality() {
    // Font size affects measured text width => must relayout.
    let a = ComputedStyle::default();
    let mut b = ComputedStyle::default();
    b.font_size += 2.0;
    assert!(!styles_geometry_equal(&a, &b));
}

#[test]
fn geometry_equal_display_change_breaks_equality() {
    use crate::css::values::Display;
    let a = ComputedStyle::default();
    let mut b = ComputedStyle::default();
    b.display = Display::None;
    assert!(!styles_geometry_equal(&a, &b));
}

#[test]
fn geometry_equal_text_align_change_breaks_equality() {
    // text-align changes the position of inline boxes -- relayout.
    use crate::css::values::TextAlign;
    let a = ComputedStyle::default();
    let mut b = ComputedStyle::default();
    b.text_align = TextAlign::Right;
    assert!(!styles_geometry_equal(&a, &b));
}

// -------------------------------------------------------------------
// Test fixtures
// -------------------------------------------------------------------

fn fresh_browser() -> BrowserWidget {
    let mut config = BrowserConfig::default();
    config.features.home_url = "vfs://test/home/index.html".to_string();
    BrowserWidget::new(config)
}

fn link(node: usize, href: &str, y: f32) -> LinkRegion {
    LinkRegion {
        rect: Rect::new(10.0, y, 80.0, 16.0),
        href: href.to_string(),
        node,
    }
}

// -------------------------------------------------------------------
// `select_next_link` / `select_prev_link` edge cases
// -------------------------------------------------------------------

#[test]
fn select_next_link_no_op_when_empty() {
    // `link_navigation_cycles` already covers the multi-link wrap path
    // in browser_tests.rs. This test pins the empty-map branch so a
    // refactor doesn't drop it back to the bare `selected_link += 1`
    // form (which would bump the index off the end of an empty Vec).
    let mut browser = fresh_browser();
    assert_eq!(browser.selected_link, -1);
    browser.select_next_link();
    assert_eq!(browser.selected_link, -1);
    browser.select_prev_link();
    assert_eq!(browser.selected_link, -1);
}

#[test]
fn select_next_link_single_item_stays_at_zero() {
    let mut browser = fresh_browser();
    browser.link_map = vec![link(1, "/only", 100.0)];
    browser.select_next_link();
    assert_eq!(browser.selected_link, 0);
    // Wrap forward back to 0.
    browser.select_next_link();
    assert_eq!(browser.selected_link, 0);
    browser.select_prev_link();
    assert_eq!(browser.selected_link, 0);
}

#[test]
fn select_next_link_starts_from_negative_one() {
    let mut browser = fresh_browser();
    browser.link_map = vec![
        link(1, "/a", 100.0),
        link(2, "/b", 130.0),
        link(3, "/c", 160.0),
    ];
    // Initial state -- nothing selected.
    assert_eq!(browser.selected_link, -1);
    // Forward from -1 lands on 0, not 1 (no off-by-one).
    browser.select_next_link();
    assert_eq!(browser.selected_link, 0);
}

#[test]
fn select_prev_link_from_negative_one_wraps_to_last() {
    // From the unselected state, "prev" should land on the last link
    // -- this is the standard Shift+Tab behavior for keyboard users.
    let mut browser = fresh_browser();
    browser.link_map = vec![link(1, "/a", 100.0), link(2, "/b", 130.0)];
    assert_eq!(browser.selected_link, -1);
    browser.select_prev_link();
    assert_eq!(browser.selected_link, 1);
}

// -------------------------------------------------------------------
// `handle_input` -- dispatch for content-mode navigation keys
// -------------------------------------------------------------------

#[test]
fn handle_input_left_right_cycles_links_in_content_focus() {
    let mut browser = fresh_browser();
    let vfs = MemoryVfs::new();
    browser.link_map = vec![link(1, "/a", 100.0), link(2, "/b", 130.0)];
    assert_eq!(browser.focus, Focus::Content);

    // Right -> next link.
    let consumed = browser.handle_input(&InputEvent::ButtonPress(Button::Right), &vfs);
    assert!(consumed);
    assert_eq!(browser.selected_link, 0);

    // Right again -> link 1.
    browser.handle_input(&InputEvent::ButtonPress(Button::Right), &vfs);
    assert_eq!(browser.selected_link, 1);

    // Left -> back to link 0.
    browser.handle_input(&InputEvent::ButtonPress(Button::Left), &vfs);
    assert_eq!(browser.selected_link, 0);
}

#[test]
fn handle_input_unhandled_event_returns_false() {
    let mut browser = fresh_browser();
    let vfs = MemoryVfs::new();
    // No focused form input, no handler match for an arbitrary Tab
    // press becomes tab_focus_forward but with no link_map it is a
    // no-op that still returns true (the dispatch consumed it).
    // For a definitively unhandled event use the Backspace branch
    // with no focused form -- that returns false.
    let consumed = browser.handle_input(&InputEvent::Backspace, &vfs);
    assert!(!consumed);
}

#[test]
fn handle_input_tab_cycles_focus_forward() {
    let mut browser = fresh_browser();
    let vfs = MemoryVfs::new();
    browser.link_map = vec![link(1, "/a", 100.0), link(2, "/b", 130.0)];

    let consumed = browser.handle_input(&InputEvent::Tab, &vfs);
    assert!(consumed);
    assert_eq!(browser.selected_link, 0);

    browser.handle_input(&InputEvent::Tab, &vfs);
    assert_eq!(browser.selected_link, 1);

    // Wrap.
    browser.handle_input(&InputEvent::Tab, &vfs);
    assert_eq!(browser.selected_link, 0);
}

#[test]
fn handle_input_shift_tab_cycles_focus_backward() {
    let mut browser = fresh_browser();
    let vfs = MemoryVfs::new();
    browser.link_map = vec![link(1, "/a", 100.0), link(2, "/b", 130.0)];

    // From -1, ShiftTab wraps to the last link.
    browser.handle_input(&InputEvent::ShiftTab, &vfs);
    assert_eq!(browser.selected_link, 1);

    browser.handle_input(&InputEvent::ShiftTab, &vfs);
    assert_eq!(browser.selected_link, 0);
}

// -------------------------------------------------------------------
// URL-bar editing -- selection collapse, cancel discard, etc.
// -------------------------------------------------------------------

fn into_url_bar(browser: &mut BrowserWidget, text: &str) {
    browser.focus = Focus::UrlBar;
    browser.url_input = text.to_string();
    browser.url_cursor = text.len();
    browser.url_selection_anchor = None;
}

#[test]
fn url_bar_typing_replaces_active_selection() {
    let mut browser = fresh_browser();
    let vfs = MemoryVfs::new();
    into_url_bar(&mut browser, "vfs://test/page.html");
    // Select-all: anchor at 0, cursor at end.
    browser.url_selection_anchor = Some(0);
    browser.url_cursor = browser.url_input.len();

    // Typing replaces the entire selection.
    browser.handle_input(&InputEvent::TextInput('x'), &vfs);
    assert_eq!(browser.url_input, "x");
    assert_eq!(browser.url_cursor, 1);
    assert!(browser.url_selection_anchor.is_none());
}

#[test]
fn url_bar_left_with_selection_collapses_to_left_edge() {
    let mut browser = fresh_browser();
    let vfs = MemoryVfs::new();
    into_url_bar(&mut browser, "abcdef");
    // Select "cde": anchor=2, cursor=5.
    browser.url_selection_anchor = Some(2);
    browser.url_cursor = 5;

    browser.handle_input(&InputEvent::ButtonPress(Button::Left), &vfs);
    // Cursor lands on the left edge (anchor < cursor case).
    assert_eq!(browser.url_cursor, 2);
    assert!(browser.url_selection_anchor.is_none());
    // Text is unchanged -- collapse only.
    assert_eq!(browser.url_input, "abcdef");
}

#[test]
fn url_bar_right_with_selection_collapses_to_right_edge() {
    let mut browser = fresh_browser();
    let vfs = MemoryVfs::new();
    into_url_bar(&mut browser, "abcdef");
    browser.url_selection_anchor = Some(2);
    browser.url_cursor = 5;

    browser.handle_input(&InputEvent::ButtonPress(Button::Right), &vfs);
    assert_eq!(browser.url_cursor, 5);
    assert!(browser.url_selection_anchor.is_none());
    assert_eq!(browser.url_input, "abcdef");
}

#[test]
fn url_bar_backspace_with_selection_deletes_range() {
    let mut browser = fresh_browser();
    let vfs = MemoryVfs::new();
    into_url_bar(&mut browser, "abcdef");
    browser.url_selection_anchor = Some(1);
    browser.url_cursor = 4;

    browser.handle_input(&InputEvent::Backspace, &vfs);
    // "bcd" deleted -> "aef", cursor at the deletion site.
    assert_eq!(browser.url_input, "aef");
    assert_eq!(browser.url_cursor, 1);
    assert!(browser.url_selection_anchor.is_none());
}

#[test]
fn url_bar_cancel_clears_input_and_returns_to_content() {
    let mut browser = fresh_browser();
    let vfs = MemoryVfs::new();
    into_url_bar(&mut browser, "https://example.com/foo");

    browser.handle_input(&InputEvent::ButtonPress(Button::Cancel), &vfs);
    assert_eq!(browser.focus, Focus::Content);
    assert!(browser.url_input.is_empty());
    assert_eq!(browser.url_cursor, 0);
    assert!(browser.url_selection_anchor.is_none());
}

#[test]
fn url_bar_left_at_start_does_not_underflow() {
    let mut browser = fresh_browser();
    let vfs = MemoryVfs::new();
    into_url_bar(&mut browser, "x");
    browser.url_cursor = 0;
    browser.url_selection_anchor = None;

    browser.handle_input(&InputEvent::ButtonPress(Button::Left), &vfs);
    // Cursor stays at 0 -- no panic on `cursor - 1` for usize.
    assert_eq!(browser.url_cursor, 0);
}

#[test]
fn url_bar_right_at_end_does_not_overflow() {
    let mut browser = fresh_browser();
    let vfs = MemoryVfs::new();
    into_url_bar(&mut browser, "x");
    browser.url_cursor = 1;
    browser.url_selection_anchor = None;

    browser.handle_input(&InputEvent::ButtonPress(Button::Right), &vfs);
    assert_eq!(browser.url_cursor, 1);
}

#[test]
fn url_bar_handles_multibyte_chars_on_left_arrow() {
    // Cursor must land on a char boundary -- arbitrary `cursor - 1`
    // would slice mid-codepoint and panic.
    let mut browser = fresh_browser();
    let vfs = MemoryVfs::new();
    into_url_bar(&mut browser, "héllo"); // é is 2 bytes
    let end = browser.url_input.len();
    browser.url_cursor = end;

    browser.handle_input(&InputEvent::ButtonPress(Button::Left), &vfs);
    // Moved past 'o' -> cursor at the start of 'o'.
    assert!(browser.url_input.is_char_boundary(browser.url_cursor));
    assert!(browser.url_cursor < end);
}

#[test]
fn url_bar_backspace_on_multibyte_removes_full_codepoint() {
    let mut browser = fresh_browser();
    let vfs = MemoryVfs::new();
    into_url_bar(&mut browser, "héllo");
    // Position cursor right after 'é' (which spans bytes 1..3).
    browser.url_cursor = 3;
    browser.url_selection_anchor = None;

    browser.handle_input(&InputEvent::Backspace, &vfs);
    // 'é' (2 bytes) removed -> "hllo".
    assert_eq!(browser.url_input, "hllo");
    assert_eq!(browser.url_cursor, 1);
}
