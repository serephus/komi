//! Game level legality: positional superko, on top of the board mechanics.
//!
//! The rule this server enforces is "no repeated board": a stone placement is
//! illegal if the resulting whole-board position already occurred in this game.
//! That subsumes the simple ko rule.
//!
//! Passes are deliberately exempt. A pass does not change the board, so under a
//! literal reading of positional superko every pass would repeat a position and
//! therefore be illegal — which would make the two-pass ending unreachable.
//! Exempting passes is the standard formulation ("no repetition caused by a
//! play").

use std::collections::HashSet;

use super::board::{Board, Cell, PlayViolation};
use super::coord::Point;

/// Every position the game has already had, as raw board bytes.
pub type Positions = HashSet<Vec<u8>>;

#[derive(Clone, Copy, PartialEq, Eq, Debug, thiserror::Error)]
pub enum RuleViolation {
    #[error("that point is already occupied")]
    Occupied,
    #[error("suicide is not allowed")]
    Suicide,
    #[error("that move would repeat an earlier position, which the rules forbid")]
    Superko,
}

impl From<PlayViolation> for RuleViolation {
    fn from(violation: PlayViolation) -> Self {
        match violation {
            PlayViolation::Occupied => RuleViolation::Occupied,
            PlayViolation::Suicide => RuleViolation::Suicide,
        }
    }
}

/// Apply a stone placement. `seen` must contain every position this game has
/// had, including the current one. On success the new board and the captured
/// points are returned; on failure `board` is untouched.
pub fn place(
    board: &Board,
    color: Cell,
    point: Point,
    seen: &Positions,
) -> Result<(Board, Vec<Point>), RuleViolation> {
    let mut next = board.clone();
    let captured = next.try_play(color, point)?;
    if seen.contains(next.as_bytes()) {
        return Err(RuleViolation::Superko);
    }
    Ok((next, captured))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny harness so the tests read like game records.
    struct Game {
        board: Board,
        seen: Positions,
        to_move: Cell,
    }

    impl Game {
        fn new() -> Self {
            let board = Board::new();
            let mut seen = Positions::new();
            seen.insert(board.as_bytes().to_vec());
            Game {
                board,
                seen,
                to_move: Cell::Black,
            }
        }

        fn play(&mut self, x: u8, y: u8) -> Result<Vec<Point>, RuleViolation> {
            let point = Point::new(x, y).unwrap();
            let (next, captured) = place(&self.board, self.to_move, point, &self.seen)?;
            self.seen.insert(next.as_bytes().to_vec());
            self.board = next;
            self.to_move = self.to_move.opponent();
            Ok(captured)
        }

        fn must_play(&mut self, x: u8, y: u8) {
            self.play(x, y).expect("expected a legal move");
        }

        fn pass(&mut self) {
            self.to_move = self.to_move.opponent();
        }

        fn at(&self, x: u8, y: u8) -> Cell {
            self.board.get(Point::new(x, y).unwrap())
        }
    }

    fn p(x: u8, y: u8) -> Point {
        Point::new(x, y).unwrap()
    }

    /// Builds the canonical ko shape and lets black capture the ko stone.
    ///
    /// White's stone at (2,2) ends up in atari with its last liberty at (2,3),
    /// and the black stone that captures it is itself surrounded by (1,3),
    /// (3,3) and (2,4), leaving exactly one liberty at (2,2).
    fn black_captures_the_ko() -> Game {
        let mut game = Game::new();
        game.must_play(1, 2); // B
        game.must_play(1, 3); // W
        game.must_play(2, 1); // B
        game.must_play(2, 2); // W  the stone that will be captured
        game.must_play(3, 2); // B
        game.must_play(3, 3); // W
        game.must_play(10, 10); // B filler
        game.must_play(2, 4); // W
        let captured = game.play(2, 3).expect("the capture is legal");
        assert_eq!(captured, vec![p(2, 2)]);
        assert_eq!(game.at(2, 3), Cell::Black);
        game
    }

    #[test]
    fn the_position_history_starts_with_the_empty_board() {
        let game = Game::new();
        assert_eq!(game.seen.len(), 1);
        assert!(game.board.is_empty());
    }

    #[test]
    fn immediate_recapture_of_a_ko_is_a_superko_violation() {
        let mut game = black_captures_the_ko();
        assert_eq!(game.play(2, 2), Err(RuleViolation::Superko));
        // The rejected move did not touch the board.
        assert_eq!(game.at(2, 3), Cell::Black);
        assert_eq!(game.at(2, 2), Cell::Empty);
    }

    #[test]
    fn a_ko_may_be_recaptured_after_play_elsewhere() {
        let mut game = black_captures_the_ko();
        game.must_play(15, 15); // W
        game.must_play(16, 16); // B
        let captured = game.play(2, 2).expect("recapture is legal by now");
        assert_eq!(captured, vec![p(2, 3)]);
        assert_eq!(game.at(2, 3), Cell::Empty);
    }

    #[test]
    fn suicides_are_reported() {
        let mut game = Game::new();
        game.must_play(1, 0); // B
        game.must_play(10, 10); // W
        game.must_play(0, 1); // B
        game.must_play(11, 11); // W
        game.must_play(10, 12); // B
        assert_eq!(game.play(0, 0), Err(RuleViolation::Suicide));
    }

    #[test]
    fn passes_do_not_change_the_board_and_are_not_repetitions() {
        let mut game = Game::new();
        let before = game.board.clone();
        game.pass();
        game.pass();
        game.pass();
        assert_eq!(game.board, before);
        assert_eq!(game.seen.len(), 1, "passing must not record a position");
    }
}
