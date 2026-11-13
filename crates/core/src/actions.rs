//! Player intent, before the handling rules interpret it.
//!
//! A `Command` is *what the player asked for*, not *what happened*: `Command::Move(Left)` may or may
//! not move the piece, depending on collisions and on the auto-shift state machine in
//! [`crate::handling`]. What happened is reported as a [`crate::GameEvent`].

/// Horizontal direction.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
    Left,
    Right,
}

impl Direction {
    pub const ALL: [Direction; 2] = [Direction::Left, Direction::Right];

    pub const fn delta_col(self) -> i32 {
        match self {
            Direction::Left => -1,
            Direction::Right => 1,
        }
    }

    pub const fn opposite(self) -> Self {
        match self {
            Direction::Left => Direction::Right,
            Direction::Right => Direction::Left,
        }
    }
}

/// Rotation direction.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum RotateDir {
    Cw,
    Ccw,
    Flip,
}

/// Everything the shell can ask the session to do.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// A direction key went down: step once, then start the DAS/ARR cycle.
    Move(Direction),
    /// A direction key came up: stop auto-repeating that direction.
    StopMove(Direction),
    Rotate(RotateDir),
    /// Soft drop held down (`true`) or released (`false`).
    SoftDrop(bool),
    /// Instant drop to the ghost position and lock.
    HardDrop,
    /// Swap the active piece with the hold slot (once per piece).
    Hold,
    Pause,
    Resume,
    /// Start a brand new match. `seed` makes the run reproducible.
    Start {
        seed: u64,
    },
    /// Give up and return to the title screen.
    QuitToTitle,
}
