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

use oasis_types::backend::{Color, SdiBackend};
use oasis_types::error::Result;

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
#[derive(Debug, Clone)]
pub struct MenuBar {
    pub menus: Vec<Menu>,
    pub open: Option<usize>,
    pub hovered_item: Option<usize>,
}

/// The result of a hit-test on the menu bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuHit {
    /// A top-level label was clicked (toggle its drop-down).
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

/// Visual parameters — Windows-95 bezel defaults.
#[derive(Debug, Clone, Copy)]
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
            font_size: 11,
        }
    }
}

/// Layout parameters (pixel sizes).
const LABEL_PAD_X: i32 = 8;
const LABEL_CHAR_W: i32 = 7;
const ITEM_H: i32 = 20;
const ITEM_PAD_X: i32 = 22;
const DROPDOWN_MIN_W: u32 = 120;
const SEPARATOR_H: i32 = 6;

impl MenuBar {
    pub fn new(menus: Vec<Menu>) -> Self {
        Self {
            menus,
            open: None,
            hovered_item: None,
        }
    }

    pub fn close(&mut self) {
        self.open = None;
        self.hovered_item = None;
    }

    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Compute the X range of a top-level label on the bar.
    /// Returns `(label_x, label_w)` in bar-local coordinates.
    fn label_range(&self, index: usize) -> (i32, i32) {
        let mut x = 6i32;
        for (i, menu) in self.menus.iter().enumerate() {
            let w = menu.label.chars().count() as i32 * LABEL_CHAR_W + LABEL_PAD_X * 2;
            if i == index {
                return (x, w);
            }
            x += w;
        }
        (x, 0)
    }

    /// Width required for the currently-open drop-down.
    fn dropdown_width(&self, menu: &Menu) -> u32 {
        let mut max_label = 0i32;
        let mut has_shortcut = false;
        let mut max_shortcut = 0i32;
        for e in &menu.entries {
            if let MenuEntry::Action {
                label, shortcut, ..
            } = e
            {
                max_label = max_label.max(label.chars().count() as i32 * LABEL_CHAR_W);
                if let Some(sc) = shortcut {
                    has_shortcut = true;
                    max_shortcut = max_shortcut.max(sc.chars().count() as i32 * LABEL_CHAR_W);
                }
            }
        }
        let total =
            ITEM_PAD_X + max_label + if has_shortcut { 16 + max_shortcut } else { 0 } + ITEM_PAD_X;
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

    /// Drop-down width and height for a given menu, using the same
    /// layout constants the widget renders with. Exposed so app-side
    /// SDI renderers (which reimplement `draw_dropdown` over named
    /// scene-graph objects) can stay aligned with the widget's hit
    /// boxes and avoid drift.
    pub fn dropdown_dimensions(&self, menu: &Menu) -> (u32, u32) {
        (self.dropdown_width(menu), Self::dropdown_height(menu))
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
        if y >= bar_y && y < bar_y + bar_h as i32 && x >= bar_x && x < bar_x + bar_w as i32 {
            let rel_x = x - bar_x;
            let mut cursor = 6i32;
            for (i, menu) in self.menus.iter().enumerate() {
                let w = menu.label.chars().count() as i32 * LABEL_CHAR_W + LABEL_PAD_X * 2;
                if rel_x >= cursor && rel_x < cursor + w {
                    return MenuHit::Label(i);
                }
                cursor += w;
            }
            // Inside the bar but not on a label — close if open.
            return MenuHit::Outside;
        }

        // Inside the open drop-down?
        if let Some(idx) = self.open {
            let menu = &self.menus[idx];
            let (label_x, _label_w) = self.label_range(idx);
            let dd_x = bar_x + label_x;
            let dd_y = bar_y + bar_h as i32;
            let dd_w = self.dropdown_width(menu) as i32;
            let dd_h = Self::dropdown_height(menu) as i32;

            if x >= dd_x && x < dd_x + dd_w && y >= dd_y && y < dd_y + dd_h {
                let mut item_y = dd_y + 4;
                for entry in &menu.entries {
                    match entry {
                        MenuEntry::Action { id, enabled, .. } => {
                            if y >= item_y && y < item_y + ITEM_H {
                                return if *enabled {
                                    MenuHit::Item { id: id.clone() }
                                } else {
                                    MenuHit::NoOp
                                };
                            }
                            item_y += ITEM_H;
                        },
                        MenuEntry::Separator => {
                            item_y += SEPARATOR_H;
                        },
                    }
                }
                return MenuHit::NoOp;
            }
        }

        MenuHit::Outside
    }

    /// Update the hover state for the currently-open drop-down so the
    /// next `draw` highlights the correct item. Safe to call with any
    /// (x, y); a point outside the drop-down clears the hover.
    ///
    /// This is a hook for hosts that have mouse-move input available.
    /// OASIS's desktop dispatcher currently forwards only clicks, text,
    /// and mouse-wheel to app windows; when/if a pointer-move event
    /// variant is added, call this from the dispatcher to enable live
    /// hover highlights.
    pub fn pointer_move(&mut self, x: i32, y: i32, bar_x: i32, bar_y: i32, bar_h: u32) {
        self.hovered_item = None;
        let Some(idx) = self.open else {
            return;
        };
        let menu = &self.menus[idx];
        let (label_x, _) = self.label_range(idx);
        let dd_x = bar_x + label_x;
        let dd_y = bar_y + bar_h as i32;
        let dd_w = self.dropdown_width(menu) as i32;
        let dd_h = Self::dropdown_height(menu) as i32;
        if x < dd_x || x >= dd_x + dd_w || y < dd_y || y >= dd_y + dd_h {
            return;
        }
        let mut item_y = dd_y + 4;
        for (i, entry) in menu.entries.iter().enumerate() {
            match entry {
                MenuEntry::Action { .. } => {
                    if y >= item_y && y < item_y + ITEM_H {
                        self.hovered_item = Some(i);
                        return;
                    }
                    item_y += ITEM_H;
                },
                MenuEntry::Separator => {
                    item_y += SEPARATOR_H;
                },
            }
        }
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
        backend.fill_rect(bar_x, bar_y, bar_w, bar_h, style.bar_bg)?;
        backend.fill_rect(bar_x, bar_y + bar_h as i32 - 1, bar_w, 1, style.bar_border)?;

        let mut cursor = 6i32;
        for (i, menu) in self.menus.iter().enumerate() {
            let w = menu.label.chars().count() as i32 * LABEL_CHAR_W + LABEL_PAD_X * 2;
            let is_open = self.open == Some(i);
            if is_open {
                backend.fill_rect(
                    bar_x + cursor,
                    bar_y + 2,
                    w as u32,
                    bar_h - 4,
                    style.label_hot_bg,
                )?;
            }
            let text_color = if is_open {
                style.label_hot_text
            } else {
                style.label_text
            };
            backend.draw_text(
                &menu.label,
                bar_x + cursor + LABEL_PAD_X,
                bar_y + (bar_h as i32 - style.font_size as i32) / 2,
                style.font_size,
                text_color,
            )?;
            cursor += w;
        }
        Ok(())
    }

    /// Draw the active drop-down (no-op if none is open). Call after
    /// the rest of the window content so the drop-down floats on top.
    pub fn draw_dropdown(
        &self,
        backend: &mut dyn SdiBackend,
        bar_x: i32,
        bar_y: i32,
        bar_h: u32,
        style: &MenuStyle,
    ) -> Result<()> {
        let Some(idx) = self.open else {
            return Ok(());
        };
        let menu = &self.menus[idx];
        let (label_x, _) = self.label_range(idx);
        let dd_x = bar_x + label_x;
        let dd_y = bar_y + bar_h as i32;
        let dd_w = self.dropdown_width(menu);
        let dd_h = Self::dropdown_height(menu);

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
                            dd_w - 4,
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
                    backend.draw_text(
                        label,
                        dd_x + ITEM_PAD_X,
                        text_y,
                        style.font_size,
                        text_color,
                    )?;
                    if let Some(sc) = shortcut {
                        let sc_w = backend.measure_text(sc, style.font_size) as i32;
                        backend.draw_text(
                            sc,
                            dd_x + dd_w as i32 - sc_w - ITEM_PAD_X,
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
                        dd_w - 8,
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
    fn hover_tracks_item() {
        let mut bar = demo_bar();
        bar.open = Some(0);
        bar.pointer_move(20, 50, 0, 20, 18);
        assert_eq!(bar.hovered_item, Some(0)); // "New" is item 0
        bar.pointer_move(20, 500, 0, 20, 18);
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
    fn menu_entry_action_id() {
        let action = MenuEntry::action("Save", "file.save");
        assert_eq!(action.action_id(), Some("file.save"));
        let disabled = MenuEntry::action("X", "x").disabled();
        assert_eq!(disabled.action_id(), None);
        let sep = MenuEntry::Separator;
        assert_eq!(sep.action_id(), None);
    }
}
