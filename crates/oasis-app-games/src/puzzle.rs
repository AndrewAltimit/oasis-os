//! Classic 15-puzzle (sliding tiles).

use crate::common::{Direction, GameState};
use crate::prng::{next_random, random_range};

/// Classic 15-puzzle (sliding tiles).
#[derive(Debug, Clone)]
pub struct SlidingPuzzle {
    pub(crate) size: u32,
    pub(crate) tiles: Vec<u8>,
    pub(crate) empty_pos: usize,
    pub(crate) moves: u32,
    pub(crate) state: GameState,
    pub(crate) rng_state: u64,
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
