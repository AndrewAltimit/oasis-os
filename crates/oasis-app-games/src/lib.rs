//! Games collection app: Snake, Memory Match, and Sliding Puzzle.
//!
//! Provides three classic games in a single app, selectable from a menu.
//! All games use text-based rendering through content lines and a
//! deterministic LCG PRNG (no external dependencies).

use std::any::Any;

use oasis_app_core::render::{
    draw_content_windowed, hide_app_sdi, render_app_chrome, render_content_sdi,
};
use oasis_app_core::{App, AppAction, ContentState};
use oasis_sdi::SdiRegistry;
use oasis_skin::ActiveTheme;
use oasis_types::backend::SdiBackend;
use oasis_types::input::Button;
use oasis_vfs::Vfs;

// ---------------------------------------------------------------
// PRNG -- deterministic LCG (no external crate)
// ---------------------------------------------------------------

/// Advance a 64-bit LCG state and return the new value.
fn next_random(state: &mut u64) -> u64 {
    *state = state
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    *state
}

/// Return a random value in `[0, bound)`.
fn random_range(state: &mut u64, bound: u64) -> u64 {
    if bound == 0 {
        return 0;
    }
    next_random(state) % bound
}

// ---------------------------------------------------------------
// Direction (shared between Snake and Sliding Puzzle)
// ---------------------------------------------------------------

/// Cardinal direction used by Snake movement and Sliding Puzzle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

impl Direction {
    /// Returns `true` if `self` is the opposite of `other`.
    fn is_opposite(self, other: Self) -> bool {
        matches!(
            (self, other),
            (Self::Up, Self::Down)
                | (Self::Down, Self::Up)
                | (Self::Left, Self::Right)
                | (Self::Right, Self::Left)
        )
    }
}

// ---------------------------------------------------------------
// GameState (shared)
// ---------------------------------------------------------------

/// State of any individual game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameState {
    Playing,
    Paused,
    GameOver,
    Won,
}

// ---------------------------------------------------------------
// Snake
// ---------------------------------------------------------------

/// Classic snake game on a text grid.
#[derive(Debug, Clone)]
pub struct SnakeGame {
    width: u32,
    height: u32,
    snake: Vec<(i32, i32)>,
    direction: Direction,
    next_direction: Direction,
    food: (i32, i32),
    score: u32,
    high_score: u32,
    state: GameState,
    tick_counter: u32,
    speed: u32,
    rng_state: u64,
}

impl SnakeGame {
    /// Create a new snake game with the given grid dimensions and seed.
    pub fn new(width: u32, height: u32, seed: u64) -> Self {
        let cx = width as i32 / 2;
        let cy = height as i32 / 2;
        let mut game = Self {
            width,
            height,
            snake: vec![(cx, cy), (cx - 1, cy), (cx - 2, cy)],
            direction: Direction::Right,
            next_direction: Direction::Right,
            food: (0, 0),
            score: 0,
            high_score: 0,
            state: GameState::Playing,
            tick_counter: 0,
            speed: 6,
            rng_state: seed,
        };
        game.spawn_food();
        game
    }

    /// Advance one game tick (movement + collision).
    pub fn tick(&mut self) {
        if self.state != GameState::Playing {
            return;
        }

        self.tick_counter += 1;
        if self.tick_counter < self.speed {
            return;
        }
        self.tick_counter = 0;

        self.direction = self.next_direction;

        let (hx, hy) = self.snake[0];
        let new_head = match self.direction {
            Direction::Up => (hx, hy - 1),
            Direction::Down => (hx, hy + 1),
            Direction::Left => (hx - 1, hy),
            Direction::Right => (hx + 1, hy),
        };

        if self.is_collision(new_head.0, new_head.1) {
            self.state = GameState::GameOver;
            if self.score > self.high_score {
                self.high_score = self.score;
            }
            return;
        }

        self.snake.insert(0, new_head);

        if new_head == self.food {
            self.score += 1;
            // Check win: snake fills entire grid.
            let total = (self.width * self.height) as usize;
            if self.snake.len() >= total {
                self.state = GameState::Won;
                if self.score > self.high_score {
                    self.high_score = self.score;
                }
                return;
            }
            self.spawn_food();
        } else {
            self.snake.pop();
        }
    }

    /// Buffer a direction change (prevents 180-degree reversal).
    pub fn set_direction(&mut self, dir: Direction) {
        if !dir.is_opposite(self.direction) {
            self.next_direction = dir;
        }
    }

    /// Place food at a random empty cell.
    pub fn spawn_food(&mut self) {
        let total = self.width as u64 * self.height as u64;
        let occupied = self.snake.len() as u64;
        if occupied >= total {
            return;
        }
        loop {
            let x = random_range(&mut self.rng_state, self.width as u64) as i32;
            let y = random_range(&mut self.rng_state, self.height as u64) as i32;
            if !self.snake.contains(&(x, y)) {
                self.food = (x, y);
                return;
            }
        }
    }

    /// Check if position collides with walls or the snake body.
    pub fn is_collision(&self, x: i32, y: i32) -> bool {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return true;
        }
        self.snake.contains(&(x, y))
    }

    /// Reset the game (keeps high score).
    pub fn reset(&mut self) {
        let cx = self.width as i32 / 2;
        let cy = self.height as i32 / 2;
        self.snake = vec![(cx, cy), (cx - 1, cy), (cx - 2, cy)];
        self.direction = Direction::Right;
        self.next_direction = Direction::Right;
        self.score = 0;
        self.state = GameState::Playing;
        self.tick_counter = 0;
        self.speed = 6;
        self.spawn_food();
    }

    /// Render the grid as text lines.
    pub fn grid_text(&self) -> Vec<String> {
        let mut lines = Vec::new();
        lines.push(format!(
            "Snake - Score: {}  High: {}",
            self.score, self.high_score
        ));
        let sep: String = std::iter::repeat_n('\u{2500}', 20).collect();
        lines.push(sep.clone());

        for y in 0..self.height as i32 {
            let mut row = String::with_capacity(self.width as usize);
            for x in 0..self.width as i32 {
                if self.snake.contains(&(x, y)) {
                    row.push('#');
                } else if self.food == (x, y) {
                    row.push('*');
                } else {
                    row.push('.');
                }
            }
            lines.push(row);
        }

        lines.push(sep);

        match self.state {
            GameState::Playing => {
                lines.push("D-pad: Move  Start: Pause".into());
            },
            GameState::Paused => {
                lines.push("PAUSED - Start to resume".into());
            },
            GameState::GameOver => {
                lines.push("GAME OVER - Select: Restart".into());
            },
            GameState::Won => {
                lines.push("YOU WIN! - Select: Restart".into());
            },
        }

        lines
    }
}

// ---------------------------------------------------------------
// Memory Match
// ---------------------------------------------------------------

/// A single card in the memory match game.
#[derive(Debug, Clone)]
pub struct Card {
    /// Symbol on the face of the card (A-H).
    pub symbol: char,
    /// Whether the card is currently face-up.
    pub face_up: bool,
}

/// Memory match (concentration) card game.
#[derive(Debug, Clone)]
pub struct MemoryGame {
    cols: u32,
    rows: u32,
    cards: Vec<Card>,
    cursor: (u32, u32),
    first_pick: Option<usize>,
    second_pick: Option<usize>,
    matched: Vec<bool>,
    moves: u32,
    pairs_found: u32,
    total_pairs: u32,
    state: GameState,
    reveal_timer: u32,
    /// Retained for re-seeding on reset.
    _rng_state: u64,
}

impl MemoryGame {
    /// Create a new memory game with the given grid size and seed.
    ///
    /// `cols * rows` must be even (for pairs).
    pub fn new(cols: u32, rows: u32, seed: u64) -> Self {
        let total = (cols * rows) as usize;
        let pairs = total / 2;
        let mut rng_state = seed;

        let symbols: Vec<char> = "ABCDEFGHIJKLMNOPQRSTUVWXYZ".chars().collect();

        // Build pairs.
        let mut card_symbols = Vec::with_capacity(total);
        for i in 0..pairs {
            let sym = symbols[i % symbols.len()];
            card_symbols.push(sym);
            card_symbols.push(sym);
        }

        // Fisher-Yates shuffle.
        for i in (1..card_symbols.len()).rev() {
            let j = random_range(&mut rng_state, (i + 1) as u64) as usize;
            card_symbols.swap(i, j);
        }

        let cards: Vec<Card> = card_symbols
            .into_iter()
            .map(|symbol| Card {
                symbol,
                face_up: false,
            })
            .collect();

        let matched = vec![false; total];

        Self {
            cols,
            rows,
            cards,
            cursor: (0, 0),
            first_pick: None,
            second_pick: None,
            matched,
            moves: 0,
            pairs_found: 0,
            total_pairs: pairs as u32,
            state: GameState::Playing,
            reveal_timer: 0,
            _rng_state: rng_state,
        }
    }

    /// Flip the card at the current cursor position.
    pub fn flip_at_cursor(&mut self) {
        if self.state != GameState::Playing || self.reveal_timer > 0 {
            return;
        }

        let idx = (self.cursor.1 * self.cols + self.cursor.0) as usize;

        // Cannot flip matched or already face-up cards.
        if idx >= self.cards.len() || self.matched[idx] || self.cards[idx].face_up {
            return;
        }

        // Cannot flip if we already have first pick selected and
        // it is the same card.
        if self.first_pick == Some(idx) {
            return;
        }

        self.cards[idx].face_up = true;

        if self.first_pick.is_none() {
            self.first_pick = Some(idx);
        } else {
            self.second_pick = Some(idx);
            self.moves += 1;
            self.check_match();
        }
    }

    /// Check if two flipped cards match.
    pub fn check_match(&mut self) {
        let (Some(a), Some(b)) = (self.first_pick, self.second_pick) else {
            return;
        };

        if self.cards[a].symbol == self.cards[b].symbol {
            self.matched[a] = true;
            self.matched[b] = true;
            self.pairs_found += 1;
            self.first_pick = None;
            self.second_pick = None;

            if self.pairs_found >= self.total_pairs {
                self.state = GameState::Won;
            }
        } else {
            // Start reveal timer so the player can see the mismatch.
            self.reveal_timer = 30;
        }
    }

    /// Handle reveal timer countdown (hides mismatched cards).
    pub fn tick(&mut self) {
        if self.reveal_timer > 0 {
            self.reveal_timer -= 1;
            if self.reveal_timer == 0 {
                // Hide the two mismatched cards.
                if let Some(a) = self.first_pick
                    && a < self.cards.len()
                {
                    self.cards[a].face_up = false;
                }
                if let Some(b) = self.second_pick
                    && b < self.cards.len()
                {
                    self.cards[b].face_up = false;
                }
                self.first_pick = None;
                self.second_pick = None;
            }
        }
    }

    /// Reset and reshuffle the game.
    pub fn reset(&mut self, seed: u64) {
        *self = Self::new(self.cols, self.rows, seed);
    }

    /// Render the grid as text lines.
    pub fn grid_text(&self) -> Vec<String> {
        let mut lines = Vec::new();
        lines.push(format!(
            "Memory - Moves: {}  Pairs: {}/{}",
            self.moves, self.pairs_found, self.total_pairs
        ));
        let sep: String = std::iter::repeat_n('\u{2500}', 20).collect();
        lines.push(sep.clone());

        for row in 0..self.rows {
            let mut text = String::new();
            for col in 0..self.cols {
                let idx = (row * self.cols + col) as usize;
                let is_cursor = self.cursor == (col, row) && self.state == GameState::Playing;
                let ch = if self.matched[idx] {
                    ' '
                } else if self.cards[idx].face_up {
                    self.cards[idx].symbol
                } else {
                    '\u{25A0}' // filled square for face-down
                };

                if is_cursor {
                    text.push('[');
                    text.push(ch);
                    text.push(']');
                } else {
                    text.push(' ');
                    text.push(ch);
                    text.push(' ');
                }
            }
            lines.push(text);
        }

        lines.push(sep);

        match self.state {
            GameState::Playing => {
                lines.push("D-pad: Move  Confirm: Flip".into());
            },
            GameState::Won => {
                lines.push(format!(
                    "COMPLETE in {} moves! Select: New game",
                    self.moves
                ));
            },
            _ => {},
        }

        lines
    }
}

// ---------------------------------------------------------------
// Sliding Puzzle (15-puzzle)
// ---------------------------------------------------------------

/// Classic 15-puzzle (sliding tiles).
#[derive(Debug, Clone)]
pub struct SlidingPuzzle {
    size: u32,
    tiles: Vec<u8>,
    empty_pos: usize,
    moves: u32,
    state: GameState,
    rng_state: u64,
}

impl SlidingPuzzle {
    /// Create a new sliding puzzle with the given size and seed.
    ///
    /// `size` is the side length (4 for a 15-puzzle).
    pub fn new(size: u32, seed: u64) -> Self {
        let total = (size * size) as usize;
        // Arrange in solved order: [1, 2, ..., N-1, 0].
        let tiles: Vec<u8> = (1..total as u8).chain(std::iter::once(0)).collect();

        let mut puzzle = Self {
            size,
            tiles,
            empty_pos: total - 1,
            moves: 0,
            state: GameState::Playing,
            rng_state: seed,
        };

        // Shuffle by performing random valid moves (guarantees solvability).
        // Use increasing move counts to ensure the puzzle eventually
        // reaches a non-solved state even with unlucky PRNG sequences.
        let mut moves = 200;
        for _ in 0..20 {
            puzzle.shuffle(moves);
            if !puzzle.is_solved() {
                break;
            }
            moves += 100;
            // Perturb the PRNG state so we don't repeat the same sequence.
            next_random(&mut puzzle.rng_state);
        }
        puzzle.moves = 0;
        puzzle
    }

    /// Slide a tile into the empty space in the given direction.
    ///
    /// The direction indicates which direction to move the tile (not
    /// the empty space). Returns `true` if a tile was moved.
    pub fn slide(&mut self, dir: Direction) -> bool {
        if self.state != GameState::Playing {
            return false;
        }

        let s = self.size as usize;
        let row = self.empty_pos / s;
        let col = self.empty_pos % s;

        // Find the tile position that would slide into the empty space.
        let (tile_row, tile_col) = match dir {
            // Player presses Up: tile below empty moves up.
            Direction::Up => (row + 1, col),
            // Player presses Down: tile above empty moves down.
            Direction::Down => {
                if row == 0 {
                    return false;
                }
                (row - 1, col)
            },
            // Player presses Left: tile to the right moves left.
            Direction::Left => (row, col + 1),
            // Player presses Right: tile to the left moves right.
            Direction::Right => {
                if col == 0 {
                    return false;
                }
                (row, col - 1)
            },
        };

        if tile_row >= s || tile_col >= s {
            return false;
        }

        let tile_pos = tile_row * s + tile_col;
        self.tiles.swap(self.empty_pos, tile_pos);
        self.empty_pos = tile_pos;
        self.moves += 1;

        if self.is_solved() {
            self.state = GameState::Won;
        }

        true
    }

    /// Check if the puzzle is in the solved state.
    pub fn is_solved(&self) -> bool {
        let total = self.tiles.len();
        for i in 0..total - 1 {
            if self.tiles[i] != (i + 1) as u8 {
                return false;
            }
        }
        self.tiles[total - 1] == 0
    }

    /// Check if a tile arrangement is solvable (for the standard puzzle).
    ///
    /// Counts inversions. For even-sized grids, also considers the
    /// row of the blank tile from the bottom.
    pub fn is_solvable(tiles: &[u8], size: u32) -> bool {
        let s = size as usize;
        let total = s * s;
        if tiles.len() != total {
            return false;
        }

        let mut inversions = 0u32;
        for i in 0..total {
            if tiles[i] == 0 {
                continue;
            }
            for j in (i + 1)..total {
                if tiles[j] == 0 {
                    continue;
                }
                if tiles[i] > tiles[j] {
                    inversions += 1;
                }
            }
        }

        if size % 2 == 1 {
            // Odd grid: solvable if inversions are even.
            inversions.is_multiple_of(2)
        } else {
            // Even grid: find blank row from bottom.
            let blank_pos = tiles.iter().position(|&t| t == 0).unwrap_or(0);
            let blank_row_from_bottom = s - 1 - blank_pos / s;
            // Solvable if (inversions + blank_row_from_bottom) is even.
            (inversions as usize + blank_row_from_bottom).is_multiple_of(2)
        }
    }

    /// Shuffle the puzzle by making random valid moves.
    pub fn shuffle(&mut self, num_moves: u32) {
        self.state = GameState::Playing;
        let all_dirs = [
            Direction::Up,
            Direction::Down,
            Direction::Left,
            Direction::Right,
        ];
        let mut prev_dir: Option<Direction> = None;
        let mut successful = 0u32;
        let mut attempts = 0u32;
        let max_attempts = num_moves * 10;

        while successful < num_moves && attempts < max_attempts {
            attempts += 1;
            let idx = random_range(&mut self.rng_state, 4) as usize;
            let dir = all_dirs[idx];

            // Skip the reverse of the last move to avoid undoing it.
            let reverse = match prev_dir {
                Some(Direction::Up) => Direction::Down,
                Some(Direction::Down) => Direction::Up,
                Some(Direction::Left) => Direction::Right,
                Some(Direction::Right) => Direction::Left,
                None => dir, // no previous, allow any
            };
            if dir == reverse && prev_dir.is_some() {
                continue;
            }

            if self.slide_raw(dir) {
                prev_dir = Some(dir);
                successful += 1;
            }
        }
        self.moves = 0;
        self.state = GameState::Playing;
    }

    /// Internal slide without win check (used during shuffle).
    fn slide_raw(&mut self, dir: Direction) -> bool {
        let s = self.size as usize;
        let row = self.empty_pos / s;
        let col = self.empty_pos % s;

        let (tile_row, tile_col) = match dir {
            Direction::Up => (row + 1, col),
            Direction::Down => {
                if row == 0 {
                    return false;
                }
                (row - 1, col)
            },
            Direction::Left => (row, col + 1),
            Direction::Right => {
                if col == 0 {
                    return false;
                }
                (row, col - 1)
            },
        };

        if tile_row >= s || tile_col >= s {
            return false;
        }

        let tile_pos = tile_row * s + tile_col;
        self.tiles.swap(self.empty_pos, tile_pos);
        self.empty_pos = tile_pos;
        true
    }

    /// Reset the puzzle with a new seed.
    pub fn reset(&mut self, seed: u64) {
        *self = Self::new(self.size, seed);
    }

    /// Render the grid as text lines.
    pub fn grid_text(&self) -> Vec<String> {
        let mut lines = Vec::new();
        lines.push(format!("Sliding Puzzle - Moves: {}", self.moves));
        let sep: String = std::iter::repeat_n('\u{2500}', 20).collect();
        lines.push(sep.clone());

        let s = self.size as usize;
        for row in 0..s {
            let mut text = String::new();
            for col in 0..s {
                let idx = row * s + col;
                let tile = self.tiles[idx];
                if tile == 0 {
                    text.push_str("  . ");
                } else {
                    text.push_str(&format!("{tile:>3} "));
                }
            }
            lines.push(text);
        }

        lines.push(sep);

        match self.state {
            GameState::Playing => {
                lines.push("D-pad: Slide tiles".into());
            },
            GameState::Won => {
                lines.push(format!("SOLVED in {} moves! Select: New game", self.moves));
            },
            _ => {},
        }

        lines
    }
}

// ---------------------------------------------------------------
// GamesApp (wrapper implementing App trait)
// ---------------------------------------------------------------

/// Which game (or menu) is currently active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveGame {
    /// Game selection menu.
    Menu,
    /// Snake game.
    Snake,
    /// Memory match game.
    Memory,
    /// Sliding puzzle game.
    Puzzle,
}

/// Games collection app containing Snake, Memory Match, and Sliding Puzzle.
#[derive(Debug)]
pub struct GamesApp {
    content: ContentState,
    active_game: ActiveGame,
    menu_cursor: usize,
    snake: SnakeGame,
    memory: MemoryGame,
    puzzle: SlidingPuzzle,
    frame_counter: u64,
}

const MENU_ITEMS: [&str; 3] = ["Snake", "Memory Match", "Sliding Puzzle"];

impl GamesApp {
    /// Create a new games app with the given VFS path.
    pub fn new(path: &str) -> Self {
        let seed = 42u64;
        let mut app = Self {
            content: ContentState::new("Games", path),
            active_game: ActiveGame::Menu,
            menu_cursor: 0,
            snake: SnakeGame::new(20, 15, seed),
            memory: MemoryGame::new(4, 4, seed.wrapping_add(1)),
            puzzle: SlidingPuzzle::new(4, seed.wrapping_add(2)),
            frame_counter: 0,
        };
        app.update_lines();
        app
    }

    /// Get the currently active game.
    pub fn active_game(&self) -> ActiveGame {
        self.active_game
    }

    /// Get a reference to the snake game.
    pub fn snake(&self) -> &SnakeGame {
        &self.snake
    }

    /// Get a reference to the memory game.
    pub fn memory(&self) -> &MemoryGame {
        &self.memory
    }

    /// Get a reference to the sliding puzzle.
    pub fn puzzle(&self) -> &SlidingPuzzle {
        &self.puzzle
    }

    /// Rebuild content lines from the active game state.
    fn update_lines(&mut self) {
        self.content.lines = match self.active_game {
            ActiveGame::Menu => self.menu_lines(),
            ActiveGame::Snake => self.snake.grid_text(),
            ActiveGame::Memory => self.memory.grid_text(),
            ActiveGame::Puzzle => self.puzzle.grid_text(),
        };
    }

    /// Generate the menu display lines.
    fn menu_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        lines.push("Games".into());
        let sep: String = std::iter::repeat_n('\u{2500}', 21).collect();
        lines.push(sep.clone());

        for (i, item) in MENU_ITEMS.iter().enumerate() {
            if i == self.menu_cursor {
                lines.push(format!("  > {item}"));
            } else {
                lines.push(format!("    {item}"));
            }
        }

        lines.push(sep);
        lines.push("  Select with Confirm".into());
        lines
    }

    /// Handle input when on the menu screen.
    fn handle_menu_input(&mut self, button: &Button) -> AppAction {
        match button {
            Button::Up => {
                if self.menu_cursor > 0 {
                    self.menu_cursor -= 1;
                }
                AppAction::None
            },
            Button::Down => {
                if self.menu_cursor + 1 < MENU_ITEMS.len() {
                    self.menu_cursor += 1;
                }
                AppAction::None
            },
            Button::Confirm => {
                self.active_game = match self.menu_cursor {
                    0 => ActiveGame::Snake,
                    1 => ActiveGame::Memory,
                    2 => ActiveGame::Puzzle,
                    _ => ActiveGame::Menu,
                };
                AppAction::None
            },
            Button::Cancel => AppAction::Exit,
            _ => AppAction::None,
        }
    }

    /// Handle input when playing Snake.
    fn handle_snake_input(&mut self, button: &Button) -> AppAction {
        match button {
            Button::Up => {
                self.snake.set_direction(Direction::Up);
                AppAction::None
            },
            Button::Down => {
                self.snake.set_direction(Direction::Down);
                AppAction::None
            },
            Button::Left => {
                self.snake.set_direction(Direction::Left);
                AppAction::None
            },
            Button::Right => {
                self.snake.set_direction(Direction::Right);
                AppAction::None
            },
            Button::Start => {
                match self.snake.state {
                    GameState::Playing => {
                        self.snake.state = GameState::Paused;
                    },
                    GameState::Paused => {
                        self.snake.state = GameState::Playing;
                    },
                    _ => {},
                }
                AppAction::None
            },
            Button::Select => {
                self.snake.reset();
                AppAction::None
            },
            Button::Cancel => {
                self.active_game = ActiveGame::Menu;
                AppAction::None
            },
            _ => AppAction::None,
        }
    }

    /// Handle input when playing Memory Match.
    fn handle_memory_input(&mut self, button: &Button) -> AppAction {
        match button {
            Button::Up => {
                if self.memory.cursor.1 > 0 {
                    self.memory.cursor.1 -= 1;
                }
                AppAction::None
            },
            Button::Down => {
                if self.memory.cursor.1 + 1 < self.memory.rows {
                    self.memory.cursor.1 += 1;
                }
                AppAction::None
            },
            Button::Left => {
                if self.memory.cursor.0 > 0 {
                    self.memory.cursor.0 -= 1;
                }
                AppAction::None
            },
            Button::Right => {
                if self.memory.cursor.0 + 1 < self.memory.cols {
                    self.memory.cursor.0 += 1;
                }
                AppAction::None
            },
            Button::Confirm => {
                self.memory.flip_at_cursor();
                AppAction::None
            },
            Button::Select => {
                self.frame_counter = self.frame_counter.wrapping_add(1);
                self.memory.reset(self.frame_counter);
                AppAction::None
            },
            Button::Cancel => {
                self.active_game = ActiveGame::Menu;
                AppAction::None
            },
            _ => AppAction::None,
        }
    }

    /// Handle input when playing Sliding Puzzle.
    fn handle_puzzle_input(&mut self, button: &Button) -> AppAction {
        match button {
            Button::Up => {
                self.puzzle.slide(Direction::Up);
                AppAction::None
            },
            Button::Down => {
                self.puzzle.slide(Direction::Down);
                AppAction::None
            },
            Button::Left => {
                self.puzzle.slide(Direction::Left);
                AppAction::None
            },
            Button::Right => {
                self.puzzle.slide(Direction::Right);
                AppAction::None
            },
            Button::Select => {
                self.frame_counter = self.frame_counter.wrapping_add(1);
                self.puzzle.reset(self.frame_counter);
                AppAction::None
            },
            Button::Cancel => {
                self.active_game = ActiveGame::Menu;
                AppAction::None
            },
            _ => AppAction::None,
        }
    }
}

impl App for GamesApp {
    fn title(&self) -> &str {
        &self.content.title
    }

    fn path(&self) -> &str {
        &self.content.app_path
    }

    fn handle_input(&mut self, button: &Button, _vfs: &dyn Vfs) -> AppAction {
        let action = match self.active_game {
            ActiveGame::Menu => self.handle_menu_input(button),
            ActiveGame::Snake => self.handle_snake_input(button),
            ActiveGame::Memory => self.handle_memory_input(button),
            ActiveGame::Puzzle => self.handle_puzzle_input(button),
        };
        self.update_lines();
        action
    }

    fn refresh(&mut self, _vfs: &dyn Vfs) {
        self.frame_counter = self.frame_counter.wrapping_add(1);

        match self.active_game {
            ActiveGame::Snake => {
                self.snake.tick();
                self.update_lines();
            },
            ActiveGame::Memory => {
                self.memory.tick();
                self.update_lines();
            },
            _ => {},
        }
    }

    fn update_sdi(&mut self, sdi: &mut SdiRegistry, at: &ActiveTheme) {
        self.content.update_layout(at);
        self.content.animate_selection(0.3);
        render_app_chrome(sdi, at);
        render_content_sdi(&self.content, sdi, at);
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
        draw_content_windowed(&self.content, cx, cy, cw, ch, backend, at)
    }

    fn hide_sdi(&self, sdi: &mut SdiRegistry) {
        hide_app_sdi(sdi);
    }

    fn lines(&self) -> &[String] {
        &self.content.lines
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
    use super::*;
    use oasis_vfs::MemoryVfs;

    fn make_vfs() -> MemoryVfs {
        MemoryVfs::new()
    }

    // -- PRNG tests --

    #[test]
    fn prng_deterministic_same_seed() {
        let mut s1 = 123u64;
        let mut s2 = 123u64;
        let a = next_random(&mut s1);
        let b = next_random(&mut s2);
        assert_eq!(a, b);
    }

    #[test]
    fn prng_different_seeds_differ() {
        let mut s1 = 1u64;
        let mut s2 = 2u64;
        let a = next_random(&mut s1);
        let b = next_random(&mut s2);
        assert_ne!(a, b);
    }

    #[test]
    fn prng_sequence_not_constant() {
        let mut s = 42u64;
        let a = next_random(&mut s);
        let b = next_random(&mut s);
        assert_ne!(a, b);
    }

    #[test]
    fn random_range_within_bound() {
        let mut s = 99u64;
        for _ in 0..100 {
            let v = random_range(&mut s, 10);
            assert!(v < 10);
        }
    }

    #[test]
    fn random_range_zero_bound() {
        let mut s = 42u64;
        assert_eq!(random_range(&mut s, 0), 0);
    }

    // -- Snake tests --

    #[test]
    fn snake_creation() {
        let game = SnakeGame::new(20, 15, 42);
        assert_eq!(game.width, 20);
        assert_eq!(game.height, 15);
        assert_eq!(game.snake.len(), 3);
        assert_eq!(game.score, 0);
        assert_eq!(game.state, GameState::Playing);
        assert_eq!(game.direction, Direction::Right);
    }

    #[test]
    fn snake_initial_position() {
        let game = SnakeGame::new(20, 15, 42);
        // Head at center.
        assert_eq!(game.snake[0], (10, 7));
        // Body extends left.
        assert_eq!(game.snake[1], (9, 7));
        assert_eq!(game.snake[2], (8, 7));
    }

    #[test]
    fn snake_movement_right() {
        let mut game = SnakeGame::new(20, 15, 42);
        game.speed = 1; // Move every tick.
        let old_head = game.snake[0];
        game.tick();
        assert_eq!(game.snake[0], (old_head.0 + 1, old_head.1));
    }

    #[test]
    fn snake_movement_down() {
        let mut game = SnakeGame::new(20, 15, 42);
        game.speed = 1;
        game.set_direction(Direction::Down);
        let old_head = game.snake[0];
        game.tick();
        assert_eq!(game.snake[0], (old_head.0, old_head.1 + 1));
    }

    #[test]
    fn snake_movement_left() {
        let mut game = SnakeGame::new(20, 15, 42);
        game.speed = 1;
        // First move down to avoid 180-degree reversal.
        game.set_direction(Direction::Down);
        game.tick();
        game.set_direction(Direction::Left);
        game.tick();
        // After two ticks: moved down (10,8), then left (9,8).
        assert_eq!(game.snake[0], (9, 8));
    }

    #[test]
    fn snake_movement_up() {
        let mut game = SnakeGame::new(20, 15, 42);
        game.speed = 1;
        // Move down first, then left, then up (to avoid 180-degree).
        game.set_direction(Direction::Down);
        game.tick(); // (10, 8)
        game.set_direction(Direction::Left);
        game.tick(); // (9, 8)
        game.set_direction(Direction::Up);
        game.tick(); // (9, 7)
        assert_eq!(game.snake[0], (9, 7));
    }

    #[test]
    fn snake_no_180_reverse() {
        let mut game = SnakeGame::new(20, 15, 42);
        game.speed = 1;
        // Snake is moving right, trying to go left should be ignored.
        game.set_direction(Direction::Left);
        assert_eq!(game.next_direction, Direction::Right);
    }

    #[test]
    fn snake_no_180_reverse_vertical() {
        let mut game = SnakeGame::new(20, 15, 42);
        game.speed = 1;
        game.set_direction(Direction::Down);
        game.tick();
        game.direction = Direction::Down;
        game.set_direction(Direction::Up);
        // Should still be Down since Up is opposite.
        assert_eq!(game.next_direction, Direction::Down);
    }

    #[test]
    fn snake_direction_buffer() {
        let mut game = SnakeGame::new(20, 15, 42);
        game.set_direction(Direction::Down);
        assert_eq!(game.next_direction, Direction::Down);
        // Direction only applied on tick.
        assert_eq!(game.direction, Direction::Right);
    }

    #[test]
    fn snake_food_spawn_not_on_snake() {
        let game = SnakeGame::new(20, 15, 42);
        assert!(!game.snake.contains(&game.food));
    }

    #[test]
    fn snake_collision_wall_left() {
        let game = SnakeGame::new(20, 15, 42);
        assert!(game.is_collision(-1, 5));
    }

    #[test]
    fn snake_collision_wall_right() {
        let game = SnakeGame::new(20, 15, 42);
        assert!(game.is_collision(20, 5));
    }

    #[test]
    fn snake_collision_wall_top() {
        let game = SnakeGame::new(20, 15, 42);
        assert!(game.is_collision(5, -1));
    }

    #[test]
    fn snake_collision_wall_bottom() {
        let game = SnakeGame::new(20, 15, 42);
        assert!(game.is_collision(5, 15));
    }

    #[test]
    fn snake_collision_with_body() {
        let game = SnakeGame::new(20, 15, 42);
        let body_pos = game.snake[1];
        assert!(game.is_collision(body_pos.0, body_pos.1));
    }

    #[test]
    fn snake_no_collision_empty() {
        let game = SnakeGame::new(20, 15, 42);
        assert!(!game.is_collision(0, 0));
    }

    #[test]
    fn snake_growth_on_eat() {
        let mut game = SnakeGame::new(20, 15, 42);
        game.speed = 1;
        // Place food directly ahead.
        let head = game.snake[0];
        game.food = (head.0 + 1, head.1);
        let old_len = game.snake.len();
        game.tick();
        assert_eq!(game.snake.len(), old_len + 1);
        assert_eq!(game.score, 1);
    }

    #[test]
    fn snake_game_over_wall() {
        let mut game = SnakeGame::new(20, 15, 42);
        game.speed = 1;
        // Move right until hitting the wall.
        for _ in 0..20 {
            game.tick();
        }
        assert_eq!(game.state, GameState::GameOver);
    }

    #[test]
    fn snake_game_over_preserves_high_score() {
        let mut game = SnakeGame::new(20, 15, 42);
        game.speed = 1;
        game.score = 5;
        game.high_score = 3;
        // Force game over by hitting wall.
        game.snake[0] = (19, 0);
        game.direction = Direction::Right;
        game.next_direction = Direction::Right;
        game.tick();
        assert_eq!(game.state, GameState::GameOver);
        assert_eq!(game.high_score, 5);
    }

    #[test]
    fn snake_reset() {
        let mut game = SnakeGame::new(20, 15, 42);
        game.score = 10;
        game.state = GameState::GameOver;
        game.reset();
        assert_eq!(game.score, 0);
        assert_eq!(game.state, GameState::Playing);
        assert_eq!(game.snake.len(), 3);
        assert_eq!(game.direction, Direction::Right);
    }

    #[test]
    fn snake_reset_keeps_high_score() {
        let mut game = SnakeGame::new(20, 15, 42);
        game.high_score = 10;
        game.reset();
        assert_eq!(game.high_score, 10);
    }

    #[test]
    fn snake_grid_text_has_lines() {
        let game = SnakeGame::new(20, 15, 42);
        let lines = game.grid_text();
        // Header + separator + 15 rows + separator + controls = 19.
        assert_eq!(lines.len(), 19);
        assert!(lines[0].contains("Snake"));
        assert!(lines[0].contains("Score: 0"));
    }

    #[test]
    fn snake_grid_text_shows_snake() {
        let game = SnakeGame::new(20, 15, 42);
        let lines = game.grid_text();
        // Snake body '#' should appear in grid rows.
        let grid_content: String = lines[2..17].join("");
        assert!(grid_content.contains('#'));
    }

    #[test]
    fn snake_grid_text_shows_food() {
        let game = SnakeGame::new(20, 15, 42);
        let lines = game.grid_text();
        let grid_content: String = lines[2..17].join("");
        assert!(grid_content.contains('*'));
    }

    #[test]
    fn snake_tick_speed_control() {
        let mut game = SnakeGame::new(20, 15, 42);
        game.speed = 3;
        let head_before = game.snake[0];
        game.tick(); // counter=1
        assert_eq!(game.snake[0], head_before);
        game.tick(); // counter=2
        assert_eq!(game.snake[0], head_before);
        game.tick(); // counter=3 -> moves
        assert_ne!(game.snake[0], head_before);
    }

    #[test]
    fn snake_paused_no_tick() {
        let mut game = SnakeGame::new(20, 15, 42);
        game.speed = 1;
        game.state = GameState::Paused;
        let head = game.snake[0];
        game.tick();
        assert_eq!(game.snake[0], head);
    }

    #[test]
    fn snake_win_fills_grid() {
        // Use a 3x2 grid (total 6 cells) and manually set up a
        // near-win state with 5-length snake about to eat the last food.
        let mut game = SnakeGame::new(3, 2, 42);
        game.speed = 1;
        game.snake = vec![(1, 0), (0, 0), (0, 1), (1, 1), (2, 1)];
        game.direction = Direction::Right;
        game.next_direction = Direction::Right;
        game.food = (2, 0);
        game.tick();
        // Snake ate food and now fills all 6 cells.
        assert_eq!(game.snake.len(), 6);
        assert_eq!(game.state, GameState::Won);
    }

    // -- Memory tests --

    #[test]
    fn memory_creation() {
        let game = MemoryGame::new(4, 4, 42);
        assert_eq!(game.cols, 4);
        assert_eq!(game.rows, 4);
        assert_eq!(game.cards.len(), 16);
        assert_eq!(game.total_pairs, 8);
        assert_eq!(game.pairs_found, 0);
        assert_eq!(game.moves, 0);
        assert_eq!(game.state, GameState::Playing);
    }

    #[test]
    fn memory_has_correct_pairs() {
        let game = MemoryGame::new(4, 4, 42);
        // Count each symbol: each should appear exactly twice.
        let mut counts = std::collections::HashMap::new();
        for card in &game.cards {
            *counts.entry(card.symbol).or_insert(0) += 1;
        }
        for &count in counts.values() {
            assert_eq!(count, 2);
        }
        assert_eq!(counts.len(), 8);
    }

    #[test]
    fn memory_all_face_down_initially() {
        let game = MemoryGame::new(4, 4, 42);
        assert!(game.cards.iter().all(|c| !c.face_up));
    }

    #[test]
    fn memory_flip_first_card() {
        let mut game = MemoryGame::new(4, 4, 42);
        game.cursor = (0, 0);
        game.flip_at_cursor();
        assert!(game.cards[0].face_up);
        assert!(game.first_pick.is_some());
        assert!(game.second_pick.is_none());
    }

    #[test]
    fn memory_flip_two_cards_increments_moves() {
        let mut game = MemoryGame::new(4, 4, 42);
        game.cursor = (0, 0);
        game.flip_at_cursor();
        game.cursor = (1, 0);
        game.flip_at_cursor();
        assert_eq!(game.moves, 1);
    }

    #[test]
    fn memory_match_detection() {
        let mut game = MemoryGame::new(4, 4, 42);
        // Find two cards with the same symbol.
        let target = game.cards[0].symbol;
        let second = game
            .cards
            .iter()
            .enumerate()
            .find(|(i, c)| *i != 0 && c.symbol == target)
            .map(|(i, _)| i)
            .expect("must have a pair");

        let col0 = 0u32;
        let row0 = 0u32;
        let col1 = (second % game.cols as usize) as u32;
        let row1 = (second / game.cols as usize) as u32;

        game.cursor = (col0, row0);
        game.flip_at_cursor();
        game.cursor = (col1, row1);
        game.flip_at_cursor();

        assert_eq!(game.pairs_found, 1);
        assert!(game.matched[0]);
        assert!(game.matched[second]);
    }

    #[test]
    fn memory_mismatch_reveal_timer() {
        let mut game = MemoryGame::new(4, 4, 42);
        // Find two cards with different symbols.
        let first_sym = game.cards[0].symbol;
        let second = game
            .cards
            .iter()
            .enumerate()
            .find(|(i, c)| *i != 0 && c.symbol != first_sym)
            .map(|(i, _)| i)
            .expect("must have a different card");

        game.cursor = (0, 0);
        game.flip_at_cursor();

        let col = (second % game.cols as usize) as u32;
        let row = (second / game.cols as usize) as u32;
        game.cursor = (col, row);
        game.flip_at_cursor();

        assert!(game.reveal_timer > 0);
        assert_eq!(game.pairs_found, 0);
    }

    #[test]
    fn memory_mismatch_hides_after_timer() {
        let mut game = MemoryGame::new(4, 4, 42);
        let first_sym = game.cards[0].symbol;
        let second = game
            .cards
            .iter()
            .enumerate()
            .find(|(i, c)| *i != 0 && c.symbol != first_sym)
            .map(|(i, _)| i)
            .expect("must have a different card");

        game.cursor = (0, 0);
        game.flip_at_cursor();
        let col = (second % game.cols as usize) as u32;
        let row = (second / game.cols as usize) as u32;
        game.cursor = (col, row);
        game.flip_at_cursor();

        // Tick until timer expires.
        for _ in 0..30 {
            game.tick();
        }

        assert!(!game.cards[0].face_up);
        assert!(!game.cards[second].face_up);
        assert!(game.first_pick.is_none());
        assert!(game.second_pick.is_none());
    }

    #[test]
    fn memory_cannot_flip_matched_card() {
        let mut game = MemoryGame::new(4, 4, 42);
        game.matched[0] = true;
        game.cursor = (0, 0);
        game.flip_at_cursor();
        assert!(!game.cards[0].face_up);
    }

    #[test]
    fn memory_cannot_flip_face_up_card() {
        let mut game = MemoryGame::new(4, 4, 42);
        game.cursor = (0, 0);
        game.flip_at_cursor();
        assert!(game.first_pick.is_some());
        // Flip same card again -- should be ignored.
        game.flip_at_cursor();
        assert!(game.second_pick.is_none());
    }

    #[test]
    fn memory_win_all_matched() {
        let mut game = MemoryGame::new(2, 2, 42);
        // 2x2 = 2 pairs. Match them all.
        let sym0 = game.cards[0].symbol;
        let pair0 = game
            .cards
            .iter()
            .enumerate()
            .find(|(i, c)| *i != 0 && c.symbol == sym0)
            .map(|(i, _)| i)
            .expect("pair");

        // First pair.
        game.cursor = (0, 0);
        game.flip_at_cursor();
        let col = (pair0 % game.cols as usize) as u32;
        let row = (pair0 / game.cols as usize) as u32;
        game.cursor = (col, row);
        game.flip_at_cursor();

        // Find remaining unmatched pair.
        let remaining: Vec<usize> = (0..4).filter(|i| !game.matched[*i]).collect();
        assert_eq!(remaining.len(), 2);

        let a = remaining[0];
        let b = remaining[1];
        let col_a = (a % game.cols as usize) as u32;
        let row_a = (a / game.cols as usize) as u32;
        let col_b = (b % game.cols as usize) as u32;
        let row_b = (b / game.cols as usize) as u32;

        game.cursor = (col_a, row_a);
        game.flip_at_cursor();
        game.cursor = (col_b, row_b);
        game.flip_at_cursor();

        assert_eq!(game.state, GameState::Won);
        assert_eq!(game.pairs_found, 2);
    }

    #[test]
    fn memory_one_pair() {
        let game = MemoryGame::new(2, 1, 42);
        assert_eq!(game.total_pairs, 1);
        assert_eq!(game.cards.len(), 2);
        assert_eq!(game.cards[0].symbol, game.cards[1].symbol);
    }

    #[test]
    fn memory_move_counting() {
        let mut game = MemoryGame::new(4, 4, 42);
        game.cursor = (0, 0);
        game.flip_at_cursor();
        assert_eq!(game.moves, 0); // First flip alone doesn't count.
        game.cursor = (1, 0);
        game.flip_at_cursor();
        assert_eq!(game.moves, 1);
    }

    #[test]
    fn memory_shuffle_randomness() {
        let game1 = MemoryGame::new(4, 4, 100);
        let game2 = MemoryGame::new(4, 4, 200);
        let syms1: Vec<char> = game1.cards.iter().map(|c| c.symbol).collect();
        let syms2: Vec<char> = game2.cards.iter().map(|c| c.symbol).collect();
        // Different seeds should produce different arrangements.
        assert_ne!(syms1, syms2);
    }

    #[test]
    fn memory_reset() {
        let mut game = MemoryGame::new(4, 4, 42);
        game.moves = 10;
        game.pairs_found = 3;
        game.state = GameState::Won;
        game.reset(99);
        assert_eq!(game.moves, 0);
        assert_eq!(game.pairs_found, 0);
        assert_eq!(game.state, GameState::Playing);
        assert!(game.cards.iter().all(|c| !c.face_up));
    }

    #[test]
    fn memory_grid_text_has_lines() {
        let game = MemoryGame::new(4, 4, 42);
        let lines = game.grid_text();
        // Header + separator + 4 rows + separator + controls.
        assert_eq!(lines.len(), 8);
        assert!(lines[0].contains("Memory"));
    }

    #[test]
    fn memory_cannot_flip_during_reveal() {
        let mut game = MemoryGame::new(4, 4, 42);
        let first_sym = game.cards[0].symbol;
        let second = game
            .cards
            .iter()
            .enumerate()
            .find(|(i, c)| *i != 0 && c.symbol != first_sym)
            .map(|(i, _)| i)
            .expect("different card");

        game.cursor = (0, 0);
        game.flip_at_cursor();
        let col = (second % game.cols as usize) as u32;
        let row = (second / game.cols as usize) as u32;
        game.cursor = (col, row);
        game.flip_at_cursor();
        assert!(game.reveal_timer > 0);

        // Try to flip another card during reveal -- should be blocked.
        game.cursor = (2, 0);
        game.flip_at_cursor();
        assert!(!game.cards[2].face_up);
    }

    // -- Sliding Puzzle tests --

    #[test]
    fn puzzle_creation() {
        let puzzle = SlidingPuzzle::new(4, 42);
        assert_eq!(puzzle.size, 4);
        assert_eq!(puzzle.tiles.len(), 16);
        assert_eq!(puzzle.moves, 0);
        assert_eq!(puzzle.state, GameState::Playing);
    }

    #[test]
    fn puzzle_contains_all_tiles() {
        let puzzle = SlidingPuzzle::new(4, 42);
        let mut sorted = puzzle.tiles.clone();
        sorted.sort();
        let expected: Vec<u8> = (0..16).collect();
        assert_eq!(sorted, expected);
    }

    #[test]
    fn puzzle_is_solvable_after_creation() {
        let puzzle = SlidingPuzzle::new(4, 42);
        assert!(SlidingPuzzle::is_solvable(&puzzle.tiles, puzzle.size));
    }

    #[test]
    fn puzzle_is_solvable_multiple_seeds() {
        for seed in 0..20 {
            let puzzle = SlidingPuzzle::new(4, seed);
            assert!(
                SlidingPuzzle::is_solvable(&puzzle.tiles, puzzle.size),
                "puzzle with seed {seed} should be solvable"
            );
        }
    }

    #[test]
    fn puzzle_solved_state() {
        let mut puzzle = SlidingPuzzle::new(4, 42);
        // Set to solved state manually.
        let total = 16usize;
        for i in 0..total - 1 {
            puzzle.tiles[i] = (i + 1) as u8;
        }
        puzzle.tiles[total - 1] = 0;
        puzzle.empty_pos = total - 1;
        assert!(puzzle.is_solved());
    }

    #[test]
    fn puzzle_not_solved_initially() {
        // With sufficient shuffling, puzzle shouldn't be in solved state.
        let puzzle = SlidingPuzzle::new(4, 42);
        // It is extremely unlikely but possible. Check a few seeds.
        // For seed 42 with 200 shuffles, it should not be solved.
        assert!(!puzzle.is_solved());
    }

    #[test]
    fn puzzle_valid_slide() {
        let mut puzzle = SlidingPuzzle::new(4, 42);
        let empty = puzzle.empty_pos;
        let s = puzzle.size as usize;
        let row = empty / s;

        // Try sliding up if possible (tile below moves up).
        if row + 1 < s {
            let moved = puzzle.slide(Direction::Up);
            assert!(moved);
            assert_eq!(puzzle.moves, 1);
            assert_ne!(puzzle.empty_pos, empty);
        }
    }

    #[test]
    fn puzzle_invalid_slide() {
        let mut puzzle = SlidingPuzzle::new(4, 42);
        // Place empty at top-left corner.
        let old_empty = puzzle.empty_pos;
        // Swap empty to position 0.
        puzzle.tiles.swap(0, old_empty);
        puzzle.empty_pos = 0;

        // Trying Down (tile above moves down) should fail at row 0.
        let moved = puzzle.slide(Direction::Down);
        assert!(!moved);

        // Trying Right (tile to the left moves right) should fail at col 0.
        let moved = puzzle.slide(Direction::Right);
        assert!(!moved);
    }

    #[test]
    fn puzzle_slide_increments_moves() {
        let mut puzzle = SlidingPuzzle::new(4, 42);
        let s = puzzle.size as usize;
        let row = puzzle.empty_pos / s;
        let col = puzzle.empty_pos % s;

        let dir = if row + 1 < s {
            Direction::Up
        } else if col + 1 < s {
            Direction::Left
        } else {
            Direction::Down
        };

        puzzle.slide(dir);
        assert!(puzzle.moves >= 1);
    }

    #[test]
    fn puzzle_solvability_check_solved() {
        let tiles: Vec<u8> = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 0];
        assert!(SlidingPuzzle::is_solvable(&tiles, 4));
    }

    #[test]
    fn puzzle_solvability_check_unsolvable() {
        // Swap two tiles to create an unsolvable state.
        let tiles: Vec<u8> = vec![2, 1, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 0];
        assert!(!SlidingPuzzle::is_solvable(&tiles, 4));
    }

    #[test]
    fn puzzle_solvability_odd_grid() {
        // 3x3 solved.
        let tiles: Vec<u8> = vec![1, 2, 3, 4, 5, 6, 7, 8, 0];
        assert!(SlidingPuzzle::is_solvable(&tiles, 3));

        // 3x3 unsolvable (swap 1 and 2).
        let tiles: Vec<u8> = vec![2, 1, 3, 4, 5, 6, 7, 8, 0];
        assert!(!SlidingPuzzle::is_solvable(&tiles, 3));
    }

    #[test]
    fn puzzle_solvability_wrong_length() {
        let tiles: Vec<u8> = vec![1, 2, 3];
        assert!(!SlidingPuzzle::is_solvable(&tiles, 4));
    }

    #[test]
    fn puzzle_shuffle_preserves_solvability() {
        let mut puzzle = SlidingPuzzle::new(4, 42);
        puzzle.shuffle(500);
        assert!(SlidingPuzzle::is_solvable(&puzzle.tiles, puzzle.size));
    }

    #[test]
    fn puzzle_reset() {
        let mut puzzle = SlidingPuzzle::new(4, 42);
        puzzle.moves = 50;
        puzzle.state = GameState::Won;
        puzzle.reset(99);
        assert_eq!(puzzle.moves, 0);
        assert_eq!(puzzle.state, GameState::Playing);
        assert!(SlidingPuzzle::is_solvable(&puzzle.tiles, puzzle.size));
    }

    #[test]
    fn puzzle_grid_text_has_lines() {
        let puzzle = SlidingPuzzle::new(4, 42);
        let lines = puzzle.grid_text();
        // Header + separator + 4 rows + separator + controls.
        assert_eq!(lines.len(), 8);
        assert!(lines[0].contains("Sliding Puzzle"));
    }

    #[test]
    fn puzzle_grid_text_shows_tiles() {
        let puzzle = SlidingPuzzle::new(4, 42);
        let lines = puzzle.grid_text();
        // Grid rows should contain tile numbers.
        let grid_text: String = lines[2..6].join(" ");
        assert!(grid_text.contains('1'));
    }

    #[test]
    fn puzzle_already_solved() {
        let mut puzzle = SlidingPuzzle::new(4, 42);
        for i in 0..15usize {
            puzzle.tiles[i] = (i + 1) as u8;
        }
        puzzle.tiles[15] = 0;
        puzzle.empty_pos = 15;
        puzzle.state = GameState::Playing;
        assert!(puzzle.is_solved());
    }

    #[test]
    fn puzzle_won_on_solve() {
        let mut puzzle = SlidingPuzzle::new(4, 42);
        // Set up one move away from solved.
        for i in 0..15usize {
            puzzle.tiles[i] = (i + 1) as u8;
        }
        puzzle.tiles[15] = 0;
        puzzle.empty_pos = 15;
        // Swap 15 with empty to be one move away.
        puzzle.tiles[15] = 15;
        puzzle.tiles[14] = 0;
        puzzle.empty_pos = 14;
        puzzle.state = GameState::Playing;
        // Slide left: tile at col+1 moves into empty (position 15 -> 14).
        let moved = puzzle.slide(Direction::Left);
        assert!(moved);
        assert_eq!(puzzle.state, GameState::Won);
    }

    #[test]
    fn puzzle_no_slide_when_won() {
        let mut puzzle = SlidingPuzzle::new(4, 42);
        puzzle.state = GameState::Won;
        assert!(!puzzle.slide(Direction::Up));
    }

    // -- Direction tests --

    #[test]
    fn direction_opposite() {
        assert!(Direction::Up.is_opposite(Direction::Down));
        assert!(Direction::Down.is_opposite(Direction::Up));
        assert!(Direction::Left.is_opposite(Direction::Right));
        assert!(Direction::Right.is_opposite(Direction::Left));
    }

    #[test]
    fn direction_not_opposite() {
        assert!(!Direction::Up.is_opposite(Direction::Left));
        assert!(!Direction::Up.is_opposite(Direction::Right));
        assert!(!Direction::Down.is_opposite(Direction::Left));
        assert!(!Direction::Left.is_opposite(Direction::Up));
    }

    // -- GamesApp tests --

    #[test]
    fn games_app_creation() {
        let app = GamesApp::new("/apps/games");
        assert_eq!(app.title(), "Games");
        assert_eq!(app.path(), "/apps/games");
        assert_eq!(app.active_game(), ActiveGame::Menu);
    }

    #[test]
    fn games_app_menu_lines() {
        let app = GamesApp::new("/apps/games");
        let lines = app.lines();
        assert!(lines.iter().any(|l| l.contains("Snake")));
        assert!(lines.iter().any(|l| l.contains("Memory Match")));
        assert!(lines.iter().any(|l| l.contains("Sliding Puzzle")));
    }

    #[test]
    fn games_app_menu_navigation() {
        let vfs = make_vfs();
        let mut app = GamesApp::new("/apps/games");
        assert_eq!(app.menu_cursor, 0);

        app.handle_input(&Button::Down, &vfs);
        assert_eq!(app.menu_cursor, 1);

        app.handle_input(&Button::Down, &vfs);
        assert_eq!(app.menu_cursor, 2);

        // Should not go past last item.
        app.handle_input(&Button::Down, &vfs);
        assert_eq!(app.menu_cursor, 2);

        app.handle_input(&Button::Up, &vfs);
        assert_eq!(app.menu_cursor, 1);

        // Should not go above first item.
        app.handle_input(&Button::Up, &vfs);
        app.handle_input(&Button::Up, &vfs);
        assert_eq!(app.menu_cursor, 0);
    }

    #[test]
    fn games_app_launch_snake() {
        let vfs = make_vfs();
        let mut app = GamesApp::new("/apps/games");
        app.handle_input(&Button::Confirm, &vfs);
        assert_eq!(app.active_game(), ActiveGame::Snake);
    }

    #[test]
    fn games_app_launch_memory() {
        let vfs = make_vfs();
        let mut app = GamesApp::new("/apps/games");
        app.handle_input(&Button::Down, &vfs);
        app.handle_input(&Button::Confirm, &vfs);
        assert_eq!(app.active_game(), ActiveGame::Memory);
    }

    #[test]
    fn games_app_launch_puzzle() {
        let vfs = make_vfs();
        let mut app = GamesApp::new("/apps/games");
        app.handle_input(&Button::Down, &vfs);
        app.handle_input(&Button::Down, &vfs);
        app.handle_input(&Button::Confirm, &vfs);
        assert_eq!(app.active_game(), ActiveGame::Puzzle);
    }

    #[test]
    fn games_app_return_to_menu_from_snake() {
        let vfs = make_vfs();
        let mut app = GamesApp::new("/apps/games");
        app.handle_input(&Button::Confirm, &vfs);
        assert_eq!(app.active_game(), ActiveGame::Snake);
        app.handle_input(&Button::Cancel, &vfs);
        assert_eq!(app.active_game(), ActiveGame::Menu);
    }

    #[test]
    fn games_app_return_to_menu_from_memory() {
        let vfs = make_vfs();
        let mut app = GamesApp::new("/apps/games");
        app.handle_input(&Button::Down, &vfs);
        app.handle_input(&Button::Confirm, &vfs);
        assert_eq!(app.active_game(), ActiveGame::Memory);
        app.handle_input(&Button::Cancel, &vfs);
        assert_eq!(app.active_game(), ActiveGame::Menu);
    }

    #[test]
    fn games_app_return_to_menu_from_puzzle() {
        let vfs = make_vfs();
        let mut app = GamesApp::new("/apps/games");
        app.handle_input(&Button::Down, &vfs);
        app.handle_input(&Button::Down, &vfs);
        app.handle_input(&Button::Confirm, &vfs);
        assert_eq!(app.active_game(), ActiveGame::Puzzle);
        app.handle_input(&Button::Cancel, &vfs);
        assert_eq!(app.active_game(), ActiveGame::Menu);
    }

    #[test]
    fn games_app_cancel_on_menu_exits() {
        let vfs = make_vfs();
        let mut app = GamesApp::new("/apps/games");
        let action = app.handle_input(&Button::Cancel, &vfs);
        assert_eq!(action, AppAction::Exit);
    }

    #[test]
    fn games_app_snake_direction_input() {
        let vfs = make_vfs();
        let mut app = GamesApp::new("/apps/games");
        app.handle_input(&Button::Confirm, &vfs); // Launch snake.
        app.handle_input(&Button::Down, &vfs);
        assert_eq!(app.snake.next_direction, Direction::Down);
    }

    #[test]
    fn games_app_snake_pause() {
        let vfs = make_vfs();
        let mut app = GamesApp::new("/apps/games");
        app.handle_input(&Button::Confirm, &vfs);
        app.handle_input(&Button::Start, &vfs);
        assert_eq!(app.snake.state, GameState::Paused);
        app.handle_input(&Button::Start, &vfs);
        assert_eq!(app.snake.state, GameState::Playing);
    }

    #[test]
    fn games_app_snake_restart() {
        let vfs = make_vfs();
        let mut app = GamesApp::new("/apps/games");
        app.handle_input(&Button::Confirm, &vfs);
        app.snake.score = 5;
        app.handle_input(&Button::Select, &vfs);
        assert_eq!(app.snake.score, 0);
        assert_eq!(app.snake.state, GameState::Playing);
    }

    #[test]
    fn games_app_memory_cursor_movement() {
        let vfs = make_vfs();
        let mut app = GamesApp::new("/apps/games");
        app.handle_input(&Button::Down, &vfs);
        app.handle_input(&Button::Confirm, &vfs); // Launch memory.

        assert_eq!(app.memory.cursor, (0, 0));
        app.handle_input(&Button::Right, &vfs);
        assert_eq!(app.memory.cursor, (1, 0));
        app.handle_input(&Button::Down, &vfs);
        assert_eq!(app.memory.cursor, (1, 1));
        app.handle_input(&Button::Left, &vfs);
        assert_eq!(app.memory.cursor, (0, 1));
        app.handle_input(&Button::Up, &vfs);
        assert_eq!(app.memory.cursor, (0, 0));
    }

    #[test]
    fn games_app_memory_cursor_bounds() {
        let vfs = make_vfs();
        let mut app = GamesApp::new("/apps/games");
        app.handle_input(&Button::Down, &vfs);
        app.handle_input(&Button::Confirm, &vfs);

        // At (0,0), Up and Left should be clamped.
        app.handle_input(&Button::Up, &vfs);
        assert_eq!(app.memory.cursor, (0, 0));
        app.handle_input(&Button::Left, &vfs);
        assert_eq!(app.memory.cursor, (0, 0));
    }

    #[test]
    fn games_app_puzzle_slide_input() {
        let vfs = make_vfs();
        let mut app = GamesApp::new("/apps/games");
        app.handle_input(&Button::Down, &vfs);
        app.handle_input(&Button::Down, &vfs);
        app.handle_input(&Button::Confirm, &vfs); // Launch puzzle.

        // Try all four directions; at least one should work.
        app.handle_input(&Button::Up, &vfs);
        app.handle_input(&Button::Down, &vfs);
        app.handle_input(&Button::Left, &vfs);
        app.handle_input(&Button::Right, &vfs);
        // Moves counter should have increased (some slides succeed).
        assert!(app.puzzle.moves > 0);
    }

    #[test]
    fn games_app_refresh_ticks_snake() {
        let vfs = make_vfs();
        let mut app = GamesApp::new("/apps/games");
        app.handle_input(&Button::Confirm, &vfs); // Launch snake.
        app.snake.speed = 1;
        let old_head = app.snake.snake[0];
        app.refresh(&vfs);
        assert_ne!(app.snake.snake[0], old_head);
    }

    #[test]
    fn games_app_refresh_ticks_memory() {
        let vfs = make_vfs();
        let mut app = GamesApp::new("/apps/games");
        app.handle_input(&Button::Down, &vfs);
        app.handle_input(&Button::Confirm, &vfs); // Launch memory.
        app.memory.reveal_timer = 5;
        app.refresh(&vfs);
        assert_eq!(app.memory.reveal_timer, 4);
    }

    #[test]
    fn games_app_downcast() {
        let app = GamesApp::new("/apps/games");
        let any = app.as_any();
        assert!(any.downcast_ref::<GamesApp>().is_some());
    }

    #[test]
    fn games_app_lines_update_on_game_switch() {
        let vfs = make_vfs();
        let mut app = GamesApp::new("/apps/games");
        let menu_lines = app.lines().to_vec();

        app.handle_input(&Button::Confirm, &vfs);
        let snake_lines = app.lines().to_vec();
        assert_ne!(menu_lines, snake_lines);
        assert!(snake_lines.iter().any(|l| l.contains("Snake")));
    }

    #[test]
    fn games_app_select_restarts_memory() {
        let vfs = make_vfs();
        let mut app = GamesApp::new("/apps/games");
        app.handle_input(&Button::Down, &vfs);
        app.handle_input(&Button::Confirm, &vfs);
        app.memory.moves = 10;
        app.handle_input(&Button::Select, &vfs);
        assert_eq!(app.memory.moves, 0);
    }

    #[test]
    fn games_app_select_restarts_puzzle() {
        let vfs = make_vfs();
        let mut app = GamesApp::new("/apps/games");
        app.handle_input(&Button::Down, &vfs);
        app.handle_input(&Button::Down, &vfs);
        app.handle_input(&Button::Confirm, &vfs);
        app.puzzle.moves = 50;
        app.handle_input(&Button::Select, &vfs);
        assert_eq!(app.puzzle.moves, 0);
    }
}
