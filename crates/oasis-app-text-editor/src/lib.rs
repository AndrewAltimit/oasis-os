//! Text editor application with modal editing, selection, clipboard,
//! grouped undo/redo, find/replace and Save As.
//!
//! `TextEditorApp` is a Notepad-style editor. Keyboard hosts drive it
//! through `App::handle_key` (Ctrl+N/S/Shift+S/Z/Y/X/C/V/A/F/H/G, arrows
//! with Shift selection, Ctrl word jumps, Home/End, PageUp/PageDown,
//! Delete); gamepad hosts reach every feature through `handle_input`
//! (Normal / Insert / Find / Replace / prompt modes).

use oasis_app_core::{App, AppAction, AppRequest, ContentState, MAX_WHEEL_STEPS};
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

use editor::UndoGroup;

/// Lines one mouse-wheel notch scrolls the editor view.
const WHEEL_LINES: usize = 3;

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
    /// Content-area size recorded by the last `App::layout` (click
    /// hit-testing).
    pub(crate) content_size: (u32, u32),
    pub(crate) buffer: EditorBuffer,
    pub(crate) mode: EditorMode,
    pub(crate) cursor_line: usize,
    /// Byte column, always on a character boundary.
    pub(crate) cursor_col: usize,
    /// Selection anchor; the selection spans anchor..cursor.
    pub(crate) anchor: Option<Pos>,
    /// Character column Up / Down / PageUp / PageDown aim for: kept across
    /// consecutive vertical moves so a short line in between does not pull
    /// the caret left for good (`None` after any other motion or edit).
    pub(crate) goal_col: Option<usize>,
    pub(crate) file_path: Option<String>,
    /// Line terminator the file was opened with (`"\r\n"` or `"\n"`);
    /// saves write it back unchanged.
    pub(crate) line_ending: &'static str,
    /// The file was not valid UTF-8 and was decoded lossily: saving it in
    /// place would replace the undecodable bytes with U+FFFD, so Save to
    /// that path is refused (Save As to another path is allowed).
    pub(crate) read_only: bool,
    pub(crate) modified: bool,
    /// `EditorBuffer::content_hash` of the text as last loaded or saved:
    /// undo / redo back to it clears `modified`.
    pub(crate) saved_hash: u64,
    pub(crate) status_message: Option<String>,
    pub(crate) find_query: String,
    /// The query was carried over from the last search when Find / Replace
    /// opened: like a pre-selected field, the first typed character
    /// replaces it (and Backspace clears it) instead of appending.
    pub(crate) query_primed: bool,
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
    /// Queued write `(path, data, buffer generation, content hash)`,
    /// flushed by `apply_vfs_ops`.
    pub(crate) pending_save: Option<(String, String, u64, u64)>,
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
    /// Text lines that fit the viewport of the last `App::layout` /
    /// fullscreen render (0 = not laid out yet).
    pub(crate) viewport_lines: usize,
    /// Row height of that viewport (0 = not laid out yet).
    pub(crate) viewport_line_h: i32,
    /// Top menu bar widget (File / Edit / View / Help) with drop-downs.
    pub(crate) menu: MenuBar,
    /// Glyph advances of the last draw, for click hit-testing.
    pub(crate) advances: std::cell::RefCell<render::Advances>,
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
        let saved_hash = buffer.content_hash();
        let mut editor = Self {
            content,
            content_size: (0, 0),
            buffer,
            mode: EditorMode::Normal,
            cursor_line: 0,
            cursor_col: 0,
            anchor: None,
            goal_col: None,
            file_path: path.map(String::from),
            line_ending: "\n",
            read_only: false,
            modified: false,
            saved_hash,
            status_message: None,
            find_query: String::new(),
            query_primed: false,
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
            viewport_lines: 0,
            viewport_line_h: 0,
            advances: std::cell::RefCell::default(),
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
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(text) => Self::open_file(path, &text),
                Err(e) => {
                    let text = String::from_utf8_lossy(e.as_bytes()).into_owned();
                    let mut editor = Self::open_file(path, &text);
                    editor.read_only = true;
                    editor.status_message =
                        Some("Not UTF-8: opened read-only (Save As to keep a copy).".into());
                    editor.rebuild_display_lines();
                    editor
                },
            },
            Err(e) => {
                let mut editor = Self::new("/apps/editor");
                editor.status_message = Some(format!("Could not read {path}: {e}"));
                editor.rebuild_display_lines();
                editor
            },
        }
    }

    /// Open a file with the given content.
    pub fn open_file(path: &str, content: &str) -> Self {
        let file_name = path.rsplit('/').next().unwrap_or(path);
        let title = format!("Text Editor - {file_name}");
        let cs = ContentState::new(&title, "/apps/editor");
        let mut editor = Self::with_state(cs, EditorBuffer::from_text(content), Some(path));
        // Keep CRLF files CRLF: the buffer stores bare lines, and saving
        // re-joins them with the ending the file was opened with.
        if content.contains("\r\n") {
            editor.line_ending = "\r\n";
        }
        editor
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

    fn on_close_requested(&mut self, _vfs: &dyn Vfs) -> AppAction {
        // Same as Cancel at top level: unsaved changes raise the prompt.
        self.menu.close();
        self.request_close(PendingClose::Exit)
    }

    fn accepts_text(&self) -> bool {
        // Typing is accepted in every mode (Normal auto-enters Insert), so
        // printable keys must never double as Triangle/trigger shortcuts.
        true
    }

    fn handle_backspace(&mut self) {
        self.handle_backspace_impl();
    }

    fn layout(&mut self, cw: u32, ch: u32, at: &ActiveTheme) {
        self.content_size = (cw, ch);
        let (lines, line_h) = render::notepad_viewport(ch, at);
        self.viewport_lines = lines.max(1);
        self.viewport_line_h = line_h;
    }

    /// The wheel scrolls the view three lines per notch without moving
    /// the caret (the next caret move scrolls it back into view).
    fn handle_scroll(&mut self, delta: i32, _vfs: &dyn Vfs) -> bool {
        let max_scroll = self
            .buffer
            .line_count()
            .saturating_sub(self.page_lines().max(1));
        let step = delta.unsigned_abs().min(MAX_WHEEL_STEPS) as usize * WHEEL_LINES;
        let scroll = if delta < 0 {
            self.content.scroll.saturating_sub(step)
        } else {
            self.content.scroll.saturating_add(step).min(max_scroll)
        };
        let changed = scroll != self.content.scroll;
        self.content.scroll = scroll;
        changed
    }

    fn handle_click(&mut self, lx: i32, ly: i32, _vfs: &dyn Vfs) -> AppAction {
        if self.close_requested {
            return AppAction::Exit;
        }
        let (cw, ch) = self.content_size;
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

        // Clicks in the text area: place the caret at the nearest character
        // boundary, measured with the advances the backend reported while
        // drawing (there is no backend here; the bitmap font's before the
        // first draw) under the same horizontal scroll `draw_notepad`
        // applies.
        let pad_top = 6i32;
        let line_h = match self.viewport_line_h {
            0 => 14,
            h => h,
        };
        let relative_y = ly - area_top - pad_top;
        let clicked_line = (relative_y / line_h).max(0) as usize;
        let target_line =
            (self.content.scroll + clicked_line).min(self.buffer.line_count().saturating_sub(1));
        let advances = self.advances.borrow();
        let measure = |s: &str| advances.width(s);
        let caret_line = self.buffer.get_line(self.cursor_line).unwrap_or("");
        let caret_px = measure(&caret_line[..caret_line.floor_char_boundary(self.cursor_col)]);
        let x = (lx - render::NP_PAD_LEFT).max(0) as u32 + render::h_scroll(caret_px, cw);
        let line_text = self.buffer.get_line(target_line).unwrap_or("");
        let mut target_col = line_text.len();
        let mut acc = 0u32;
        for (i, c) in line_text.char_indices() {
            let adv = advances.advance(c);
            // Nearest boundary: past a glyph's midpoint means after it.
            if x < acc + adv / 2 {
                target_col = i;
                break;
            }
            acc += adv;
        }
        drop(advances);

        if matches!(self.mode, EditorMode::Normal | EditorMode::Saving) {
            self.mode = EditorMode::Insert;
        }
        self.move_to((target_line, target_col), false);
        AppAction::None
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

    fn drain_requests(&mut self, out: &mut Vec<AppRequest>) {
        out.append(&mut self.content.requests);
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
}

// ---------------------------------------------------------------
// Tests
// ---------------------------------------------------------------

#[cfg(test)]
mod tests;
