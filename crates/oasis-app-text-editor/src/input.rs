//! Input routing: gamepad buttons (`handle_input`), raw keyboard keys
//! (`handle_key`), typed text and backspace, per editor mode.
//!
//! Every feature stays reachable from the gamepad path (PSP has no raw
//! keys); `handle_key` adds the desktop accelerators the menu advertises.

use oasis_app_core::AppAction;
use oasis_types::input::{Button, Key, Modifiers};
use oasis_ui::menu_bar::MenuKey;

use crate::editor::Motion;
use crate::{EditorMode, PendingClose, TextEditorApp};

impl TextEditorApp {
    // ---------------------------------------------------------------
    // Raw keys
    // ---------------------------------------------------------------

    /// Keyboard accelerators. Returns `Some` when the key was consumed.
    pub(crate) fn handle_key_impl(&mut self, key: &Key, mods: Modifiers) -> Option<AppAction> {
        if self.close_requested {
            return Some(AppAction::Exit);
        }
        // F10 opens the menu bar; an open menu takes every key.
        if !matches!(self.mode, EditorMode::ConfirmDiscard | EditorMode::Saving) {
            match self.menu.handle_key(*key, self.content_size.0) {
                MenuKey::Ignored => {},
                MenuKey::Handled => return Some(AppAction::None),
                MenuKey::Activate(id) => return Some(self.run_menu_action(&id)),
            }
        }
        match self.mode {
            EditorMode::Normal | EditorMode::Insert => self.key_editing(key, mods),
            EditorMode::Find | EditorMode::Replace => self.key_search(key, mods),
            EditorMode::GoToLine | EditorMode::SaveAs => self.key_prompt(key),
            EditorMode::ConfirmDiscard => self.key_confirm(key, mods),
            EditorMode::Saving => None,
        }
    }

    /// Ctrl/Cmd shortcuts shared by editing and search modes.
    fn command_shortcut(&mut self, c: char, shift: bool) -> Option<AppAction> {
        let action = match c {
            'n' => return Some(self.request_close(PendingClose::New)),
            's' if shift => {
                self.enter_mode(EditorMode::SaveAs);
                AppAction::None
            },
            's' => {
                self.save();
                AppAction::None
            },
            'z' if shift => {
                self.redo();
                AppAction::None
            },
            'z' => {
                self.undo();
                AppAction::None
            },
            'y' => {
                self.redo();
                AppAction::None
            },
            'f' => {
                self.enter_mode(EditorMode::Find);
                AppAction::None
            },
            'h' => {
                self.enter_mode(EditorMode::Replace);
                AppAction::None
            },
            'g' => {
                self.enter_mode(EditorMode::GoToLine);
                AppAction::None
            },
            _ => return None,
        };
        Some(action)
    }

    fn key_editing(&mut self, key: &Key, mods: Modifiers) -> Option<AppAction> {
        let shift = mods.shift();
        let command = mods.ctrl() || mods.super_key();
        if command && let Key::Char(c) = key {
            let c = c.to_ascii_lowercase();
            if let Some(action) = self.command_shortcut(c, shift) {
                return Some(action);
            }
            match c {
                'a' => self.select_all(),
                'c' => {
                    self.copy();
                },
                'x' => {
                    if self.cut() {
                        self.mode = EditorMode::Insert;
                    }
                },
                'v' => {
                    if self.paste() {
                        self.mode = EditorMode::Insert;
                    }
                },
                _ => return None,
            }
            return Some(AppAction::None);
        }
        // Alt+<key> is reserved for the host (e.g. Alt+Tab).
        if mods.alt() {
            return None;
        }
        let motion = match key {
            Key::Left if command => Some(Motion::WordLeft),
            Key::Right if command => Some(Motion::WordRight),
            Key::Home if command => Some(Motion::DocStart),
            Key::End if command => Some(Motion::DocEnd),
            Key::Left => Some(Motion::Left),
            Key::Right => Some(Motion::Right),
            Key::Up => Some(Motion::Up),
            Key::Down => Some(Motion::Down),
            Key::Home => Some(Motion::Home),
            Key::End => Some(Motion::End),
            Key::PageUp => Some(Motion::PageUp),
            Key::PageDown => Some(Motion::PageDown),
            _ => None,
        };
        if let Some(motion) = motion {
            self.motion(motion, shift);
            return Some(AppAction::None);
        }
        match key {
            Key::Delete if command => self.delete_word_forward(),
            Key::Delete => self.delete_forward(),
            Key::Backspace if command => self.delete_word_back(),
            Key::Enter if !command => self.new_line(),
            Key::Tab if !command => self.insert_tab(),
            Key::F(3) => {
                self.find_next();
            },
            _ => return None,
        }
        if !matches!(key, Key::F(3)) {
            self.mode = EditorMode::Insert;
        }
        Some(AppAction::None)
    }

    fn key_search(&mut self, key: &Key, mods: Modifiers) -> Option<AppAction> {
        let command = mods.ctrl() || mods.super_key();
        if command && let Key::Char(c) = key {
            return self.command_shortcut(c.to_ascii_lowercase(), mods.shift());
        }
        match key {
            Key::Enter if self.mode == EditorMode::Replace && (command || mods.alt()) => {
                self.run_replace_all();
            },
            Key::Enter if self.mode == EditorMode::Replace => self.run_replace_next(),
            Key::Enter | Key::F(3) => self.run_find_next(),
            Key::Escape => self.leave_mode(),
            Key::Tab if self.mode == EditorMode::Replace => {
                self.query_primed = false;
                self.replace_focus = !self.replace_focus;
                self.rebuild_display_lines();
            },
            _ => return None,
        }
        Some(AppAction::None)
    }

    fn key_prompt(&mut self, key: &Key) -> Option<AppAction> {
        match key {
            Key::Enter => self.commit_prompt(),
            Key::Escape => self.leave_mode(),
            _ => return None,
        }
        Some(AppAction::None)
    }

    fn key_confirm(&mut self, key: &Key, mods: Modifiers) -> Option<AppAction> {
        if mods.has_command() {
            return Some(AppAction::None);
        }
        match key {
            Key::Enter | Key::Char('s') | Key::Char('y') => self.confirm_save(),
            Key::Char('d') | Key::Char('n') => return Some(self.confirm_discard()),
            Key::Escape | Key::Char('c') => self.leave_mode(),
            _ => {},
        }
        // Swallow everything else while the modal prompt is up.
        Some(AppAction::None)
    }

    fn commit_prompt(&mut self) {
        match self.mode {
            EditorMode::GoToLine => self.commit_go_to_line(),
            EditorMode::SaveAs => {
                let path = self.prompt_input.clone();
                self.save_as(&path);
            },
            _ => {},
        }
    }

    fn run_find_next(&mut self) {
        self.query_primed = false;
        let found = self.find_next();
        self.status_message = if found {
            None
        } else {
            Some(format!("Not found: '{}'", self.find_query))
        };
        self.rebuild_display_lines();
    }

    fn run_replace_next(&mut self) {
        self.query_primed = false;
        let replaced = self.replace_next();
        let still = self.selected_text().as_deref() == Some(self.find_query.as_str());
        self.status_message = match (replaced, still) {
            (_, true) => None,
            (true, false) => Some("Replaced last match.".into()),
            (false, false) => Some(format!("Not found: '{}'", self.find_query)),
        };
        self.rebuild_display_lines();
    }

    fn run_replace_all(&mut self) {
        self.query_primed = false;
        let n = self.replace_all();
        self.status_message = Some(format!("Replaced {n} occurrence(s)."));
        self.rebuild_display_lines();
    }

    // ---------------------------------------------------------------
    // Typed text / backspace
    // ---------------------------------------------------------------

    pub(crate) fn handle_text_impl(&mut self, ch: char) {
        match self.mode {
            EditorMode::Normal | EditorMode::Insert => {
                // Typing in Normal mode auto-drops into Insert mode, like
                // Windows Notepad.
                self.mode = EditorMode::Insert;
                self.insert_char(ch);
            },
            EditorMode::Find => self.type_query(ch),
            EditorMode::Replace => {
                if self.replace_focus {
                    self.replace_text.push(ch);
                } else {
                    self.type_query(ch);
                }
            },
            EditorMode::GoToLine => {
                if ch.is_ascii_digit() {
                    self.prompt_input.push(ch);
                }
            },
            EditorMode::SaveAs => self.prompt_input.push(ch),
            EditorMode::Saving | EditorMode::ConfirmDiscard => {},
        }
        self.rebuild_display_lines();
    }

    /// Type into the find query; a primed (carried-over) query is
    /// replaced, as if it had been selected.
    fn type_query(&mut self, ch: char) {
        if std::mem::take(&mut self.query_primed) {
            self.find_query.clear();
        }
        self.find_query.push(ch);
    }

    /// Backspace in the find query: a primed query is cleared whole.
    fn backspace_query(&mut self) {
        if std::mem::take(&mut self.query_primed) {
            self.find_query.clear();
        } else {
            self.find_query.pop();
        }
    }

    pub(crate) fn handle_backspace_impl(&mut self) {
        match self.mode {
            EditorMode::Insert => self.delete_char(),
            EditorMode::Normal => {
                // Backspace in Normal mode also deletes — matches what a
                // user expects after clicking into a window.
                self.mode = EditorMode::Insert;
                self.delete_char();
            },
            EditorMode::Find => self.backspace_query(),
            EditorMode::Replace => {
                if self.replace_focus {
                    self.replace_text.pop();
                } else {
                    self.backspace_query();
                }
            },
            EditorMode::GoToLine | EditorMode::SaveAs => {
                self.prompt_input.pop();
            },
            EditorMode::Saving | EditorMode::ConfirmDiscard => {},
        }
        self.rebuild_display_lines();
    }

    // ---------------------------------------------------------------
    // Gamepad buttons
    // ---------------------------------------------------------------

    pub(crate) fn handle_button(&mut self, button: &Button) -> AppAction {
        if self.close_requested {
            return AppAction::Exit;
        }
        match self.mode {
            EditorMode::Normal => self.handle_normal_input(button),
            EditorMode::Insert => self.handle_insert_input(button),
            EditorMode::Find => self.handle_find_input(button),
            EditorMode::Replace => self.handle_replace_input(button),
            EditorMode::GoToLine | EditorMode::SaveAs => self.handle_prompt_input(button),
            EditorMode::Saving => self.handle_saving_input(button),
            EditorMode::ConfirmDiscard => self.handle_confirm_input(button),
        }
    }

    /// D-pad movement shared by Normal and Insert modes.
    fn dpad(&mut self, button: &Button) -> bool {
        match button {
            Button::Up => self.cursor_up(),
            Button::Down => self.cursor_down(),
            Button::Left => self.cursor_left(),
            Button::Right => self.cursor_right(),
            _ => return false,
        }
        true
    }

    fn handle_normal_input(&mut self, button: &Button) -> AppAction {
        if self.dpad(button) {
            return AppAction::None;
        }
        match button {
            Button::Confirm => {
                self.mode = EditorMode::Insert;
                self.status_message = Some("-- INSERT --".to_string());
                self.rebuild_display_lines();
            },
            Button::Triangle => self.enter_mode(EditorMode::Find),
            Button::Select => self.undo(),
            Button::Square => self.redo(),
            Button::Start => {
                self.mode = EditorMode::Saving;
                self.status_message = Some("Save? Confirm=Yes Cancel=No".to_string());
                self.rebuild_display_lines();
            },
            Button::Cancel => return self.request_close(PendingClose::Exit),
            _ => {},
        }
        AppAction::None
    }

    fn handle_insert_input(&mut self, button: &Button) -> AppAction {
        if self.dpad(button) {
            return AppAction::None;
        }
        match button {
            Button::Cancel => {
                self.mode = EditorMode::Normal;
                self.anchor = None;
                self.status_message = None;
                self.rebuild_display_lines();
            },
            Button::Confirm => self.new_line(),
            Button::Select => self.delete_char(),
            Button::Square => self.delete_forward(),
            _ => {},
        }
        AppAction::None
    }

    fn handle_find_input(&mut self, button: &Button) -> AppAction {
        match button {
            Button::Cancel => self.leave_mode(),
            Button::Confirm | Button::Square => self.run_find_next(),
            Button::Triangle => self.enter_mode(EditorMode::Replace),
            _ => {},
        }
        AppAction::None
    }

    fn handle_replace_input(&mut self, button: &Button) -> AppAction {
        match button {
            Button::Cancel => self.leave_mode(),
            Button::Confirm => self.run_replace_next(),
            Button::Square => self.run_replace_all(),
            Button::Triangle => {
                self.query_primed = false;
                self.replace_focus = !self.replace_focus;
                self.rebuild_display_lines();
            },
            _ => {},
        }
        AppAction::None
    }

    fn handle_prompt_input(&mut self, button: &Button) -> AppAction {
        match button {
            Button::Confirm => self.commit_prompt(),
            Button::Cancel => self.leave_mode(),
            _ => {},
        }
        AppAction::None
    }

    fn handle_saving_input(&mut self, button: &Button) -> AppAction {
        match button {
            Button::Confirm => self.save(),
            _ => {
                self.mode = EditorMode::Normal;
                self.status_message = None;
                self.rebuild_display_lines();
            },
        }
        AppAction::None
    }

    fn handle_confirm_input(&mut self, button: &Button) -> AppAction {
        match button {
            Button::Confirm => self.confirm_save(),
            Button::Square => return self.confirm_discard(),
            Button::Cancel => self.leave_mode(),
            _ => {},
        }
        AppAction::None
    }
}

#[cfg(test)]
mod tests {
    use oasis_app_core::{App, AppAction};
    use oasis_types::input::{Button, Key, Modifiers};
    use oasis_vfs::{MemoryVfs, Vfs};

    use crate::{EditorMode, FileType, TextEditorApp};

    const CTRL: Modifiers = Modifiers::CTRL;
    const SHIFT: Modifiers = Modifiers::SHIFT;
    const NONE: Modifiers = Modifiers::NONE;

    fn vfs() -> MemoryVfs {
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/docs").expect("mkdir");
        vfs
    }

    fn app(text: &str) -> TextEditorApp {
        let mut app = TextEditorApp::open_file("/docs/a.txt", text);
        app.content.cached_max_visible = 11; // 10-line page
        app
    }

    fn app_dirty() -> TextEditorApp {
        let mut app = app("x");
        type_str(&mut app, "y");
        assert!(app.is_modified());
        app
    }

    fn key(app: &mut TextEditorApp, k: Key, mods: Modifiers) -> Option<AppAction> {
        app.handle_key(&k, mods, &MemoryVfs::new())
    }

    /// Simulate a keyboard host typing `s`: Key, then its TextInput.
    fn type_str(app: &mut TextEditorApp, s: &str) {
        for ch in s.chars() {
            let k = if ch == ' ' { Key::Space } else { Key::Char(ch) };
            assert_eq!(key(app, k, NONE), None, "plain keys type via TextInput");
            app.handle_text_input(ch);
        }
    }

    fn text(app: &TextEditorApp) -> String {
        app.save_content()
    }

    // -- Advertised shortcuts --

    #[test]
    fn ctrl_s_saves_through_apply_vfs_ops() {
        let mut vfs = vfs();
        let mut app = app("hello");
        type_str(&mut app, "X");
        assert_eq!(key(&mut app, Key::Char('s'), CTRL), Some(AppAction::None));
        assert!(app.apply_vfs_ops(&mut vfs));
        assert_eq!(vfs.read("/docs/a.txt").expect("saved"), b"Xhello");
        assert!(!app.is_modified());
    }

    #[test]
    fn ctrl_n_clean_resets_and_dirty_prompts() {
        let mut app = app("keep");
        assert_eq!(key(&mut app, Key::Char('n'), CTRL), Some(AppAction::None));
        assert_eq!(text(&app), "");
        assert!(app.viewing_file().is_none());

        let mut app = app_dirty();
        key(&mut app, Key::Char('n'), CTRL);
        assert_eq!(app.mode(), EditorMode::ConfirmDiscard);
        assert_eq!(key(&mut app, Key::Char('d'), NONE), Some(AppAction::None));
        app.handle_text_input('d'); // swallowed TextInput twin
        assert_eq!(text(&app), "");
        assert_eq!(app.mode(), EditorMode::Normal);
    }

    #[test]
    fn ctrl_z_and_ctrl_y_undo_redo() {
        let mut app = app("");
        type_str(&mut app, "hello world");
        key(&mut app, Key::Char('z'), CTRL);
        assert_eq!(text(&app), "hello ");
        key(&mut app, Key::Char('z'), CTRL);
        assert_eq!(text(&app), "");
        key(&mut app, Key::Char('y'), CTRL);
        assert_eq!(text(&app), "hello ");
        key(&mut app, Key::Char('z'), CTRL | SHIFT);
        assert_eq!(text(&app), "hello world");
    }

    #[test]
    fn ctrl_f_finds_and_f3_advances() {
        let mut app = app("one two\ntwo three\nfour two");
        key(&mut app, Key::Char('f'), CTRL);
        assert_eq!(app.mode(), EditorMode::Find);
        type_str(&mut app, "two");
        key(&mut app, Key::Enter, NONE);
        assert_eq!(app.cursor_position(), (0, 4));
        assert_eq!(app.selected_text().as_deref(), Some("two"));
        key(&mut app, Key::F(3), NONE);
        assert_eq!(app.cursor_position(), (1, 0));
        key(&mut app, Key::Escape, NONE);
        assert_eq!(app.mode(), EditorMode::Normal);
        // F3 keeps working after the bar closes, and wraps.
        key(&mut app, Key::F(3), NONE);
        assert_eq!(app.cursor_position(), (2, 5));
        key(&mut app, Key::F(3), NONE);
        assert_eq!(app.cursor_position(), (0, 4));
    }

    #[test]
    fn reopened_find_replaces_the_old_query() {
        let mut app = app("alpha beta
beta alpha");
        key(&mut app, Key::Char('f'), CTRL);
        type_str(&mut app, "beta");
        key(&mut app, Key::Enter, NONE);
        key(&mut app, Key::Escape, NONE);
        app.motion(crate::editor::Motion::DocStart, false); // drop the selection
        // Ctrl+F again: the old query is shown selected, typing replaces it.
        key(&mut app, Key::Char('f'), CTRL);
        assert!(app.status_left(false).contains("Find: [beta]"));
        type_str(&mut app, "alpha");
        assert_eq!(app.find_query, "alpha");
        key(&mut app, Key::Enter, NONE);
        assert_eq!(app.selected_text().as_deref(), Some("alpha"));
        // Backspace on a primed query clears it whole.
        key(&mut app, Key::Escape, NONE);
        key(&mut app, Key::Char('f'), CTRL);
        app.handle_backspace();
        assert_eq!(app.find_query, "");
        // Once edited, typing appends as usual.
        type_str(&mut app, "al");
        type_str(&mut app, "pha");
        assert_eq!(app.find_query, "alpha");
    }

    #[test]
    fn undo_back_to_saved_text_is_clean() {
        let mut vfs = vfs();
        let mut app = app("base");
        type_str(&mut app, "X");
        assert!(app.is_modified());
        key(&mut app, Key::Char('z'), CTRL);
        assert_eq!(text(&app), "base");
        assert!(!app.is_modified(), "undo to the opened text is clean");
        key(&mut app, Key::Char('y'), CTRL);
        assert!(app.is_modified(), "redo away from it is dirty");
        // Save the edited text: undoing from there is dirty, redoing back
        // to it clean.
        key(&mut app, Key::Char('s'), CTRL);
        assert!(app.apply_vfs_ops(&mut vfs));
        assert!(!app.is_modified());
        key(&mut app, Key::Char('z'), CTRL);
        assert!(app.is_modified());
        key(&mut app, Key::Char('y'), CTRL);
        assert!(!app.is_modified());
        // A clean document closes without the unsaved-changes prompt.
        app.handle_input(&Button::Cancel, &vfs); // Insert -> Normal
        assert_eq!(app.handle_input(&Button::Cancel, &vfs), AppAction::Exit);
    }

    #[test]
    fn ctrl_h_replace_next_and_replace_all() {
        let mut app = app("cat cat\ncat");
        key(&mut app, Key::Char('h'), CTRL);
        assert_eq!(app.mode(), EditorMode::Replace);
        type_str(&mut app, "cat");
        key(&mut app, Key::Tab, NONE);
        type_str(&mut app, "dog");
        // First Enter selects the first match, second replaces it.
        key(&mut app, Key::Enter, NONE);
        key(&mut app, Key::Enter, NONE);
        assert_eq!(text(&app), "dog cat\ncat");
        // Ctrl+Enter replaces the rest in one undo step.
        key(&mut app, Key::Enter, CTRL);
        assert_eq!(text(&app), "dog dog\ndog");
        app.undo();
        assert_eq!(text(&app), "dog cat\ncat");
    }

    #[test]
    fn replace_all_handles_replacement_containing_query() {
        let mut app = app("a a");
        app.find_query = "a".into();
        app.replace_text = "aa".into();
        assert_eq!(app.replace_all(), 2);
        assert_eq!(text(&app), "aa aa");
    }

    #[test]
    fn ctrl_g_goes_to_line() {
        let mut app = app("1\n2\n3\n4\n5");
        key(&mut app, Key::Char('g'), CTRL);
        assert_eq!(app.mode(), EditorMode::GoToLine);
        type_str(&mut app, "4x");
        key(&mut app, Key::Enter, NONE);
        assert_eq!(app.mode(), EditorMode::Normal);
        assert_eq!(app.cursor_position(), (3, 0));
        // Out-of-range lines clamp to the last line.
        app.go_to_line(99);
        assert_eq!(app.cursor_position(), (4, 0));
    }

    #[test]
    fn menu_shortcuts_all_reach_handlers() {
        // Every shortcut advertised in the menu is claimed by handle_key.
        let mut app = app("abc");
        for c in ['n', 's', 'z', 'y', 'f', 'h', 'g', 'a', 'c', 'x', 'v'] {
            app.mode = EditorMode::Normal;
            app.modified = false;
            assert!(key(&mut app, Key::Char(c), CTRL).is_some(), "Ctrl+{c}");
        }
        app.mode = EditorMode::Normal;
        assert!(key(&mut app, Key::Char('s'), CTRL | SHIFT).is_some());
        assert_eq!(app.mode(), EditorMode::SaveAs);
    }

    // -- Save As --

    #[test]
    fn untitled_ctrl_s_opens_save_as_prompt() {
        let mut vfs = vfs();
        let mut app = TextEditorApp::new("/apps/editor");
        type_str(&mut app, "int x;");
        key(&mut app, Key::Char('s'), CTRL);
        assert_eq!(app.mode(), EditorMode::SaveAs);
        // Clear the suggested name and type a new one.
        while !app.prompt_input.is_empty() {
            app.handle_backspace();
        }
        type_str(&mut app, "/docs/new.c");
        key(&mut app, Key::Enter, NONE);
        assert_eq!(app.viewing_file(), None, "path switches after the write");
        assert!(app.apply_vfs_ops(&mut vfs));
        assert_eq!(app.viewing_file(), Some("/docs/new.c"));
        assert_eq!(app.file_type, FileType::C);
        assert!(app.title().contains("new.c"));
        assert_eq!(vfs.read("/docs/new.c").expect("saved"), b"int x;");
    }

    #[test]
    fn save_as_relative_path_uses_current_directory() {
        let mut vfs = vfs();
        let mut app = app("data");
        assert!(app.save_as("copy.txt"));
        app.apply_vfs_ops(&mut vfs);
        assert_eq!(app.viewing_file(), Some("/docs/copy.txt"));
        assert_eq!(vfs.read("/docs/copy.txt").expect("saved"), b"data");
    }

    #[test]
    fn save_failure_is_reported_and_keeps_modified() {
        let mut vfs = vfs();
        let mut app = app_dirty();
        app.save_as("/missing/dir/x.txt");
        app.apply_vfs_ops(&mut vfs);
        assert!(app.is_modified());
        let status = app.status_message.clone().unwrap_or_default();
        assert!(status.contains("Save failed"), "{status}");
    }

    #[test]
    fn failed_save_as_keeps_the_original_path() {
        let mut vfs = vfs();
        let mut app = TextEditorApp::open_file("/docs/a.c", "int x;");
        app.save_as("/missing/dir/x.txt");
        app.apply_vfs_ops(&mut vfs);
        assert_eq!(app.viewing_file(), Some("/docs/a.c"));
        assert_eq!(app.file_type, FileType::C);
        assert_eq!(app.content.title, "Text Editor - a.c");
    }

    #[test]
    fn untitled_relative_save_as_goes_to_home_user() {
        let mut vfs = vfs();
        vfs.mkdir("/home/user").expect("mkdir");
        let mut app = TextEditorApp::new("/apps/editor");
        app.insert_char('z');
        assert!(app.save_as("notes.txt"));
        app.apply_vfs_ops(&mut vfs);
        assert_eq!(app.viewing_file(), Some("/home/user/notes.txt"));
        assert_eq!(vfs.read("/home/user/notes.txt").expect("saved"), b"z");
    }

    #[test]
    fn crlf_files_are_saved_as_crlf() {
        let mut vfs = vfs();
        vfs.write("/docs/w.txt", b"one\r\ntwo\r\n").expect("write");
        let mut app = TextEditorApp::open_from_vfs("/docs/w.txt", &vfs);
        app.insert_char('!');
        app.save();
        app.apply_vfs_ops(&mut vfs);
        assert_eq!(vfs.read("/docs/w.txt").expect("read"), b"!one\r\ntwo\r\n");
    }

    #[test]
    fn non_utf8_files_open_read_only() {
        let mut vfs = vfs();
        let original = b"caf\xe9\n".to_vec();
        vfs.write("/docs/latin1.txt", &original).expect("write");
        let mut app = TextEditorApp::open_from_vfs("/docs/latin1.txt", &vfs);
        app.insert_char('x');
        app.save();
        assert!(!app.apply_vfs_ops(&mut vfs), "in-place save refused");
        assert_eq!(vfs.read("/docs/latin1.txt").expect("read"), original);
        let status = app.status_message.clone().unwrap_or_default();
        assert!(status.contains("Read-only"), "{status}");
        assert!(!app.save_as("/docs/latin1.txt"), "same path refused too");
        // Save As to another path writes a copy and leaves read-only mode.
        assert!(app.save_as("/docs/copy.txt"));
        assert!(app.apply_vfs_ops(&mut vfs));
        assert!(vfs.exists("/docs/copy.txt"));
        app.save();
        assert!(app.apply_vfs_ops(&mut vfs), "the copy is writable");
        assert_eq!(vfs.read("/docs/latin1.txt").expect("read"), original);
    }

    // -- Selection + clipboard --

    #[test]
    fn shift_arrows_select_and_copy_paste() {
        let mut app = app("hello world");
        key(&mut app, Key::Right, SHIFT);
        key(&mut app, Key::Right, SHIFT);
        assert_eq!(app.selected_text().as_deref(), Some("he"));
        key(&mut app, Key::Char('c'), CTRL);
        key(&mut app, Key::End, NONE);
        assert!(app.selection().is_none(), "plain motion clears selection");
        key(&mut app, Key::Char('v'), CTRL);
        assert_eq!(text(&app), "hello worldhe");
        // Shift+Home selects back to the line start.
        key(&mut app, Key::Home, SHIFT);
        assert_eq!(app.selected_text().as_deref(), Some("hello worldhe"));
    }

    #[test]
    fn shift_down_selects_across_lines_and_cut_removes() {
        let mut app = app("ab\ncd\nef");
        key(&mut app, Key::Right, NONE);
        key(&mut app, Key::Down, SHIFT);
        assert_eq!(app.selected_text().as_deref(), Some("b\nc"));
        key(&mut app, Key::Char('x'), CTRL);
        assert_eq!(text(&app), "ad\nef");
        key(&mut app, Key::Char('v'), CTRL);
        assert_eq!(text(&app), "ab\ncd\nef");
        // Paste is a single undo step.
        app.undo();
        assert_eq!(text(&app), "ad\nef");
    }

    #[test]
    fn select_all_then_typing_replaces_everything() {
        let mut app = app("one\ntwo");
        key(&mut app, Key::Char('a'), CTRL);
        assert_eq!(app.selected_text().as_deref(), Some("one\ntwo"));
        type_str(&mut app, "z");
        assert_eq!(text(&app), "z");
        app.undo();
        assert_eq!(text(&app), "one\ntwo");
    }

    #[test]
    fn backspace_and_delete_remove_selection() {
        let mut app = app("abcdef");
        key(&mut app, Key::Right, SHIFT);
        key(&mut app, Key::Right, SHIFT);
        app.handle_backspace();
        assert_eq!(text(&app), "cdef");
        key(&mut app, Key::End, SHIFT);
        key(&mut app, Key::Delete, NONE);
        assert_eq!(text(&app), "");
    }

    // -- Navigation --

    #[test]
    fn delete_home_end_keys() {
        let mut app = app("  indented\nx");
        key(&mut app, Key::End, NONE);
        assert_eq!(app.cursor_position(), (0, 10));
        key(&mut app, Key::Home, NONE);
        assert_eq!(app.cursor_position(), (0, 2), "smart home: first non-blank");
        key(&mut app, Key::Home, NONE);
        assert_eq!(app.cursor_position(), (0, 0));
        key(&mut app, Key::Delete, NONE);
        assert_eq!(text(&app), " indented\nx");
        key(&mut app, Key::End, NONE);
        key(&mut app, Key::Delete, NONE);
        assert_eq!(text(&app), " indentedx", "Delete at EOL joins lines");
    }

    #[test]
    fn page_keys_and_ctrl_home_end() {
        let body: Vec<String> = (1..=50).map(|i| format!("line {i}")).collect();
        let mut app = app(&body.join("\n"));
        key(&mut app, Key::PageDown, NONE);
        assert_eq!(app.cursor_position().0, 10);
        assert_eq!(app.content.scroll, 10);
        key(&mut app, Key::PageDown, NONE);
        assert_eq!(app.cursor_position().0, 20);
        key(&mut app, Key::PageUp, NONE);
        assert_eq!(app.cursor_position().0, 10);
        key(&mut app, Key::End, CTRL);
        assert_eq!(app.cursor_position(), (49, 7));
        assert!(
            app.content.scroll + 10 > 49,
            "cursor visible after Ctrl+End"
        );
        key(&mut app, Key::Home, CTRL | SHIFT);
        assert_eq!(app.cursor_position(), (0, 0));
        assert_eq!(app.selected_text().map(|s| s.lines().count()), Some(50));
    }

    #[test]
    fn ctrl_left_right_jump_words() {
        let mut app = app("foo_bar  baz.qux\nnext");
        key(&mut app, Key::Right, CTRL);
        assert_eq!(app.cursor_position(), (0, 7));
        key(&mut app, Key::Right, CTRL);
        assert_eq!(app.cursor_position(), (0, 12));
        key(&mut app, Key::Right, CTRL);
        assert_eq!(app.cursor_position(), (0, 16));
        key(&mut app, Key::Right, CTRL);
        assert_eq!(app.cursor_position(), (1, 0), "wraps to next line");
        key(&mut app, Key::Left, CTRL);
        assert_eq!(app.cursor_position(), (0, 16));
        key(&mut app, Key::Left, CTRL);
        assert_eq!(app.cursor_position(), (0, 13));
        key(&mut app, Key::Left, CTRL);
        assert_eq!(app.cursor_position(), (0, 9));
        key(&mut app, Key::Backspace, CTRL);
        assert_eq!(text(&app), "baz.qux\nnext");
    }

    // -- Undo grouping --

    #[test]
    fn backspace_run_is_one_undo_step() {
        let mut app = app("hello");
        key(&mut app, Key::End, NONE);
        for _ in 0..3 {
            app.handle_backspace();
        }
        assert_eq!(text(&app), "he");
        app.undo();
        assert_eq!(text(&app), "hello");
        assert_eq!(app.cursor_position(), (0, 5));
    }

    #[test]
    fn cursor_motion_breaks_typing_group() {
        let mut app = app("");
        type_str(&mut app, "ab");
        key(&mut app, Key::Left, NONE);
        type_str(&mut app, "X");
        assert_eq!(text(&app), "aXb");
        app.undo();
        assert_eq!(text(&app), "ab");
        app.undo();
        assert_eq!(text(&app), "");
    }

    #[test]
    fn enter_key_inserts_newline_with_indent() {
        let mut app = app("    code");
        key(&mut app, Key::End, NONE);
        assert_eq!(key(&mut app, Key::Enter, NONE), Some(AppAction::None));
        assert_eq!(text(&app), "    code\n    ");
        assert_eq!(app.cursor_position(), (1, 4));
        assert_eq!(app.mode(), EditorMode::Insert);
    }

    #[test]
    fn multibyte_typing_and_motion_do_not_panic() {
        let mut app = app("");
        type_str(&mut app, "héllo ✓");
        assert_eq!(text(&app), "héllo ✓");
        key(&mut app, Key::Left, NONE);
        key(&mut app, Key::Left, SHIFT);
        assert_eq!(app.selected_text().as_deref(), Some(" "));
        key(&mut app, Key::Home, NONE);
        key(&mut app, Key::Right, NONE);
        key(&mut app, Key::Right, NONE);
        assert_eq!(app.cursor_position(), (0, 3));
        app.handle_backspace();
        assert_eq!(text(&app), "hllo ✓");
    }

    // -- Unsaved-changes prompt --

    #[test]
    fn exit_with_unsaved_changes_prompts_and_cancel_returns() {
        let vfs = vfs();
        let mut app = app_dirty();
        app.mode = EditorMode::Normal;
        assert_eq!(app.handle_input(&Button::Cancel, &vfs), AppAction::None);
        assert_eq!(app.mode(), EditorMode::ConfirmDiscard);
        assert_eq!(key(&mut app, Key::Escape, NONE), Some(AppAction::None));
        assert_eq!(app.mode(), EditorMode::Normal);
        assert_eq!(text(&app), "yx");
    }

    #[test]
    fn exit_prompt_discard_exits() {
        let vfs = vfs();
        let mut app = app_dirty();
        app.mode = EditorMode::Normal;
        app.handle_input(&Button::Cancel, &vfs);
        assert_eq!(key(&mut app, Key::Char('d'), NONE), Some(AppAction::Exit));
        // Gamepad: Square discards.
        let mut app = app_dirty();
        app.mode = EditorMode::Normal;
        app.handle_input(&Button::Cancel, &vfs);
        assert_eq!(app.handle_input(&Button::Square, &vfs), AppAction::Exit);
    }

    #[test]
    fn exit_prompt_save_writes_then_closes() {
        let mut vfs = vfs();
        let mut app = app_dirty();
        app.mode = EditorMode::Normal;
        app.handle_input(&Button::Cancel, &vfs);
        assert_eq!(key(&mut app, Key::Char('s'), NONE), Some(AppAction::None));
        app.handle_text_input('s'); // the swallowed TextInput twin
        assert!(!app.close_requested());
        assert!(!app.take_close_request(), "no close before the write");
        assert!(app.apply_vfs_ops(&mut vfs));
        assert_eq!(vfs.read("/docs/a.txt").expect("saved"), b"yx");
        assert!(app.close_requested());
        // The host's per-frame poll sees the close right after the write,
        // without waiting for another input event; the request is consumed.
        assert!(app.take_close_request());
        assert!(!app.take_close_request());
        assert_eq!(app.handle_input(&Button::Up, &vfs), AppAction::Exit);
    }

    #[test]
    fn exit_prompt_save_untitled_goes_through_save_as() {
        let mut vfs = vfs();
        let mut app = TextEditorApp::new("/apps/editor");
        type_str(&mut app, "hi");
        app.mode = EditorMode::Normal;
        app.handle_input(&Button::Cancel, &vfs);
        app.handle_input(&Button::Confirm, &vfs);
        assert_eq!(app.mode(), EditorMode::SaveAs);
        app.prompt_input = "/docs/hi.txt".into();
        app.handle_input(&Button::Confirm, &vfs);
        app.apply_vfs_ops(&mut vfs);
        assert!(app.close_requested());
        assert_eq!(vfs.read("/docs/hi.txt").expect("saved"), b"hi");
    }

    #[test]
    fn clean_exit_needs_no_prompt() {
        let vfs = vfs();
        let mut app = app("clean");
        assert_eq!(app.handle_input(&Button::Cancel, &vfs), AppAction::Exit);
    }

    // -- Gamepad reachability --

    #[test]
    fn gamepad_find_replace_flow() {
        let vfs = vfs();
        let mut app = app("a b a");
        app.handle_input(&Button::Triangle, &vfs);
        assert_eq!(app.mode(), EditorMode::Find);
        app.handle_text_input('a');
        app.handle_input(&Button::Triangle, &vfs);
        assert_eq!(app.mode(), EditorMode::Replace);
        app.handle_input(&Button::Triangle, &vfs); // focus replacement
        app.handle_text_input('c');
        app.handle_input(&Button::Square, &vfs); // replace all
        assert_eq!(text(&app), "c b c");
    }
}
