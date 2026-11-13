//! Things that happened in the domain — the only channel from gameplay to the outside world.
//!
//! The session *emits*; the view model, sound and score screens *consume*. Nothing downstream ever
//! mutates gameplay state.

use crate::actions::{Direction, RotateDir};
use crate::piece::Tetromino;
use crate::scoring::{ClearKind, ScoreReason, SpinKind};
use crate::session::Stats;

/// An immutable fact produced by [`crate::Session`].
#[derive(Clone, Debug, PartialEq)]
pub enum GameEvent {
    /// A new match began (the title screen started, or a restart happened).
    Started {
        seed: u64,
        level: u32,
    },
    /// A piece appeared at the top; `queue` is the upcoming preview.
    PieceSpawned {
        piece: Tetromino,
        queue: Vec<Tetromino>,
        hold: Option<Tetromino>,
    },
    /// A queued/hold piece was placed directly onto the board after `Hold`.
    HoldSwapped {
        piece: Tetromino,
        hold: Option<Tetromino>,
    },
    /// A piece was rejected by the hold rule (already used for this piece).
    HoldRejected,
    /// The piece moved one column.
    Moved {
        dir: Direction,
    },
    /// A move was blocked by the stack or a wall.
    MoveBlocked {
        dir: Direction,
    },
    /// A rotation succeeded; `kick` is the offset applied (if any).
    Rotated {
        dir: RotateDir,
        kick: Option<(i32, i32)>,
    },
    /// Every rotation offset collided.
    RotateBlocked {
        dir: RotateDir,
    },
    /// The piece fell one row (gravity or soft drop). `soft` distinguishes the scoring.
    Dropped {
        rows: u32,
        soft: bool,
    },
    /// Hard drop landed: `rows` is the distance travelled.
    HardDropped {
        rows: u32,
    },
    /// The active piece became part of the stack.
    Locked {
        piece: Tetromino,
        cells: Vec<crate::piece::Cell>,
        spin: SpinKind,
    },
    /// Rows were removed. Emitted right after [`GameEvent::Locked`] of the same lock.
    Cleared {
        rows: Vec<u32>,
        kind: ClearKind,
    },
    /// Score changed; `reason` is what paid for it.
    Scored {
        reason: ScoreReason,
        amount: u64,
        total: u64,
    },
    /// Combo counter advanced (`count` is 1-based: the second consecutive clearing piece is 1).
    Combo {
        count: u32,
    },
    /// Back-to-back chain extended (`chain` is 1-based on the second hard clear).
    BackToBack {
        chain: u32,
    },
    /// A new line cleared into the level goal.
    LevelUp {
        level: u32,
    },
    /// A warning that the stack reached the danger zone (emitted on entering the zone).
    Danger(bool),
    Paused,
    Resumed,
    /// The match ended; `stats` is final.
    GameOver {
        stats: Stats,
        reason: EndReason,
    },
    /// The board was abandoned and we are back at the title.
    ReturnedToTitle,
}

/// Why the match ended.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum EndReason {
    /// A new piece could not be placed (overlap at spawn).
    SpawnBlocked,
    /// A piece locked entirely inside the buffer rows.
    LockedOut,
    /// The player quit.
    Quit,
}
