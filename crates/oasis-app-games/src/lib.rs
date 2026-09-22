//! Games collection app: Snake, Memory Match, and Sliding Puzzle.
//!
//! Provides three classic games in a single app, selectable from a menu.
//! All games use text-based rendering through content lines and a
//! deterministic LCG PRNG (no external dependencies).
//!
//! Real-time games (Snake, Memory Match's reveal timer) run on a fixed
//! 60 Hz simulation clock driven by [`App::tick`], so their speed is
//! independent of the host's frame rate. The Snake high score persists
//! to [`HIGH_SCORES_PATH`].

mod common;
mod memory;
mod prng;
mod puzzle;
mod snake;

pub use common::{Direction, GameState};
pub use memory::{Card, MemoryGame};
pub use puzzle::SlidingPuzzle;
pub use snake::SnakeGame;

use oasis_app_core::{App, AppAction, ContentState, impl_content_app_methods};
use oasis_types::input::Button;
use oasis_vfs::Vfs;

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
    /// Unsimulated wall time, in thirds of a millisecond (one 60 Hz
    /// simulation step = [`SIM_STEP_UNITS`]).
    sim_accum: u32,
    /// Whether the high-score file has been read (lazily, on the first
    /// hook that sees the VFS).
    scores_loaded: bool,
    /// Snake high score as last read from / written to disk.
    persisted_snake_high: u32,
}

const MENU_ITEMS: [&str; 3] = ["Snake", "Memory Match", "Sliding Puzzle"];

/// VFS path of the persisted high-score table.
pub const HIGH_SCORES_PATH: &str = "/home/user/.games.toml";

/// Wall time per simulation step, in thirds of a millisecond: 50/3 ms is
/// exactly one 60 Hz frame, the rate the per-step game logic (Snake's
/// `speed`, Memory's reveal timer) was tuned for.
const SIM_STEP_UNITS: u32 = 50;

/// Longest wall-time gap simulated in one [`App::tick`] (ms). A host
/// stall (window drag, debugger, suspended tab) must not replay seconds
/// of game time at once and run the snake into a wall unseen.
const MAX_TICK_MS: u32 = 250;

/// Parse the Snake high score out of the `.games.toml` table.
///
/// Hand-rolled for the one key it needs (`high_score` under `[snake]`);
/// anything malformed reads as 0.
fn parse_snake_high_score(text: &str) -> u32 {
    let mut in_snake = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_snake = line == "[snake]";
        } else if in_snake
            && let Some((key, value)) = line.split_once('=')
            && key.trim() == "high_score"
        {
            return value.trim().parse().unwrap_or(0);
        }
    }
    0
}

/// Serialize the high-score table.
fn format_high_scores(snake_high: u32) -> String {
    format!("# OASIS_OS games high scores\n\n[snake]\nhigh_score = {snake_high}\n")
}

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
            sim_accum: 0,
            scores_loaded: false,
            persisted_snake_high: 0,
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

    /// Read the persisted high scores once.
    fn load_high_scores(&mut self, vfs: &dyn Vfs) {
        if self.scores_loaded {
            return;
        }
        self.scores_loaded = true;
        let Ok(data) = vfs.read(HIGH_SCORES_PATH) else {
            return;
        };
        let high = parse_snake_high_score(&String::from_utf8_lossy(&data));
        self.persisted_snake_high = high;
        if high > self.snake.high_score {
            self.snake.high_score = high;
            self.update_lines();
        }
    }

    /// Whether the in-memory high score is newer than the file.
    fn scores_dirty(&self) -> bool {
        self.scores_loaded && self.snake.high_score > self.persisted_snake_high
    }

    /// Run one fixed 60 Hz simulation step of the active game.
    fn sim_step(&mut self) {
        match self.active_game {
            ActiveGame::Snake => self.snake.tick(),
            ActiveGame::Memory => self.memory.tick(),
            ActiveGame::Menu | ActiveGame::Puzzle => {},
        }
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
    impl_content_app_methods!(content);

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

    fn refresh(&mut self, vfs: &dyn Vfs) {
        // Game time advances in `tick` only: refresh runs on clicks, and
        // stepping here made the snake move per click instead of in time.
        self.load_high_scores(vfs);
    }

    fn tick(&mut self, dt_ms: u32, vfs: &dyn Vfs) -> bool {
        self.load_high_scores(vfs);
        self.frame_counter = self.frame_counter.wrapping_add(1);

        if matches!(self.active_game, ActiveGame::Menu | ActiveGame::Puzzle) {
            // Turn-based screens: drop the backlog so resuming a real-time
            // game does not replay the time spent here.
            self.sim_accum = 0;
            return false;
        }

        self.sim_accum += dt_ms.min(MAX_TICK_MS) * 3;
        let mut stepped = false;
        while self.sim_accum >= SIM_STEP_UNITS {
            self.sim_accum -= SIM_STEP_UNITS;
            self.sim_step();
            stepped = true;
        }
        if !stepped {
            return false;
        }

        let before = std::mem::take(&mut self.content.lines);
        self.update_lines();
        self.content.lines != before
    }

    fn apply_vfs_ops(&mut self, vfs: &mut dyn Vfs) -> bool {
        if !self.scores_dirty() {
            return false;
        }
        let high = self.snake.high_score;
        // Record the attempt either way: a read-only VFS must not turn
        // into a write retry every frame.
        self.persisted_snake_high = high;
        if let Some((dir, _)) = HIGH_SCORES_PATH.rsplit_once('/')
            && !vfs.exists(dir)
        {
            let _ = vfs.mkdir(dir);
        }
        let _ = vfs.write(HIGH_SCORES_PATH, format_high_scores(high).as_bytes());
        true
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
    fn games_app_tick_steps_snake() {
        let vfs = make_vfs();
        let mut app = GamesApp::new("/apps/games");
        app.handle_input(&Button::Confirm, &vfs); // Launch snake.
        app.snake.speed = 1;
        let old_head = app.snake.snake[0];
        assert!(app.tick(17, &vfs), "a moving snake must request a redraw");
        assert_ne!(app.snake.snake[0], old_head);
    }

    #[test]
    fn games_app_refresh_does_not_step_snake() {
        let vfs = make_vfs();
        let mut app = GamesApp::new("/apps/games");
        app.handle_input(&Button::Confirm, &vfs);
        app.snake.speed = 1;
        let old_head = app.snake.snake[0];
        app.refresh(&vfs);
        assert_eq!(
            app.snake.snake[0], old_head,
            "clicks must not advance game time"
        );
    }

    #[test]
    fn games_app_tick_counts_down_memory() {
        let vfs = make_vfs();
        let mut app = GamesApp::new("/apps/games");
        app.handle_input(&Button::Down, &vfs);
        app.handle_input(&Button::Confirm, &vfs); // Launch memory.
        app.memory.reveal_timer = 5;
        // 50 ms = exactly three 60 Hz steps.
        app.tick(50, &vfs);
        assert_eq!(app.memory.reveal_timer, 2);
    }

    /// Cells the snake head advanced over `total_ms` of wall time
    /// delivered in `dt_ms` frames. The head starts at x=10 on a 20-wide
    /// grid moving right, so keep `total_ms` under ~900 ms.
    fn snake_cells_after(total_ms: u32, dt_ms: u32) -> usize {
        let vfs = make_vfs();
        let mut app = GamesApp::new("/apps/games");
        app.handle_input(&Button::Confirm, &vfs);
        let start = app.snake.snake[0].0;
        let mut t = 0;
        while t < total_ms {
            app.tick(dt_ms, &vfs);
            t += dt_ms;
        }
        (app.snake.snake[0].0 - start) as usize
    }

    #[test]
    fn snake_speed_is_independent_of_frame_rate() {
        // Default speed 6 = one cell per 100 ms of game time.
        let at_30fps = snake_cells_after(600, 33);
        let at_60fps = snake_cells_after(600, 16);
        let at_144fps = snake_cells_after(600, 7);
        let at_10fps = snake_cells_after(600, 100);
        assert!((5..=6).contains(&at_60fps), "60 fps moved {at_60fps}");
        for (fps, cells) in [(30, at_30fps), (144, at_144fps), (10, at_10fps)] {
            assert!(
                cells.abs_diff(at_60fps) <= 1,
                "{fps} fps moved {cells}, 60 fps moved {at_60fps}"
            );
        }
    }

    #[test]
    fn snake_ticks_between_steps_request_no_redraw() {
        let vfs = make_vfs();
        let mut app = GamesApp::new("/apps/games");
        app.handle_input(&Button::Confirm, &vfs);
        // 1 ms is well under one 60 Hz step: nothing simulated.
        assert!(!app.tick(1, &vfs));
        // Simulated steps that don't move the snake (speed 6) don't
        // redraw either.
        assert!(!app.tick(17, &vfs));
        app.handle_input(&Button::Start, &vfs); // Pause.
        assert!(!app.tick(1000, &vfs), "a paused game never redraws");
    }

    #[test]
    fn menu_time_is_not_banked() {
        let vfs = make_vfs();
        let mut app = GamesApp::new("/apps/games");
        assert!(!app.tick(5000, &vfs));
        app.handle_input(&Button::Confirm, &vfs); // Snake.
        let head = app.snake.snake[0];
        app.tick(1, &vfs);
        assert_eq!(app.snake.snake[0], head, "menu time must not replay");
    }

    #[test]
    fn long_stall_is_clamped() {
        let vfs = make_vfs();
        let mut app = GamesApp::new("/apps/games");
        app.handle_input(&Button::Confirm, &vfs);
        app.tick(60_000, &vfs);
        assert_eq!(app.snake.state, GameState::Playing);
        assert!(app.snake.snake[0].0 <= 13, "at most 250 ms simulated");
    }

    #[test]
    fn high_score_round_trips_through_vfs() {
        let mut vfs = make_vfs();
        let mut app = GamesApp::new("/apps/games");
        app.handle_input(&Button::Confirm, &vfs);
        app.tick(0, &vfs); // Loads (absent) scores.
        app.snake.high_score = 7;
        assert!(app.apply_vfs_ops(&mut vfs), "new high score is written");
        assert!(!app.apply_vfs_ops(&mut vfs), "written once");
        let text = String::from_utf8(vfs.read(HIGH_SCORES_PATH).unwrap()).unwrap();
        assert_eq!(parse_snake_high_score(&text), 7);

        let mut fresh = GamesApp::new("/apps/games");
        fresh.refresh(&vfs);
        assert_eq!(fresh.snake().high_score, 7);
        assert!(!fresh.apply_vfs_ops(&mut vfs), "loaded score is not dirty");
    }

    #[test]
    fn game_over_persists_high_score() {
        let mut vfs = make_vfs();
        let mut app = GamesApp::new("/apps/games");
        app.handle_input(&Button::Confirm, &vfs);
        app.snake.score = 3;
        // Run right into the wall (10 cells at 100 ms each).
        for _ in 0..40 {
            app.tick(100, &vfs);
        }
        assert_eq!(app.snake.state, GameState::GameOver);
        // (Food on the row may have added to the score on the way.)
        assert!(app.snake.high_score >= 3);
        assert!(app.apply_vfs_ops(&mut vfs));
        let text = String::from_utf8(vfs.read(HIGH_SCORES_PATH).unwrap()).unwrap();
        assert_eq!(parse_snake_high_score(&text), app.snake.high_score);
    }

    #[test]
    fn parse_high_scores_tolerates_junk() {
        assert_eq!(parse_snake_high_score(""), 0);
        assert_eq!(parse_snake_high_score("[snake]\nhigh_score = x"), 0);
        assert_eq!(parse_snake_high_score("[other]\nhigh_score = 9"), 0);
        assert_eq!(parse_snake_high_score(&format_high_scores(42)), 42);
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
