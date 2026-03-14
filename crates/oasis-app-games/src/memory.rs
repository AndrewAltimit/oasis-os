//! Memory match (concentration) card game.

use crate::common::GameState;
use crate::prng::random_range;

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
    pub(crate) cols: u32,
    pub(crate) rows: u32,
    pub(crate) cards: Vec<Card>,
    pub(crate) cursor: (u32, u32),
    pub(crate) first_pick: Option<usize>,
    pub(crate) second_pick: Option<usize>,
    pub(crate) matched: Vec<bool>,
    pub(crate) moves: u32,
    pub(crate) pairs_found: u32,
    pub(crate) total_pairs: u32,
    pub(crate) state: GameState,
    pub(crate) reveal_timer: u32,
    /// Retained for re-seeding on reset.
    pub(crate) _rng_state: u64,
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
