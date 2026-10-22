//! The rules of this variant of Go.
//!
//! Standard 19x19 play, capture, and no suicide. There is no scoring and no
//! result: the only terminal state is two consecutive passes.

pub mod board;
pub mod coord;
pub mod rules;
pub mod sgf;

pub use board::{Board, Cell};
pub use coord::{Point, BOARD_SIZE, POINTS};
pub use rules::{place, Positions, RuleViolation};
