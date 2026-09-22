//! Text editor application with modal editing, selection, clipboard,
//! grouped undo/redo, find/replace and Save As.
//!
//! `TextEditorApp` is a Notepad-style editor. Keyboard hosts drive it
//! through `App::handle_key` (Ctrl+N/S/Shift+S/Z/Y/X/C/V/A/F/H/G, arrows
//! with Shift selection, Ctrl word jumps, Home/End, PageUp/PageDown,
//! Delete); gamepad hosts reach every feature through `handle_input`
//! (Normal / Insert / Find / Replace / prompt modes).

use std::any::Any;
use std::cell::Cell;

use oasis_app_core::render::{hide_app_sdi, render_app_chrome};
use oasis_app_core::{App, AppAction, ContentState};
use oasis_sdi::SdiRegistry;
use oasis_skin::ActiveTheme;
use oasis_types::backend::{InMemoryClipboard, SdiBackend};
use oasis_types::input::{Button, Key, Modifiers};
use oasis_ui::menu_bar::{Menu, MenuBar, MenuEntry, MenuHit};
use oasis_vfs::Vfs;

pub mod buffer;
mod cache;
pub mod colors;
mod editor;
pub mod highlight;
mod input;
mod render;

pub use buffer::{EditOperation, EditorBuffer, Pos};
pub use highlight::{FileType, detect_file_type};
pub use render::hide_notepad_sdi_objects;

use editor::UndoGroup;

// ---------------------------------------------------------------
// EditorMode
// ---------------------------------------------------------------

/// Modal editing mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorMode {
    /// Viewing and cursor movement.
    Normal,
    /// Typing text.
    Insert,
    /// Find bar active (typing edits the query).
    Find,
    /// Find + replace bar active (Tab / Triangle switches field).
    Replace,
    /// "Go to line" prompt.
    GoToLine,
    /// "Save as" path prompt.
    SaveAs,
    /// Save confirmation pending.
    Saving,
    /// "Unsaved changes: Save / Discard / Cancel" prompt.
    ConfirmDiscard,
}

/// What to do once unsaved changes are resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PendingClose {
    /// Close the editor.
    Exit,
    /// Start a new, empty document.
    New,
}

// ---------------------------------------------------------------
// TextEditorApp
// ---------------------------------------------------------------

/// Text editor application with modal editing and undo/redo.
#[derive(Debug)]
pub struct TextEditorApp {
    pub(crate) content: ContentState,
    pub(crate) buffer: EditorBuffer,
    pub(crate) mode: EditorMode,
    pub(crate) cursor_line: usize,
    /// Byte column, always on a character boundary.
    pub(crate) cursor_col: usize,
    /// Selection anchor; the selection spans anchor..cursor.
    pub(crate) anchor: Option<Pos>,
    pub(crate) scroll_x: usize,
    pub(crate) file_path: Option<String>,
    pub(crate) modified: bool,
    pub(crate) status_message: Option<String>,
    pub(crate) find_query: String,
    pub(crate) replace_text: String,
    /// In Replace mode: typing goes to the replacement (vs. the query).
    pub(crate) replace_focus: bool,
    pub(crate) find_active: bool,
    /// Text of the Go-to-line / Save-as prompt.
    pub(crate) prompt_input: String,
    pub(crate) undo_stack: Vec<UndoGroup>,
    pub(crate) redo_stack: Vec<UndoGroup>,
    /// Whether the newest undo group may still absorb typing.
    pub(crate) group_open: bool,
    pub(crate) clipboard: InMemoryClipboard,
    pub(crate) file_type: FileType,
    /// Queued write `(path, data, buffer generation)`, flushed by
    /// `apply_vfs_ops`.
    pub(crate) pending_save: Option<(String, String, u64)>,
    /// Close / New waiting on the unsaved-changes prompt or a save.
    pub(crate) pending_close: Option<PendingClose>,
    /// Set once "Save & close" finished; every input hook returns Exit.
    pub(crate) close_requested: bool,
    /// Whether `close_requested` was already handed to the host via
    /// `App::take_close_request`.
    pub(crate) close_reported: bool,
    /// A printable key consumed by `handle_key`; its `TextInput` twin
    /// (which hosts still deliver) must not also type.
    pub(crate) swallow_text: Option<char>,
    /// Text lines that fit the last rendered viewport (0 = not drawn yet).
    pub(crate) viewport_lines: Cell<usize>,
    /// Row height of the last rendered viewport (0 = not drawn yet).
    pub(crate) viewport_line_h: Cell<i32>,
    /// Top menu bar widget (File / Edit / View / Help) with drop-downs.
    pub(crate) menu: MenuBar,
}

/// Build the default Text-Editor menu.
fn default_menu_bar() -> MenuBar {
    MenuBar::new(vec![
        Menu::new(
            "File",
            vec![
                MenuEntry::action("New", "file.new").with_shortcut("Ctrl+N"),
                MenuEntry::action("Save", "file.save").with_shortcut("Ctrl+S"),
                MenuEntry::action("Save As…", "file.save_as").with_shortcut("Ctrl+Shift+S"),
                MenuEntry::Separator,
                MenuEntry::action("Exit", "file.exit").with_shortcut("Esc"),
            ],
        ),
        Menu::new(
            "Edit",
            vec![
                MenuEntry::action("Undo", "edit.undo").with_shortcut("Ctrl+Z"),
                MenuEntry::action("Redo", "edit.redo").with_shortcut("Ctrl+Y"),
                MenuEntry::Separator,
                MenuEntry::action("Cut", "edit.cut").with_shortcut("Ctrl+X"),
                MenuEntry::action("Copy", "edit.copy").with_shortcut("Ctrl+C"),
                MenuEntry::action("Paste", "edit.paste").with_shortcut("Ctrl+V"),
                MenuEntry::action("Select All", "edit.select_all").with_shortcut("Ctrl+A"),
                MenuEntry::Separator,
                MenuEntry::action("Find…", "edit.find").with_shortcut("Ctrl+F"),
                MenuEntry::action("Find Next", "edit.find_next").with_shortcut("F3"),
                MenuEntry::action("Replace…", "edit.replace").with_shortcut("Ctrl+H"),
                MenuEntry::action("Go To Line…", "edit.goto").with_shortcut("Ctrl+G"),
            ],
        ),
        Menu::new("View", vec![MenuEntry::action("Status Bar", "view.status")]),
        Menu::new("Help", vec![MenuEntry::action("About", "help.about")]),
    ])
}

impl TextEditorApp {
    fn with_state(content: ContentState, buffer: EditorBuffer, path: Option<&str>) -> Self {
        let mut editor = Self {
            content,
            buffer,
            mode: EditorMode::Normal,
            cursor_line: 0,
            cursor_col: 0,
            anchor: None,
            scroll_x: 0,
            file_path: path.map(String::from),
            modified: false,
            status_message: None,
            find_query: String::new(),
            replace_text: String::new(),
            replace_focus: false,
            find_active: false,
            prompt_input: String::new(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            group_open: false,
            clipboard: InMemoryClipboard::new(),
            file_type: path.map_or(FileType::Plain, detect_file_type),
            pending_save: None,
            pending_close: None,
            close_requested: false,
            close_reported: false,
            swallow_text: None,
            viewport_lines: Cell::new(0),
            viewport_line_h: Cell::new(0),
            menu: default_menu_bar(),
        };
        editor.rebuild_display_lines();
        editor
    }

    /// Create a new empty text editor.
    pub fn new(path: &str) -> Self {
        let content = ContentState::new("Text Editor", path);
        Self::with_state(content, EditorBuffer::new(), None)
    }

    /// Open a file from the VFS. If the file doesn't exist or can't be
    /// read, returns an editor with a status message rather than panicking.
    pub fn open_from_vfs(path: &str, vfs: &dyn Vfs) -> Self {
        match vfs.read(path) {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes).into_owned();
                Self::open_file(path, &text)
            },
            Err(e) => {
                let mut editor = Self::new("/apps/editor");
                editor.status_message = Some(format!("Could not read {path}: {e}"));
                editor
            },
        }
    }

    /// Open a file with the given content.
    pub fn open_file(path: &str, content: &str) -> Self {
        let file_name = path.rsplit('/').next().unwrap_or(path);
        let title = format!("Text Editor - {file_name}");
        let cs = ContentState::new(&title, "/apps/editor");
        Self::with_state(cs, EditorBuffer::from_text(content), Some(path))
    }

    /// Execute a menu action by its registered id string. Returns an
    /// `AppAction` so the host can act on exit requests.
    pub(crate) fn run_menu_action(&mut self, id: &str) -> AppAction {
        match id {
            "file.new" => return self.request_close(PendingClose::New),
            "file.save" => self.save(),
            "file.save_as" => self.enter_mode(EditorMode::SaveAs),
            "file.exit" => return self.request_close(PendingClose::Exit),
            "edit.undo" => self.undo(),
            "edit.redo" => self.redo(),
            "edit.cut" => {
                self.cut();
            },
            "edit.copy" => {
                self.copy();
            },
            "edit.paste" => {
                self.paste();
            },
            "edit.select_all" => self.select_all(),
            "edit.find" => self.enter_mode(EditorMode::Find),
            "edit.find_next" => {
                self.find_next();
            },
            "edit.replace" => self.enter_mode(EditorMode::Replace),
            "edit.goto" => self.enter_mode(EditorMode::GoToLine),
            "view.status" => {
                self.status_message = Some("Status bar is always visible.".into());
                self.rebuild_display_lines();
            },
            "help.about" => {
                self.status_message =
                    Some("OASIS_OS Text Editor — Ctrl+S save, Ctrl+F find, Ctrl+H replace.".into());
                self.rebuild_display_lines();
            },
            _ => {},
        }
        AppAction::None
    }
}

// ---------------------------------------------------------------
// App trait implementation
// ---------------------------------------------------------------

impl App for TextEditorApp {
    fn title(&self) -> &str {
        &self.content.title
    }

    fn path(&self) -> &str {
        &self.content.app_path
    }

    fn handle_input(&mut self, button: &Button, _vfs: &dyn Vfs) -> AppAction {
        // An open drop-down absorbs every key press — it's a modal
        // widget. Any button closes it (Win95-style: Esc cancels,
        // anything else is treated as "cancel and continue").
        if self.menu.is_open() {
            self.menu.close();
            return AppAction::None;
        }
        self.handle_button(button)
    }

    fn handle_key(&mut self, key: &Key, mods: Modifiers, _vfs: &dyn Vfs) -> Option<AppAction> {
        self.swallow_text = None;
        let result = self.handle_key_impl(key, mods);
        if result.is_some() && !mods.has_command() {
            // The host still delivers the key's TextInput after a consumed
            // printable key (e.g. "s" in the unsaved-changes prompt).
            self.swallow_text = match key {
                Key::Char(c) => Some(*c),
                Key::Space => Some(' '),
                _ => None,
            };
        }
        result
    }

    fn handle_text_input(&mut self, ch: char) {
        if let Some(swallow) = self.swallow_text.take()
            && swallow.eq_ignore_ascii_case(&ch)
        {
            return;
        }
        // Typing while a drop-down is open cancels it rather than
        // inserting a glyph behind the menu.
        if self.menu.is_open() {
            self.menu.close();
            return;
        }
        // Non-printable control characters (Enter, Tab, etc. that
        // already come through as ButtonPress / Key events) are filtered
        // out so they don't produce spurious glyphs in the buffer.
        if ch.is_control() {
            return;
        }
        self.handle_text_impl(ch);
    }

    fn accepts_text(&self) -> bool {
        // Typing is accepted in every mode (Normal auto-enters Insert), so
        // printable keys must never double as Triangle/trigger shortcuts.
        true
    }

    fn handle_backspace(&mut self) {
        self.handle_backspace_impl();
    }

    fn handle_click(&mut self, lx: i32, ly: i32, cw: u32, ch: u32, _fullscreen: bool) -> AppAction {
        if self.close_requested {
            return AppAction::Exit;
        }
        // Layout constants — must mirror `draw_notepad`: the menu bar sits
        // at the very top of the content area (no inner title bar — the WM
        // titlebar shows the app title), text area below it, status strip
        // at the bottom.
        let menu_h = 18i32;
        let menu_y = 0i32;
        let area_top = menu_h;
        let status_h = 18i32;
        let area_bottom = ch as i32 - status_h;

        // Route first through the menu-bar widget: clicks on labels
        // toggle the drop-down, clicks on drop-down items dispatch an
        // action, anything else just closes an open drop-down.
        let hit = self.menu.hit_test(lx, ly, 0, menu_y, cw, menu_h as u32);
        match hit {
            MenuHit::Label(i) => {
                if self.menu.open == Some(i) {
                    self.menu.close();
                } else {
                    self.menu.open = Some(i);
                    self.menu.hovered_item = None;
                }
                return AppAction::None;
            },
            MenuHit::Item { id } => {
                self.menu.close();
                return self.run_menu_action(&id);
            },
            MenuHit::NoOp => {
                // Click landed on a separator / disabled item — keep
                // the drop-down open.
                return AppAction::None;
            },
            MenuHit::Outside => {
                // If a drop-down was open, close it now and don't
                // also move the caret — avoids the user accidentally
                // typing where the menu just was.
                if self.menu.is_open() {
                    self.menu.close();
                    return AppAction::None;
                }
            },
        }

        // Clicks in the title / menu-bar / status strips outside the
        // menu labels are ignored, as are clicks while a prompt is up.
        if ly < area_top || ly >= area_bottom || self.mode == EditorMode::ConfirmDiscard {
            return AppAction::None;
        }

        // Clicks in the text area: place the caret at the nearest
        // character. Approximate 7px per glyph for the 12px bitmap
        // font; we don't have a backend here to call `measure_text`.
        let pad_left = 8i32;
        let pad_top = 6i32;
        let line_h = match self.viewport_line_h.get() {
            0 => 14,
            h => h,
        };
        let relative_y = ly - area_top - pad_top;
        let clicked_line = (relative_y / line_h).max(0) as usize;
        let target_line =
            (self.content.scroll + clicked_line).min(self.buffer.line_count().saturating_sub(1));
        let line_text = self.buffer.get_line(target_line).unwrap_or("");
        let approx_col = ((lx - pad_left - 8).max(0) / 7) as usize;
        let target_col = line_text
            .char_indices()
            .nth(approx_col)
            .map_or(line_text.len(), |(i, _)| i);

        if matches!(self.mode, EditorMode::Normal | EditorMode::Saving) {
            self.mode = EditorMode::Insert;
        }
        self.move_to((target_line, target_col), false);
        AppAction::None
    }

    fn update_sdi(&mut self, sdi: &mut SdiRegistry, at: &ActiveTheme) {
        self.content.title = self.build_title();
        self.content.update_layout(at);
        self.content.animate_selection(0.3);
        render_app_chrome(sdi, at);
        // Render Notepad-style chrome (menu bar, text area, status bar)
        // and our own per-line SDI objects. We deliberately do NOT call
        // `render_content_sdi` — its listing-style output is wrong for
        // an editor and would leave a stale "> 1 | text" row visible.
        self.render_notepad_sdi(sdi, at);
    }

    fn hide_sdi(&self, sdi: &mut SdiRegistry) {
        hide_app_sdi(sdi);
        self.hide_notepad_sdi(sdi);
    }

    fn draw_windowed(
        &self,
        cx: i32,
        cy: i32,
        cw: u32,
        ch: u32,
        backend: &mut dyn SdiBackend,
        at: &ActiveTheme,
    ) -> oasis_types::error::Result<()> {
        // Windows-Notepad-style GUI: menu bar, text area, status bar.
        // The (shared with `draw_highlighted`) syntax-highlighting
        // pipeline is used for known file types; plain files render
        // with a single foreground colour.
        self.draw_notepad(cx, cy, cw, ch, backend, at)
    }

    fn take_pending_request(&mut self) -> Option<(String, String)> {
        self.content.pending_vfs_request.take()
    }

    fn peek_pending_request(&self) -> Option<&(String, String)> {
        self.content.pending_vfs_request.as_ref()
    }

    fn apply_vfs_ops(&mut self, vfs: &mut dyn Vfs) -> bool {
        self.flush_save(vfs)
    }

    fn take_close_request(&mut self) -> bool {
        // "Save & close" completes in `apply_vfs_ops`; the host polls this
        // right after, so the editor closes the same frame the file is
        // written instead of waiting for the next input event.
        self.close_requested && !std::mem::replace(&mut self.close_reported, true)
    }

    fn lines(&self) -> &[String] {
        &self.content.lines
    }

    fn viewing_file(&self) -> Option<&str> {
        self.file_path.as_deref()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ---------------------------------------------------------------
// Tests
// ---------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::EditorBuffer;
    use super::EditorMode;
    use super::TextEditorApp;
    use oasis_app_core::App;
    use oasis_app_core::AppAction;
    use oasis_sdi::SdiRegistry;
    use oasis_skin::ActiveTheme;
    use oasis_types::input::Button;
    use oasis_vfs::MemoryVfs;

    fn make_vfs() -> MemoryVfs {
        MemoryVfs::new()
    }

    /// Plain character input through the `handle_text_input` path
    /// must reach the buffer — this was the core "only Enter types"
    /// bug. Typing in Normal mode also auto-drops into Insert mode.
    #[test]
    fn text_input_inserts_chars_and_enters_insert_mode() {
        let mut app = TextEditorApp::new("/apps/editor");
        assert_eq!(app.mode, EditorMode::Normal);
        app.handle_text_input('h');
        app.handle_text_input('i');
        assert_eq!(app.mode, EditorMode::Insert);
        assert_eq!(app.buffer.get_line(0), Some("hi"));
        assert_eq!(app.cursor_col, 2);
    }

    /// Control characters (Tab, newline, etc.) must not produce
    /// stray glyphs in the buffer — they're handled through the
    /// ButtonPress path.
    #[test]
    fn text_input_ignores_control_chars() {
        let mut app = TextEditorApp::new("/apps/editor");
        app.handle_text_input('\n');
        app.handle_text_input('\t');
        assert_eq!(app.buffer.get_line(0), Some(""));
    }

    /// Backspace via the text-input channel must delete the last
    /// character, matching Notepad behaviour when a user presses
    /// backspace after typing.
    #[test]
    fn backspace_deletes_in_insert_mode() {
        let mut app = TextEditorApp::new("/apps/editor");
        app.handle_text_input('a');
        app.handle_text_input('b');
        app.handle_text_input('c');
        app.handle_backspace();
        assert_eq!(app.buffer.get_line(0), Some("ab"));
    }

    /// Clicking inside the text area places the caret at the
    /// clicked position and switches to Insert mode so the next
    /// keystroke types there.
    #[test]
    fn click_in_text_area_places_cursor_and_enters_insert() {
        let mut app = TextEditorApp::open_file("/test.txt", "hello world");
        // Click roughly at column 6 in the first line. Area top is
        // menu_h = 18 (menu bar sits at the top of the content area),
        // so ly = 18 + 6 (pad) + 2 puts us on line 0.
        let action = app.handle_click(8 + 6 * 7, 26, 400, 300, false);
        assert_eq!(action, AppAction::None);
        assert_eq!(app.mode, EditorMode::Insert);
        assert_eq!(app.cursor_line, 0);
        assert!(app.cursor_col > 0 && app.cursor_col <= 11);
    }

    /// Clicking on a menu label opens that menu's drop-down. Clicking
    /// the same label again closes it.
    #[test]
    fn click_on_menu_label_toggles_dropdown() {
        let mut app = TextEditorApp::new("/apps/editor");
        assert!(!app.menu.is_open());
        // "File" label at x=6, menu strip y=0..18 so ly=8 hits it.
        let _ = app.handle_click(10, 8, 400, 300, false);
        assert_eq!(app.menu.open, Some(0), "File dropdown should be open");
        // Clicking File again toggles it closed.
        let _ = app.handle_click(10, 8, 400, 300, false);
        assert!(!app.menu.is_open(), "second click should close");
    }

    /// Clicking a File > Save item runs the save action, closes the
    /// menu, and writes the file on the next `apply_vfs_ops`.
    #[test]
    fn click_file_save_item_emits_vfs_write() {
        use oasis_vfs::Vfs;
        let mut vfs = make_vfs();
        vfs.mkdir("/tmp").expect("mkdir");
        let mut app = TextEditorApp::open_file("/tmp/notes.txt", "hello");
        // Open the File menu.
        let _ = app.handle_click(10, 8, 400, 300, false);
        assert!(app.menu.is_open());
        // File dropdown starts just below the menu bar (y=18) with
        // 4px padding, so first item row is y=22..42. New, then
        // Save is the second entry (y=42..62).
        let action = app.handle_click(20, 46, 400, 300, false);
        assert_eq!(action, AppAction::None);
        assert!(!app.menu.is_open(), "menu should close after dispatch");
        assert!(app.apply_vfs_ops(&mut vfs), "save must be applied");
        assert_eq!(vfs.read("/tmp/notes.txt").expect("written"), b"hello");
    }

    /// Clicking File > Exit returns `AppAction::Exit` so the host
    /// closes the editor window.
    #[test]
    fn click_file_exit_item_returns_exit() {
        let mut app = TextEditorApp::new("/apps/editor");
        let _ = app.handle_click(10, 8, 400, 300, false);
        // Exit is the 5th entry (New, Save, Save As, Separator, Exit):
        // y=22+20+20+20+6 = 88 is the exit row.
        let action = app.handle_click(20, 92, 400, 300, false);
        assert_eq!(action, AppAction::Exit);
    }

    /// When a drop-down is open, a plain key press closes it rather
    /// than typing through to the buffer.
    #[test]
    fn key_closes_open_dropdown() {
        let vfs = make_vfs();
        let mut app = TextEditorApp::new("/apps/editor");
        app.menu.open = Some(0);
        app.handle_input(&Button::Cancel, &vfs);
        assert!(!app.menu.is_open());
    }

    /// `update_sdi` must populate the Notepad chrome (menu bar, text
    /// area, status bar) — not leave the generic content-listing
    /// objects behind. This guards against the old "> 1 | text" CLI
    /// style showing in the classic fullscreen path.
    #[test]
    fn update_sdi_renders_notepad_chrome() {
        let mut app = TextEditorApp::open_file("/welcome.txt", "hello\nworld");
        let mut sdi = SdiRegistry::new();
        app.update_sdi(&mut sdi, &ActiveTheme::default());
        // Notepad chrome objects exist and are visible.
        for name in [
            "np_menu_bg",
            "np_area_bg",
            "np_status_bg",
            "np_menu_0",
            "np_status_left",
            "np_status_right",
        ] {
            let obj = sdi
                .get(name)
                .unwrap_or_else(|e| panic!("{name} should exist after update_sdi: {e:?}"));
            assert!(obj.visible, "{name} should be visible");
        }
        // Menu labels present.
        let file_label = sdi.get("np_menu_0").unwrap();
        assert_eq!(file_label.text.as_deref(), Some("File"));
        // First buffer line rendered as an np_line_* object.
        let line0 = sdi.get("np_line_0").unwrap();
        assert!(line0.visible);
        assert_eq!(line0.text.as_deref(), Some("hello"));
    }

    /// After the editor populates its SDI, a subsequent `hide_sdi`
    /// must drop every Notepad object — otherwise the chrome leaks
    /// onto whichever app is opened next.
    #[test]
    fn hide_sdi_hides_notepad_chrome() {
        let mut app = TextEditorApp::open_file("/welcome.txt", "hi");
        let mut sdi = SdiRegistry::new();
        app.update_sdi(&mut sdi, &ActiveTheme::default());
        app.hide_sdi(&mut sdi);
        for name in ["np_menu_bg", "np_area_bg", "np_status_bg", "np_line_0"] {
            let obj = sdi.get(name).unwrap();
            assert!(!obj.visible, "{name} should be hidden");
        }
    }

    // -- EditorBuffer tests --

    #[test]
    fn buffer_new_has_one_empty_line() {
        let buf = EditorBuffer::new();
        assert_eq!(buf.line_count(), 1);
        assert_eq!(buf.get_line(0), Some(""));
    }

    #[test]
    fn buffer_from_text() {
        let buf = EditorBuffer::from_text("hello\nworld");
        assert_eq!(buf.line_count(), 2);
        assert_eq!(buf.get_line(0), Some("hello"));
        assert_eq!(buf.get_line(1), Some("world"));
    }

    #[test]
    fn buffer_from_empty_text() {
        let buf = EditorBuffer::from_text("");
        assert_eq!(buf.line_count(), 1);
        assert_eq!(buf.get_line(0), Some(""));
    }

    #[test]
    fn buffer_insert_char() {
        let mut buf = EditorBuffer::from_text("abc");
        buf.insert_char(0, 1, 'X');
        assert_eq!(buf.get_line(0), Some("aXbc"));
    }

    #[test]
    fn buffer_insert_char_at_end() {
        let mut buf = EditorBuffer::from_text("abc");
        buf.insert_char(0, 3, 'Z');
        assert_eq!(buf.get_line(0), Some("abcZ"));
    }

    #[test]
    fn buffer_insert_char_beyond_length_clamps() {
        let mut buf = EditorBuffer::from_text("ab");
        buf.insert_char(0, 99, 'X');
        assert_eq!(buf.get_line(0), Some("abX"));
    }

    #[test]
    fn buffer_delete_char() {
        let mut buf = EditorBuffer::from_text("abc");
        let ch = buf.delete_char(0, 1);
        assert_eq!(ch, Some('b'));
        assert_eq!(buf.get_line(0), Some("ac"));
    }

    #[test]
    fn buffer_delete_char_out_of_range() {
        let mut buf = EditorBuffer::from_text("abc");
        let ch = buf.delete_char(0, 5);
        assert_eq!(ch, None);
        assert_eq!(buf.get_line(0), Some("abc"));
    }

    #[test]
    fn buffer_split_line() {
        let mut buf = EditorBuffer::from_text("abcdef");
        buf.split_line(0, 3);
        assert_eq!(buf.line_count(), 2);
        assert_eq!(buf.get_line(0), Some("abc"));
        assert_eq!(buf.get_line(1), Some("def"));
    }

    #[test]
    fn buffer_split_line_at_start() {
        let mut buf = EditorBuffer::from_text("hello");
        buf.split_line(0, 0);
        assert_eq!(buf.line_count(), 2);
        assert_eq!(buf.get_line(0), Some(""));
        assert_eq!(buf.get_line(1), Some("hello"));
    }

    #[test]
    fn buffer_split_line_at_end() {
        let mut buf = EditorBuffer::from_text("hello");
        buf.split_line(0, 5);
        assert_eq!(buf.line_count(), 2);
        assert_eq!(buf.get_line(0), Some("hello"));
        assert_eq!(buf.get_line(1), Some(""));
    }

    #[test]
    fn buffer_join_lines() {
        let mut buf = EditorBuffer::from_text("abc\ndef");
        buf.join_lines(0);
        assert_eq!(buf.line_count(), 1);
        assert_eq!(buf.get_line(0), Some("abcdef"));
    }

    #[test]
    fn buffer_join_lines_last_line_noop() {
        let mut buf = EditorBuffer::from_text("abc\ndef");
        buf.join_lines(1);
        assert_eq!(buf.line_count(), 2);
    }

    #[test]
    fn buffer_insert_line() {
        let mut buf = EditorBuffer::from_text("a\nb");
        buf.insert_line(1);
        assert_eq!(buf.line_count(), 3);
        assert_eq!(buf.get_line(0), Some("a"));
        assert_eq!(buf.get_line(1), Some(""));
        assert_eq!(buf.get_line(2), Some("b"));
    }

    #[test]
    fn buffer_delete_line() {
        let mut buf = EditorBuffer::from_text("a\nb\nc");
        let removed = buf.delete_line(1);
        assert_eq!(removed, Some("b".to_string()));
        assert_eq!(buf.line_count(), 2);
        assert_eq!(buf.get_line(0), Some("a"));
        assert_eq!(buf.get_line(1), Some("c"));
    }

    #[test]
    fn buffer_delete_last_remaining_line_noop() {
        let mut buf = EditorBuffer::from_text("only");
        let removed = buf.delete_line(0);
        assert_eq!(removed, None);
        assert_eq!(buf.line_count(), 1);
    }

    #[test]
    fn buffer_set_line() {
        let mut buf = EditorBuffer::from_text("old");
        buf.set_line(0, "new".to_string());
        assert_eq!(buf.get_line(0), Some("new"));
    }

    #[test]
    fn buffer_text_roundtrip() {
        let text = "line1\nline2\nline3";
        let buf = EditorBuffer::from_text(text);
        assert_eq!(buf.text(), text);
    }

    #[test]
    fn buffer_line_len() {
        let buf = EditorBuffer::from_text("hello");
        assert_eq!(buf.line_len(0), 5);
        assert_eq!(buf.line_len(999), 0);
    }

    #[test]
    fn buffer_get_line_out_of_range() {
        let buf = EditorBuffer::new();
        assert_eq!(buf.get_line(100), None);
    }

    // -- Cursor movement tests --

    #[test]
    fn cursor_up_at_top_stays() {
        let mut app = TextEditorApp::open_file("/test.txt", "a\nb\nc");
        app.content.cached_max_visible = 20;
        assert_eq!(app.cursor_line, 0);
        app.cursor_up();
        assert_eq!(app.cursor_line, 0);
    }

    #[test]
    fn cursor_down_moves() {
        let mut app = TextEditorApp::open_file("/test.txt", "a\nb\nc");
        app.content.cached_max_visible = 20;
        app.cursor_down();
        assert_eq!(app.cursor_line, 1);
    }

    #[test]
    fn cursor_down_at_bottom_stays() {
        let mut app = TextEditorApp::open_file("/test.txt", "a\nb\nc");
        app.content.cached_max_visible = 20;
        app.cursor_down();
        app.cursor_down();
        assert_eq!(app.cursor_line, 2);
        app.cursor_down();
        assert_eq!(app.cursor_line, 2);
    }

    #[test]
    fn cursor_right_moves() {
        let mut app = TextEditorApp::open_file("/test.txt", "abc");
        app.content.cached_max_visible = 20;
        app.cursor_right();
        assert_eq!(app.cursor_col, 1);
    }

    #[test]
    fn cursor_right_wraps_to_next_line() {
        let mut app = TextEditorApp::open_file("/test.txt", "ab\ncd");
        app.content.cached_max_visible = 20;
        app.cursor_right();
        app.cursor_right();
        // Now at end of line 0, col 2
        app.cursor_right();
        // Should wrap to line 1, col 0
        assert_eq!(app.cursor_line, 1);
        assert_eq!(app.cursor_col, 0);
    }

    #[test]
    fn cursor_left_moves() {
        let mut app = TextEditorApp::open_file("/test.txt", "abc");
        app.content.cached_max_visible = 20;
        app.cursor_col = 2;
        app.cursor_left();
        assert_eq!(app.cursor_col, 1);
    }

    #[test]
    fn cursor_left_wraps_to_previous_line() {
        let mut app = TextEditorApp::open_file("/test.txt", "ab\ncd");
        app.content.cached_max_visible = 20;
        app.cursor_line = 1;
        app.cursor_col = 0;
        app.cursor_left();
        assert_eq!(app.cursor_line, 0);
        assert_eq!(app.cursor_col, 2);
    }

    #[test]
    fn cursor_left_at_origin_stays() {
        let mut app = TextEditorApp::open_file("/test.txt", "abc");
        app.cursor_left();
        assert_eq!(app.cursor_line, 0);
        assert_eq!(app.cursor_col, 0);
    }

    #[test]
    fn cursor_home() {
        let mut app = TextEditorApp::open_file("/test.txt", "hello");
        app.cursor_col = 3;
        app.cursor_home();
        assert_eq!(app.cursor_col, 0);
    }

    #[test]
    fn cursor_end() {
        let mut app = TextEditorApp::open_file("/test.txt", "hello");
        app.cursor_end();
        assert_eq!(app.cursor_col, 5);
    }

    #[test]
    fn cursor_col_clamps_on_up() {
        let mut app = TextEditorApp::open_file("/test.txt", "long line\nhi");
        app.content.cached_max_visible = 20;
        app.cursor_line = 0;
        app.cursor_col = 8;
        app.cursor_down();
        // "hi" is only 2 chars, so col should clamp.
        assert_eq!(app.cursor_col, 2);
    }

    // -- Editing tests --

    #[test]
    fn insert_char_basic() {
        let mut app = TextEditorApp::open_file("/test.txt", "ac");
        app.cursor_col = 1;
        app.insert_char('b');
        assert_eq!(app.buffer.get_line(0), Some("abc"));
        assert_eq!(app.cursor_col, 2);
        assert!(app.modified);
    }

    #[test]
    fn delete_char_backspace() {
        let mut app = TextEditorApp::open_file("/test.txt", "abc");
        app.cursor_col = 2;
        app.delete_char();
        assert_eq!(app.buffer.get_line(0), Some("ac"));
        assert_eq!(app.cursor_col, 1);
    }

    #[test]
    fn delete_char_joins_lines() {
        let mut app = TextEditorApp::open_file("/test.txt", "abc\ndef");
        app.content.cached_max_visible = 20;
        app.cursor_line = 1;
        app.cursor_col = 0;
        app.delete_char();
        assert_eq!(app.buffer.line_count(), 1);
        assert_eq!(app.buffer.get_line(0), Some("abcdef"));
        assert_eq!(app.cursor_line, 0);
        assert_eq!(app.cursor_col, 3);
    }

    #[test]
    fn delete_forward_basic() {
        let mut app = TextEditorApp::open_file("/test.txt", "abc");
        app.cursor_col = 1;
        app.delete_forward();
        assert_eq!(app.buffer.get_line(0), Some("ac"));
        assert_eq!(app.cursor_col, 1);
    }

    #[test]
    fn delete_forward_joins_lines() {
        let mut app = TextEditorApp::open_file("/test.txt", "abc\ndef");
        app.content.cached_max_visible = 20;
        app.cursor_col = 3;
        app.delete_forward();
        assert_eq!(app.buffer.line_count(), 1);
        assert_eq!(app.buffer.get_line(0), Some("abcdef"));
    }

    #[test]
    fn new_line_splits() {
        let mut app = TextEditorApp::open_file("/test.txt", "abcdef");
        app.content.cached_max_visible = 20;
        app.cursor_col = 3;
        app.new_line();
        assert_eq!(app.buffer.line_count(), 2);
        assert_eq!(app.buffer.get_line(0), Some("abc"));
        assert_eq!(app.buffer.get_line(1), Some("def"));
        assert_eq!(app.cursor_line, 1);
        assert_eq!(app.cursor_col, 0);
    }

    // -- Undo/Redo tests --

    #[test]
    fn undo_insert_char() {
        let mut app = TextEditorApp::open_file("/test.txt", "ab");
        app.cursor_col = 2;
        app.insert_char('c');
        assert_eq!(app.buffer.get_line(0), Some("abc"));
        app.undo();
        assert_eq!(app.buffer.get_line(0), Some("ab"));
        assert_eq!(app.cursor_col, 2);
    }

    #[test]
    fn redo_insert_char() {
        let mut app = TextEditorApp::open_file("/test.txt", "ab");
        app.cursor_col = 2;
        app.insert_char('c');
        app.undo();
        assert_eq!(app.buffer.get_line(0), Some("ab"));
        app.redo();
        assert_eq!(app.buffer.get_line(0), Some("abc"));
        assert_eq!(app.cursor_col, 3);
    }

    #[test]
    fn undo_delete_char() {
        let mut app = TextEditorApp::open_file("/test.txt", "abc");
        app.cursor_col = 3;
        app.delete_char();
        assert_eq!(app.buffer.get_line(0), Some("ab"));
        app.undo();
        assert_eq!(app.buffer.get_line(0), Some("abc"));
    }

    #[test]
    fn undo_new_line() {
        let mut app = TextEditorApp::open_file("/test.txt", "abcdef");
        app.content.cached_max_visible = 20;
        app.cursor_col = 3;
        app.new_line();
        assert_eq!(app.buffer.line_count(), 2);
        app.undo();
        assert_eq!(app.buffer.line_count(), 1);
        assert_eq!(app.buffer.get_line(0), Some("abcdef"));
    }

    #[test]
    fn undo_on_empty_stack_noop() {
        let mut app = TextEditorApp::open_file("/test.txt", "abc");
        app.undo();
        assert_eq!(app.buffer.get_line(0), Some("abc"));
    }

    #[test]
    fn redo_on_empty_stack_noop() {
        let mut app = TextEditorApp::open_file("/test.txt", "abc");
        app.redo();
        assert_eq!(app.buffer.get_line(0), Some("abc"));
    }

    #[test]
    fn new_edit_clears_redo_stack() {
        let mut app = TextEditorApp::open_file("/test.txt", "ab");
        app.cursor_col = 2;
        app.insert_char('c');
        app.undo();
        // Now insert a different char -- redo should be cleared.
        app.insert_char('X');
        app.redo();
        // Redo stack was cleared, so this should be a no-op.
        assert_eq!(app.buffer.get_line(0), Some("abX"));
    }

    // -- Find tests --

    #[test]
    fn find_basic() {
        let mut app = TextEditorApp::open_file("/test.txt", "hello world");
        app.content.cached_max_visible = 20;
        let found = app.find("world");
        assert!(found);
        assert_eq!(app.cursor_line, 0);
        assert_eq!(app.cursor_col, 6);
    }

    #[test]
    fn find_not_found() {
        let mut app = TextEditorApp::open_file("/test.txt", "hello");
        app.content.cached_max_visible = 20;
        let found = app.find("xyz");
        assert!(!found);
    }

    #[test]
    fn find_on_second_line() {
        let mut app = TextEditorApp::open_file("/test.txt", "aaa\nbbb\nccc");
        app.content.cached_max_visible = 20;
        let found = app.find("bbb");
        assert!(found);
        assert_eq!(app.cursor_line, 1);
        assert_eq!(app.cursor_col, 0);
    }

    #[test]
    fn find_next_advances() {
        let mut app = TextEditorApp::open_file("/test.txt", "abab\nabab");
        app.content.cached_max_visible = 20;
        app.find("ab");
        assert_eq!(app.cursor_line, 0);
        assert_eq!(app.cursor_col, 0);
        app.find_next();
        assert_eq!(app.cursor_col, 2);
    }

    #[test]
    fn find_next_wraps_around() {
        let mut app = TextEditorApp::open_file("/test.txt", "ab\ncd");
        app.content.cached_max_visible = 20;
        app.find("cd");
        assert_eq!(app.cursor_line, 1);
        // Find next should wrap to find "ab" if we search for
        // something at the start.
        let found = app.find("ab");
        assert!(found);
        assert_eq!(app.cursor_line, 0);
    }

    #[test]
    fn find_empty_query_returns_false() {
        let mut app = TextEditorApp::open_file("/test.txt", "hello");
        assert!(!app.find(""));
    }

    #[test]
    fn find_next_empty_query_returns_false() {
        let mut app = TextEditorApp::open_file("/test.txt", "hello");
        assert!(!app.find_next());
    }

    // -- File open/save roundtrip --

    #[test]
    fn open_and_save_roundtrip() {
        let text = "line1\nline2\nline3";
        let app = TextEditorApp::open_file("/test.txt", text);
        assert_eq!(app.save_content(), text);
    }

    #[test]
    fn open_empty_content() {
        let app = TextEditorApp::open_file("/test.txt", "");
        assert_eq!(app.buffer.line_count(), 1);
        assert_eq!(app.save_content(), "");
    }

    #[test]
    fn save_after_edit() {
        let mut app = TextEditorApp::open_file("/test.txt", "ab");
        app.cursor_col = 2;
        app.insert_char('c');
        assert_eq!(app.save_content(), "abc");
    }

    // -- Display formatting --

    #[test]
    fn display_lines_have_line_numbers() {
        let app = TextEditorApp::open_file("/test.txt", "hello\nworld");
        let lines = app.format_display_lines();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("1"));
        assert!(lines[0].contains("hello"));
        assert!(lines[1].contains("2"));
        assert!(lines[1].contains("world"));
    }

    #[test]
    fn display_lines_mark_cursor_line() {
        let mut app = TextEditorApp::open_file("/test.txt", "a\nb\nc");
        app.content.cached_max_visible = 20;
        app.cursor_line = 1;
        let lines = app.format_display_lines();
        // Line at index 1 should start with '>'.
        assert!(lines[1].starts_with('>'));
        // Others should start with ' '.
        assert!(lines[0].starts_with(' '));
        assert!(lines[2].starts_with(' '));
    }

    #[test]
    fn display_lines_contain_pipe_separator() {
        let app = TextEditorApp::open_file("/test.txt", "test");
        let lines = app.format_display_lines();
        assert!(lines[0].contains('|'));
    }

    // -- Edge cases --

    #[test]
    fn empty_buffer_cursor_stays() {
        let mut app = TextEditorApp::new("/apps/editor");
        app.cursor_up();
        assert_eq!(app.cursor_line, 0);
        app.cursor_down();
        assert_eq!(app.cursor_line, 0);
    }

    #[test]
    fn single_line_operations() {
        let mut app = TextEditorApp::open_file("/test.txt", "x");
        app.content.cached_max_visible = 20;
        app.cursor_col = 1;
        app.delete_char();
        assert_eq!(app.buffer.get_line(0), Some(""));
        app.insert_char('y');
        assert_eq!(app.buffer.get_line(0), Some("y"));
    }

    #[test]
    fn very_long_line() {
        let long = "a".repeat(500);
        let app = TextEditorApp::open_file("/test.txt", &long);
        assert_eq!(app.buffer.line_len(0), 500);
        let saved = app.save_content();
        assert_eq!(saved.len(), 500);
    }

    // -- App trait tests --

    #[test]
    fn title_without_file() {
        let app = TextEditorApp::new("/apps/editor");
        assert_eq!(app.title(), "Text Editor");
    }

    #[test]
    fn title_with_file() {
        let app = TextEditorApp::open_file("/docs/readme.txt", "hi");
        assert!(app.title().contains("readme.txt"));
    }

    #[test]
    fn cancel_exits_in_normal_mode() {
        let vfs = make_vfs();
        let mut app = TextEditorApp::new("/apps/editor");
        let action = app.handle_input(&Button::Cancel, &vfs);
        assert_eq!(action, AppAction::Exit);
    }

    #[test]
    fn confirm_enters_insert_mode() {
        let vfs = make_vfs();
        let mut app = TextEditorApp::new("/apps/editor");
        app.handle_input(&Button::Confirm, &vfs);
        assert_eq!(app.mode, EditorMode::Insert);
    }

    #[test]
    fn cancel_in_insert_returns_to_normal() {
        let vfs = make_vfs();
        let mut app = TextEditorApp::new("/apps/editor");
        app.handle_input(&Button::Confirm, &vfs);
        assert_eq!(app.mode, EditorMode::Insert);
        app.handle_input(&Button::Cancel, &vfs);
        assert_eq!(app.mode, EditorMode::Normal);
    }

    #[test]
    fn triangle_enters_find_mode() {
        let vfs = make_vfs();
        let mut app = TextEditorApp::new("/apps/editor");
        app.handle_input(&Button::Triangle, &vfs);
        assert_eq!(app.mode, EditorMode::Find);
    }

    #[test]
    fn start_enters_saving_mode() {
        let vfs = make_vfs();
        let mut app = TextEditorApp::new("/apps/editor");
        app.handle_input(&Button::Start, &vfs);
        assert_eq!(app.mode, EditorMode::Saving);
    }

    #[test]
    fn lines_returns_display_lines() {
        let app = TextEditorApp::open_file("/test.txt", "hello\nworld");
        let lines = app.lines();
        // Should include display lines + status bar.
        assert!(lines.len() >= 2);
    }

    #[test]
    fn viewing_file_returns_path() {
        let app = TextEditorApp::open_file("/test.txt", "hello");
        assert_eq!(app.viewing_file(), Some("/test.txt"));
    }

    #[test]
    fn new_editor_not_modified() {
        let app = TextEditorApp::new("/apps/editor");
        assert!(!app.is_modified());
    }

    #[test]
    fn modified_after_insert() {
        let mut app = TextEditorApp::open_file("/test.txt", "a");
        app.insert_char('b');
        assert!(app.is_modified());
    }

    #[test]
    fn downcast_works() {
        let app = TextEditorApp::new("/apps/editor");
        let any = app.as_any();
        assert!(any.downcast_ref::<TextEditorApp>().is_some());
    }

    #[test]
    fn open_from_vfs_loads_existing_file() {
        use oasis_vfs::Vfs;
        let mut vfs = make_vfs();
        vfs.write("/notes.txt", b"line one\nline two\n").unwrap();
        let app = TextEditorApp::open_from_vfs("/notes.txt", &vfs);
        assert_eq!(app.viewing_file(), Some("/notes.txt"));
        assert_eq!(app.buffer.get_line(0), Some("line one"));
        assert_eq!(app.buffer.get_line(1), Some("line two"));
    }

    #[test]
    fn open_from_vfs_missing_file_returns_editor_with_status() {
        let vfs = make_vfs();
        let app = TextEditorApp::open_from_vfs("/does_not_exist.txt", &vfs);
        // Empty editor + status message.
        assert!(app.viewing_file().is_none());
        assert!(app.status_message.is_some());
    }

    #[test]
    fn no_pending_request_initially() {
        let mut app = TextEditorApp::new("/apps/editor");
        assert!(app.take_pending_request().is_none());
    }

    #[test]
    fn save_via_start_confirm_writes_file() {
        use oasis_vfs::Vfs;
        let mut vfs = make_vfs();
        let mut app = TextEditorApp::open_file("/test.txt", "data");
        app.insert_char('!');
        // Enter saving mode and confirm.
        app.handle_input(&Button::Start, &vfs);
        app.handle_input(&Button::Confirm, &vfs);
        assert!(app.apply_vfs_ops(&mut vfs));
        assert_eq!(vfs.read("/test.txt").expect("written"), b"!data");
        assert!(!app.is_modified(), "a successful write clears the flag");
        assert!(!app.apply_vfs_ops(&mut vfs), "nothing left to apply");
    }

    #[test]
    fn undo_redo_multiple_steps() {
        // Undo is grouped by word: each word (plus the spaces typed after
        // it) is one step.
        let mut app = TextEditorApp::open_file("/test.txt", "");
        for ch in "ab cd ef".chars() {
            app.insert_char(ch);
        }
        assert_eq!(app.buffer.get_line(0), Some("ab cd ef"));
        app.undo();
        assert_eq!(app.buffer.get_line(0), Some("ab cd "));
        app.undo();
        assert_eq!(app.buffer.get_line(0), Some("ab "));
        app.redo();
        assert_eq!(app.buffer.get_line(0), Some("ab cd "));
        app.redo();
        assert_eq!(app.buffer.get_line(0), Some("ab cd ef"));
        assert_eq!(app.cursor_col, 8);
    }

    #[test]
    fn cursor_position_getter() {
        let mut app = TextEditorApp::open_file("/test.txt", "ab\ncd");
        app.content.cached_max_visible = 20;
        app.cursor_down();
        app.cursor_right();
        assert_eq!(app.cursor_position(), (1, 1));
    }

    #[test]
    fn mode_getter() {
        let app = TextEditorApp::new("/apps/editor");
        assert_eq!(app.mode(), EditorMode::Normal);
    }

    #[test]
    fn ltrigger_undo_in_normal() {
        let vfs = make_vfs();
        let mut app = TextEditorApp::open_file("/test.txt", "ab");
        app.cursor_col = 2;
        app.insert_char('c');
        app.mode = EditorMode::Normal;
        app.handle_input(&Button::Select, &vfs);
        assert_eq!(app.buffer.get_line(0), Some("ab"));
    }

    #[test]
    fn rtrigger_redo_in_normal() {
        let vfs = make_vfs();
        let mut app = TextEditorApp::open_file("/test.txt", "ab");
        app.cursor_col = 2;
        app.insert_char('c');
        app.undo();
        app.mode = EditorMode::Normal;
        app.handle_input(&Button::Square, &vfs);
        assert_eq!(app.buffer.get_line(0), Some("abc"));
    }

    #[test]
    fn insert_mode_newline_via_confirm() {
        let vfs = make_vfs();
        let mut app = TextEditorApp::open_file("/test.txt", "abcd");
        app.content.cached_max_visible = 20;
        app.mode = EditorMode::Insert;
        app.cursor_col = 2;
        app.handle_input(&Button::Confirm, &vfs);
        assert_eq!(app.buffer.line_count(), 2);
        assert_eq!(app.buffer.get_line(0), Some("ab"));
        assert_eq!(app.buffer.get_line(1), Some("cd"));
    }

    #[test]
    fn insert_mode_backspace_via_ltrigger() {
        let vfs = make_vfs();
        let mut app = TextEditorApp::open_file("/test.txt", "abc");
        app.mode = EditorMode::Insert;
        app.cursor_col = 3;
        app.handle_input(&Button::Select, &vfs);
        assert_eq!(app.buffer.get_line(0), Some("ab"));
    }

    #[test]
    fn insert_mode_delete_forward_via_rtrigger() {
        let vfs = make_vfs();
        let mut app = TextEditorApp::open_file("/test.txt", "abc");
        app.mode = EditorMode::Insert;
        app.cursor_col = 1;
        app.handle_input(&Button::Square, &vfs);
        assert_eq!(app.buffer.get_line(0), Some("ac"));
    }
}
