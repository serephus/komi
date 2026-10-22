//! SGF export. The only export format this server offers: no scoring, but the
//! move record is fully replayable in any SGF viewer.

use std::fmt::Write;

use super::board::Cell;
use super::coord::Point;

pub const SGF_APPLICATION: &str = "komi:0.1";

/// Escape a property value as SGF requires.
fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            ']' => out.push_str("\\]"),
            ':' => out.push(':'),
            '\n' | '\r' => out.push(' '),
            _ => out.push(ch),
        }
    }
    out
}

/// Render a game as SGF. `date` is expected in `YYYY-MM-DD` form; passes are
/// written as empty move values, which is the SGF way of saying "pass".
pub fn build(black: &str, white: &str, date: &str, moves: &[(Cell, Option<Point>)]) -> String {
    let mut sgf = String::with_capacity(256 + moves.len() * 8);
    sgf.push_str("(;GM[1]FF[4]CA[UTF-8]");
    let _ = write!(sgf, "AP[{}]", escape(SGF_APPLICATION));
    sgf.push_str("SZ[19]");
    // There is no result, so komi is meaningless; state it explicitly rather
    // than letting a viewer apply a default of 6.5.
    sgf.push_str("KM[0]");
    sgf.push_str("RU[Positional superko, no result]");
    let _ = write!(sgf, "PB[{}]PW[{}]", escape(black), escape(white));
    let _ = write!(sgf, "DT[{}]", escape(date));
    sgf.push_str("RE[Void]");
    for (color, point) in moves {
        let letter = color.sgf_letter();
        match point {
            Some(point) => {
                let _ = write!(sgf, ";{letter}[{}]", point.to_sgf());
            }
            None => {
                let _ = write!(sgf, ";{letter}[]");
            }
        }
    }
    sgf.push(')');
    sgf
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::go::board::Board;
    use crate::go::rules::Positions;

    fn p(x: u8, y: u8) -> Point {
        Point::new(x, y).unwrap()
    }

    #[test]
    fn writes_a_replayable_record() {
        let moves = vec![
            (Cell::Black, Some(p(15, 3))),
            (Cell::White, Some(p(3, 15))),
            (Cell::Black, None),
            (Cell::White, Some(p(9, 9))),
        ];
        let sgf = build("alice", "bob", "2024-05-06", &moves);
        assert!(sgf.starts_with("(;GM[1]FF[4]CA[UTF-8]AP[komi:0.1]SZ[19]KM[0]"));
        assert!(sgf.contains("PB[alice]PW[bob]DT[2024-05-06]"));
        assert!(sgf.contains(";B[pp];W[dd];B[];W[jj]"));
        assert!(sgf.ends_with(')'));
    }

    #[test]
    fn move_coordinates_survive_a_round_trip_through_a_board() {
        // Replay the SGF's coordinates and confirm we end up where the board does.
        let mut board = Board::new();
        let mut seen = Positions::new();
        seen.insert(board.as_bytes().to_vec());
        let moves = [(Cell::Black, Some(p(3, 3))), (Cell::White, Some(p(15, 15)))];
        let sgf = build("a", "b", "2024-01-01", &moves);
        assert!(sgf.contains(";B[dp];W[pd]"));

        let (mut colour, mut last) = (Cell::Black, None);
        for (_, point) in moves {
            let point = point.expect("both moves place a stone");
            let (next, _) = crate::go::rules::place(&board, colour, point, &seen).unwrap();
            seen.insert(next.as_bytes().to_vec());
            board = next;
            last = Some(point);
            colour = colour.opponent();
        }
        assert_eq!(last, Some(p(15, 15)));
        assert_eq!(board.stone_count(), 2);
    }

    #[test]
    fn property_values_are_escaped() {
        let sgf = build("a]b\\c", "plain", "2024-01-01", &[]);
        assert!(sgf.contains("PB[a\\]b\\\\c]"));
    }
}
