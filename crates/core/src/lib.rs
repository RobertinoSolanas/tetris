//! `tetris-core` — the gameplay domain of pi-tetris.
//!
//! Six bounded contexts live here (see `docs/domain/bounded-contexts.md`):
//!
//! | Context            | Module       | Responsibility                                    |
//! |--------------------|--------------|---------------------------------------------------|
//! | Tetromino Geometry | [`piece`]    | the seven shapes and their rotation states         |
//! |                    | [`kicks`]    | SRS wall-kick tables                               |
//! | Playfield          | [`board`]    | the well: collision, locking, line clear, ghost    |
//! | Piece Supply       | [`supply`]   | 7-bag randomisation, preview queue                 |
//! |                    | [`rng`]      | the (seeded, deterministic) entropy port           |
//! | Scoring & Progression | [`scoring`] | values, combo, back-to-back, level curve, gravity |
//! | Handling & Timing  | [`handling`] | DAS/ARR, soft drop, lock delay with reset budget   |
//! | Match Lifecycle    | [`session`]  | aggregate root: `Command` in, [`GameEvent`] out    |
//!
//! Design rules that keep this crate honest:
//!
//! * No dependencies at all — no clock, no files, no global RNG, no threads.
//! * Time only enters through [`Session::tick`](session::Session::tick) as a [`std::time::Duration`].
//! * Randomness only enters through a seeded [`Rng`](rng::Rng).
//! * Nothing here knows what a pixel, a colour byte or a sound is; [`palette`] is the single
//!   agreed-upon exception (an RGB triple is a property of a tetromino, a renderer is not).

pub mod actions;
pub mod board;
pub mod events;
pub mod handling;
pub mod kicks;
pub mod palette;
pub mod piece;
pub mod rng;
pub mod scoring;
pub mod session;
pub mod supply;

pub use actions::{Command, Direction, RotateDir};
pub use board::{Board, BOARD_COLS, BOARD_ROWS, BUFFER_ROWS, VISIBLE_ROWS};
pub use events::{EndReason, GameEvent};
pub use handling::{
    AutoShift, DropTimer, HandlingConfig, InputBuffer, LockTimer, ShiftPhase, Touch,
};
pub use palette::COLORS;
pub use piece::{ActivePiece, Cell, Rotation, Tetromino};
pub use rng::{Rng, XorShift64};
pub use scoring::{
    ClearKind, ClearOutcome, Progression, ScoreBreakdown, ScoreReason, ScoreState, ScoreTable,
    SpinKind,
};
pub use session::{Phase, Session, SessionConfig, Stats};
pub use supply::{PieceSupply, SevenBag};

/// Library version, mirrored in the window title.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
