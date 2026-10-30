//! Calculator application with expression evaluation.
//!
//! Provides a full-featured calculator with operator precedence, parentheses,
//! memory registers, and calculation history. The expression evaluator is a
//! pure recursive-descent parser with no external dependencies.
//!
//! Input: type `0-9 . + - * / ^ % ( )`, `=` / Enter evaluates, Backspace
//! deletes, Escape clears (and closes an already-clear calculator), Up /
//! Down recall history. Windowed mode draws a themed display panel and a
//! clickable key grid ([`keypad`]); on a gamepad the d-pad moves a keypad
//! cursor and Confirm presses the key under it. History persists to
//! [`history::HISTORY_PATH`].

use oasis_app_core::{App, AppAction, ContentState};
use oasis_skin::ActiveTheme;
use oasis_types::backend::SdiBackend;
use oasis_types::input::{Button, Key, Modifiers};
use oasis_vfs::Vfs;

mod eval;
pub mod history;
pub mod keypad;
mod render;

use eval::evaluate_after;
pub use eval::{CalcError, evaluate};
use keypad::{CalcKey, CalcLayout, Dir, KEYS};
pub use render::CalcColors;

// ---------------------------------------------------------------
// CalcHistoryEntry
// ---------------------------------------------------------------

/// A single calculation stored in history.
#[derive(Debug, Clone)]
pub struct CalcHistoryEntry {
    /// The expression that was evaluated.
    pub expression: String,
    /// The computed result.
    pub result: f64,
}

// ---------------------------------------------------------------
// CalculatorApp
// ---------------------------------------------------------------

/// How long (ms of wall time) a pressed key stays drawn pressed.
const FLASH_MS: u32 = 100;

/// Calculator application state.
#[derive(Debug)]
pub struct CalculatorApp {
    content: ContentState,
    /// Content-area size recorded by the last `App::layout` (click
    /// hit-testing).
    content_size: (u32, u32),
    /// Current display value (formatted result or input echo).
    display: String,
    /// Expression being typed by the user.
    input_buffer: String,
    /// Previous calculations (oldest first).
    history: Vec<CalcHistoryEntry>,
    /// Memory register (M+, MS, MR, MC).
    memory: f64,
    /// Result of the last successful evaluation.
    last_result: Option<f64>,
    /// Set when an operator started the entry from `last_result`: the
    /// exact value and the length of its display text at the start of
    /// `input_buffer`. While that prefix is intact, evaluation uses the
    /// exact value instead of re-parsing the rounded text.
    chained: Option<(usize, f64)>,
    /// Current error message (cleared on next input).
    error_message: Option<String>,
    /// Keypad cursor: index into [`keypad::KEYS`] (d-pad navigation).
    cursor: usize,
    /// Whether the keypad cursor is drawn (d-pad in use, not mouse/typing).
    cursor_visible: bool,
    /// History entry currently recalled with Up/Down (index into `history`).
    recall_index: Option<usize>,
    /// Key drawn pressed, with the ms left before it pops back up (aged in
    /// `tick`, so it pops back up even while the host elides idle frames).
    flash: Option<(usize, u32)>,
    /// Whether history has been loaded from the VFS yet.
    history_loaded: bool,
    /// Whether history changed since it was last written to the VFS.
    history_dirty: bool,
}

impl CalculatorApp {
    /// Create a new calculator app at the given VFS path.
    ///
    /// Persisted history is loaded from [`history::HISTORY_PATH`] on the
    /// first input ([`App::handle_input`] / [`App::handle_key`] /
    /// [`App::handle_click`]) or [`App::apply_vfs_ops`] call.
    pub fn new(path: &str) -> Self {
        let content = ContentState::new("Calculator", path);
        let mut app = Self {
            content,
            display: "0".to_string(),
            input_buffer: String::new(),
            history: Vec::new(),
            memory: 0.0,
            last_result: None,
            chained: None,
            error_message: None,
            cursor: keypad::index_of(CalcKey::Digit('5')).unwrap_or(0),
            cursor_visible: false,
            recall_index: None,
            flash: None,
            history_loaded: false,
            history_dirty: false,
            content_size: (0, 0),
        };
        app.refresh_lines();
        app
    }

    /// Append a digit or decimal point to the input buffer.
    pub fn push_digit(&mut self, d: char) {
        self.error_message = None;
        self.recall_index = None;

        // Prevent multiple leading zeros (allow "0." but not "00").
        if d == '0' && self.input_buffer == "0" {
            return;
        }

        // Prevent multiple decimal points in the current number.
        if d == '.' {
            // Find the last number segment (after last operator/paren).
            let last_num_start = self
                .input_buffer
                .rfind(|c: char| "+-*/%^()".contains(c))
                .map_or(0, |p| p + 1);
            let current_num = &self.input_buffer[last_num_start..];
            if current_num.contains('.') {
                return;
            }
        }

        // Replace lone "0" with the digit (unless it's "0.").
        if self.input_buffer == "0" && d != '.' {
            self.input_buffer.clear();
        }

        self.input_buffer.push(d);
        self.display = self.input_buffer.clone();
        self.refresh_lines();
    }

    /// Append an operator (+, -, *, /, ^, %) to the input buffer.
    pub fn push_operator(&mut self, op: char) {
        self.error_message = None;
        self.recall_index = None;

        // If buffer is empty but we have a last result, start from it.
        if self.input_buffer.is_empty() {
            if let Some(result) = self.last_result {
                self.input_buffer = format_number(result);
                self.chained = Some((self.input_buffer.len(), result));
            } else {
                self.input_buffer.push('0');
            }
        }

        // Replace trailing operator with the new one.
        if let Some(last) = self.input_buffer.chars().last()
            && "+-*/%^".contains(last)
        {
            self.input_buffer.pop();
        }

        self.input_buffer.push(op);
        self.display = self.input_buffer.clone();
        self.refresh_lines();
    }

    /// Type a `-`: a unary minus at the start of the entry or after an
    /// operator / `(`, otherwise the subtraction operator.
    pub fn push_minus(&mut self) {
        let unary = match self.input_buffer.chars().last() {
            None => self.last_result.is_none(),
            Some(c) => "*/^%(".contains(c),
        };
        if unary {
            self.error_message = None;
            self.recall_index = None;
            self.input_buffer.push('-');
            self.display = self.input_buffer.clone();
            self.refresh_lines();
        } else {
            self.push_operator('-');
        }
    }

    /// Append an opening or closing parenthesis.
    pub fn push_paren(&mut self, open: bool) {
        self.error_message = None;
        self.recall_index = None;

        if open {
            self.input_buffer.push('(');
        } else {
            self.input_buffer.push(')');
        }
        self.display = self.input_buffer.clone();
        self.refresh_lines();
    }

    /// Evaluate the current input expression.
    pub fn evaluate_input(&mut self) {
        self.recall_index = None;
        if self.input_buffer.is_empty() {
            return;
        }

        let outcome = match self.chained_operand() {
            Some((len, value)) => evaluate_after(value, &self.input_buffer[len..]),
            None => evaluate(&self.input_buffer),
        };
        match outcome {
            Ok(result) => {
                let entry = CalcHistoryEntry {
                    expression: self.input_buffer.clone(),
                    result,
                };
                self.history.push(entry);
                let excess = self.history.len().saturating_sub(history::MAX_HISTORY);
                self.history.drain(..excess);
                self.history_dirty = true;
                self.display = format_number(result);
                self.last_result = Some(result);
                self.chained = None;
                self.error_message = None;
                self.input_buffer.clear();
            },
            Err(e) => {
                self.error_message = Some(e.to_string());
                self.display = e.to_string();
            },
        }
        self.refresh_lines();
    }

    /// The exact chained value and its prefix length, if the entry still
    /// starts with that value's unedited display text.
    fn chained_operand(&self) -> Option<(usize, f64)> {
        let (len, value) = self.chained?;
        let prefix = self.input_buffer.get(..len)?;
        let extended = self.input_buffer[len..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit() || c == '.');
        (prefix == format_number(value) && !extended).then_some((len, value))
    }

    /// Clear the current input (C).
    pub fn clear(&mut self) {
        self.chained = None;
        self.input_buffer.clear();
        self.display = "0".to_string();
        self.error_message = None;
        self.recall_index = None;
        self.refresh_lines();
    }

    /// Clear input, last result and history (AC). The cleared history is
    /// persisted on the next [`App::apply_vfs_ops`].
    pub fn clear_all(&mut self) {
        self.chained = None;
        self.input_buffer.clear();
        self.display = "0".to_string();
        if !self.history.is_empty() {
            self.history.clear();
            self.history_dirty = true;
        }
        self.last_result = None;
        self.error_message = None;
        self.recall_index = None;
        self.refresh_lines();
    }

    /// Delete the last character from the input buffer.
    pub fn backspace(&mut self) {
        self.error_message = None;
        self.recall_index = None;
        self.input_buffer.pop();
        if self.input_buffer.is_empty() {
            self.display = "0".to_string();
        } else {
            self.display = self.input_buffer.clone();
        }
        self.refresh_lines();
    }

    /// Value MS / M+ act on: the typed number if the entry is a plain
    /// number, otherwise the last result.
    fn memory_operand(&self) -> Option<f64> {
        if !self.input_buffer.is_empty() {
            return self
                .input_buffer
                .parse::<f64>()
                .ok()
                .filter(|v| v.is_finite())
                .or(self.last_result);
        }
        self.last_result
    }

    /// Store the current value to memory (MS).
    pub fn memory_store(&mut self) {
        if let Some(v) = self.memory_operand() {
            self.memory = v;
        }
        self.refresh_lines();
    }

    /// Recall memory value into the input buffer (MR).
    pub fn memory_recall(&mut self) {
        self.insert_value(self.memory);
    }

    /// Insert the last result into the input buffer (Ans).
    pub fn insert_ans(&mut self) {
        if let Some(r) = self.last_result {
            self.insert_value(r);
        }
    }

    /// Append `value` to the entry (a fresh entry replaces a lone "0").
    fn insert_value(&mut self, value: f64) {
        self.error_message = None;
        self.recall_index = None;
        if self.input_buffer == "0" {
            self.input_buffer.clear();
        }
        self.input_buffer.push_str(&format_number(value));
        self.display = self.input_buffer.clone();
        self.refresh_lines();
    }

    /// Add the current value to memory (M+).
    pub fn memory_add(&mut self) {
        // An overflowing sum would leave `inf` in memory, which MR can't
        // insert as a typeable number: keep the old value instead.
        if let Some(sum) = self.memory_operand().map(|v| self.memory + v)
            && sum.is_finite()
        {
            self.memory = sum;
        }
        self.refresh_lines();
    }

    /// Clear the memory register (MC).
    pub fn memory_clear(&mut self) {
        self.memory = 0.0;
        self.refresh_lines();
    }

    /// Toggle the sign of the current input.
    pub fn negate(&mut self) {
        self.error_message = None;
        self.recall_index = None;

        if self.input_buffer.is_empty() {
            if let Some(result) = self.last_result {
                self.last_result = Some(-result);
                self.display = format_number(-result);
                self.refresh_lines();
            }
            return;
        }

        toggle_last_sign(&mut self.input_buffer);

        if self.input_buffer.is_empty() {
            self.display = "0".to_string();
        } else {
            self.display = self.input_buffer.clone();
        }
        self.refresh_lines();
    }

    /// Recall an older (`older == true`) or newer history expression into
    /// the entry. Stepping newer past the newest entry clears the entry.
    pub fn recall_history(&mut self, older: bool) {
        if self.history.is_empty() {
            return;
        }
        let next = match (self.recall_index, older) {
            (None, true) => Some(self.history.len() - 1),
            (None, false) => None,
            (Some(i), true) => Some(i.saturating_sub(1)),
            (Some(i), false) => (i + 1 < self.history.len()).then_some(i + 1),
        };
        self.error_message = None;
        self.chained = None;
        match next {
            Some(i) => {
                self.input_buffer = self.history[i].expression.clone();
                self.display = self.input_buffer.clone();
            },
            None => {
                self.input_buffer.clear();
                self.display = "0".to_string();
            },
        }
        self.recall_index = next;
        self.refresh_lines();
    }

    /// Press a keypad key (click, d-pad Confirm or a typed character).
    pub fn press_key(&mut self, key: CalcKey) {
        if let Some(i) = keypad::index_of(key) {
            self.flash = Some((i, FLASH_MS));
        }
        match key {
            CalcKey::Digit(d) => self.push_digit(d),
            CalcKey::Op('-') => self.push_minus(),
            CalcKey::Op(op) => self.push_operator(op),
            CalcKey::OpenParen => self.push_paren(true),
            CalcKey::CloseParen => self.push_paren(false),
            CalcKey::Equals => self.evaluate_input(),
            CalcKey::Clear => self.clear(),
            CalcKey::AllClear => self.clear_all(),
            CalcKey::Backspace => self.backspace(),
            CalcKey::Negate => self.negate(),
            CalcKey::Ans => self.insert_ans(),
            CalcKey::MemClear => self.memory_clear(),
            CalcKey::MemRecall => self.memory_recall(),
            CalcKey::MemStore => self.memory_store(),
            CalcKey::MemAdd => self.memory_add(),
        }
    }

    /// Key a typed character stands for, if any.
    fn key_for_char(ch: char) -> Option<CalcKey> {
        Some(match ch {
            '0'..='9' | '.' => CalcKey::Digit(ch),
            ',' => CalcKey::Digit('.'),
            '+' | '-' | '*' | '/' | '^' | '%' => CalcKey::Op(ch),
            'x' | 'X' => CalcKey::Op('*'),
            '(' => CalcKey::OpenParen,
            ')' => CalcKey::CloseParen,
            '=' | '\n' | '\r' => CalcKey::Equals,
            _ => return None,
        })
    }

    /// Whether Escape has anything to clear (otherwise it closes the app).
    fn has_entry(&self) -> bool {
        !self.input_buffer.is_empty() || self.error_message.is_some() || self.display != "0"
    }

    /// Load persisted history once, keeping entries made before the load.
    fn ensure_history_loaded(&mut self, vfs: &dyn Vfs) -> bool {
        if self.history_loaded {
            return false;
        }
        self.history_loaded = true;
        let mut loaded = history::load(vfs);
        if loaded.is_empty() {
            return false;
        }
        loaded.append(&mut self.history);
        let excess = loaded.len().saturating_sub(history::MAX_HISTORY);
        loaded.drain(..excess);
        self.history = loaded;
        self.recall_index = None;
        self.refresh_lines();
        true
    }

    /// Format the calculator state into display lines (full-screen text
    /// mode and the generic line renderer).
    pub fn format_display_lines(&self) -> Vec<String> {
        let separator = "\u{2500}".repeat(30); // box-drawing horizontal line

        let mut lines = Vec::new();
        // No heading line — the app title already shows in the WM /
        // app-chrome title bar.

        // Input / result area.
        if self.input_buffer.is_empty() {
            lines.push(format!("  Result: {}", self.display));
        } else {
            lines.push(format!("  Input: {}", self.input_buffer));
            if let Some(ref err) = self.error_message {
                lines.push(format!("  Error: {err}"));
            } else if let Some(result) = self.last_result {
                lines.push(format!("  Last: {}", format_number(result)));
            }
        }

        // Keypad cursor (d-pad selects, Confirm presses).
        let key = KEYS.get(self.cursor).map_or("", |d| d.key.label());
        lines.push(format!(
            "  Key: [{key}]    Memory: {}",
            format_number(self.memory)
        ));

        lines.push(separator.clone());

        // History section.
        if self.history.is_empty() {
            lines.push("  History: (empty)".to_string());
        } else {
            lines.push("  History:".to_string());
            // Show most recent entries last (up to 10).
            let start = self.history.len().saturating_sub(10);
            for entry in &self.history[start..] {
                lines.push(format!(
                    "    {} = {}",
                    entry.expression,
                    format_number(entry.result)
                ));
            }
        }

        lines.push(separator);

        // Controls help.
        lines.push("  [D-pad]=Move  [Confirm]=Press  [Square]=DEL".to_string());
        lines.push("  [Triangle]=C  [Start]=AC  [Select]=Recall".to_string());

        lines
    }

    /// Rebuild the `content.lines` from the current state.
    fn refresh_lines(&mut self) {
        self.content.lines = self.format_display_lines();
    }
}

/// Toggle the sign of the last number in `buf` (`5+3` -> `5+-3`): remove
/// the unary minus in front of it, or insert one. With no trailing number
/// (`5*`), toggle a unary minus at the end; after `)` negate the group.
fn toggle_last_sign(buf: &mut String) {
    let mut start = buf
        .rfind(|c: char| !(c.is_ascii_digit() || c == '.'))
        .map_or(0, |p| p + 1);
    if start == buf.len() && buf.ends_with(')') {
        // Negate the whole trailing `( ... )` group.
        let mut depth = 0usize;
        let mut open = None;
        for (i, c) in buf.char_indices().rev() {
            match c {
                ')' => depth += 1,
                '(' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        open = Some(i);
                        break;
                    }
                },
                _ => {},
            }
        }
        let Some(open) = open else {
            return;
        };
        start = open;
    }
    // A '-' right before `start` is unary when it opens the entry or
    // follows an operator / '('.
    let unary_minus_before = start > 0
        && buf[..start].ends_with('-')
        && buf[..start - 1]
            .chars()
            .last()
            .is_none_or(|c| "+-*/%^(".contains(c));
    if unary_minus_before {
        buf.remove(start - 1);
    } else {
        buf.insert(start, '-');
    }
}

/// Format a number for display, removing trailing zeros from decimals.
fn format_number(n: f64) -> String {
    if n.is_infinite() {
        return if n.is_sign_positive() {
            "Infinity".to_string()
        } else {
            "-Infinity".to_string()
        };
    }
    if n.is_nan() {
        return "NaN".to_string();
    }
    // If the number is effectively an integer, display without decimals.
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        // Up to 10 decimal places, strip trailing zeros.
        let s = format!("{n:.10}");
        let s = s.trim_end_matches('0');
        let s = s.trim_end_matches('.');
        s.to_string()
    }
}

// ---------------------------------------------------------------
// App trait implementation
// ---------------------------------------------------------------

impl App for CalculatorApp {
    fn title(&self) -> &str {
        &self.content.title
    }

    fn path(&self) -> &str {
        &self.content.app_path
    }

    fn handle_input(&mut self, button: &Button, vfs: &dyn Vfs) -> AppAction {
        self.ensure_history_loaded(vfs);
        let dir = match button {
            Button::Cancel => return AppAction::Exit,
            Button::Up => Some(Dir::Up),
            Button::Down => Some(Dir::Down),
            Button::Left => Some(Dir::Left),
            Button::Right => Some(Dir::Right),
            // Confirm presses the key under the keypad cursor.
            Button::Confirm => {
                if let Some(def) = KEYS.get(self.cursor) {
                    self.press_key(def.key);
                }
                None
            },
            Button::Triangle => {
                self.press_key(CalcKey::Clear);
                None
            },
            Button::Square => {
                self.press_key(CalcKey::Backspace);
                None
            },
            Button::Start => {
                self.press_key(CalcKey::AllClear);
                None
            },
            // Select steps back through history (gamepad Up/Down move
            // the keypad cursor).
            Button::Select => {
                self.recall_history(true);
                None
            },
        };
        if let Some(dir) = dir {
            self.cursor = keypad::step(self.cursor, dir);
            self.cursor_visible = true;
            self.refresh_lines();
        }
        AppAction::None
    }

    fn handle_key(&mut self, key: &Key, mods: Modifiers, vfs: &dyn Vfs) -> Option<AppAction> {
        if mods.has_command() {
            return None;
        }
        self.ensure_history_loaded(vfs);
        match key {
            Key::Enter => self.press_key(CalcKey::Equals),
            Key::Backspace => self.press_key(CalcKey::Backspace),
            Key::Delete => self.press_key(CalcKey::Clear),
            // Escape clears; on an already-clear calculator it falls
            // through to Cancel and closes the app.
            Key::Escape if self.has_entry() => self.press_key(CalcKey::Clear),
            Key::Up => self.recall_history(true),
            Key::Down => self.recall_history(false),
            _ => return None,
        }
        self.cursor_visible = false;
        Some(AppAction::None)
    }

    fn accepts_text(&self) -> bool {
        true
    }

    fn handle_text_input(&mut self, ch: char) {
        if let Some(key) = Self::key_for_char(ch) {
            self.cursor_visible = false;
            self.press_key(key);
        }
    }

    fn handle_backspace(&mut self) {
        self.press_key(CalcKey::Backspace);
    }

    /// Up / Down move keypad focus (or recall history), which the wheel
    /// shouldn't do: the calculator has nothing to scroll.
    fn handle_scroll(&mut self, _delta: i32, _vfs: &dyn Vfs) -> bool {
        false
    }

    fn layout(&mut self, cw: u32, ch: u32, _at: &ActiveTheme) {
        self.content_size = (cw, ch);
    }

    fn handle_click(&mut self, lx: i32, ly: i32, vfs: &dyn Vfs) -> AppAction {
        self.ensure_history_loaded(vfs);
        let (cw, ch) = self.content_size;
        let l = CalcLayout::compute(0, 0, cw, ch);
        if let Some(i) = l.key_at(lx, ly) {
            self.cursor = i;
            self.cursor_visible = false;
            self.press_key(KEYS[i].key);
        } else if let Some(row) = l.history_row_at(lx, ly)
            && let Some(idx) = self.history.len().checked_sub(row + 1)
        {
            // History rows are newest first; clicking one recalls it.
            self.recall_index = Some(idx);
            self.input_buffer = self.history[idx].expression.clone();
            self.display = self.input_buffer.clone();
            self.error_message = None;
            self.refresh_lines();
        }
        AppAction::None
    }

    fn tick(&mut self, dt_ms: u32, _vfs: &dyn Vfs) -> bool {
        // Age the pressed-key flash; redraw when it pops back up.
        let Some((key, left)) = self.flash else {
            return false;
        };
        self.flash = (left > dt_ms).then(|| (key, left - dt_ms));
        self.flash.is_none()
    }

    fn apply_vfs_ops(&mut self, vfs: &mut dyn Vfs) -> bool {
        let mut changed = self.ensure_history_loaded(vfs);
        if self.history_dirty {
            self.history_dirty = false;
            if let Err(e) = history::save(vfs, &self.history) {
                self.error_message = Some(format!("History not saved: {e}"));
                self.refresh_lines();
            }
            changed = true;
        }
        changed
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
        self.draw_calculator(cx, cy, cw, ch, backend, at)
    }

    fn lines(&self) -> &[String] {
        &self.content.lines
    }
}

#[cfg(test)]
mod tests;
