//! Points, board coordinates and the two letter-encodings we need
//! (human readable labels and SGF coordinates).

use std::fmt;

pub const BOARD_SIZE: usize = 19;
pub const POINTS: usize = BOARD_SIZE * BOARD_SIZE;

/// Column letters, standard Go convention: `I` is skipped.
const COLUMNS: [char; BOARD_SIZE] = [
    'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'J', 'K', 'L', 'M', 'N', 'O', 'P', 'Q', 'R', 'S', 'T',
];

/// The column letter for a zero based column index (`I` is skipped).
pub fn column_letter(x: usize) -> char {
    COLUMNS[x]
}

/// A board point. `x` counts from the left, `y` counts from the *bottom*,
/// both zero based, which matches how Go players read coordinates.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Point {
    pub x: u8,
    pub y: u8,
}

impl Point {
    pub fn new(x: u8, y: u8) -> Option<Self> {
        let limit = BOARD_SIZE as u8;
        (x < limit && y < limit).then_some(Point { x, y })
    }

    /// Row-major index into a board array.
    pub fn index(self) -> usize {
        self.y as usize * BOARD_SIZE + self.x as usize
    }

    pub fn from_index(index: usize) -> Self {
        debug_assert!(index < POINTS);
        Point {
            x: (index % BOARD_SIZE) as u8,
            y: (index / BOARD_SIZE) as u8,
        }
    }

    /// Human readable form, e.g. `D16` (column letter + row number).
    pub fn label(self) -> String {
        format!("{}{}", COLUMNS[self.x as usize], self.y + 1)
    }

    /// Parse a human readable coordinate such as `d4` or `Q16`.
    pub fn parse_label(text: &str) -> Option<Self> {
        let text = text.trim();
        let mut chars = text.chars();
        let column = chars.next()?.to_ascii_uppercase();
        let x = COLUMNS.iter().position(|c| *c == column)?;
        let row: u8 = chars.as_str().parse().ok()?;
        if chars.as_str().len() != 1 && chars.as_str().len() != 2 {
            return None;
        }
        if !(1..=BOARD_SIZE as u8).contains(&row) {
            return None;
        }
        Point::new(x as u8, row - 1)
    }

    /// SGF coordinates are two letters from `a`, and the y axis is inverted
    /// (SGF counts rows from the top).
    pub fn to_sgf(self) -> String {
        let x = (b'a' + self.x) as char;
        let y = (b'a' + (BOARD_SIZE as u8 - 1 - self.y)) as char;
        format!("{x}{y}")
    }

    pub fn from_sgf(text: &str) -> Option<Self> {
        let bytes = text.as_bytes();
        if bytes.len() != 2 {
            return None;
        }
        let x = bytes[0].wrapping_sub(b'a');
        let y = bytes[1].wrapping_sub(b'a');
        if x >= BOARD_SIZE as u8 || y >= BOARD_SIZE as u8 {
            return None;
        }
        Point::new(x, BOARD_SIZE as u8 - 1 - y)
    }
}

impl fmt::Display for Point {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.label())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_skip_the_letter_i() {
        assert_eq!(Point::new(0, 0).unwrap().label(), "A1");
        assert_eq!(Point::new(7, 0).unwrap().label(), "H1");
        assert_eq!(Point::new(8, 0).unwrap().label(), "J1");
        assert_eq!(Point::new(18, 18).unwrap().label(), "T19");
        assert_eq!(Point::new(3, 15).unwrap().label(), "D16");
    }

    #[test]
    fn every_point_round_trips_through_labels() {
        for index in 0..POINTS {
            let point = Point::from_index(index);
            assert_eq!(point.index(), index);
            assert_eq!(Point::parse_label(&point.label()), Some(point));
        }
        assert_eq!(Point::parse_label("I5"), None);
        assert_eq!(Point::parse_label("A0"), None);
        assert_eq!(Point::parse_label("A20"), None);
        assert_eq!(Point::parse_label(""), None);
        assert_eq!(Point::parse_label("Z9"), None);
    }

    #[test]
    fn labels_are_case_insensitive() {
        let d16 = Point::parse_label("d16").unwrap();
        assert_eq!(d16, Point::new(3, 15).unwrap());
    }

    #[test]
    fn sgf_inverts_the_y_axis() {
        let top_left = Point::new(0, 18).unwrap();
        assert_eq!(top_left.to_sgf(), "aa");
        let bottom_left = Point::new(0, 0).unwrap();
        assert_eq!(bottom_left.to_sgf(), "as");
        let centre = Point::new(9, 9).unwrap();
        assert_eq!(centre.to_sgf(), "jj");
        for index in 0..POINTS {
            let point = Point::from_index(index);
            assert_eq!(Point::from_sgf(&point.to_sgf()), Some(point));
        }
        assert_eq!(Point::from_sgf("zz"), None);
    }
}
