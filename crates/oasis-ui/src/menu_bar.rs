//! Windows-95-style top menu bar with drop-down menus.
//!
//! Apps populate a [`MenuBar`] with a list of [`Menu`]s (File / Edit /
//! …) and dispatch mouse input through [`MenuBar::hit_test`]. When an
//! item is clicked the bar returns the `id` string set on that entry,
//! which the host maps to a concrete action.
//!
//! The widget owns open/close state and hover tracking; it renders its
//! bar + any active drop-down via the [`oasis_types::backend::SdiBackend`]
//! trait so it works on every backend (SDL3, WASM, UE5, PSP).
//!
//! Keyboard hosts route raw keys through [`MenuBar::handle_key`] first:
//! F10 opens the first menu (Alt belongs to the window manager), Left /
//! Right switch menus, Up / Down / Home / End pick an item (the highlight
//! is [`MenuBar::hovered_item`]), Enter or Space runs it, Esc or F10
//! closes.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;

use oasis_types::backend::{Color, SdiBackend, bitmap_measure_text};
use oasis_types::error::Result;
use oasis_types::input::Key;

/// An entry inside a drop-down. Either a clickable action with an
/// `id` the host recognises, or a horizontal separator.
#[derive(Debug, Clone)]
pub enum MenuEntry {
    /// Clickable action. `id` is an app-defined string (e.g.
    /// `"file.save"`). `shortcut` is a right-aligned hint like
    /// `"Ctrl+S"` — purely decorative; the host implements the
    /// shortcut itself.
    Action {
        label: String,
        id: String,
        shortcut: Option<String>,
        enabled: bool,
    },
    /// Horizontal separator between groups of actions.
    Separator,
}

impl MenuEntry {
    pub fn action(label: impl Into<String>, id: impl Into<String>) -> Self {
        Self::Action {
            label: label.into(),
            id: id.into(),
            shortcut: None,
            enabled: true,
        }
    }

    pub fn with_shortcut(mut self, shortcut: impl Into<String>) -> Self {
        if let Self::Action { shortcut: s, .. } = &mut self {
            *s = Some(shortcut.into());
        }
        self
    }

    pub fn disabled(mut self) -> Self {
        if let Self::Action { enabled, .. } = &mut self {
            *enabled = false;
        }
        self
    }

    pub fn action_id(&self) -> Option<&str> {
        match self {
            Self::Action {
                id, enabled: true, ..
            } => Some(id.as_str()),
            _ => None,
        }
    }
}

/// A single top-level menu (the "File", "Edit" labels and their
/// drop-down contents).
#[derive(Debug, Clone)]
pub struct Menu {
    pub label: String,
    pub entries: Vec<MenuEntry>,
}

impl Menu {
    pub fn new(label: impl Into<String>, entries: Vec<MenuEntry>) -> Self {
        Self {
            label: label.into(),
            entries,
        }
    }
}

/// Top-level menu bar state + renderer.
///
/// `hovered_item` is exposed (and read by the renderer) so hosts
/// that *do* have mouse-move input can light up the highlighted
/// row before the user clicks. The current desktop input dispatcher
/// only plumbs click + text + wheel to app windows — no
/// pointer-move — so most hosts will leave `hovered_item = None`
/// and the drop-down simply renders without a hover row. The
/// [`MenuBar::pointer_move`] helper is provided for any future
/// dispatcher that wants to wire it in.
///
/// Label and drop-down widths come from the backend's `measure_text`,
/// recorded by the last draw so hit-testing matches what is on screen
/// (before the first draw the bitmap-font metrics stand in). On a bar
/// too narrow for every label the padding shrinks, then the trailing
/// menus fold into a `»` overflow menu (index `menus.len()`, reported as
/// [`MenuHit::Label`] like any other) that lists them under disabled
/// headings. Drop-downs stay inside the bar's width, dropping shortcut
/// hints and ellipsizing labels that do not fit.
#[derive(Debug, Clone)]
pub struct MenuBar {
    pub menus: Vec<Menu>,
    pub open: Option<usize>,
    pub hovered_item: Option<usize>,
    /// Text widths measured by the last draw.
    metrics: RefCell<TextMetrics>,
}

/// Text widths measured with the backend that last drew the bar.
#[derive(Debug, Clone, Default)]
struct TextMetrics {
    font_size: u16,
    widths: HashMap<String, u32>,
}

/// The result of a hit-test on the menu bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuHit {
    /// A top-level label was clicked (toggle its drop-down).
    /// `menus.len()` is the overflow (`»`) menu.
    Label(usize),
    /// A drop-down entry was clicked. `id` is the caller's action key.
    Item { id: String },
    /// The click landed on the open drop-down but on a separator or
    /// disabled entry — the drop-down should stay open.
    NoOp,
    /// The click was outside the bar and any open drop-down — the
    /// host should close the drop-down.
    Outside,
}

/// What [`MenuBar::handle_key`] did with a key press.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuKey {
    /// Not for the menus (they are closed and the key is not F10): handle
    /// it as usual.
    Ignored,
    /// The key opened, navigated or closed the menus: the app consumes it.
    Handled,
    /// Enter / Space on an item: the menus closed; run action `id`.
    Activate(String),
}

/// Visual parameters — Windows-95 bezel defaults.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MenuStyle {
    pub bar_bg: Color,
    pub bar_border: Color,
    pub label_text: Color,
    pub label_hot_bg: Color,
    pub label_hot_text: Color,
    pub dropdown_bg: Color,
    pub dropdown_border_light: Color,
    pub dropdown_border_dark: Color,
    pub item_text: Color,
    pub item_hot_bg: Color,
    pub item_hot_text: Color,
    pub item_disabled_text: Color,
    pub separator: Color,
    pub font_size: u16,
}

impl Default for MenuStyle {
    fn default() -> Self {
        Self {
            bar_bg: Color::rgb(240, 240, 240),
            bar_border: Color::rgb(180, 180, 180),
            label_text: Color::rgb(30, 30, 30),
            label_hot_bg: Color::rgb(49, 106, 197),
            label_hot_text: Color::rgb(255, 255, 255),
            dropdown_bg: Color::rgb(236, 236, 236),
            dropdown_border_light: Color::rgb(255, 255, 255),
            dropdown_border_dark: Color::rgb(105, 105, 105),
            item_text: Color::rgb(20, 20, 20),
            item_hot_bg: Color::rgb(49, 106, 197),
            item_hot_text: Color::rgb(255, 255, 255),
            item_disabled_text: Color::rgb(150, 150, 150),
            separator: Color::rgb(170, 170, 170),
            font_size: DEFAULT_FONT_SIZE,
        }
    }
}

impl MenuStyle {
    /// Build a style from the theme's `menu_*` slots.
    ///
    /// Every built-in [`Theme`](crate::theme::Theme) defaults those
    /// slots to the same Win95 grays as [`MenuStyle::default`], so
    /// switching a call site from `default()` to `from_theme` is
    /// pixel-identical until a skin overrides `[widget_states.menu]`.
    pub fn from_theme(theme: &crate::theme::Theme) -> Self {
        Self {
            bar_bg: theme.menu_bg,
            bar_border: theme.menu_border,
            label_text: theme.menu_text,
            label_hot_bg: theme.menu_hover_bg,
            label_hot_text: theme.menu_hover_text,
            dropdown_bg: theme.menu_dropdown_bg,
            dropdown_border_light: theme.menu_dropdown_border_light,
            dropdown_border_dark: theme.menu_dropdown_border_dark,
            item_text: theme.menu_item_text,
            item_hot_bg: theme.menu_hover_bg,
            item_hot_text: theme.menu_hover_text,
            item_disabled_text: theme.menu_disabled_text,
            separator: theme.menu_separator,
            font_size: DEFAULT_FONT_SIZE,
        }
    }
}

/// Layout parameters (pixel sizes).
const DEFAULT_FONT_SIZE: u16 = 11;
const BAR_PAD_X: i32 = 6;
const LABEL_PAD_X: i32 = 8;
/// Narrowest label padding / bar inset before menus fold into `»`.
const MIN_LABEL_PAD_X: i32 = 3;
const MIN_BAR_PAD_X: i32 = 2;
const ITEM_H: i32 = 20;
const ITEM_PAD_X: i32 = 22;
/// Item padding of a drop-down squeezed below its natural width.
const MIN_ITEM_PAD_X: i32 = 6;
/// Gap between an item's label and its shortcut hint.
const SHORTCUT_GAP: i32 = 16;
const DROPDOWN_MIN_W: u32 = 120;
const SEPARATOR_H: i32 = 6;
/// Label of the overflow menu holding the menus that do not fit.
const OVERFLOW_LABEL: &str = "\u{bb}";
/// Truncation marker `SdiText::draw_text_ellipsis` appends.
const ELLIPSIS: &str = "...";
/// Measured strings kept per bar (menus are small; this only bounds
/// hosts that keep rebuilding entries with new labels).
const METRICS_CAP: usize = 512;

/// `(x, y, w, h)` screen rect.
pub type Rect = (i32, i32, u32, u32);

/// Whether `(x, y)` lies inside `rect` (right / bottom edges exclusive).
fn contains((rx, ry, rw, rh): Rect, x: i32, y: i32) -> bool {
    x >= rx
        && y >= ry
        && i64::from(x) < i64::from(rx) + i64::from(rw)
        && i64::from(y) < i64::from(ry) + i64::from(rh)
}

/// Where each label sits on a bar of a given width.
#[derive(Debug, Clone, PartialEq)]
struct BarLayout {
    /// `(x, w)` (bar-local) of each shown menu: `menus[..slots.len()]`.
    slots: Vec<(i32, i32)>,
    /// `(x, w)` of the `»` label when trailing menus are folded.
    overflow: Option<(i32, i32)>,
    /// Horizontal text padding inside each label.
    pad: i32,
}

impl BarLayout {
    /// `(x, w)` of label `index` (`menus.len()` = overflow); a folded
    /// menu answers with the overflow label it lives under.
    fn slot(&self, index: usize) -> Option<(i32, i32)> {
        self.slots.get(index).copied().or(self.overflow)
    }

    /// Label index under bar-local `x` (`menu_count` = overflow).
    fn index_at(&self, x: i32, menu_count: usize) -> Option<usize> {
        let hit = |&(sx, sw): &(i32, i32)| x >= sx && x < sx + sw;
        self.slots
            .iter()
            .position(hit)
            .or_else(|| self.overflow.filter(hit).map(|_| menu_count))
    }
}

impl MenuBar {
    pub fn new(menus: Vec<Menu>) -> Self {
        Self {
            menus,
            open: None,
            hovered_item: None,
            metrics: RefCell::new(TextMetrics::default()),
        }
    }

    pub fn close(&mut self) {
        self.open = None;
        self.hovered_item = None;
    }

    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Measure `texts` with `backend` at `font_size` for the layout of
    /// the following hit-tests and draws.
    fn remember<'t>(
        &self,
        backend: &dyn SdiBackend,
        font_size: u16,
        texts: impl IntoIterator<Item = &'t str>,
    ) {
        let mut m = self.metrics.borrow_mut();
        if m.font_size != font_size || m.widths.len() > METRICS_CAP {
            m.widths.clear();
            m.font_size = font_size;
        }
        for t in texts {
            if !m.widths.contains_key(t) {
                let w = backend.measure_text(t, font_size);
                m.widths.insert(t.to_string(), w);
            }
        }
    }

    /// Width of `text` as the last draw measured it (bitmap-font metrics
    /// for text not drawn yet).
    fn text_w(&self, text: &str) -> i32 {
        let m = self.metrics.borrow();
        let w = m.widths.get(text).copied().unwrap_or_else(|| {
            let size = if m.font_size == 0 {
                DEFAULT_FONT_SIZE
            } else {
                m.font_size
            };
            bitmap_measure_text(text, size)
        });
        w as i32
    }

    /// Lay the labels out on a bar `bar_w` wide.
    fn bar_layout(&self, bar_w: u32) -> BarLayout {
        let bar_w = bar_w as i32;
        let n = self.menus.len() as i32;
        let text: Vec<i32> = self.menus.iter().map(|m| self.text_w(&m.label)).collect();
        let text_sum: i32 = text.iter().sum();
        let place = |inset: i32, pad: i32, count: usize| {
            let mut x = inset;
            text[..count]
                .iter()
                .map(|tw| {
                    let slot = (x, tw + 2 * pad);
                    x += slot.1;
                    slot
                })
                .collect::<Vec<_>>()
        };
        if n == 0 || BAR_PAD_X + text_sum + 2 * LABEL_PAD_X * n <= bar_w {
            return BarLayout {
                slots: place(BAR_PAD_X, LABEL_PAD_X, self.menus.len()),
                overflow: None,
                pad: LABEL_PAD_X,
            };
        }
        // Squeeze the padding before folding any menu away.
        let pad = ((bar_w - MIN_BAR_PAD_X - text_sum) / (2 * n)).min(LABEL_PAD_X);
        if pad >= MIN_LABEL_PAD_X {
            return BarLayout {
                slots: place(MIN_BAR_PAD_X, pad, self.menus.len()),
                overflow: None,
                pad,
            };
        }
        let pad = MIN_LABEL_PAD_X;
        let overflow_w = self.text_w(OVERFLOW_LABEL) + 2 * pad;
        let mut x = MIN_BAR_PAD_X;
        let mut shown = 0;
        for tw in &text {
            if x + tw + 2 * pad + overflow_w > bar_w {
                break;
            }
            x += tw + 2 * pad;
            shown += 1;
        }
        BarLayout {
            slots: place(MIN_BAR_PAD_X, pad, shown),
            overflow: Some((x, overflow_w)),
            pad,
        }
    }

    /// The menu behind label `index` on a bar laid out as `layout`: a
    /// real menu, or (for `menus.len()`) the overflow menu listing the
    /// folded ones, each under a disabled heading.
    fn menu_at(&self, index: usize, layout: &BarLayout) -> Option<Cow<'_, Menu>> {
        if let Some(menu) = self.menus.get(index) {
            return Some(Cow::Borrowed(menu));
        }
        if index != self.menus.len() || layout.overflow.is_none() {
            return None;
        }
        let mut entries = Vec::new();
        for menu in &self.menus[layout.slots.len()..] {
            if !entries.is_empty() {
                entries.push(MenuEntry::Separator);
            }
            entries.push(MenuEntry::action(menu.label.clone(), "").disabled());
            entries.extend(menu.entries.iter().cloned());
        }
        Some(Cow::Owned(Menu::new(OVERFLOW_LABEL, entries)))
    }

    /// Label indices of a bar `bar_w` wide in on-screen order: the shown
    /// menus, then the `»` overflow menu (`menus.len()`) if any.
    fn label_order(&self, bar_w: u32) -> (Vec<usize>, BarLayout) {
        let layout = self.bar_layout(bar_w);
        let mut order: Vec<usize> = (0..layout.slots.len()).collect();
        if layout.overflow.is_some() {
            order.push(self.menus.len());
        }
        (order, layout)
    }

    /// Open label `index` from the keyboard, highlighting its first
    /// enabled item.
    fn open_from_keyboard(&mut self, index: usize, layout: &BarLayout) {
        let first = self
            .menu_at(index, layout)
            .and_then(|menu| menu.entries.iter().position(|e| e.action_id().is_some()));
        self.open = Some(index);
        self.hovered_item = first;
    }

    /// Route a raw key press to the menus of a bar `bar_w` wide (the
    /// width hit-tests and draws use, which decides what is folded into
    /// `»`). See the module docs for the keys. While a menu is open every
    /// key is consumed; one that means nothing to the menus closes them.
    pub fn handle_key(&mut self, key: Key, bar_w: u32) -> MenuKey {
        let (order, layout) = self.label_order(bar_w);
        let Some(open) = self.open else {
            if key == Key::F(10)
                && let Some(&first) = order.first()
            {
                self.open_from_keyboard(first, &layout);
                return MenuKey::Handled;
            }
            return MenuKey::Ignored;
        };
        // Enabled entries of the open menu (`None`: it no longer exists).
        let entries: Option<Vec<Option<String>>> = self.menu_at(open, &layout).map(|menu| {
            menu.entries
                .iter()
                .map(|e| e.action_id().map(str::to_string))
                .collect()
        });
        let Some(entries) = entries else {
            self.close();
            return MenuKey::Handled;
        };
        let enabled: Vec<usize> = (0..entries.len())
            .filter(|&i| entries[i].is_some())
            .collect();
        let at = self
            .hovered_item
            .and_then(|h| enabled.iter().position(|&i| i == h));
        match key {
            Key::Left | Key::Right => {
                let pos = order.iter().position(|&i| i == open).unwrap_or(0);
                let n = order.len().max(1);
                let next = if key == Key::Left {
                    (pos + n - 1) % n
                } else {
                    (pos + 1) % n
                };
                if let Some(&index) = order.get(next) {
                    self.open_from_keyboard(index, &layout);
                }
            },
            Key::Up | Key::Down | Key::Home | Key::End => {
                if let Some(last) = enabled.len().checked_sub(1) {
                    let pos = match (key, at) {
                        (Key::Home, _) | (Key::Down, None) => 0,
                        (Key::Down, Some(p)) => (p + 1) % enabled.len(),
                        (Key::Up, Some(p)) => p.checked_sub(1).unwrap_or(last),
                        // End, or Up with nothing highlighted.
                        _ => last,
                    };
                    self.hovered_item = Some(enabled[pos]);
                }
            },
            Key::Enter | Key::Space => {
                if let Some(id) = at.and_then(|p| entries[enabled[p]].clone()) {
                    self.close();
                    return MenuKey::Activate(id);
                }
            },
            _ => self.close(),
        }
        MenuKey::Handled
    }

    /// Natural width of a drop-down listing `menu`.
    fn dropdown_width(&self, menu: &Menu) -> u32 {
        let mut max_label = 0i32;
        let mut has_shortcut = false;
        let mut max_shortcut = 0i32;
        for e in &menu.entries {
            if let MenuEntry::Action {
                label, shortcut, ..
            } = e
            {
                max_label = max_label.max(self.text_w(label));
                if let Some(sc) = shortcut {
                    has_shortcut = true;
                    max_shortcut = max_shortcut.max(self.text_w(sc));
                }
            }
        }
        let shortcut_w = if has_shortcut {
            SHORTCUT_GAP + max_shortcut
        } else {
            0
        };
        let total = ITEM_PAD_X + max_label + shortcut_w + ITEM_PAD_X;
        (total as u32).max(DROPDOWN_MIN_W)
    }

    fn dropdown_height(menu: &Menu) -> u32 {
        let mut h = 4; // top + bottom padding
        for e in &menu.entries {
            h += match e {
                MenuEntry::Action { .. } => ITEM_H,
                MenuEntry::Separator => SEPARATOR_H,
            };
        }
        (h + 4) as u32
    }

    /// The open menu and its drop-down rect `(x, y, w, h)` for the bar
    /// `bar = (x, y, w, h)`. `None` when nothing is open (or `open` names
    /// a menu that no longer exists). The drop-down hangs under its label,
    /// shifted left (and on a very narrow bar shrunk) to stay within the
    /// bar's width.
    fn open_dropdown(&self, bar: Rect) -> Option<(Cow<'_, Menu>, Rect)> {
        let (bar_x, bar_y, bar_w, bar_h) = bar;
        let idx = self.open?;
        let layout = self.bar_layout(bar_w);
        let menu = self.menu_at(idx, &layout)?;
        let (label_x, _) = layout.slot(idx)?;
        let natural = self.dropdown_width(&menu);
        let w = if bar_w == 0 {
            natural
        } else {
            natural.min(bar_w)
        };
        let x = (bar_x + label_x)
            .min(bar_x + bar_w as i32 - w as i32)
            .max(bar_x);
        let h = Self::dropdown_height(&menu);
        Some((menu, (x, bar_y + bar_h as i32, w, h)))
    }

    /// Index of the drop-down entry whose row contains `y`, for a
    /// drop-down whose top edge is `dd_y`.
    fn entry_at(menu: &Menu, dd_y: i32, y: i32) -> Option<usize> {
        let mut item_y = dd_y + 4;
        for (i, entry) in menu.entries.iter().enumerate() {
            let h = match entry {
                MenuEntry::Action { .. } => ITEM_H,
                MenuEntry::Separator => SEPARATOR_H,
            };
            if y >= item_y && y < item_y + h {
                return Some(i);
            }
            item_y += h;
        }
        None
    }

    /// Test a click at `(x, y)` against the bar at `(bar_x, bar_y)`
    /// with a given `bar_w, bar_h`. Call on every pointer click
    /// while the bar is visible.
    pub fn hit_test(
        &self,
        x: i32,
        y: i32,
        bar_x: i32,
        bar_y: i32,
        bar_w: u32,
        bar_h: u32,
    ) -> MenuHit {
        // On the bar itself?
        if contains((bar_x, bar_y, bar_w, bar_h), x, y) {
            let layout = self.bar_layout(bar_w);
            return match layout.index_at(x - bar_x, self.menus.len()) {
                Some(i) => MenuHit::Label(i),
                // Inside the bar but not on a label — close if open.
                None => MenuHit::Outside,
            };
        }

        // Inside the open drop-down?
        if let Some((menu, dd)) = self.open_dropdown((bar_x, bar_y, bar_w, bar_h))
            && contains(dd, x, y)
        {
            return match Self::entry_at(&menu, dd.1, y).map(|i| &menu.entries[i]) {
                Some(MenuEntry::Action {
                    id, enabled: true, ..
                }) => MenuHit::Item { id: id.clone() },
                // Disabled entry, separator or padding.
                _ => MenuHit::NoOp,
            };
        }

        MenuHit::Outside
    }

    /// Update the hover state for the currently-open drop-down of the
    /// bar `bar = (x, y, w, h)` so the next `draw` highlights the correct
    /// item. Safe to call with any (x, y); a point outside the drop-down
    /// clears the hover.
    ///
    /// This is a hook for hosts that have mouse-move input available.
    /// OASIS's desktop dispatcher currently forwards only clicks, text,
    /// and mouse-wheel to app windows; when/if a pointer-move event
    /// variant is added, call this from the dispatcher to enable live
    /// hover highlights.
    pub fn pointer_move(&mut self, x: i32, y: i32, bar: Rect) {
        self.hovered_item = self
            .open_dropdown(bar)
            .filter(|(_, dd)| contains(*dd, x, y))
            .and_then(|(menu, dd)| {
                Self::entry_at(&menu, dd.1, y)
                    .filter(|&i| matches!(menu.entries[i], MenuEntry::Action { .. }))
            });
    }

    /// Draw the bar itself (labels only). The host should call this
    /// inside its normal windowed draw; if a drop-down is open, call
    /// [`Self::draw_dropdown`] AFTER the rest of the window so the
    /// drop-down floats above its content.
    pub fn draw_bar(
        &self,
        backend: &mut dyn SdiBackend,
        bar_x: i32,
        bar_y: i32,
        bar_w: u32,
        bar_h: u32,
        style: &MenuStyle,
    ) -> Result<()> {
        let labels = self.menus.iter().map(|m| m.label.as_str());
        self.remember(&*backend, style.font_size, labels.chain([OVERFLOW_LABEL]));
        backend.fill_rect(bar_x, bar_y, bar_w, bar_h, style.bar_bg)?;
        backend.fill_rect(bar_x, bar_y + bar_h as i32 - 1, bar_w, 1, style.bar_border)?;

        let layout = self.bar_layout(bar_w);
        // The open label is lit; a folded menu lights the overflow label.
        let open_slot = self.open.and_then(|o| layout.slot(o));
        let labels = self
            .menus
            .iter()
            .map(|m| m.label.as_str())
            .zip(layout.slots.iter().copied())
            .chain(layout.overflow.map(|slot| (OVERFLOW_LABEL, slot)));
        let text_y = bar_y + (bar_h as i32 - style.font_size as i32) / 2;
        for (label, (x, w)) in labels {
            let is_open = open_slot == Some((x, w));
            if is_open {
                backend.fill_rect(
                    bar_x + x,
                    bar_y + 2,
                    w as u32,
                    bar_h.saturating_sub(4),
                    style.label_hot_bg,
                )?;
            }
            let text_color = if is_open {
                style.label_hot_text
            } else {
                style.label_text
            };
            let text_x = bar_x + x + layout.pad;
            backend.draw_text(label, text_x, text_y, style.font_size, text_color)?;
        }
        Ok(())
    }

    /// Draw the active drop-down (no-op if none is open) of the bar
    /// `bar = (x, y, w, h)`. Call after the rest of the window content so
    /// the drop-down floats on top.
    pub fn draw_dropdown(
        &self,
        backend: &mut dyn SdiBackend,
        bar: Rect,
        style: &MenuStyle,
    ) -> Result<()> {
        if let Some(idx) = self.open
            && let Some(menu) = self.menu_at(idx, &self.bar_layout(bar.2))
        {
            let mut texts = vec![ELLIPSIS];
            for entry in &menu.entries {
                if let MenuEntry::Action {
                    label, shortcut, ..
                } = entry
                {
                    texts.push(label);
                    texts.extend(shortcut.as_deref());
                }
            }
            self.remember(&*backend, style.font_size, texts);
        }
        let Some((menu, (dd_x, dd_y, dd_w, dd_h))) = self.open_dropdown(bar) else {
            return Ok(());
        };
        // A drop-down squeezed below its natural width trims its padding.
        let pad_x = if dd_w < self.dropdown_width(&menu) {
            MIN_ITEM_PAD_X
        } else {
            ITEM_PAD_X
        };
        let inner_w = dd_w as i32 - 2 * pad_x;

        // Win95 raised bezel: 1px white top+left over 1px dark
        // bottom+right over grey fill.
        backend.fill_rect(dd_x, dd_y, dd_w, dd_h, style.dropdown_bg)?;
        backend.fill_rect(dd_x, dd_y, dd_w, 1, style.dropdown_border_light)?;
        backend.fill_rect(dd_x, dd_y, 1, dd_h, style.dropdown_border_light)?;
        backend.fill_rect(
            dd_x,
            dd_y + dd_h as i32 - 1,
            dd_w,
            1,
            style.dropdown_border_dark,
        )?;
        backend.fill_rect(
            dd_x + dd_w as i32 - 1,
            dd_y,
            1,
            dd_h,
            style.dropdown_border_dark,
        )?;

        let mut item_y = dd_y + 4;
        for (i, entry) in menu.entries.iter().enumerate() {
            match entry {
                MenuEntry::Action {
                    label,
                    shortcut,
                    enabled,
                    ..
                } => {
                    let hot = self.hovered_item == Some(i) && *enabled;
                    if hot {
                        backend.fill_rect(
                            dd_x + 2,
                            item_y,
                            dd_w.saturating_sub(4),
                            ITEM_H as u32,
                            style.item_hot_bg,
                        )?;
                    }
                    let text_color = if !enabled {
                        style.item_disabled_text
                    } else if hot {
                        style.item_hot_text
                    } else {
                        style.item_text
                    };
                    let text_y = item_y + (ITEM_H - style.font_size as i32) / 2;
                    let label_w = self.text_w(label);
                    if label_w <= inner_w {
                        let label_x = dd_x + pad_x;
                        backend.draw_text(label, label_x, text_y, style.font_size, text_color)?;
                    } else if self.text_w(ELLIPSIS) <= inner_w {
                        backend.draw_text_ellipsis(
                            label,
                            dd_x + pad_x,
                            text_y,
                            style.font_size,
                            text_color,
                            inner_w as u32,
                        )?;
                    }
                    // The shortcut hint is dropped first when space runs out.
                    if let Some(sc) = shortcut
                        && label_w + SHORTCUT_GAP + self.text_w(sc) <= inner_w
                    {
                        backend.draw_text(
                            sc,
                            dd_x + dd_w as i32 - self.text_w(sc) - pad_x,
                            text_y,
                            style.font_size,
                            text_color,
                        )?;
                    }
                    item_y += ITEM_H;
                },
                MenuEntry::Separator => {
                    backend.fill_rect(
                        dd_x + 4,
                        item_y + SEPARATOR_H / 2,
                        dd_w.saturating_sub(8),
                        1,
                        style.separator,
                    )?;
                    item_y += SEPARATOR_H;
                },
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo_bar() -> MenuBar {
        MenuBar::new(vec![
            Menu::new(
                "File",
                vec![
                    MenuEntry::action("New", "file.new"),
                    MenuEntry::action("Save", "file.save").with_shortcut("Ctrl+S"),
                    MenuEntry::Separator,
                    MenuEntry::action("Exit", "file.exit"),
                ],
            ),
            Menu::new(
                "Edit",
                vec![
                    MenuEntry::action("Undo", "edit.undo").with_shortcut("Ctrl+Z"),
                    MenuEntry::action("Redo", "edit.redo").disabled(),
                ],
            ),
        ])
    }

    #[test]
    fn click_label_returns_label_hit() {
        let bar = demo_bar();
        // Bar at (0, 20), width 400, height 18. First label starts
        // at x=6; "File" is 4 chars → width ≈ 4*7 + 16 = 44.
        let hit = bar.hit_test(10, 30, 0, 20, 400, 18);
        assert_eq!(hit, MenuHit::Label(0));
    }

    #[test]
    fn click_second_label() {
        let bar = demo_bar();
        // "File" label ends around x=6+44=50. "Edit" starts there.
        let hit = bar.hit_test(60, 30, 0, 20, 400, 18);
        assert_eq!(hit, MenuHit::Label(1));
    }

    #[test]
    fn click_on_open_dropdown_item() {
        let mut bar = demo_bar();
        bar.open = Some(0);
        // First item (New) starts at y = bar_y + bar_h + 4 = 42.
        // Each item is 20px tall.
        let hit = bar.hit_test(20, 50, 0, 20, 400, 18);
        match hit {
            MenuHit::Item { id } => assert_eq!(id, "file.new"),
            other => panic!("expected Item, got {other:?}"),
        }
    }

    #[test]
    fn click_on_disabled_item_is_noop() {
        let mut bar = demo_bar();
        bar.open = Some(1);
        // Edit menu: Undo (enabled, 20px), Redo (disabled, next 20px).
        // Dropdown starts at x = 6 + 44 = 50. y=bar_y+bar_h+4=42,
        // redo at y=62.
        let hit = bar.hit_test(60, 70, 0, 20, 400, 18);
        assert_eq!(hit, MenuHit::NoOp);
    }

    #[test]
    fn click_outside_closes() {
        let bar = demo_bar();
        let hit = bar.hit_test(500, 500, 0, 20, 400, 18);
        assert_eq!(hit, MenuHit::Outside);
    }

    #[test]
    fn keyboard_opens_navigates_and_activates() {
        let mut bar = demo_bar();
        // Closed: only F10 is a menu key.
        assert_eq!(bar.handle_key(Key::Down, 400), MenuKey::Ignored);
        assert_eq!(bar.handle_key(Key::F(10), 400), MenuKey::Handled);
        assert_eq!((bar.open, bar.hovered_item), (Some(0), Some(0)));
        // Down skips the separator; Down from the last item wraps.
        bar.handle_key(Key::Down, 400);
        bar.handle_key(Key::Down, 400);
        assert_eq!(bar.hovered_item, Some(3), "Exit, past the separator");
        bar.handle_key(Key::Down, 400);
        assert_eq!(bar.hovered_item, Some(0), "wraps to New");
        bar.handle_key(Key::Up, 400);
        assert_eq!(bar.hovered_item, Some(3));
        // Right opens Edit at its first enabled item; Right again wraps.
        bar.handle_key(Key::Right, 400);
        assert_eq!((bar.open, bar.hovered_item), (Some(1), Some(0)));
        // Redo is disabled: Down stays on Undo.
        bar.handle_key(Key::Down, 400);
        assert_eq!(bar.hovered_item, Some(0));
        bar.handle_key(Key::Right, 400);
        assert_eq!(bar.open, Some(0));
        bar.handle_key(Key::Left, 400);
        assert_eq!(bar.open, Some(1));
        assert_eq!(
            bar.handle_key(Key::Enter, 400),
            MenuKey::Activate("edit.undo".into())
        );
        assert!(!bar.is_open());
    }

    #[test]
    fn keyboard_closes_on_escape_f10_and_other_keys() {
        let mut bar = demo_bar();
        for close in [Key::Escape, Key::F(10), Key::Char('x')] {
            bar.handle_key(Key::F(10), 400);
            assert!(bar.is_open());
            assert_eq!(bar.handle_key(close, 400), MenuKey::Handled);
            assert!(!bar.is_open(), "{close:?} closes");
            assert_eq!(bar.hovered_item, None);
        }
        assert_eq!(bar.handle_key(Key::Escape, 400), MenuKey::Ignored);
    }

    #[test]
    fn keyboard_reaches_folded_menus_through_overflow() {
        let mut bar = demo_bar();
        bar.menus.push(Menu::new(
            "Help",
            vec![MenuEntry::action("About", "help.about")],
        ));
        // Narrow enough that the trailing menus fold into `»`.
        let w = 60;
        let (order, layout) = bar.label_order(w);
        assert!(layout.overflow.is_some(), "{layout:?}");
        assert_eq!(order.last(), Some(&bar.menus.len()));
        bar.handle_key(Key::F(10), w);
        bar.handle_key(Key::Left, w);
        assert_eq!(
            bar.open,
            Some(bar.menus.len()),
            "Left wraps to the overflow"
        );
        // The folded menus' headings are disabled: End reaches the last
        // folded item.
        assert_eq!(bar.handle_key(Key::End, w), MenuKey::Handled);
        assert_eq!(
            bar.handle_key(Key::Enter, w),
            MenuKey::Activate("help.about".into())
        );
    }

    #[test]
    fn hover_tracks_item() {
        let mut bar = demo_bar();
        bar.open = Some(0);
        bar.pointer_move(20, 50, (0, 20, 400, 18));
        assert_eq!(bar.hovered_item, Some(0)); // "New" is item 0
        bar.pointer_move(20, 500, (0, 20, 400, 18));
        assert_eq!(bar.hovered_item, None);
    }

    #[test]
    fn close_resets_state() {
        let mut bar = demo_bar();
        bar.open = Some(0);
        bar.hovered_item = Some(2);
        bar.close();
        assert!(!bar.is_open());
        assert_eq!(bar.hovered_item, None);
    }

    #[test]
    fn from_theme_matches_default_for_builtin_themes() {
        // Every built-in theme keeps the Win95 gray menu defaults, so
        // the theme-derived style is pixel-identical to `default()`.
        for theme in crate::test_utils::all_themes() {
            assert_eq!(MenuStyle::from_theme(&theme), MenuStyle::default());
        }
    }

    #[test]
    fn from_theme_reads_overridden_slots() {
        use crate::theme::Theme;
        let mut theme = Theme::dark();
        theme.menu_bg = Color::rgb(10, 20, 30);
        theme.menu_hover_bg = Color::rgb(200, 0, 0);
        let style = MenuStyle::from_theme(&theme);
        assert_eq!(style.bar_bg, Color::rgb(10, 20, 30));
        assert_eq!(style.label_hot_bg, Color::rgb(200, 0, 0));
        assert_eq!(style.item_hot_bg, Color::rgb(200, 0, 0));
    }

    #[test]
    fn menu_entry_action_id() {
        let action = MenuEntry::action("Save", "file.save");
        assert_eq!(action.action_id(), Some("file.save"));
        let disabled = MenuEntry::action("X", "x").disabled();
        assert_eq!(disabled.action_id(), None);
        let sep = MenuEntry::Separator;
        assert_eq!(sep.action_id(), None);
    }

    #[test]
    fn stale_open_index_is_harmless() {
        let mut bar = demo_bar();
        bar.open = Some(99);
        assert_eq!(bar.hit_test(10, 30, 0, 0, 300, 20), MenuHit::Outside);
        bar.pointer_move(10, 30, (0, 0, 300, 20));
        assert_eq!(bar.hovered_item, None);
        let mut rec = oasis_test_backend::RecordingBackend::default();
        bar.draw_dropdown(&mut rec, (0, 0, 300, 20), &MenuStyle::default())
            .unwrap();
    }

    #[test]
    fn short_bar_does_not_underflow() {
        let mut bar = demo_bar();
        bar.open = Some(0);
        let mut rec = oasis_test_backend::RecordingBackend::default();
        bar.draw_bar(&mut rec, 0, 0, 200, 2, &MenuStyle::default())
            .unwrap();
    }

    #[test]
    fn hover_skips_separators() {
        let mut bar = demo_bar();
        bar.open = Some(0);
        let (dx, dy, _, _) = bar.open_dropdown((0, 0, 400, 20)).unwrap().1;
        // Walk every row of the drop-down: hover only ever lands on actions.
        for y in dy..dy + 200 {
            bar.pointer_move(dx + 5, y, (0, 0, 400, 20));
            if let Some(i) = bar.hovered_item {
                assert!(matches!(bar.menus[0].entries[i], MenuEntry::Action { .. }));
            }
        }
    }

    /// Paint's six menus: wider than a 200 px bar at full padding.
    fn wide_bar() -> MenuBar {
        let menu = |label: &str| {
            Menu::new(
                label,
                vec![MenuEntry::action(
                    format!("{label} item"),
                    label.to_lowercase(),
                )],
            )
        };
        MenuBar::new(
            ["File", "Edit", "Tool", "Brush", "Layer", "View"]
                .map(menu)
                .to_vec(),
        )
    }

    /// Right edge of every drawn text run (measured like the backend).
    fn text_right_edges(rec: &oasis_test_backend::RecordingBackend) -> Vec<(String, i32)> {
        rec.text_positions()
            .into_iter()
            .map(|(t, x, _, size)| (t.to_string(), x + bitmap_measure_text(t, size) as i32))
            .collect()
    }

    #[test]
    fn hit_boxes_follow_the_backend_measure() {
        let bar = demo_bar();
        let mut rec = oasis_test_backend::RecordingBackend::default();
        // A wide font: 12 px per character.
        rec.set_measure_text_fn(|t, _| t.chars().count() as u32 * 12);
        bar.draw_bar(&mut rec, 0, 0, 400, 18, &MenuStyle::default())
            .unwrap();
        // "File" spans 6 + 4 * 12 + 16 = 70: x = 60 is still File (it was
        // Edit under the fixed 7 px estimate).
        assert_eq!(bar.hit_test(60, 5, 0, 0, 400, 18), MenuHit::Label(0));
        assert_eq!(bar.hit_test(75, 5, 0, 0, 400, 18), MenuHit::Label(1));
    }

    #[test]
    fn narrow_bar_squeezes_then_folds_menus_into_overflow() {
        let mut bar = wide_bar();
        let style = MenuStyle::default();
        let text_sum: i32 = bar.menus.iter().map(|m| bar.text_w(&m.label)).sum();
        let natural = (BAR_PAD_X + text_sum + 2 * LABEL_PAD_X * 6) as u32;
        let squeezed = (MIN_BAR_PAD_X + text_sum + 2 * MIN_LABEL_PAD_X * 6) as u32;
        assert!(squeezed > 200, "the 200 px case below must overflow");
        // Narrower than natural but wide enough with squeezed padding:
        // every label fits, no `»`.
        for w in [natural - 1, squeezed] {
            let mut rec = oasis_test_backend::RecordingBackend::default();
            bar.draw_bar(&mut rec, 0, 0, w, 18, &style).unwrap();
            assert!(!rec.has_text(OVERFLOW_LABEL), "{w}");
            let edges = text_right_edges(&rec);
            assert!(edges.iter().all(|(_, r)| *r <= w as i32), "{w}: {edges:?}");
        }

        // 200 px: the trailing menus fold into `»`, nothing spills out.
        let mut rec = oasis_test_backend::RecordingBackend::default();
        bar.draw_bar(&mut rec, 0, 0, 200, 18, &style).unwrap();
        let edges = text_right_edges(&rec);
        assert!(edges.iter().all(|(_, r)| *r <= 200), "{edges:?}");
        assert!(rec.has_text(OVERFLOW_LABEL));
        assert!(!rec.has_text("View"));

        // The overflow label opens a menu listing the folded menus.
        let layout = bar.bar_layout(200);
        let (ox, _) = layout.overflow.unwrap();
        assert_eq!(bar.hit_test(ox + 2, 5, 0, 0, 200, 18), MenuHit::Label(6));
        bar.open = Some(6);
        let mut rec = oasis_test_backend::RecordingBackend::default();
        bar.draw_dropdown(&mut rec, (0, 0, 200, 18), &style)
            .unwrap();
        assert!(rec.has_text("View item"), "{:?}", rec.texts());
        let (_, (dx, dy, dw, dh)) = bar.open_dropdown((0, 0, 200, 18)).unwrap();
        assert!(dx >= 0 && dx + dw as i32 <= 200, "drop-down inside the bar");
        // Every folded entry is clickable and dispatches its own id.
        let ids: Vec<MenuHit> = (dy..dy + dh as i32)
            .step_by(ITEM_H as usize / 2)
            .map(|y| bar.hit_test(dx + 10, y, 0, 0, 200, 18))
            .filter(|h| matches!(h, MenuHit::Item { .. }))
            .collect();
        assert!(
            ids.contains(&MenuHit::Item { id: "view".into() }),
            "{ids:?}"
        );
    }

    #[test]
    fn dropdown_is_shifted_and_squeezed_into_the_bar() {
        let mut bar = demo_bar();
        bar.menus[1].entries[0] =
            MenuEntry::action("A very long undo label", "edit.undo").with_shortcut("Ctrl+Z");
        bar.open = Some(1);
        let style = MenuStyle::default();
        let mut rec = oasis_test_backend::RecordingBackend::default();
        bar.draw_bar(&mut rec, 0, 0, 140, 18, &style).unwrap();
        bar.draw_dropdown(&mut rec, (0, 0, 140, 18), &style)
            .unwrap();
        let (_, (dx, _, dw, _)) = bar.open_dropdown((0, 0, 140, 18)).unwrap();
        assert!(dx >= 0 && dx + dw as i32 <= 140);
        let edges = text_right_edges(&rec);
        assert!(edges.iter().all(|(_, r)| *r <= 140), "{edges:?}");
        // The shortcut hint gave way to the label.
        assert!(!rec.has_text("Ctrl+Z"), "{:?}", rec.texts());
    }
}
