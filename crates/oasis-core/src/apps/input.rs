//! Input handling methods for `AppRunner`.

use crate::input::Button;
use crate::vfs::Vfs;

use super::app_trait::AppAction;
use super::runner::AppRunner;

impl AppRunner {
    /// Handle input while the app is active.
    pub fn handle_input(&mut self, button: &Button, vfs: &dyn Vfs) -> AppAction {
        // Delegate to extracted app if present.
        if let Some(ref mut app) = self.delegate {
            let action = app.handle_input(button, vfs);
            self.sync_from_delegate();
            return action;
        }

        AppAction::None
    }

    /// Forward a typed character to the app delegate.
    pub fn handle_text_input(&mut self, ch: char) {
        if let Some(ref mut app) = self.delegate {
            app.handle_text_input(ch);
        }
    }

    /// Forward a backspace to the app delegate.
    pub fn handle_backspace(&mut self) {
        if let Some(ref mut app) = self.delegate {
            app.handle_backspace();
        }
    }
}
