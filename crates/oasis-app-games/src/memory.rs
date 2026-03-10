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
