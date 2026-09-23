//! Calculator keypad: key table, content-area layout and hit-testing.
//!
//! [`CalcLayout::compute`] is the single source of truth for where the
//! display panel, the key grid and the history pane sit. Both the windowed
//! renderer and the click hit-tester call it with the same content size,
//! so a click on a drawn key always presses that key.

/// A calculator key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalcKey {
    /// Digit `0`-`9` or the decimal point.
    Digit(char),
    /// Binary operator (`+ - * / ^ %`).
    Op(char),
    /// Opening parenthesis.
    OpenParen,
    /// Closing parenthesis.
    CloseParen,
    /// Evaluate (`=`).
    Equals,
    /// Clear the current entry (C).
    Clear,
    /// Clear entry, last result and history (AC).
    AllClear,
    /// Delete the last character.
    Backspace,
    /// Toggle the sign of the entry.
    Negate,
    /// Insert the last result.
    Ans,
    /// Memory clear.
    MemClear,
    /// Memory recall.
    MemRecall,
    /// Memory store.
    MemStore,
    /// Memory add.
    MemAdd,
}

impl CalcKey {
    /// Label drawn on the key cap.
    pub fn label(self) -> &'static str {
        match self {
            CalcKey::Digit(c) => match c {
                '0' => "0",
                '1' => "1",
                '2' => "2",
                '3' => "3",
                '4' => "4",
                '5' => "5",
                '6' => "6",
                '7' => "7",
                '8' => "8",
                '9' => "9",
                _ => ".",
            },
            CalcKey::Op(c) => match c {
                '+' => "+",
                '-' => "-",
                '*' => "x",
                '/' => "/",
                '^' => "^",
                _ => "%",
            },
            CalcKey::OpenParen => "(",
            CalcKey::CloseParen => ")",
            CalcKey::Equals => "=",
            CalcKey::Clear => "C",
            CalcKey::AllClear => "AC",
            CalcKey::Backspace => "DEL",
            CalcKey::Negate => "+/-",
            CalcKey::Ans => "Ans",
            CalcKey::MemClear => "MC",
            CalcKey::MemRecall => "MR",
            CalcKey::MemStore => "MS",
            CalcKey::MemAdd => "M+",
        }
    }

    /// Visual group, used to pick the key cap style.
    pub fn group(self) -> KeyGroup {
        match self {
            CalcKey::Digit(_) | CalcKey::Negate => KeyGroup::Digit,
            CalcKey::Op(_) | CalcKey::OpenParen | CalcKey::CloseParen | CalcKey::Ans => {
                KeyGroup::Operator
            },
            CalcKey::Equals => KeyGroup::Equals,
            CalcKey::Clear | CalcKey::AllClear | CalcKey::Backspace => KeyGroup::Edit,
            CalcKey::MemClear | CalcKey::MemRecall | CalcKey::MemStore | CalcKey::MemAdd => {
                KeyGroup::Memory
            },
        }
    }
}

/// Key cap style groups.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyGroup {
    /// Digits, decimal point, sign toggle.
    Digit,
    /// Operators, parentheses, Ans.
    Operator,
    /// The `=` key.
    Equals,
    /// C / AC / DEL.
    Edit,
    /// MC / MR / MS / M+.
    Memory,
}

/// One key on the grid: its cell position and span (in grid cells).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyDef {
    /// The key.
    pub key: CalcKey,
    /// Leftmost grid column.
    pub col: u8,
    /// Grid row.
    pub row: u8,
    /// Number of columns spanned.
    pub span: u8,
}

/// Grid columns.
pub const COLS: u8 = 5;
/// Grid rows.
pub const ROWS: u8 = 6;

const fn k(key: CalcKey, col: u8, row: u8) -> KeyDef {
    KeyDef {
        key,
        col,
        row,
        span: 1,
    }
}

/// The keypad, row-major. Every grid cell is covered by exactly one key.
///
/// ```text
///  MC   MR   MS   M+   AC
///  (    )    ^    %    /
///  7    8    9    x    DEL
///  4    5    6    -    C
///  1    2    3    +    Ans
///  +/-  0    .    [   =   ]
/// ```
pub const KEYS: &[KeyDef] = &[
    k(CalcKey::MemClear, 0, 0),
    k(CalcKey::MemRecall, 1, 0),
    k(CalcKey::MemStore, 2, 0),
    k(CalcKey::MemAdd, 3, 0),
    k(CalcKey::AllClear, 4, 0),
    k(CalcKey::OpenParen, 0, 1),
    k(CalcKey::CloseParen, 1, 1),
    k(CalcKey::Op('^'), 2, 1),
    k(CalcKey::Op('%'), 3, 1),
    k(CalcKey::Op('/'), 4, 1),
    k(CalcKey::Digit('7'), 0, 2),
    k(CalcKey::Digit('8'), 1, 2),
    k(CalcKey::Digit('9'), 2, 2),
    k(CalcKey::Op('*'), 3, 2),
    k(CalcKey::Backspace, 4, 2),
    k(CalcKey::Digit('4'), 0, 3),
    k(CalcKey::Digit('5'), 1, 3),
    k(CalcKey::Digit('6'), 2, 3),
    k(CalcKey::Op('-'), 3, 3),
    k(CalcKey::Clear, 4, 3),
    k(CalcKey::Digit('1'), 0, 4),
    k(CalcKey::Digit('2'), 1, 4),
    k(CalcKey::Digit('3'), 2, 4),
    k(CalcKey::Op('+'), 3, 4),
    k(CalcKey::Ans, 4, 4),
    k(CalcKey::Negate, 0, 5),
    k(CalcKey::Digit('0'), 1, 5),
    k(CalcKey::Digit('.'), 2, 5),
    KeyDef {
        key: CalcKey::Equals,
        col: 3,
        row: 5,
        span: 2,
    },
];

/// Index into [`KEYS`] of the key covering grid cell `(col, row)`.
pub fn key_at_cell(col: u8, row: u8) -> Option<usize> {
    KEYS.iter()
        .position(|d| d.row == row && col >= d.col && col < d.col + d.span)
}

/// Index into [`KEYS`] of `key`.
pub fn index_of(key: CalcKey) -> Option<usize> {
    KEYS.iter().position(|d| d.key == key)
}

/// Direction for keypad cursor movement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Up,
    Down,
    Left,
    Right,
}

/// Key reached by moving the keypad cursor from key `from` in `dir`
/// (wrapping around the grid edges).
pub fn step(from: usize, dir: Dir) -> usize {
    let Some(d) = KEYS.get(from) else {
        return 0;
    };
    let (col, row) = match dir {
        Dir::Up => (d.col, (d.row + ROWS - 1) % ROWS),
        Dir::Down => (d.col, (d.row + 1) % ROWS),
        Dir::Left => ((d.col + COLS - 1) % COLS, d.row),
        Dir::Right => ((d.col + d.span) % COLS, d.row),
    };
    key_at_cell(col, row).unwrap_or(from)
}

/// Axis-aligned rectangle in content-local coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    /// Whether `(x, y)` lies inside the rectangle.
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w as i32 && y < self.y + self.h as i32
    }
}

/// Outer padding around the content.
const PAD: u32 = 4;
/// Gap between key caps.
const GAP: u32 = 3;
/// Display panel height.
pub const DISPLAY_H: u32 = 44;
/// Minimum content width that gets a history pane beside the keypad.
const HISTORY_MIN_W: u32 = 340;
/// Row height of the history pane list.
pub const HISTORY_ROW_H: u32 = 12;

/// Resolved layout for one content rect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CalcLayout {
    /// Display panel (expression + result).
    pub display: Rect,
    /// Key grid area.
    pub grid: Rect,
    /// History pane, when the content is wide enough.
    pub history: Option<Rect>,
    /// Width of one grid cell (including its gap).
    pub cell_w: u32,
    /// Height of one grid cell (including its gap).
    pub cell_h: u32,
}

impl CalcLayout {
    /// Lay out a `cw` x `ch` content rect whose top-left is `(cx, cy)`.
    pub fn compute(cx: i32, cy: i32, cw: u32, ch: u32) -> Self {
        let inner_w = cw.saturating_sub(2 * PAD).max(1);
        let (keypad_w, history) = if cw >= HISTORY_MIN_W {
            let hist_w = inner_w * 2 / 5;
            let keypad_w = inner_w - hist_w - PAD;
            let hist = Rect {
                x: cx + (PAD + keypad_w + PAD) as i32,
                y: cy + PAD as i32,
                w: hist_w,
                h: ch.saturating_sub(2 * PAD).max(1),
            };
            (keypad_w, Some(hist))
        } else {
            (inner_w, None)
        };
        let display = Rect {
            x: cx + PAD as i32,
            y: cy + PAD as i32,
            w: keypad_w,
            h: DISPLAY_H.min(ch.saturating_sub(2 * PAD) / 3).max(1),
        };
        let grid_y = display.y + display.h as i32 + PAD as i32;
        let grid_h = (cy + ch as i32 - PAD as i32 - grid_y).max(ROWS as i32) as u32;
        let cell_w = (keypad_w / COLS as u32).max(1);
        let cell_h = (grid_h / ROWS as u32).max(1);
        let grid = Rect {
            x: display.x,
            y: grid_y,
            w: cell_w * COLS as u32,
            h: cell_h * ROWS as u32,
        };
        Self {
            display,
            grid,
            history,
            cell_w,
            cell_h,
        }
    }

    /// Screen rectangle of the key cap at `KEYS[index]` (gap excluded).
    pub fn key_rect(&self, index: usize) -> Rect {
        let Some(d) = KEYS.get(index) else {
            return Rect {
                x: self.grid.x,
                y: self.grid.y,
                w: 0,
                h: 0,
            };
        };
        let gap = GAP.min(self.cell_w / 4).min(self.cell_h / 4);
        Rect {
            x: self.grid.x + (d.col as u32 * self.cell_w) as i32,
            y: self.grid.y + (d.row as u32 * self.cell_h) as i32,
            w: (self.cell_w * d.span as u32).saturating_sub(gap).max(1),
            h: self.cell_h.saturating_sub(gap).max(1),
        }
    }

    /// Index into [`KEYS`] of the key cap under `(x, y)`. Clicks on the
    /// gaps between caps hit nothing.
    pub fn key_at(&self, x: i32, y: i32) -> Option<usize> {
        if !self.grid.contains(x, y) {
            return None;
        }
        let col = ((x - self.grid.x) as u32 / self.cell_w) as u8;
        let row = ((y - self.grid.y) as u32 / self.cell_h) as u8;
        let idx = key_at_cell(col, row)?;
        self.key_rect(idx).contains(x, y).then_some(idx)
    }

    /// Number of history rows the history pane can show (below its header).
    pub fn history_rows(&self) -> usize {
        self.history
            .map_or(0, |h| (h.h / HISTORY_ROW_H).saturating_sub(1) as usize)
    }

    /// History pane row (0 = newest visible entry) under `(x, y)`.
    pub fn history_row_at(&self, x: i32, y: i32) -> Option<usize> {
        let h = self.history?;
        if !h.contains(x, y) {
            return None;
        }
        let rel = (y - h.y) as u32 / HISTORY_ROW_H;
        (rel >= 1).then(|| (rel - 1) as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_cell_covered_exactly_once() {
        for row in 0..ROWS {
            for col in 0..COLS {
                let covering = KEYS
                    .iter()
                    .filter(|d| d.row == row && col >= d.col && col < d.col + d.span)
                    .count();
                assert_eq!(covering, 1, "cell ({col},{row})");
            }
        }
    }

    #[test]
    fn key_rects_round_trip_through_hit_test() {
        for (cw, ch) in [(300, 220), (460, 240), (200, 160), (800, 560)] {
            let l = CalcLayout::compute(0, 0, cw, ch);
            for i in 0..KEYS.len() {
                let r = l.key_rect(i);
                assert_eq!(l.key_at(r.x, r.y), Some(i), "{cw}x{ch} key {i} top-left");
                let (bx, by) = (r.x + r.w as i32 - 1, r.y + r.h as i32 - 1);
                assert_eq!(l.key_at(bx, by), Some(i), "{cw}x{ch} key {i} bottom-right");
            }
        }
    }

    #[test]
    fn offset_layout_hit_tests_in_its_own_frame() {
        let l = CalcLayout::compute(30, 40, 300, 220);
        let seven = index_of(CalcKey::Digit('7')).expect("7 key");
        let r = l.key_rect(seven);
        assert_eq!(l.key_at(r.x + 2, r.y + 2), Some(seven));
        assert_eq!(l.key_at(l.display.x + 1, l.display.y + 1), None);
    }

    #[test]
    fn gaps_and_outside_hit_nothing() {
        let l = CalcLayout::compute(0, 0, 300, 220);
        let r = l.key_rect(0);
        // Just right of the first cap is the gap before the second.
        assert_eq!(l.key_at(r.x + r.w as i32, r.y + 1), None);
        assert_eq!(l.key_at(-1, -1), None);
        assert_eq!(l.key_at(l.grid.x, l.grid.y + l.grid.h as i32), None);
    }

    #[test]
    fn wide_layout_has_history_pane_beside_grid() {
        let l = CalcLayout::compute(0, 0, 460, 240);
        let h = l.history.expect("history pane");
        assert!(h.x >= l.grid.x + l.grid.w as i32);
        assert!(h.x + h.w as i32 <= 460);
        assert!(l.history_rows() > 0);
        assert!(CalcLayout::compute(0, 0, 260, 240).history.is_none());
    }

    #[test]
    fn cursor_steps_wrap_and_cross_spans() {
        let seven = index_of(CalcKey::Digit('7')).expect("7");
        let eight = index_of(CalcKey::Digit('8')).expect("8");
        let four = index_of(CalcKey::Digit('4')).expect("4");
        assert_eq!(step(seven, Dir::Right), eight);
        assert_eq!(step(seven, Dir::Down), four);
        // Wraps from the top row to the bottom row.
        let mc = index_of(CalcKey::MemClear).expect("MC");
        let negate = index_of(CalcKey::Negate).expect("+/-");
        assert_eq!(step(mc, Dir::Up), negate);
        // The double-width `=` is reached from both columns above it and
        // moving right off it wraps to the first column.
        let eq = index_of(CalcKey::Equals).expect("=");
        let plus = index_of(CalcKey::Op('+')).expect("+");
        let ans = index_of(CalcKey::Ans).expect("Ans");
        assert_eq!(step(plus, Dir::Down), eq);
        assert_eq!(step(ans, Dir::Down), eq);
        assert_eq!(step(eq, Dir::Right), negate);
    }

    #[test]
    fn tiny_content_does_not_panic() {
        let l = CalcLayout::compute(0, 0, 4, 4);
        for i in 0..KEYS.len() {
            let _ = l.key_rect(i);
        }
        let _ = l.key_at(1, 1);
    }
}
