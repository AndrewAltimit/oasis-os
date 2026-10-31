//! Input handling methods for [`BrowserWidget`].

use oasis_types::input::{Button, InputEvent, Key, Modifiers, Trigger};
use oasis_vfs::Vfs;
use rustc_hash::FxHashMap;

use crate::css;
use crate::css::transition::TransitionEngine;
use crate::css::values::ComputedStyle;
use crate::css::values::types::Transition;
use crate::html;
use crate::html::dom::NodeId;
use crate::layout::box_model::LayoutBox;
use crate::loader::Url;
use crate::{BrowserWidget, Focus};

/// Compare two computed styles for geometry-affecting properties only.
///
/// Returns `true` if only visual properties (color, background-color,
/// opacity, text-decoration, outline, box-shadow, text-shadow) differ.
/// When this returns `true`, a relayout can be skipped -- only a
/// repaint is needed.
pub(crate) fn styles_geometry_equal(a: &ComputedStyle, b: &ComputedStyle) -> bool {
    // Display / positioning
    a.display == b.display
        && a.position == b.position
        && a.float == b.float
        // Box model
        && (a.margin_top - b.margin_top).abs() < f32::EPSILON
        && (a.margin_right - b.margin_right).abs() < f32::EPSILON
        && (a.margin_bottom - b.margin_bottom).abs() < f32::EPSILON
        && (a.margin_left - b.margin_left).abs() < f32::EPSILON
        && (a.padding_top - b.padding_top).abs() < f32::EPSILON
        && (a.padding_right - b.padding_right).abs() < f32::EPSILON
        && (a.padding_bottom - b.padding_bottom).abs() < f32::EPSILON
        && (a.padding_left - b.padding_left).abs() < f32::EPSILON
        && (a.border_top_width - b.border_top_width).abs() < f32::EPSILON
        && (a.border_right_width - b.border_right_width).abs() < f32::EPSILON
        && (a.border_bottom_width - b.border_bottom_width).abs() < f32::EPSILON
        && (a.border_left_width - b.border_left_width).abs() < f32::EPSILON
        // Dimensions
        && a.width == b.width
        && a.height == b.height
        && a.max_width == b.max_width
        && a.min_width == b.min_width
        && a.max_height == b.max_height
        && a.min_height == b.min_height
        // Font (affects text measurement, thus geometry)
        && (a.font_size - b.font_size).abs() < f32::EPSILON
        && a.font_weight == b.font_weight
        && a.font_style == b.font_style
        && a.font_family == b.font_family
        && (a.line_height - b.line_height).abs() < f32::EPSILON
        && (a.letter_spacing - b.letter_spacing).abs() < f32::EPSILON
        && (a.word_spacing - b.word_spacing).abs() < f32::EPSILON
        && a.white_space == b.white_space
        && a.word_break == b.word_break
        && a.overflow_wrap == b.overflow_wrap
        // Flex
        && a.flex_direction == b.flex_direction
        && a.flex_wrap == b.flex_wrap
        && a.justify_content == b.justify_content
        && a.align_items == b.align_items
        && a.align_content == b.align_content
        && a.align_self == b.align_self
        && a.order == b.order
        && (a.flex_grow - b.flex_grow).abs() < f32::EPSILON
        && (a.flex_shrink - b.flex_shrink).abs() < f32::EPSILON
        && a.flex_basis == b.flex_basis
        && (a.gap - b.gap).abs() < f32::EPSILON
        // Grid
        && a.grid_template_columns == b.grid_template_columns
        && a.grid_template_rows == b.grid_template_rows
        && a.grid_column_start == b.grid_column_start
        && a.grid_column_end == b.grid_column_end
        && a.grid_row_start == b.grid_row_start
        && a.grid_row_end == b.grid_row_end
        && (a.column_gap - b.column_gap).abs() < f32::EPSILON
        && (a.row_gap - b.row_gap).abs() < f32::EPSILON
        // Margin auto flags
        && a.margin_left_auto == b.margin_left_auto
        && a.margin_right_auto == b.margin_right_auto
        && a.margin_top_auto == b.margin_top_auto
        && a.margin_bottom_auto == b.margin_bottom_auto
        // Box sizing
        && a.box_sizing == b.box_sizing
        // Visibility (can affect layout in some cases)
        && a.visibility == b.visibility
        // Text indent, text-align, overflow
        && (a.text_indent - b.text_indent).abs() < f32::EPSILON
        && a.text_align == b.text_align
        && a.overflow == b.overflow
        // Clear
        && a.clear == b.clear
        // Percentage padding/margin
        && a.padding_top_pct == b.padding_top_pct
        && a.padding_right_pct == b.padding_right_pct
        && a.padding_bottom_pct == b.padding_bottom_pct
        && a.padding_left_pct == b.padding_left_pct
        && a.margin_top_pct == b.margin_top_pct
        && a.margin_right_pct == b.margin_right_pct
        && a.margin_bottom_pct == b.margin_bottom_pct
        && a.margin_left_pct == b.margin_left_pct
        // Positioning offsets
        && a.top == b.top
        && a.right == b.right
        && a.bottom == b.bottom
        && a.left == b.left
        // Vertical alignment (affects inline/table-cell layout)
        && a.vertical_align == b.vertical_align
        // Table layout
        && a.border_collapse == b.border_collapse
        && (a.border_spacing - b.border_spacing).abs() < f32::EPSILON
        && a.table_layout_fixed == b.table_layout_fixed
        // List markers (affect layout)
        && a.list_style_type == b.list_style_type
        && a.list_style_position == b.list_style_position
        // Text transform (can change measured text width)
        && a.text_transform == b.text_transform
        // Generated content (affects layout when present)
        && a.content == b.content
        && a.before_content == b.before_content
        && a.after_content == b.after_content
        // Multi-column
        && a.column_count == b.column_count
        && (a.column_width - b.column_width).abs() < f32::EPSILON
        // Tab size (affects preformatted text width)
        && a.tab_size == b.tab_size
        // Replaced element sizing
        && a.object_fit == b.object_fit
        // Grid extensions
        && a.grid_auto_flow_column == b.grid_auto_flow_column
        && a.grid_auto_flow_dense == b.grid_auto_flow_dense
        && a.grid_template_areas == b.grid_template_areas
        && a.grid_area == b.grid_area
        && a.grid_auto_rows == b.grid_auto_rows
        && a.grid_auto_columns == b.grid_auto_columns
}

impl BrowserWidget {
    // ---------------------------------------------------------------
    // URL-bar selection helpers
    // ---------------------------------------------------------------

    /// Return the `(lo, hi)` byte range of the current URL-bar
    /// selection — `lo` and `hi` are sorted so callers don't need to
    /// care which way the user dragged. Returns `None` if there is no
    /// selection or the selection is collapsed (anchor == cursor).
    pub(crate) fn url_selection_range(&self) -> Option<(usize, usize)> {
        let anchor = self.url_selection_anchor?;
        if anchor == self.url_cursor {
            None
        } else if anchor < self.url_cursor {
            Some((anchor, self.url_cursor))
        } else {
            Some((self.url_cursor, anchor))
        }
    }

    /// Delete the selected range from `url_input` and position the
    /// cursor at the deletion point. Returns `true` if anything was
    /// deleted. Callers use the return value to decide whether a
    /// follow-up action (like Backspace's single-char delete) is still
    /// needed.
    fn url_delete_selection(&mut self) -> bool {
        if let Some((lo, hi)) = self.url_selection_range() {
            self.url_input.replace_range(lo..hi, "");
            self.url_cursor = lo;
            self.url_selection_anchor = None;
            true
        } else {
            self.url_selection_anchor = None;
            false
        }
    }

    // ---------------------------------------------------------------
    // Input handling
    // ---------------------------------------------------------------

    /// Whether keyboard typing currently goes into a text field (the URL
    /// bar or a focused form control). Hosts use this to stop printable
    /// keys from also firing their gamepad-style shortcuts (Space ->
    /// reader mode, Q/E -> page scroll) while the user is typing.
    pub fn accepts_text(&self) -> bool {
        self.focus == Focus::UrlBar || self.form_manager.focused_accepts_text()
    }

    /// Handle a raw keyboard shortcut (hosts deliver [`InputEvent::Key`]
    /// here before its gamepad-style twin). Returns `true` if the key was
    /// a browser shortcut and has been handled; the host must then drop
    /// the twin.
    ///
    /// | Key | Action |
    /// |---|---|
    /// | F5, Ctrl+R | Reload |
    /// | Alt+Left / Alt+Right | Back / forward |
    /// | Ctrl+L, F6 | Focus the URL bar (whole URL selected) |
    /// | Page Up / Page Down | Scroll one page (not while typing) |
    /// | Home / End | Scroll to top / bottom (not while typing) |
    pub fn handle_key(&mut self, key: Key, mods: Modifiers, vfs: &dyn Vfs) -> bool {
        match key {
            Key::F(5) if mods.is_empty() => self.reload(vfs),
            Key::Char('r') if mods.only(Modifiers::CTRL) => self.reload(vfs),
            Key::Left if mods.only(Modifiers::ALT) => {
                self.leave_url_bar();
                self.go_back(vfs);
            },
            Key::Right if mods.only(Modifiers::ALT) => {
                self.leave_url_bar();
                self.go_forward(vfs);
            },
            Key::F(6) if mods.is_empty() => self.focus_url_bar(),
            Key::Char('l') if mods.only(Modifiers::CTRL) => self.focus_url_bar(),
            _ if !mods.is_empty() || self.accepts_text() => return false,
            Key::PageUp => self.scroll.page_up(),
            Key::PageDown => self.scroll.page_down(),
            Key::Home => self.scroll.scroll_to_top(),
            Key::End => self.scroll.scroll_to_bottom(),
            _ => return false,
        }
        true
    }

    /// Put keyboard focus in the URL bar with the whole URL selected, as
    /// a click on it does.
    pub fn focus_url_bar(&mut self) {
        self.focus = Focus::UrlBar;
        self.url_input = self.nav.current_url().unwrap_or("about:blank").to_string();
        self.url_cursor = self.url_input.len();
        self.url_selection_anchor = if self.url_input.is_empty() {
            None
        } else {
            Some(0)
        };
    }

    /// Whether the URL bar has keyboard focus.
    pub fn url_bar_focused(&self) -> bool {
        self.focus == Focus::UrlBar
    }

    /// Leave URL-bar editing without navigating.
    fn leave_url_bar(&mut self) {
        self.focus = Focus::Content;
        self.url_selection_anchor = None;
    }

    /// Handle an input event. Returns `true` if the event was
    /// consumed.
    pub fn handle_input(&mut self, event: &InputEvent, vfs: &dyn Vfs) -> bool {
        // URL-bar editing mode intercepts most keys.
        if self.focus == Focus::UrlBar {
            match event {
                InputEvent::TextInput(ch) => {
                    // If there's an active selection, typing replaces it.
                    // This is the standard "click URL bar, type to
                    // replace" flow users expect from Firefox/Chrome.
                    self.url_delete_selection();
                    self.url_input.insert(self.url_cursor, *ch);
                    self.url_cursor += ch.len_utf8();
                    return true;
                },
                InputEvent::Backspace => {
                    if self.url_delete_selection() {
                        // Selection was non-empty — deletion already
                        // happened, nothing more to do.
                    } else if self.url_cursor > 0 {
                        // No selection: delete the character before
                        // the cursor on a character boundary.
                        let prev = self.url_input[..self.url_cursor]
                            .char_indices()
                            .next_back()
                            .map(|(i, _)| i)
                            .unwrap_or(0);
                        self.url_input.remove(prev);
                        self.url_cursor = prev;
                    }
                    return true;
                },
                InputEvent::ButtonPress(Button::Confirm) => {
                    let url = self.url_input.clone();
                    self.focus = Focus::Content;
                    self.url_selection_anchor = None;
                    if !url.is_empty() {
                        self.navigate_to(&url, vfs);
                    }
                    return true;
                },
                InputEvent::ButtonPress(Button::Cancel) => {
                    // Discard edits.
                    self.focus = Focus::Content;
                    self.url_input.clear();
                    self.url_cursor = 0;
                    self.url_selection_anchor = None;
                    return true;
                },
                InputEvent::ButtonPress(Button::Left) => {
                    // If there's a selection, collapse to its left edge
                    // instead of moving further. Matches GTK/Chrome.
                    if let Some((lo, _hi)) = self.url_selection_range() {
                        self.url_cursor = lo;
                        self.url_selection_anchor = None;
                    } else if self.url_cursor > 0 {
                        let prev = self.url_input[..self.url_cursor]
                            .char_indices()
                            .next_back()
                            .map(|(i, _)| i)
                            .unwrap_or(0);
                        self.url_cursor = prev;
                    }
                    return true;
                },
                InputEvent::ButtonPress(Button::Right) => {
                    if let Some((_lo, hi)) = self.url_selection_range() {
                        self.url_cursor = hi;
                        self.url_selection_anchor = None;
                    } else if self.url_cursor < self.url_input.len() {
                        let next = self.url_input[self.url_cursor..]
                            .chars()
                            .next()
                            .map(|c| self.url_cursor + c.len_utf8())
                            .unwrap_or(self.url_input.len());
                        self.url_cursor = next;
                    }
                    return true;
                },
                InputEvent::Tab => {
                    // Tab from URL bar enters content focus at the first
                    // focusable element.
                    self.focus = Focus::Content;
                    self.tab_focus_forward();
                    return true;
                },
                InputEvent::ShiftTab => {
                    self.focus = Focus::Content;
                    self.tab_focus_backward();
                    return true;
                },
                InputEvent::PointerClick { x, y } => {
                    self.handle_click(*x, *y, vfs);
                    return true;
                },
                _ => return false,
            }
        }

        match event {
            InputEvent::ButtonPress(Button::Up) => {
                // A focused <select> takes the arrows to change option.
                if self.form_manager.focused_is_select() {
                    self.dispatch_form_key(crate::forms::FormKey::Up, vfs);
                    return true;
                }
                self.scroll.scroll_up();
                true
            },
            InputEvent::ButtonPress(Button::Down) => {
                if self.form_manager.focused_is_select() {
                    self.dispatch_form_key(crate::forms::FormKey::Down, vfs);
                    return true;
                }
                self.scroll.scroll_down();
                true
            },
            InputEvent::ButtonPress(Button::Left) => {
                self.select_prev_link();
                true
            },
            InputEvent::ButtonPress(Button::Right) => {
                self.select_next_link();
                true
            },
            InputEvent::ButtonPress(Button::Confirm) => {
                // Enter / Confirm on a focused text input submits its
                // form — the dominant interaction path for search
                // boxes. Falls through to link activation when no
                // form element has focus.
                if self.form_manager.focused_element.is_some() {
                    self.dispatch_form_key(crate::forms::FormKey::Enter, vfs);
                    return true;
                }
                self.activate_selected_link(vfs);
                true
            },
            InputEvent::ButtonPress(Button::Cancel) => {
                // Cancel in a focused text field leaves the field (like
                // Escape blurring an input); otherwise it goes back.
                if self.form_manager.focused_accepts_text() {
                    self.form_manager.focused_element = None;
                    self.form_manager.focused_form = None;
                    self.layout_dirty = true;
                    return true;
                }
                self.go_back(vfs);
                true
            },
            InputEvent::ButtonPress(Button::Triangle) => {
                self.toggle_reader_mode();
                true
            },
            InputEvent::ButtonPress(Button::Square) => {
                self.go_home(vfs);
                true
            },
            InputEvent::TriggerPress(Trigger::Left) => {
                self.scroll.page_up();
                true
            },
            InputEvent::TriggerPress(Trigger::Right) => {
                self.scroll.page_down();
                true
            },
            InputEvent::MouseWheel { delta } => {
                // Check if cursor is over a nested scroll container.
                if let Some(nid) = self.find_scroll_container_at_cursor() {
                    let amount = *delta as f32 * crate::scroll::SCROLL_WHEEL as f32;
                    let entry = self.nested_scroll_offsets.entry(nid).or_insert((0.0, 0.0));
                    let prev = entry.1;
                    entry.1 += amount;
                    // Clamp to content bounds (computed during layout).
                    if let Some(layout) = &self.layout_root
                        && let Some(bounds) = Self::find_scroll_bounds(layout, nid)
                    {
                        entry.1 = entry.1.clamp(0.0, bounds);
                    }
                    if (entry.1 - prev).abs() > f32::EPSILON {
                        self.layout_dirty = true;
                    } else {
                        // At scroll limit — bubble to main page scroll.
                        self.scroll.wheel_scroll(*delta);
                    }
                } else {
                    self.scroll.wheel_scroll(*delta);
                }
                true
            },
            InputEvent::CursorMove { x, y } => {
                self.last_cursor = Some((*x, *y));
                self.handle_cursor_move(*x, *y);
                true
            },
            InputEvent::Tab => {
                self.tab_focus_forward();
                true
            },
            InputEvent::ShiftTab => {
                self.tab_focus_backward();
                true
            },
            InputEvent::PointerClick { x, y } => {
                self.handle_click(*x, *y, vfs);
                true
            },
            InputEvent::Backspace => {
                // Delete a char from the focused text input, if any.
                // Without this branch Backspace is silently dropped in
                // Content focus — forms that rely on physical keyboard
                // editing (e.g. the Google search box) appear broken.
                if self.form_manager.focused_accepts_text() {
                    self.dispatch_form_key(crate::forms::FormKey::Backspace, vfs);
                    self.layout_dirty = true;
                    return true;
                }
                false
            },
            // keydown, the edit, then keyup + input (so `input`
            // handlers read the new value), to the focused node or body.
            InputEvent::TextInput(ch) => {
                // When a text input is focused, deliver the character
                // to the form manager instead of treating it as a page
                // shortcut — typing in Google's search box shouldn't
                // trigger zoom because the user pressed `+`.
                let typing = self.form_manager.focused_accepts_text();
                #[cfg(feature = "javascript")]
                let target = self.focused_node.or(self.body_node_id);
                #[cfg(feature = "javascript")]
                if let (Some(engine), Some(nid)) = (&self.js_engine, target) {
                    Self::dispatch_js_key_event(engine, nid, *ch);
                }
                #[cfg(feature = "javascript")]
                self.apply_js_dom_mutations();
                if typing {
                    self.dispatch_form_key(crate::forms::FormKey::Char(*ch), vfs);
                    self.layout_dirty = true;
                }
                #[cfg(feature = "javascript")]
                if let (Some(engine), Some(nid)) = (&self.js_engine, target) {
                    Self::dispatch_js_key_event_typed(engine, nid, *ch, "keyup");
                    if typing {
                        Self::dispatch_js_event(engine, nid, "input");
                    }
                }
                #[cfg(feature = "javascript")]
                self.apply_js_dom_mutations();
                if typing {
                    return true;
                }
                // Zoom: + / - / 0 keys when not in URL bar.
                match ch {
                    '+' | '=' => self.zoom_in(),
                    '-' => self.zoom_out(),
                    '0' => self.reset_zoom(),
                    _ => {},
                }
                true
            },
            _ => false,
        }
    }
}

mod click;
mod focus;
mod forms;
mod history;
mod hover;
mod js_events;
mod scroll;

// Siblings reach each other's free helpers through this.
#[allow(unused_imports)]
use hover::*;
