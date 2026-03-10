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
