//! The playfield: the well, collision, locking, line clearing, ghost projection.
//!
//! This context answers geometry questions only. It does not know *when* to lock (that is
//! [`crate::handling`]), what a clear is *worth* (that is [`crate::scoring`]), or what any of it
//! looks like (that is `tetris-render`).

use crate::piece::{ActivePiece, Cell};
use crate::rng::Fnv;

/// Columns in the well (Guideline width).
pub const BOARD_COLS: usize = 10;
/// Rows the player can see and play in.
pub const VISIBLE_ROWS: usize = 20;
/// Rows above the visible field where pieces spawn. A piece resting entirely in here is a lock-out.
pub const BUFFER_ROWS: usize = 2;
/// Total rows stored, buffer included.
pub const BOARD_ROWS: usize = VISIBLE_ROWS + BUFFER_ROWS;

/// Why a lock could not be written to the board.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LockError {
    /// At least one cell collides with the stack or a wall/floor.
    Collision,
    /// At least one cell came to rest above the stored field (row < 0).
    AboveField,
}

/// The stack. Row-major grid of colour indices (`Some` = filled).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Board {
    cols: usize,
    rows: usize,
    filled: Vec<Option<u8>>,
}

impl Default for Board {
    fn default() -> Self {
        Self::new()
    }
}

impl Board {
    /// The standard 10 × (20 + 2) well.
    pub fn new() -> Self {
        Self::with_size(BOARD_COLS, BOARD_ROWS)
    }

    /// A custom well (used by tests and for very wide debugging layouts).
    pub fn with_size(cols: usize, rows: usize) -> Self {
        let cols = cols.max(1);
        let rows = rows.max(1);
        Self {
            cols,
            rows,
            filled: vec![None; cols * rows],
        }
    }

    pub const fn cols(&self) -> usize {
        self.cols
    }

    pub const fn rows(&self) -> usize {
        self.rows
    }

    /// Number of rows above the visible area.
    pub const fn buffer_rows(&self) -> usize {
        self.rows.saturating_sub(VISIBLE_ROWS)
    }

    fn index(&self, col: i32, row: i32) -> Option<usize> {
        if col < 0 || row < 0 {
            return None;
        }
        let (c, r) = (col as usize, row as usize);
        if c >= self.cols || r >= self.rows {
            return None;
        }
        Some(r * self.cols + c)
    }

    /// `true` if the square is filled, or is a side wall / the floor.
    ///
    /// Squares *above* the stored field (`row < 0`) are **not** occupied: pieces may legally stick
    /// out of the top while spawning or kicking.
    pub fn is_occupied(&self, col: i32, row: i32) -> bool {
        if row < 0 {
            return false;
        }
        match self.index(col, row) {
            Some(i) => self.filled[i].is_some(),
            None => true, // outside the columns or below the floor: a wall
        }
    }

    /// Colour index of a filled square, `None` when empty or out of bounds.
    pub fn color_at(&self, col: i32, row: i32) -> Option<u8> {
        self.index(col, row).and_then(|i| self.filled[i])
    }

    /// Would all of `cells` be legal here?
    pub fn fits(&self, cells: &[Cell]) -> bool {
        cells.iter().all(|c| !self.is_occupied(c.col, c.row))
    }

    /// Convenience over [`Board::fits`] for a positioned piece.
    pub fn fits_piece(&self, piece: &ActivePiece) -> bool {
        self.fits(&piece.cells())
    }

    /// Write the cells into the stack.
    ///
    /// Returns [`LockError::AboveField`] when any cell is above the field (`row < 0`): the caller
    /// (`Session`) turns that into a lock-out, and the in-field cells are still written so the final
    /// board shows what the player did.
    pub fn lock(&mut self, cells: &[Cell], color: u8) -> Result<(), LockError> {
        if !self.fits(cells) {
            return Err(LockError::Collision);
        }
        let above = cells.iter().any(|c| c.row < 0);
        for c in cells {
            if let Some(i) = self.index(c.col, c.row) {
                self.filled[i] = Some(color);
            }
        }
        if above {
            return Err(LockError::AboveField);
        }
        Ok(())
    }

    /// Rows (as row indices) that are completely filled, ascending.
    pub fn full_rows(&self) -> Vec<u32> {
        (0..self.rows as i32)
            .filter(|&r| (0..self.cols as i32).all(|c| self.is_occupied(c, r)))
            .map(|r| r as u32)
            .collect()
    }

    /// Remove the given rows, dropping everything above them by that many rows.
    ///
    /// Returns the number of rows actually removed. Rows are cleared bottom-up so indices stay valid.
    pub fn remove_rows(&mut self, rows: &[u32]) -> usize {
        let mut rows: Vec<usize> = rows.iter().map(|&r| r as usize).collect();
        rows.sort_unstable();
        rows.dedup();
        rows.retain(|&r| r < self.rows);
        if rows.is_empty() {
            return 0;
        }
        // Rebuild bottom-up: walk the surviving rows from the floor upwards and pack them against
        // the floor. O(rows*cols), which for a 10x22 field is nothing.
        let removed = |r: usize| rows.binary_search(&r).is_ok();
        let mut out = vec![None; self.cols * self.rows];
        let mut dst = self.rows;
        for src in (0..self.rows).rev() {
            if removed(src) {
                continue;
            }
            dst -= 1;
            let s = src * self.cols;
            let d = dst * self.cols;
            out[d..d + self.cols].copy_from_slice(&self.filled[s..s + self.cols]);
        }
        self.filled = out;
        rows.len()
    }

    /// Every stored square empty?
    pub fn is_empty(&self) -> bool {
        self.filled.iter().all(|c| c.is_none())
    }

    /// Row of the highest filled square (`None` for an empty board).
    pub fn highest_row(&self) -> Option<i32> {
        (0..self.rows as i32).find(|&r| (0..self.cols as i32).any(|c| self.is_occupied(c, r)))
    }

    /// Row of the lowest filled square (`None` for an empty board).
    pub fn lowest_row(&self) -> Option<i32> {
        (0..self.rows as i32)
            .rev()
            .find(|&r| (0..self.cols as i32).any(|c| self.is_occupied(c, r)))
    }

    /// How many rows `piece` can still fall before it would collide (0 when already grounded).
    pub fn drop_distance(&self, piece: ActivePiece) -> i32 {
        let mut dist = 0;
        loop {
            let candidate = piece.moved(0, dist + 1);
            if self.fits_piece(&candidate) {
                dist += 1;
            } else {
                return dist;
            }
        }
    }

    /// Where the piece would come to rest (the "ghost").
    pub fn ghost(&self, piece: ActivePiece) -> ActivePiece {
        piece.moved(0, self.drop_distance(piece))
    }

    /// Would this piece overlap the stack at spawn?
    pub fn spawn_blocked(&self, piece: ActivePiece) -> bool {
        !self.fits_piece(&piece)
    }

    /// Fraction of the visible field's height occupied by the stack, `0.0..=1.0`.
    ///
    /// Used for the danger meter; `0.0` for an empty board.
    pub fn stack_height_fraction(&self) -> f32 {
        match self.highest_row() {
            None => 0.0,
            Some(top) => {
                let buffer = self.buffer_rows() as i32;
                let top_visible = top.max(buffer);
                (self.rows as i32 - top_visible).max(0) as f32 / VISIBLE_ROWS as f32
            }
        }
    }

    /// Erase everything.
    pub fn clear(&mut self) {
        for cell in self.filled.iter_mut() {
            *cell = None;
        }
    }

    /// Feed the grid into a fingerprint (see [`crate::rng::Fnv`]).
    pub fn fingerprint(&self, hash: &mut Fnv) {
        hash.hash_u64(self.cols as u64);
        hash.hash_u64(self.rows as u64);
        for c in &self.filled {
            hash.hash(match c {
                None => 0,
                Some(v) => v.wrapping_add(1),
            });
        }
    }

    /// Row-major snapshot, `Some` = filled. Handy for tests and for the scene builder.
    pub fn snapshot(&self) -> Vec<Option<u8>> {
        self.filled.clone()
    }

    /// A text rendering of the field, `.` for empty and a piece letter for filled.
    ///
    /// Not used by the game; it is what failing tests print.
    pub fn ascii(&self) -> String {
        let mut out = String::new();
        for r in 0..self.rows as i32 {
            for c in 0..self.cols as i32 {
                out.push(match self.color_at(c, r) {
                    None => '.',
                    Some(i) => crate::Tetromino::ALL
                        .get(i as usize)
                        .map(|k| k.letter())
                        .unwrap_or('?'),
                });
            }
            out.push('\n');
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::piece::{Rotation, Tetromino};

    fn row_cells(row: i32, cols: &[i32]) -> Vec<Cell> {
        cols.iter().map(|&c| Cell::new(c, row)).collect()
    }

    #[test]
    fn standard_dimensions() {
        let b = Board::new();
        assert_eq!(b.cols(), 10);
        assert_eq!(b.rows(), 22);
        assert_eq!(b.buffer_rows(), 2);
        assert!(b.is_empty());
    }

    #[test]
    fn walls_and_floor_are_occupied_above_is_not() {
        let b = Board::new();
        assert!(b.is_occupied(-1, 5), "left wall");
        assert!(b.is_occupied(10, 5), "right wall");
        assert!(b.is_occupied(5, 22), "floor");
        assert!(!b.is_occupied(5, -1), "above the field is open space");
        assert!(!b.is_occupied(0, 0));
    }

    #[test]
    fn lock_and_read_back() {
        let mut b = Board::new();
        assert!(b.lock(&row_cells(21, &[0, 1, 2]), 3).is_ok());
        assert_eq!(b.color_at(0, 21), Some(3));
        assert_eq!(b.color_at(3, 21), None);
        assert!(!b.is_empty());
    }

    #[test]
    fn collision_blocks_lock_and_moves() {
        let mut b = Board::new();
        b.lock(&row_cells(21, &[3, 4, 5]), 0).unwrap();

        let cells = row_cells(21, &[4, 5, 6]);
        assert!(!b.fits(&cells));
        assert_eq!(b.lock(&cells, 1), Err(LockError::Collision));
        assert_eq!(b.color_at(4, 21), Some(0), "original stack untouched");

        // The same shape one row up fits.
        assert!(b.fits(&row_cells(20, &[4, 5, 6])));
    }

    #[test]
    fn horizontal_movement_is_limited_by_walls() {
        let b = Board::new();
        let piece = ActivePiece::spawn(Tetromino::O);
        let mut p = piece;
        while b.fits_piece(&p.moved(-1, 0)) {
            p = p.moved(-1, 0);
        }
        assert_eq!(p.cells().iter().map(|c| c.col).min().unwrap(), 0);
        assert!(!b.fits_piece(&p.moved(-1, 0)));
    }

    #[test]
    fn single_and_multi_row_clear_detection() {
        let mut b = Board::new();
        b.lock(&row_cells(21, &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]), 1)
            .unwrap();
        assert_eq!(b.full_rows(), vec![21]);

        // One gap: not full.
        let mut c = Board::new();
        c.lock(&row_cells(21, &[0, 1, 2, 3, 4, 5, 6, 7, 8]), 1)
            .unwrap();
        assert!(c.full_rows().is_empty());

        // Two full rows separated by a partial one.
        let mut d = Board::new();
        d.lock(&row_cells(21, &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]), 1)
            .unwrap();
        d.lock(&row_cells(19, &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]), 1)
            .unwrap();
        d.lock(&row_cells(20, &[0, 1, 2]), 2).unwrap();
        assert_eq!(d.full_rows(), vec![19, 21]);
    }

    #[test]
    fn removing_rows_shifts_the_stack_down() {
        let mut b = Board::new();
        // Full row at the bottom, plus a lone block two rows above it.
        b.lock(&row_cells(21, &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]), 1)
            .unwrap();
        b.lock(&[Cell::new(4, 19)], 6).unwrap();

        assert_eq!(b.remove_rows(&[21]), 1);
        assert!(b.color_at(4, 21).is_none(), "cleared row is gone");
        assert_eq!(b.color_at(4, 20), Some(6), "block fell one row");
        assert_eq!(b.color_at(4, 19), None);
    }

    #[test]
    fn removing_several_rows_shifts_by_that_many() {
        let mut b = Board::new();
        b.lock(&row_cells(21, &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]), 1)
            .unwrap();
        b.lock(&row_cells(20, &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]), 1)
            .unwrap();
        b.lock(&[Cell::new(0, 19), Cell::new(9, 18)], 4).unwrap();

        assert_eq!(b.remove_rows(&[20, 21]), 2);
        assert_eq!(b.color_at(0, 21), Some(4), "block fell two rows");
        assert_eq!(b.color_at(9, 20), Some(4));
        assert!(b.color_at(0, 20).is_none());
    }

    #[test]
    fn remove_rows_is_idempotent_on_empty_input() {
        let mut b = Board::new();
        b.lock(&[Cell::new(0, 21)], 2).unwrap();
        assert_eq!(b.remove_rows(&[]), 0);
        assert_eq!(b.remove_rows(&[999]), 0);
        assert_eq!(b.color_at(0, 21), Some(2));
        // Duplicate rows count once.
        b.lock(&row_cells(20, &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]), 1)
            .unwrap();
        assert_eq!(b.remove_rows(&[20, 20, 20]), 1);
    }

    #[test]
    fn clearing_the_whole_board_leaves_it_empty() {
        let mut b = Board::new();
        for r in 20..22 {
            b.lock(&row_cells(r, &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]), 1)
                .unwrap();
        }
        let rows = b.full_rows();
        assert_eq!(rows.len(), 2);
        b.remove_rows(&rows);
        assert!(b.is_empty());
    }

    #[test]
    fn drop_distance_and_ghost_land_on_the_floor() {
        let b = Board::new();
        let p = ActivePiece::spawn(Tetromino::T);
        let d = b.drop_distance(p);
        let g = b.ghost(p);
        assert_eq!(g.row, p.row + d);
        assert!(b.fits_piece(&g));
        assert!(!b.fits_piece(&g.moved(0, 1)), "ghost must be resting");
        assert_eq!(g.bottom_row(), 21, "empty field: ghost on the floor");
    }

    #[test]
    fn ghost_lands_on_the_stack() {
        let mut b = Board::new();
        b.lock(&row_cells(21, &[3, 4, 5]), 2).unwrap();
        let p = ActivePiece::spawn(Tetromino::O); // occupies cols 4,5
        let g = b.ghost(p);
        assert_eq!(g.bottom_row(), 20);
        assert!(!b.fits_piece(&g.moved(0, 1)));
    }

    #[test]
    fn spawn_blocked_detects_top_out() {
        let b = Board::new();
        assert!(!b.spawn_blocked(ActivePiece::spawn(Tetromino::T)));

        // The T's nub at (4, 0) is not part of I's spawn row (which sits in row 1).
        let mut b = Board::new();
        b.lock(&[Cell::new(4, 0)], 5).unwrap();
        assert!(b.spawn_blocked(ActivePiece::spawn(Tetromino::T)));
        assert!(!b.spawn_blocked(ActivePiece::spawn(Tetromino::I)));

        // Blocking the whole spawn row blocks every piece.
        let mut b = Board::new();
        for c in 3..=6 {
            b.lock(&[Cell::new(c, 1)], 5).unwrap();
        }
        for kind in Tetromino::ALL {
            assert!(b.spawn_blocked(ActivePiece::spawn(kind)), "{kind}");
        }
    }

    #[test]
    fn locking_above_the_field_reports_lock_out() {
        let mut b = Board::new();
        let cells = [
            Cell::new(3, -1),
            Cell::new(4, -1),
            Cell::new(3, 0),
            Cell::new(4, 0),
        ];
        assert_eq!(b.lock(&cells, 1), Err(LockError::AboveField));
        // The in-field part was still written so the final board is faithful.
        assert_eq!(b.color_at(3, 0), Some(1));
    }

    #[test]
    fn stack_height_fraction_tracks_the_stack() {
        let mut b = Board::new();
        assert_eq!(b.stack_height_fraction(), 0.0);
        b.lock(&[Cell::new(0, 21)], 1).unwrap();
        assert!(b.stack_height_fraction() < 0.1);
        for r in 2..22 {
            let _ = b.lock(&row_cells(r, &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]), 1);
        }
        assert!(
            b.stack_height_fraction() > 0.99,
            "reaching the top means 1.0"
        );
    }

    #[test]
    fn highest_and_lowest_rows() {
        let b = Board::new();
        assert_eq!(b.highest_row(), None);
        assert_eq!(b.lowest_row(), None);
        let mut b = Board::new();
        b.lock(&[Cell::new(1, 7), Cell::new(2, 15)], 1).unwrap();
        assert_eq!(b.highest_row(), Some(7));
        assert_eq!(b.lowest_row(), Some(15));
    }

    #[test]
    fn fingerprint_changes_with_every_filled_cell() {
        let a = Board::new();
        let mut hash_a = Fnv::new();
        a.fingerprint(&mut hash_a);
        let base = hash_a.finish();

        let mut b = Board::new();
        b.lock(&[Cell::new(5, 21)], 1).unwrap();
        let mut hash_b = Fnv::new();
        b.fingerprint(&mut hash_b);
        assert_ne!(base, hash_b.finish());

        // Same cells, different colour index still differs (colour is part of the visible state).
        let mut c = Board::new();
        c.lock(&[Cell::new(5, 21)], 2).unwrap();
        let mut hash_c = Fnv::new();
        c.fingerprint(&mut hash_c);
        assert_ne!(hash_b.finish(), hash_c.finish());

        // Identical boards fingerprint identically.
        let mut d = Board::new();
        d.lock(&[Cell::new(5, 21)], 1).unwrap();
        let mut hash_d = Fnv::new();
        d.fingerprint(&mut hash_d);
        assert_eq!(hash_b.finish(), hash_d.finish());
    }

    #[test]
    fn custom_sized_board_behaves() {
        let mut b = Board::with_size(3, 5);
        assert_eq!(b.buffer_rows(), 0);
        b.lock(&row_cells(4, &[0, 1, 2]), 1).unwrap();
        assert_eq!(b.full_rows(), vec![4]);
        assert!(!b.fits(&[Cell::new(3, 4)]));
    }

    #[test]
    fn ascii_prints_the_stack() {
        let mut b = Board::with_size(4, 4);
        b.lock(&[Cell::new(0, 3), Cell::new(1, 3)], 6).unwrap(); // Z
        let art = b.ascii();
        assert_eq!(art, "....\n....\n....\nZZ..\n");
    }

    #[test]
    fn rotation_and_drop_interop_smoke() {
        // Vertical I dropped into an empty well lands with its bottom on the floor.
        let b = Board::new();
        let p = ActivePiece::spawn(Tetromino::I).rotated(crate::RotateDir::Cw);
        let g = b.ghost(p);
        assert_eq!(g.bottom_row(), 21);
        assert_eq!(g.top_row(), 18);
        assert_eq!(Rotation::Cw, p.rot);
    }
}
