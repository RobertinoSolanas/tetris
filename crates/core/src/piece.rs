//! Tetromino geometry: the seven shapes, their rotation states, and the active piece.
//!
//! Shapes are declared **once**, in their SRS spawn orientation, inside a square bounding box
//! (4×4 for `I`, 2×2 for `O`, 3×3 for the rest). Every other rotation state is derived by rotating
//! the box mathematically instead of hand-transcribing twelve more tables — a transcription typo in
//! a shape is the classic silent Tetris bug, and `tests::derived_states_match_srs` pins the derived
//! values against the SRS reference sets anyway.

use core::fmt;

/// One grid square, in board coordinates. `col` grows right, `row` grows **down**.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Cell {
    pub col: i32,
    pub row: i32,
}

impl Cell {
    pub const fn new(col: i32, row: i32) -> Self {
        Self { col, row }
    }

    pub const fn shifted(self, dc: i32, dr: i32) -> Self {
        Self {
            col: self.col + dc,
            row: self.row + dr,
        }
    }
}

impl fmt::Display for Cell {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({}, {})", self.col, self.row)
    }
}

/// The four SRS rotation states.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Rotation {
    /// State 0 — spawn orientation.
    Spawn,
    /// State 1 — clockwise from spawn.
    Cw,
    /// State 2 — 180° from spawn.
    Flip,
    /// State 3 — counter-clockwise from spawn.
    Ccw,
}

impl Rotation {
    pub const ALL: [Rotation; 4] = [Rotation::Spawn, Rotation::Cw, Rotation::Flip, Rotation::Ccw];

    pub const fn index(self) -> usize {
        match self {
            Rotation::Spawn => 0,
            Rotation::Cw => 1,
            Rotation::Flip => 2,
            Rotation::Ccw => 3,
        }
    }

    pub const fn from_index(i: usize) -> Self {
        match i & 3 {
            0 => Rotation::Spawn,
            1 => Rotation::Cw,
            2 => Rotation::Flip,
            _ => Rotation::Ccw,
        }
    }

    pub const fn cw(self) -> Self {
        Self::from_index(self.index() + 1)
    }

    pub const fn ccw(self) -> Self {
        Self::from_index(self.index() + 3)
    }

    pub const fn flip(self) -> Self {
        Self::from_index(self.index() + 2)
    }
}

/// The seven tetrominoes.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Tetromino {
    I,
    J,
    L,
    O,
    S,
    T,
    Z,
}

/// Spawn orientation inside the piece's bounding box, `[(col, row); 4]`.
const SPAWN: [[(i32, i32); 4]; 7] = [
    // I — 4×4 box, horizontal bar on the second row.
    [(0, 1), (1, 1), (2, 1), (3, 1)],
    // J — 3×3 box.
    [(0, 0), (0, 1), (1, 1), (2, 1)],
    // L — 3×3 box.
    [(2, 0), (0, 1), (1, 1), (2, 1)],
    // O — 2×2 box, rotation is the identity (SRS gives O a single orientation).
    [(0, 0), (1, 0), (0, 1), (1, 1)],
    // S — 3×3 box.
    [(1, 0), (2, 0), (0, 1), (1, 1)],
    // T — 3×3 box.
    [(1, 0), (0, 1), (1, 1), (2, 1)],
    // Z — 3×3 box.
    [(0, 0), (1, 0), (1, 1), (2, 1)],
];

impl Tetromino {
    pub const ALL: [Tetromino; 7] = [
        Tetromino::I,
        Tetromino::J,
        Tetromino::L,
        Tetromino::O,
        Tetromino::S,
        Tetromino::T,
        Tetromino::Z,
    ];

    pub const fn letter(self) -> char {
        match self {
            Tetromino::I => 'I',
            Tetromino::J => 'J',
            Tetromino::L => 'L',
            Tetromino::O => 'O',
            Tetromino::S => 'S',
            Tetromino::T => 'T',
            Tetromino::Z => 'Z',
        }
    }

    /// Side length of the piece's rotation box.
    pub const fn box_size(self) -> i32 {
        match self {
            Tetromino::I => 4,
            Tetromino::O => 2,
            _ => 3,
        }
    }

    /// Index into [`crate::palette::COLORS`].
    pub const fn color_index(self) -> usize {
        self as usize
    }

    const fn spawn_offsets(self) -> [(i32, i32); 4] {
        SPAWN[self as usize]
    }

    /// The four cells of this piece in its spawn orientation, relative to the box top-left.
    pub fn spawn_cells(self) -> [Cell; 4] {
        self.offsets(Rotation::Spawn)
    }

    /// The four cells of this piece in `rot`, relative to the box top-left.
    pub fn offsets(self, rot: Rotation) -> [Cell; 4] {
        let base = self.spawn_offsets();
        let n = self.box_size();
        let steps = rot.index();
        let mut out = [Cell::new(0, 0); 4];
        for (i, c) in base.iter().enumerate() {
            let (mut col, mut row) = *c;
            for _ in 0..steps {
                // Clockwise rotation inside an n×n box (row grows downward).
                let next = (n - 1 - row, col);
                col = next.0;
                row = next.1;
            }
            out[i] = Cell::new(col, row);
        }
        out
    }

    /// Absolute board cells for a piece at box position `(col, row)` in `rot`.
    pub fn cells_at(self, rot: Rotation, col: i32, row: i32) -> [Cell; 4] {
        let mut out = self.offsets(rot);
        for c in out.iter_mut() {
            c.col += col;
            c.row += row;
        }
        out
    }

    /// Column of the box top-left so the piece spawns centred horizontally.
    ///
    /// 3-wide pieces and `I` spawn with their box at column 3 (`I` then covers columns 3..=6,
    /// the SRS convention); `O` covers columns 4..=5.
    pub const fn spawn_col(self) -> i32 {
        match self {
            Tetromino::O => 4,
            _ => 3,
        }
    }

    /// Row of the box top-left at spawn: row 0 is the topmost buffer row.
    pub const fn spawn_row(self) -> i32 {
        0
    }
}

impl fmt::Display for Tetromino {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.letter())
    }
}

/// The piece currently under player control.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ActivePiece {
    pub kind: Tetromino,
    pub rot: Rotation,
    /// Column of the bounding box top-left.
    pub col: i32,
    /// Row of the bounding box top-left.
    pub row: i32,
}

impl ActivePiece {
    /// A piece at its spawn position and orientation.
    pub fn spawn(kind: Tetromino) -> Self {
        Self {
            kind,
            rot: Rotation::Spawn,
            col: kind.spawn_col(),
            row: kind.spawn_row(),
        }
    }

    pub fn cells(&self) -> [Cell; 4] {
        self.kind.cells_at(self.rot, self.col, self.row)
    }

    pub fn moved(mut self, dc: i32, dr: i32) -> Self {
        self.col += dc;
        self.row += dr;
        self
    }

    /// The piece after rotating, *without* wall kicks (position unchanged).
    pub fn rotated(mut self, dir: crate::actions::RotateDir) -> Self {
        self.rot = match dir {
            crate::actions::RotateDir::Cw => self.rot.cw(),
            crate::actions::RotateDir::Ccw => self.rot.ccw(),
            crate::actions::RotateDir::Flip => self.rot.flip(),
        };
        self
    }

    /// Lowest row of the piece's occupied cells (not the box).
    pub fn bottom_row(&self) -> i32 {
        self.cells().iter().map(|c| c.row).max().unwrap_or(self.row)
    }

    /// Row of the highest occupied cell.
    pub fn top_row(&self) -> i32 {
        self.cells().iter().map(|c| c.row).min().unwrap_or(self.row)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::RotateDir;

    fn set(cells: &[Cell]) -> Vec<(i32, i32)> {
        let mut v: Vec<(i32, i32)> = cells.iter().map(|c| (c.col, c.row)).collect();
        v.sort_unstable();
        v
    }

    #[test]
    fn every_piece_has_four_cells_in_every_rotation() {
        for kind in Tetromino::ALL {
            for rot in Rotation::ALL {
                let cells = kind.offsets(rot);
                assert_eq!(cells.len(), 4, "{kind} {rot:?}");
                let uniq = set(&cells);
                assert_eq!(
                    uniq.len(),
                    4,
                    "{kind} {rot:?} has duplicate cells: {uniq:?}"
                );
                // Everything stays inside the bounding box.
                let n = kind.box_size();
                for c in cells {
                    assert!(c.col >= 0 && c.col < n, "{kind} {rot:?} col {}", c.col);
                    assert!(c.row >= 0 && c.row < n, "{kind} {rot:?} row {}", c.row);
                }
            }
        }
    }

    /// Reference values derived independently by rotating the SRS spawn *pictures* (the ASCII art
    /// in `docs/domain`), so they do not share a code path with `Tetromino::offsets`. This is the
    /// test that catches a broken rotation formula.
    #[test]
    fn derived_states_match_srs() {
        type State = (Tetromino, Rotation, &'static [(i32, i32)]);
        let expected: &[State] = &[
            (
                Tetromino::I,
                Rotation::Spawn,
                &[(0, 1), (1, 1), (2, 1), (3, 1)],
            ),
            (
                Tetromino::I,
                Rotation::Cw,
                &[(2, 0), (2, 1), (2, 2), (2, 3)],
            ),
            (
                Tetromino::I,
                Rotation::Flip,
                &[(0, 2), (1, 2), (2, 2), (3, 2)],
            ),
            (
                Tetromino::I,
                Rotation::Ccw,
                &[(1, 0), (1, 1), (1, 2), (1, 3)],
            ),
            (
                Tetromino::J,
                Rotation::Spawn,
                &[(0, 0), (0, 1), (1, 1), (2, 1)],
            ),
            (
                Tetromino::J,
                Rotation::Cw,
                &[(1, 0), (1, 1), (1, 2), (2, 0)],
            ),
            (
                Tetromino::J,
                Rotation::Flip,
                &[(0, 1), (1, 1), (2, 1), (2, 2)],
            ),
            (
                Tetromino::J,
                Rotation::Ccw,
                &[(0, 2), (1, 0), (1, 1), (1, 2)],
            ),
            (
                Tetromino::L,
                Rotation::Spawn,
                &[(2, 0), (0, 1), (1, 1), (2, 1)],
            ),
            (
                Tetromino::L,
                Rotation::Cw,
                &[(1, 0), (1, 1), (1, 2), (2, 2)],
            ),
            (
                Tetromino::L,
                Rotation::Flip,
                &[(0, 1), (0, 2), (1, 1), (2, 1)],
            ),
            (
                Tetromino::L,
                Rotation::Ccw,
                &[(0, 0), (1, 0), (1, 1), (1, 2)],
            ),
            (
                Tetromino::O,
                Rotation::Spawn,
                &[(0, 0), (1, 0), (0, 1), (1, 1)],
            ),
            (
                Tetromino::O,
                Rotation::Cw,
                &[(0, 0), (1, 0), (0, 1), (1, 1)],
            ),
            (
                Tetromino::O,
                Rotation::Flip,
                &[(0, 0), (1, 0), (0, 1), (1, 1)],
            ),
            (
                Tetromino::O,
                Rotation::Ccw,
                &[(0, 0), (1, 0), (0, 1), (1, 1)],
            ),
            (
                Tetromino::S,
                Rotation::Spawn,
                &[(1, 0), (2, 0), (0, 1), (1, 1)],
            ),
            (
                Tetromino::S,
                Rotation::Cw,
                &[(1, 0), (1, 1), (2, 1), (2, 2)],
            ),
            (
                Tetromino::S,
                Rotation::Flip,
                &[(0, 2), (1, 1), (1, 2), (2, 1)],
            ),
            (
                Tetromino::S,
                Rotation::Ccw,
                &[(0, 0), (0, 1), (1, 1), (1, 2)],
            ),
            (
                Tetromino::T,
                Rotation::Spawn,
                &[(1, 0), (0, 1), (1, 1), (2, 1)],
            ),
            (
                Tetromino::T,
                Rotation::Cw,
                &[(1, 0), (1, 1), (1, 2), (2, 1)],
            ),
            (
                Tetromino::T,
                Rotation::Flip,
                &[(0, 1), (1, 1), (1, 2), (2, 1)],
            ),
            (
                Tetromino::T,
                Rotation::Ccw,
                &[(0, 1), (1, 0), (1, 1), (1, 2)],
            ),
            (
                Tetromino::Z,
                Rotation::Spawn,
                &[(0, 0), (1, 0), (1, 1), (2, 1)],
            ),
            (
                Tetromino::Z,
                Rotation::Cw,
                &[(2, 0), (2, 1), (1, 1), (1, 2)],
            ),
            (
                Tetromino::Z,
                Rotation::Flip,
                &[(0, 1), (1, 1), (1, 2), (2, 2)],
            ),
            (
                Tetromino::Z,
                Rotation::Ccw,
                &[(0, 1), (0, 2), (1, 0), (1, 1)],
            ),
        ];

        for (kind, rot, want) in expected {
            assert_eq!(
                set(&kind.offsets(*rot)),
                set(&want
                    .iter()
                    .map(|&(c, r)| Cell::new(c, r))
                    .collect::<Vec<_>>()),
                "{kind} in {rot:?}"
            );
        }
    }

    #[test]
    fn four_cw_rotations_are_the_identity() {
        for kind in Tetromino::ALL {
            for rot in Rotation::ALL {
                let mut r = rot;
                for _ in 0..4 {
                    r = r.cw();
                }
                assert_eq!(r, rot);
                assert_eq!(set(&kind.offsets(r)), set(&kind.offsets(rot)));
            }
        }
    }

    #[test]
    fn ccw_is_inverse_of_cw() {
        for rot in Rotation::ALL {
            assert_eq!(rot.cw().ccw(), rot);
            assert_eq!(rot.ccw().cw(), rot);
            assert_eq!(rot.flip().flip(), rot);
        }
    }

    #[test]
    fn o_piece_never_changes_shape() {
        for rot in Rotation::ALL {
            assert_eq!(
                set(&Tetromino::O.offsets(rot)),
                set(&Tetromino::O.offsets(Rotation::Spawn))
            );
        }
    }

    #[test]
    fn spawn_positions_are_centred() {
        // I covers 3..=6, O covers 4..=5, everything else 3..=5.
        let i = Tetromino::I.cells_at(Rotation::Spawn, Tetromino::I.spawn_col(), 0);
        assert_eq!(set(&i), vec![(3, 1), (4, 1), (5, 1), (6, 1)]);
        let o = Tetromino::O.cells_at(Rotation::Spawn, Tetromino::O.spawn_col(), 0);
        assert_eq!(set(&o), vec![(4, 0), (4, 1), (5, 0), (5, 1)]);
        let t = Tetromino::T.cells_at(Rotation::Spawn, Tetromino::T.spawn_col(), 0);
        assert_eq!(set(&t), vec![(3, 1), (4, 0), (4, 1), (5, 1)]);
    }

    #[test]
    fn active_piece_moves_and_rotates() {
        let p = ActivePiece::spawn(Tetromino::T);
        assert_eq!(p.rot, Rotation::Spawn);
        assert_eq!(p.moved(-1, 2).cells()[0].col, p.cells()[0].col - 1);

        let r = p.rotated(RotateDir::Cw);
        assert_eq!(r.rot, Rotation::Cw);
        assert_eq!(r.col, p.col, "rotation must not translate the box");

        assert_eq!(p.rotated(RotateDir::Flip).rot, Rotation::Flip);
        assert_eq!(p.rotated(RotateDir::Ccw).rot, Rotation::Ccw);
    }

    #[test]
    fn bottom_and_top_rows_track_the_shape() {
        let p = ActivePiece::spawn(Tetromino::I);
        assert_eq!(p.top_row(), 1);
        assert_eq!(p.bottom_row(), 1);
        let v = p.rotated(RotateDir::Cw);
        assert_eq!(v.top_row(), v.row);
        assert_eq!(v.bottom_row(), v.row + 3);
    }

    #[test]
    fn colour_index_matches_all_order() {
        for (i, kind) in Tetromino::ALL.iter().enumerate() {
            assert_eq!(kind.color_index(), i);
        }
    }
}
