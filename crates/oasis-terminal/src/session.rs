//! Interactive shell session: the glue between host input events and the
//! [`LineEditor`], tab completion, and persistent history.
//!
//! A host (desktop app, WASM, ...) owns one [`ShellSession`] per terminal
//! and feeds it input through [`ShellSession::handle_key`] (raw keyboard
//! shortcuts) and [`ShellSession::handle_event`] (text, Backspace, Tab and
//! gamepad buttons). The returned [`SessionEvent`] tells the host what to
//! do: execute a submitted line, print completion candidates, clear the
//! screen, and so on.
//!
//! # Keys
//!
//! | Input | Action |
//! |---|---|
//! | Left / Right (d-pad too) | Move the cursor |
//! | Home / Ctrl+A, End / Ctrl+E | Start / end of line |
//! | Ctrl+Left / Alt+B, Ctrl+Right / Alt+F | Word left / right |
//! | Backspace (Square), Delete / Ctrl+D | Delete before / at the cursor |
//! | Ctrl+W, Ctrl+Backspace, Alt+Backspace | Delete the word before the cursor |
//! | Ctrl+K / Ctrl+U | Kill to end / start of line; Ctrl+Y pastes it back |
//! | Ctrl+T | Swap the two characters before the cursor |
//! | Up / Down (d-pad too), Ctrl+P / Ctrl+N | History |
//! | Ctrl+R | Reverse history search (again: older match; Esc / Ctrl+G cancel) |
//! | Tab | Complete commands, `$VARS` and VFS paths |
//! | Ctrl+L | Clear the screen |
//! | Ctrl+C | Abandon the line |
//! | Enter (Cross) | Submit |
//!
//! Every shortcut uses plain keys, Ctrl or Alt+letter, so none collides with
//! the window manager's Alt+Tab / Super / Ctrl+Alt combos.

use oasis_types::error::Result;
use oasis_types::input::{Button, InputEvent, Key, Modifiers};
use oasis_vfs::Vfs;

use crate::completion::Completer;
use crate::interpreter::CommandRegistry;
use crate::line_edit::{EditAction, EditResult, LineEditor};

/// Default VFS path of the persisted history file.
pub const HISTORY_PATH: &str = "/home/user/.oasis_history";

/// What the host should do after an input was applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionEvent {
    /// The line or cursor changed; redraw the prompt.
    Redraw,
    /// The user pressed Enter: execute this line (may be empty).
    Submit(String),
    /// Ctrl+C: the line was abandoned (contains the discarded text).
    Interrupted(String),
    /// Ctrl+L: clear the scrollback.
    ClearScreen,
    /// Tab found several candidates and could not extend the word: show
    /// them (display names, e.g. file names without their directory).
    Candidates(Vec<String>),
}

/// Line-editing shell session with history and completion.
pub struct ShellSession {
    editor: LineEditor,
    history_path: String,
}

impl Default for ShellSession {
    fn default() -> Self {
        Self::new()
    }
}

impl ShellSession {
    /// New session persisting history to [`HISTORY_PATH`].
    pub fn new() -> Self {
        Self::with_history_path(HISTORY_PATH)
    }

    /// New session persisting history to `path`.
    pub fn with_history_path(path: &str) -> Self {
        Self {
            editor: LineEditor::new(),
            history_path: path.to_string(),
        }
    }

    /// The underlying line editor.
    pub fn editor(&self) -> &LineEditor {
        &self.editor
    }

    /// Current input line.
    pub fn buffer(&self) -> &str {
        self.editor.buffer()
    }

    /// Cursor position in characters.
    pub fn cursor_col(&self) -> usize {
        self.editor.display_cursor()
    }

    /// Whether Ctrl+R reverse search is active.
    pub fn is_searching(&self) -> bool {
        self.editor.is_searching()
    }

    /// Replace the input line (cursor at end), e.g. from an on-screen
    /// keyboard.
    pub fn set_line(&mut self, line: &str) {
        self.editor.set_buffer(line);
    }

    /// The text to show after the prompt and the cursor column (in
    /// characters) within it. While searching this is
    /// `(reverse-i-search)'query': match`.
    pub fn display(&self) -> (String, usize) {
        if self.editor.is_searching() {
            let text = format!(
                "(reverse-i-search)'{}': {}",
                self.editor.search_query(),
                self.editor.buffer()
            );
            let col = text.chars().count();
            (text, col)
        } else {
            (
                self.editor.buffer().to_string(),
                self.editor.display_cursor(),
            )
        }
    }

    // -- Input mapping -----------------------------------------------------

    /// Map a raw key press to an edit action.
    ///
    /// Only keys without a sensible gamepad-style twin are mapped here
    /// (Home/End/Delete, Ctrl/Alt shortcuts, Esc while searching). Plain
    /// arrows, Enter, Backspace and Tab are left to their twin events,
    /// which [`Self::handle_event`] handles, so keyboard and gamepad share
    /// one path. When this returns `Some`, the host should consume the key
    /// and drop its twin.
    pub fn key_action(&self, key: Key, mods: Modifiers) -> Option<EditAction> {
        let searching = self.editor.is_searching();
        if mods.only(Modifiers::CTRL) {
            return Some(match key {
                Key::Char('a') => EditAction::MoveToStart,
                Key::Char('e') => EditAction::MoveToEnd,
                Key::Char('b') => EditAction::MoveLeft,
                Key::Char('f') => EditAction::MoveRight,
                Key::Char('d') => EditAction::DeleteCharForward,
                Key::Char('k') => EditAction::KillToEnd,
                Key::Char('u') => EditAction::KillToStart,
                Key::Char('w') | Key::Backspace => EditAction::DeleteWordBack,
                Key::Char('y') => EditAction::Yank,
                Key::Char('t') => EditAction::SwapChars,
                Key::Char('l') => EditAction::ClearScreen,
                Key::Char('c') => EditAction::Cancel,
                Key::Char('p') => EditAction::HistoryPrev,
                Key::Char('n') => EditAction::HistoryNext,
                Key::Char('r') if searching => EditAction::SearchNext,
                Key::Char('r') => EditAction::StartSearch,
                Key::Char('g') if searching => EditAction::CancelSearch,
                Key::Left => EditAction::MoveWordLeft,
                Key::Right => EditAction::MoveWordRight,
                _ => return None,
            });
        }
        if mods.only(Modifiers::ALT) {
            return match key {
                Key::Char('b') => Some(EditAction::MoveWordLeft),
                Key::Char('f') => Some(EditAction::MoveWordRight),
                Key::Backspace => Some(EditAction::DeleteWordBack),
                _ => None,
            };
        }
        if mods.has_command() {
            return None;
        }
        match key {
            Key::Home => Some(EditAction::MoveToStart),
            Key::End => Some(EditAction::MoveToEnd),
            Key::Delete => Some(EditAction::DeleteCharForward),
            Key::Escape if searching => Some(EditAction::CancelSearch),
            _ => None,
        }
    }

    /// Map a text / gamepad-style event to an edit action.
    pub fn event_action(&self, event: &InputEvent) -> Option<EditAction> {
        let searching = self.editor.is_searching();
        Some(match event {
            InputEvent::TextInput(ch) if ch.is_control() => return None,
            InputEvent::TextInput(ch) if searching => EditAction::SearchChar(*ch),
            InputEvent::TextInput(ch) => EditAction::InsertChar(*ch),
            InputEvent::Backspace | InputEvent::ButtonPress(Button::Square) if searching => {
                EditAction::SearchBackspace
            },
            InputEvent::Backspace | InputEvent::ButtonPress(Button::Square) => {
                EditAction::DeleteCharBack
            },
            InputEvent::Tab => EditAction::Complete,
            InputEvent::ButtonPress(Button::Up) => EditAction::HistoryPrev,
            InputEvent::ButtonPress(Button::Down) => EditAction::HistoryNext,
            InputEvent::ButtonPress(Button::Left) => EditAction::MoveLeft,
            InputEvent::ButtonPress(Button::Right) => EditAction::MoveRight,
            InputEvent::ButtonPress(Button::Confirm) => EditAction::AcceptLine,
            _ => return None,
        })
    }

    /// Apply a raw key press. Returns `None` when the key is not a line
    /// editing shortcut (the host should process it -- and its twin --
    /// normally).
    pub fn handle_key(
        &mut self,
        key: Key,
        mods: Modifiers,
        reg: &CommandRegistry,
        cwd: &str,
        vfs: &dyn Vfs,
    ) -> Option<SessionEvent> {
        let action = self.key_action(key, mods)?;
        Some(self.apply(action, reg, cwd, vfs))
    }

    /// Apply a text / Backspace / Tab / gamepad event. Returns `None` for
    /// events the session does not handle (Cancel, Start, pointer, ...).
    pub fn handle_event(
        &mut self,
        event: &InputEvent,
        reg: &CommandRegistry,
        cwd: &str,
        vfs: &dyn Vfs,
    ) -> Option<SessionEvent> {
        let action = self.event_action(event)?;
        Some(self.apply(action, reg, cwd, vfs))
    }

    /// Apply one edit action.
    pub fn apply(
        &mut self,
        action: EditAction,
        reg: &CommandRegistry,
        cwd: &str,
        vfs: &dyn Vfs,
    ) -> SessionEvent {
        match action {
            EditAction::Complete => self.complete(reg, cwd, vfs),
            EditAction::AcceptLine => {
                if self.editor.is_searching() {
                    reg.with_history(|h| self.editor.apply(EditAction::AcceptSearch, h));
                }
                let line = self.editor.buffer().to_string();
                self.editor.clear();
                SessionEvent::Submit(line)
            },
            EditAction::Cancel => {
                let line = self.editor.buffer().to_string();
                self.editor.clear();
                SessionEvent::Interrupted(line)
            },
            EditAction::StartSearch if self.editor.is_searching() => {
                reg.with_history(|h| self.editor.apply(EditAction::SearchNext, h));
                SessionEvent::Redraw
            },
            action => match reg.with_history(|h| self.editor.apply(action, h)) {
                EditResult::ClearScreen => SessionEvent::ClearScreen,
                _ => SessionEvent::Redraw,
            },
        }
    }

    // -- Completion --------------------------------------------------------

    /// Tab completion at the cursor.
    ///
    /// Completes the command name in command position (start of line or
    /// after `|`, `;`, `&`), `$VARIABLES`, and VFS paths elsewhere. A
    /// single match is inserted (plus a space, or nothing after a
    /// directory's `/`); several matches extend the word to their common
    /// prefix, and when that makes no progress the candidates are returned
    /// for display.
    fn complete(&mut self, reg: &CommandRegistry, cwd: &str, vfs: &dyn Vfs) -> SessionEvent {
        let input = self.editor.buffer().to_string();
        let cursor = self.editor.cursor();
        // Complete within the current pipeline / chain segment so the
        // command after `|` or `;` is in command position.
        let seg_start = input[..cursor].rfind(['|', ';', '&']).map_or(0, |i| i + 1);
        let seg_start = seg_start
            + (input[seg_start..cursor].len() - input[seg_start..cursor].trim_start().len());
        let segment = &input[seg_start..cursor];

        let mut names = reg.completion_names();
        names.sort();
        names.dedup();
        let vars: Vec<String> = reg.variables().into_keys().collect();
        let mut completer = Completer::new();
        let Some(result) =
            completer.complete(segment, segment.len(), &names, &[], &[], &vars, cwd, vfs)
        else {
            return SessionEvent::Redraw;
        };

        let start = seg_start + result.start;
        let end = seg_start + result.end;
        let partial_len = end - start;
        let mut replacement = result.replacement;
        if result.is_complete && !replacement.ends_with('/') {
            replacement.push(' ');
        }
        if !result.is_complete && replacement.len() <= partial_len {
            // Ambiguous and no common prefix to add: list the options.
            let mut shown: Vec<String> = result
                .candidates
                .iter()
                .map(|c| display_name(c).to_string())
                .collect();
            shown.dedup();
            return SessionEvent::Candidates(shown);
        }
        let mut line = String::with_capacity(input.len() + replacement.len());
        line.push_str(&input[..start]);
        line.push_str(&replacement);
        line.push_str(&input[end..]);
        self.editor
            .set_buffer_with_cursor(&line, start + replacement.len());
        SessionEvent::Redraw
    }

    // -- History persistence -------------------------------------------------

    /// Load the persisted history file into `reg` (replacing its history).
    /// Returns the number of entries loaded (0 when there is no file).
    pub fn load_history(&self, reg: &CommandRegistry, vfs: &dyn Vfs) -> usize {
        let Ok(data) = vfs.read(&self.history_path) else {
            return 0;
        };
        let text = String::from_utf8_lossy(&data);
        reg.set_history(text.lines().map(str::to_string).collect());
        reg.with_history(<[String]>::len)
    }

    /// Write `reg`'s history (bounded, oldest first) to the history file,
    /// creating its directory if needed.
    pub fn save_history(&self, reg: &CommandRegistry, vfs: &mut dyn Vfs) -> Result<()> {
        if let Some((dir, _)) = self.history_path.rsplit_once('/')
            && !dir.is_empty()
            && !vfs.exists(dir)
        {
            vfs.mkdir(dir)?;
        }
        let mut data = reg.with_history(|h| h.join("\n"));
        if !data.is_empty() {
            data.push('\n');
        }
        vfs.write(&self.history_path, data.as_bytes())
    }
}

/// The part of a completion candidate worth showing in a listing: the last
/// path component (keeping a directory's trailing `/`).
fn display_name(candidate: &str) -> &str {
    let trimmed = candidate.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(i) => &candidate[i + 1..],
        None => candidate,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oasis_vfs::MemoryVfs;

    fn reg() -> CommandRegistry {
        let mut reg = CommandRegistry::new();
        crate::register_builtins(&mut reg);
        reg
    }

    fn vfs() -> MemoryVfs {
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/home/user/docs").expect("mkdir");
        vfs.mkdir("/home/user/downloads").expect("mkdir");
        vfs.write("/home/user/notes.txt", b"hi").expect("write");
        vfs
    }

    fn typed(session: &mut ShellSession, reg: &CommandRegistry, vfs: &MemoryVfs, s: &str) {
        for ch in s.chars() {
            session.handle_event(&InputEvent::TextInput(ch), reg, "/", vfs);
        }
    }

    fn key(
        session: &mut ShellSession,
        reg: &CommandRegistry,
        vfs: &MemoryVfs,
        key: Key,
        mods: Modifiers,
    ) -> Option<SessionEvent> {
        session.handle_key(key, mods, reg, "/home/user", vfs)
    }

    fn event(
        session: &mut ShellSession,
        reg: &CommandRegistry,
        vfs: &MemoryVfs,
        ev: InputEvent,
    ) -> Option<SessionEvent> {
        session.handle_event(&ev, reg, "/home/user", vfs)
    }

    #[test]
    fn cursor_editing_ops() {
        let (reg, vfs) = (reg(), vfs());
        let mut s = ShellSession::new();
        typed(&mut s, &reg, &vfs, "echo wrld");
        // Left x3 (d-pad / arrow twin), insert 'o'.
        for _ in 0..3 {
            event(&mut s, &reg, &vfs, InputEvent::ButtonPress(Button::Left));
        }
        typed(&mut s, &reg, &vfs, "o");
        assert_eq!(s.buffer(), "echo world");
        assert_eq!(s.cursor_col(), 7);
        // Home, Delete -> "cho world".
        key(&mut s, &reg, &vfs, Key::Home, Modifiers::NONE);
        assert_eq!(s.cursor_col(), 0);
        key(&mut s, &reg, &vfs, Key::Delete, Modifiers::NONE);
        assert_eq!(s.buffer(), "cho world");
        // End + Backspace.
        key(&mut s, &reg, &vfs, Key::End, Modifiers::NONE);
        event(&mut s, &reg, &vfs, InputEvent::Backspace);
        assert_eq!(s.buffer(), "cho worl");
        // Ctrl+W deletes the word before the cursor; Ctrl+Y yanks it back.
        key(&mut s, &reg, &vfs, Key::Char('w'), Modifiers::CTRL);
        assert_eq!(s.buffer(), "cho ");
        key(&mut s, &reg, &vfs, Key::Char('y'), Modifiers::CTRL);
        assert_eq!(s.buffer(), "cho worl");
        // Ctrl+A then Ctrl+K kills the whole line; Ctrl+U on empty is a no-op.
        key(&mut s, &reg, &vfs, Key::Char('a'), Modifiers::CTRL);
        key(&mut s, &reg, &vfs, Key::Char('k'), Modifiers::CTRL);
        assert_eq!(s.buffer(), "");
        typed(&mut s, &reg, &vfs, "abc def");
        key(&mut s, &reg, &vfs, Key::Char('b'), Modifiers::ALT);
        key(&mut s, &reg, &vfs, Key::Char('u'), Modifiers::CTRL);
        assert_eq!(s.buffer(), "def");
        assert_eq!(s.cursor_col(), 0);
        key(&mut s, &reg, &vfs, Key::Char('e'), Modifiers::CTRL);
        assert_eq!(s.cursor_col(), 3);
    }

    #[test]
    fn control_keys_report_events() {
        let (reg, vfs) = (reg(), vfs());
        let mut s = ShellSession::new();
        typed(&mut s, &reg, &vfs, "oops");
        assert_eq!(
            key(&mut s, &reg, &vfs, Key::Char('c'), Modifiers::CTRL),
            Some(SessionEvent::Interrupted("oops".into()))
        );
        assert_eq!(s.buffer(), "");
        assert_eq!(
            key(&mut s, &reg, &vfs, Key::Char('l'), Modifiers::CTRL),
            Some(SessionEvent::ClearScreen)
        );
        typed(&mut s, &reg, &vfs, "ls");
        assert_eq!(
            event(&mut s, &reg, &vfs, InputEvent::ButtonPress(Button::Confirm)),
            Some(SessionEvent::Submit("ls".into()))
        );
        // WM combos and plain Escape / letters are not claimed.
        assert!(key(&mut s, &reg, &vfs, Key::Tab, Modifiers::ALT).is_none());
        assert!(key(&mut s, &reg, &vfs, Key::Left, Modifiers::SUPER).is_none());
        let ctrl_alt = Modifiers::CTRL | Modifiers::ALT;
        assert!(key(&mut s, &reg, &vfs, Key::Char('t'), ctrl_alt).is_none());
        assert!(key(&mut s, &reg, &vfs, Key::Escape, Modifiers::NONE).is_none());
        assert!(key(&mut s, &reg, &vfs, Key::Char('q'), Modifiers::NONE).is_none());
        assert!(event(&mut s, &reg, &vfs, InputEvent::ButtonPress(Button::Cancel)).is_none());
    }

    #[test]
    fn history_recall_with_up_down() {
        let (reg, vfs) = (reg(), vfs());
        reg.set_history(vec!["ls".into(), "pwd".into(), "echo hi".into()]);
        let mut s = ShellSession::new();
        typed(&mut s, &reg, &vfs, "draft");
        event(&mut s, &reg, &vfs, InputEvent::ButtonPress(Button::Up));
        assert_eq!(s.buffer(), "echo hi");
        event(&mut s, &reg, &vfs, InputEvent::ButtonPress(Button::Up));
        event(&mut s, &reg, &vfs, InputEvent::ButtonPress(Button::Up));
        event(&mut s, &reg, &vfs, InputEvent::ButtonPress(Button::Up));
        assert_eq!(s.buffer(), "ls");
        event(&mut s, &reg, &vfs, InputEvent::ButtonPress(Button::Down));
        assert_eq!(s.buffer(), "pwd");
        key(&mut s, &reg, &vfs, Key::Char('n'), Modifiers::CTRL);
        key(&mut s, &reg, &vfs, Key::Char('n'), Modifiers::CTRL);
        assert_eq!(
            s.buffer(),
            "draft",
            "Down past the newest restores the draft"
        );
    }

    #[test]
    fn reverse_search_finds_and_submits() {
        let (reg, vfs) = (reg(), vfs());
        reg.set_history(vec![
            "cat notes.txt".into(),
            "ls /home".into(),
            "cat other".into(),
        ]);
        let mut s = ShellSession::new();
        key(&mut s, &reg, &vfs, Key::Char('r'), Modifiers::CTRL);
        assert!(s.is_searching());
        typed(&mut s, &reg, &vfs, "cat");
        assert_eq!(s.buffer(), "cat other");
        assert_eq!(s.display().0, "(reverse-i-search)'cat': cat other");
        // Ctrl+R again: next older match.
        key(&mut s, &reg, &vfs, Key::Char('r'), Modifiers::CTRL);
        assert_eq!(s.buffer(), "cat notes.txt");
        // Backspace edits the query.
        event(&mut s, &reg, &vfs, InputEvent::Backspace);
        assert_eq!(s.editor().search_query(), "ca");
        // Esc cancels back to the empty line.
        key(&mut s, &reg, &vfs, Key::Escape, Modifiers::NONE);
        assert!(!s.is_searching());
        assert_eq!(s.buffer(), "");
        // Enter while searching submits the match.
        key(&mut s, &reg, &vfs, Key::Char('r'), Modifiers::CTRL);
        typed(&mut s, &reg, &vfs, "ls");
        assert_eq!(
            event(&mut s, &reg, &vfs, InputEvent::ButtonPress(Button::Confirm)),
            Some(SessionEvent::Submit("ls /home".into()))
        );
    }

    #[test]
    fn tab_completes_commands_and_lists_candidates() {
        let (reg, vfs) = (reg(), vfs());
        let mut s = ShellSession::new();
        typed(&mut s, &reg, &vfs, "hist");
        event(&mut s, &reg, &vfs, InputEvent::Tab);
        assert_eq!(s.buffer(), "history ");
        // After a pipe the next word is in command position again.
        typed(&mut s, &reg, &vfs, "| wh");
        match event(&mut s, &reg, &vfs, InputEvent::Tab) {
            Some(SessionEvent::Candidates(c)) => {
                assert!(c.contains(&"which".to_string()), "{c:?}");
                assert!(c.contains(&"whoami".to_string()), "{c:?}");
            },
            other => panic!("expected candidates, got {other:?}"),
        }
    }

    #[test]
    fn tab_completes_vfs_paths() {
        let (reg, vfs) = (reg(), vfs());
        let mut s = ShellSession::new();
        typed(&mut s, &reg, &vfs, "cat no");
        event(&mut s, &reg, &vfs, InputEvent::Tab);
        assert_eq!(s.buffer(), "cat notes.txt ");

        // Ambiguous: common prefix "d" + "o" is added first ...
        let mut s = ShellSession::new();
        typed(&mut s, &reg, &vfs, "ls /home/user/d");
        event(&mut s, &reg, &vfs, InputEvent::Tab);
        assert_eq!(s.buffer(), "ls /home/user/do");
        // ... then the candidates are listed by name.
        assert_eq!(
            event(&mut s, &reg, &vfs, InputEvent::Tab),
            Some(SessionEvent::Candidates(vec![
                "docs/".into(),
                "downloads/".into()
            ]))
        );
        // A unique directory gets its slash and no trailing space.
        typed(&mut s, &reg, &vfs, "c");
        event(&mut s, &reg, &vfs, InputEvent::Tab);
        assert_eq!(s.buffer(), "ls /home/user/docs/");

        // Completion in the middle of the line keeps the tail.
        let mut s = ShellSession::new();
        typed(&mut s, &reg, &vfs, "cat no | wc");
        for _ in 0..5 {
            event(&mut s, &reg, &vfs, InputEvent::ButtonPress(Button::Left));
        }
        event(&mut s, &reg, &vfs, InputEvent::Tab);
        assert_eq!(s.buffer(), "cat notes.txt  | wc");
    }

    #[test]
    fn history_persistence_round_trip() {
        let mut vfs = MemoryVfs::new();
        let reg = reg();
        let session = ShellSession::new();
        assert_eq!(session.load_history(&reg, &vfs), 0);

        let mut env = crate::Environment {
            cwd: "/".to_string(),
            vfs: &mut vfs,
            power: None,
            time: None,
            usb: None,
            network: None,
            tls: None,
            stdin: None,
            stderr: String::new(),
        };
        reg.execute("echo one", &mut env).expect("echo");
        reg.execute("echo two", &mut env).expect("echo");
        drop(env);
        session.save_history(&reg, &mut vfs).expect("save");
        let saved = vfs.read(HISTORY_PATH).expect("history file");
        assert_eq!(saved, b"echo one\necho two\n");

        let fresh = reg_without_history();
        assert_eq!(session.load_history(&fresh, &vfs), 2);
        assert_eq!(fresh.history(), ["echo one", "echo two"]);
    }

    #[test]
    fn history_is_bounded_on_load() {
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/home/user").expect("mkdir");
        let lines: Vec<String> = (0..2000).map(|i| format!("cmd {i}")).collect();
        vfs.write(HISTORY_PATH, lines.join("\n").as_bytes())
            .expect("write");
        let reg = reg();
        let n = ShellSession::new().load_history(&reg, &vfs);
        assert_eq!(n, crate::types::MAX_HISTORY);
        assert_eq!(reg.history().last().map(String::as_str), Some("cmd 1999"));
    }

    fn reg_without_history() -> CommandRegistry {
        CommandRegistry::new()
    }
}
