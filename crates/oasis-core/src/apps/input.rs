//! Input handling methods for `AppRunner`.

use crate::input::{Button, Key, Modifiers};
use crate::vfs::Vfs;

use super::app_trait::AppAction;
use super::runner::AppRunner;

impl AppRunner {
    /// Handle input while the app is active.
    pub fn handle_input(&mut self, button: &Button, vfs: &dyn Vfs) -> AppAction {
        // Delegate to extracted app if present.
        if let Some(ref mut app) = self.delegate {
            let action = app.handle_input(button, vfs);
            self.redraw_pending = true;
            self.sync_from_delegate();
            return action;
        }

        AppAction::None
    }

    /// Forward a raw key press to the app delegate.
    ///
    /// Returns `Some(action)` when the app consumed the key (the host must
    /// then drop the key's gamepad-style twin), `None` otherwise. See
    /// [`crate::apps::App::handle_key`].
    pub fn handle_key(&mut self, key: &Key, mods: Modifiers, vfs: &dyn Vfs) -> Option<AppAction> {
        let app = self.delegate.as_mut()?;
        let action = app.handle_key(key, mods, vfs);
        if action.is_some() {
            self.redraw_pending = true;
            self.sync_from_delegate();
        }
        action
    }

    /// Whether the app delegate is currently a text-entry target.
    pub fn accepts_text(&self) -> bool {
        self.delegate.as_ref().is_some_and(|app| app.accepts_text())
    }

    /// Forward a typed character to the app delegate.
    pub fn handle_text_input(&mut self, ch: char) {
        if let Some(ref mut app) = self.delegate {
            app.handle_text_input(ch);
            self.redraw_pending = true;
        }
    }

    /// Forward a backspace to the app delegate.
    pub fn handle_backspace(&mut self) {
        if let Some(ref mut app) = self.delegate {
            app.handle_backspace();
            self.redraw_pending = true;
        }
    }
}
