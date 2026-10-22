//! The board: stone placement, groups, liberties and captures.

use super::coord::{Point, BOARD_SIZE, POINTS};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cell {
    Empty,
    Black,
    White,
}

impl Cell {
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Cell::Empty),
            1 => Some(Cell::Black),
            2 => Some(Cell::White),
            _ => None,
        }
    }

    pub fn to_u8(self) -> u8 {
        match self {
            Cell::Empty => 0,
            Cell::Black => 1,
            Cell::White => 2,
        }
    }

    pub fn opponent(self) -> Cell {
        match self {
            Cell::Black => Cell::White,
            Cell::White => Cell::Black,
            Cell::Empty => Cell::Empty,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Cell::Black => "Black",
            Cell::White => "White",
            Cell::Empty => "Empty",
        }
    }

    pub fn lower_name(self) -> &'static str {
        match self {
            Cell::Black => "black",
            Cell::White => "white",
            Cell::Empty => "empty",
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            Cell::Black => "●",
            Cell::White => "○",
            Cell::Empty => "",
        }
    }

    pub fn sgf_letter(self) -> &'static str {
        match self {
            Cell::Black => "B",
            Cell::White => "W",
            Cell::Empty => "",
        }
    }
}

/// A 19x19 position. Stored one byte per point so that a position can be
/// compared exactly (that is what positional superko needs) and persisted
/// verbatim into the database.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Board {
    cells: [u8; POINTS],
}

impl Default for Board {
    fn default() -> Self {
        Board::new()
    }
}

impl Board {
    pub fn new() -> Self {
        Board { cells: [0; POINTS] }
    }

    /// Rebuild a position previously produced by [`Board::as_bytes`].
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != POINTS {
            return None;
        }
        let mut cells = [0u8; POINTS];
        for (slot, value) in cells.iter_mut().zip(bytes) {
            if *value > 2 {
                return None;
            }
            *slot = *value;
        }
        Some(Board { cells })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.cells
    }

    pub fn get(&self, point: Point) -> Cell {
        Cell::from_u8(self.cells[point.index()]).expect("only legal values are stored")
    }

    pub fn get_index(&self, index: usize) -> Cell {
        Cell::from_u8(self.cells[index]).expect("only legal values are stored")
    }

    pub fn set_index(&mut self, index: usize, cell: Cell) {
        self.cells[index] = cell.to_u8();
    }

    pub fn is_empty(&self) -> bool {
        self.cells.iter().all(|c| *c == 0)
    }

    pub fn stone_count(&self) -> usize {
        self.cells.iter().filter(|c| **c != 0).count()
    }

    /// The four orthogonal neighbours of a point (fewer on the edge).
    pub fn neighbours(index: usize) -> impl Iterator<Item = usize> {
        let point = Point::from_index(index);
        let x = point.x as usize;
        let y = point.y as usize;
        let mut buffer = [None; 4];
        let mut len = 0;
        if x > 0 {
            buffer[len] = Some(index - 1);
            len += 1;
        }
        if x + 1 < BOARD_SIZE {
            buffer[len] = Some(index + 1);
            len += 1;
        }
        if y > 0 {
            buffer[len] = Some(index - BOARD_SIZE);
            len += 1;
        }
        if y + 1 < BOARD_SIZE {
            buffer[len] = Some(index + BOARD_SIZE);
            len += 1;
        }
        buffer.into_iter().take(len).flatten()
    }

    /// Flood fill the group that `start` belongs to. Returns the stones of the
    /// group and its distinct liberties.
    pub fn group(&self, start: usize) -> (Vec<usize>, Vec<usize>) {
        let color = self.cells[start];
        if color == 0 {
            return (Vec::new(), vec![start]);
        }
        let mut stones = Vec::new();
        let mut liberties: Vec<usize> = Vec::new();
        let mut seen = [false; POINTS];
        let mut stack = vec![start];
        seen[start] = true;
        while let Some(current) = stack.pop() {
            stones.push(current);
            for neighbour in Self::neighbours(current) {
                match self.cells[neighbour] {
                    0 => {
                        if !liberties.contains(&neighbour) {
                            liberties.push(neighbour);
                        }
                    }
                    value if value == color && !seen[neighbour] => {
                        seen[neighbour] = true;
                        stack.push(neighbour);
                    }
                    _ => {}
                }
            }
        }
        (stones, liberties)
    }

    /// Number of liberties of the group at `index`.
    pub fn liberties(&self, index: usize) -> usize {
        self.group(index).1.len()
    }

    /// Play a stone, removing any adjacent opponent groups that were reduced to
    /// zero liberties. Returns the captured points.
    ///
    /// The board is left untouched when the move is rejected, so this is safe to
    /// call with a position that a caller might still want to render.
    pub fn try_play(&mut self, color: Cell, point: Point) -> Result<Vec<Point>, PlayViolation> {
        assert_ne!(color, Cell::Empty, "only black or white can move");
        let index = point.index();
        if self.cells[index] != 0 {
            return Err(PlayViolation::Occupied);
        }

        let mut next = self.clone();
        next.cells[index] = color.to_u8();
        let opponent = color.opponent();
        let mut captured = Vec::new();
        let mut handled = [false; POINTS];

        for neighbour in Self::neighbours(index) {
            if next.cells[neighbour] != opponent.to_u8() || handled[neighbour] {
                continue;
            }
            let (stones, liberties) = next.group(neighbour);
            for stone in &stones {
                handled[*stone] = true;
            }
            if liberties.is_empty() {
                for stone in stones {
                    next.cells[stone] = 0;
                    captured.push(Point::from_index(stone));
                }
            }
        }

        // Suicide: after the captures, our own group still has no liberty.
        if next.group(index).1.is_empty() {
            return Err(PlayViolation::Suicide);
        }

        captured.sort();
        *self = next;
        Ok(captured)
    }
}

/// Why [`Board::try_play`] refused a move. Positional superko is checked one
/// level up, in [`super::rules`], because it needs the game history.
#[derive(Clone, Copy, PartialEq, Eq, Debug, thiserror::Error)]
pub enum PlayViolation {
    #[error("that point is already occupied")]
    Occupied,
    #[error("suicide is not allowed")]
    Suicide,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(x: u8, y: u8) -> Point {
        Point::new(x, y).unwrap()
    }

    fn play(board: &mut Board, color: Cell, x: u8, y: u8) -> Vec<Point> {
        board.try_play(color, point(x, y)).expect("legal move")
    }

    #[test]
    fn board_starts_empty() {
        let board = Board::new();
        assert!(board.is_empty());
        assert_eq!(board.stone_count(), 0);
    }

    #[test]
    fn a_lone_stone_has_four_liberties() {
        let mut board = Board::new();
        play(&mut board, Cell::Black, 9, 9);
        assert_eq!(board.liberties(point(9, 9).index()), 4);
    }

    #[test]
    fn a_corner_stone_has_two_liberties() {
        let mut board = Board::new();
        play(&mut board, Cell::Black, 0, 0);
        assert_eq!(board.liberties(0), 2);
        play(&mut board, Cell::Black, 18, 18);
        assert_eq!(board.liberties(point(18, 18).index()), 2);
    }

    #[test]
    fn neighbours_respect_the_edges() {
        assert_eq!(Board::neighbours(0).count(), 2);
        assert_eq!(Board::neighbours(point(9, 9).index()).count(), 4);
        assert_eq!(Board::neighbours(point(0, 9).index()).count(), 3);
    }

    #[test]
    fn occupied_points_are_rejected() {
        let mut board = Board::new();
        play(&mut board, Cell::Black, 9, 9);
        assert_eq!(
            board.try_play(Cell::White, point(9, 9)),
            Err(PlayViolation::Occupied)
        );
    }

    #[test]
    fn a_single_stone_is_captured() {
        let mut board = Board::new();
        play(&mut board, Cell::Black, 1, 0);
        play(&mut board, Cell::White, 0, 0);
        assert_eq!(board.get(point(0, 0)), Cell::White);
        let captured = play(&mut board, Cell::Black, 0, 1);
        assert_eq!(captured, vec![point(0, 0)]);
        assert_eq!(board.get(point(0, 0)), Cell::Empty);
    }

    #[test]
    fn a_whole_group_is_captured_at_once() {
        let mut board = Board::new();
        // White stones on the first line, walled in by black. The final liberty
        // is (2,1), filling it captures both stones in one move.
        play(&mut board, Cell::White, 1, 0);
        play(&mut board, Cell::White, 2, 0);
        play(&mut board, Cell::Black, 0, 0);
        play(&mut board, Cell::Black, 3, 0);
        play(&mut board, Cell::Black, 1, 1);
        assert_eq!(board.get(point(1, 0)), Cell::White);
        let captured = play(&mut board, Cell::Black, 2, 1);
        assert_eq!(captured, vec![point(1, 0), point(2, 0)]);
        assert_eq!(board.get(point(1, 0)), Cell::Empty);
        assert_eq!(board.get(point(2, 0)), Cell::Empty);
    }

    #[test]
    fn multi_group_capture_in_one_move() {
        // Black plays the shared liberty of two separate white groups.
        let mut board = Board::new();
        // White groups at (0,1) and (2,1), each with liberties (0,0),(1,0) and
        // (2,0),(3,0)... build a cleaner shape: two white stones each in an atari.
        play(&mut board, Cell::White, 1, 2);
        play(&mut board, Cell::White, 3, 2);
        play(&mut board, Cell::Black, 0, 2);
        play(&mut board, Cell::Black, 2, 2);
        play(&mut board, Cell::Black, 4, 2);
        play(&mut board, Cell::Black, 1, 3);
        play(&mut board, Cell::Black, 3, 3);
        // White (1,2) has liberties (1,1). White (3,2) has liberties (3,1).
        let captured = play(&mut board, Cell::Black, 1, 1);
        assert_eq!(captured, vec![point(1, 2)]);
        // The other group is untouched because (3,1) is still empty.
        assert_eq!(board.get(point(3, 2)), Cell::White);
    }

    #[test]
    fn suicide_is_rejected_and_leaves_the_board_alone() {
        let mut board = Board::new();
        // Black surrounds the point (0,0) with (1,0) and (0,1).
        play(&mut board, Cell::Black, 1, 0);
        play(&mut board, Cell::Black, 0, 1);
        let before = board.clone();
        assert_eq!(
            board.try_play(Cell::White, point(0, 0)),
            Err(PlayViolation::Suicide)
        );
        assert_eq!(board, before);
    }

    #[test]
    fn a_move_that_would_be_suicide_is_legal_when_it_captures() {
        // The playing stone at the corner (0,0) has no liberty of its own: both
        // its neighbours are white. It is legal only because it captures the two
        // white stones, which is the shape some snapbacks are built from.
        let mut board = Board::new();
        play(&mut board, Cell::Black, 1, 1);
        play(&mut board, Cell::Black, 2, 0);
        play(&mut board, Cell::Black, 0, 2);
        play(&mut board, Cell::White, 1, 0);
        play(&mut board, Cell::White, 0, 1);
        assert_eq!(board.liberties(point(1, 0).index()), 1);
        assert_eq!(board.liberties(point(0, 1).index()), 1);

        let mut captured = play(&mut board, Cell::Black, 0, 0);
        captured.sort();
        assert_eq!(captured, vec![point(0, 1), point(1, 0)]);
        assert_eq!(board.get(point(0, 0)), Cell::Black);
        assert_eq!(board.liberties(0), 2);
    }

    #[test]
    fn bytes_round_trip_and_validate() {
        let mut board = Board::new();
        play(&mut board, Cell::Black, 3, 3);
        play(&mut board, Cell::White, 15, 15);
        let restored = Board::from_bytes(board.as_bytes()).unwrap();
        assert_eq!(board, restored);

        assert!(Board::from_bytes(&[0u8; 10]).is_none());
        let mut bad = [0u8; POINTS];
        bad[7] = 3;
        assert!(Board::from_bytes(&bad).is_none());
    }
}
