//! Input handling methods for `AppRunner`.

use crate::active_theme::ActiveTheme;
use crate::input::{Button, Key, Modifiers};
use crate::vfs::Vfs;

use super::AppAction;
use super::runner::AppRunner;

impl AppRunner {
    /// Handle input while the app is active.
    pub fn handle_input(&mut self, button: &Button, vfs: &dyn Vfs) -> AppAction {
        let action = self.delegate.handle_input(button, vfs);
        self.redraw_pending = true;
        action
    }

    /// Forward a raw key press to the app delegate.
    ///
    /// Returns `Some(action)` when the app consumed the key (the host must
    /// then drop the key's gamepad-style twin), `None` otherwise. See
    /// [`crate::apps::App::handle_key`].
    pub fn handle_key(&mut self, key: &Key, mods: Modifiers, vfs: &dyn Vfs) -> Option<AppAction> {
        let action = self.delegate.handle_key(key, mods, vfs);
        if action.is_some() {
            self.redraw_pending = true;
        }
        action
    }

    /// Whether the app delegate is currently a text-entry target.
    pub fn accepts_text(&self) -> bool {
        self.delegate.accepts_text()
    }

    /// Forward a typed character to the app delegate.
    pub fn handle_text_input(&mut self, ch: char) {
        self.delegate.handle_text_input(ch);
        self.redraw_pending = true;
    }

    /// Forward a backspace to the app delegate.
    pub fn handle_backspace(&mut self) {
        self.delegate.handle_backspace();
        self.redraw_pending = true;
    }

    /// Forward a mouse-wheel event over the app's window (see
    /// [`crate::apps::App::handle_scroll`]). Returns whether it changed
    /// anything.
    pub fn handle_scroll(&mut self, delta: i32, vfs: &dyn Vfs) -> bool {
        let changed = self.delegate.handle_scroll(delta, vfs);
        if changed {
            self.redraw_pending = true;
        }
        changed
    }

    /// Handle a click in the app's `cw` x `ch` content area at local
    /// `(lx, ly)`. Lays the app out for that size first (see
    /// [`crate::apps::App::layout`]), exactly as [`Self::draw_windowed`]
    /// does, so the app hit-tests against what it drew.
    pub fn handle_click(
        &mut self,
        lx: i32,
        ly: i32,
        cw: u32,
        ch: u32,
        at: &ActiveTheme,
        vfs: &dyn Vfs,
    ) -> AppAction {
        self.delegate.layout(cw, ch, at);
        let action = self.delegate.handle_click(lx, ly, cw, ch, vfs);
        self.redraw_pending = true;
        action
    }

    /// Let the delegate apply its queued VFS mutations (see
    /// [`crate::apps::App::apply_vfs_ops`]). Hosts call this once per frame
    /// for every open runner.
    pub fn apply_vfs_ops(&mut self, vfs: &mut dyn Vfs) {
        if self.delegate.apply_vfs_ops(vfs) {
            self.redraw_pending = true;
        }
    }
}
