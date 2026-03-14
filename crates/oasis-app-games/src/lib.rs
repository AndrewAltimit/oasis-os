//! Games collection app: Snake, Memory Match, and Sliding Puzzle.
//!
//! Provides three classic games in a single app, selectable from a menu.
//! All games use text-based rendering through content lines and a
//! deterministic LCG PRNG (no external dependencies).

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
