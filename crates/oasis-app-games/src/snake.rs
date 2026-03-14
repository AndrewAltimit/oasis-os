//! Classic snake game on a text grid.

use crate::common::{Direction, GameState};
use crate::prng::random_range;

/// Classic snake game on a text grid.
#[derive(Debug, Clone)]
pub struct SnakeGame {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) snake: Vec<(i32, i32)>,
    pub(crate) direction: Direction,
    pub(crate) next_direction: Direction,
    pub(crate) food: (i32, i32),
    pub(crate) score: u32,
    pub(crate) high_score: u32,
    pub(crate) state: GameState,
    pub(crate) tick_counter: u32,
    pub(crate) speed: u32,
    pub(crate) rng_state: u64,
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
