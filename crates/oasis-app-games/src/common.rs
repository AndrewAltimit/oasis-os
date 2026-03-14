//! Shared types used across games.

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
    pub(crate) fn is_opposite(self, other: Self) -> bool {
        matches!(
            (self, other),
            (Self::Up, Self::Down)
                | (Self::Down, Self::Up)
                | (Self::Left, Self::Right)
                | (Self::Right, Self::Left)
        )
    }
}

/// State of any individual game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameState {
    Playing,
    Paused,
    GameOver,
    Won,
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
