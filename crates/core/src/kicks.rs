//! Wall-kick tables.
//!
//! Rotation in Super-Rotation-System (SRS) means "try these offsets in order until the piece fits".
//! Offsets are given as `(col, row)` deltas where **row grows downward**; the published SRS tables
//! use `y` growing upward, so every `y` below has already been negated.
//!
//! * [`JLSTZ`] — the shared table for J, L, S, T, Z.
//! * [`I`]     — the I piece needs its own table (4×4 box).
//! * [`O`]     — the O piece has a single orientation, so its only "kick" is the no-op.
//! * [`FLIP_180`] — SRS defines no 180° rotation, so we ship our own deliberately small table
//!   (no-op, then up, then sideways, then down). It is *our* rule, documented as such, and pinned by
//!   tests rather than claimed to be normative.

use crate::piece::{Rotation, Tetromino};

/// A candidate offset applied to a piece when rotating.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Kick {
    pub dc: i32,
    pub dr: i32,
}

impl Kick {
    pub const fn new(dc: i32, dr: i32) -> Self {
        Self { dc, dr }
    }

    pub const fn is_none(self) -> bool {
        self.dc == 0 && self.dr == 0
    }
}

/// `(from, to)` pair, both as [`Rotation::index`].
type Pair = (usize, usize);

/// One rotation transition and its candidate offsets.
struct Entry {
    pair: Pair,
    kicks: &'static [Kick],
}

macro_rules! kicks {
    ($($dc:expr, $dr:expr);* $(;)?) => {
        &[$(Kick::new($dc, $dr)),*]
    };
}

/// J, L, S, T, Z — SRS wall kicks (`y` already flipped to "down is positive").
const JLSTZ: &[Entry] = &[
    // clockwise
    Entry {
        pair: (0, 1),
        kicks: kicks!(0,0; -1,0; -1,-1; 0,2; -1,2),
    },
    Entry {
        pair: (1, 2),
        kicks: kicks!(0,0; 1,0; 1,1; 0,-2; 1,-2),
    },
    Entry {
        pair: (2, 3),
        kicks: kicks!(0,0; 1,0; 1,-1; 0,2; 1,2),
    },
    Entry {
        pair: (3, 0),
        kicks: kicks!(0,0; -1,0; -1,1; 0,-2; -1,-2),
    },
    // counter-clockwise
    Entry {
        pair: (1, 0),
        kicks: kicks!(0,0; 1,0; 1,1; 0,-2; 1,-2),
    },
    Entry {
        pair: (2, 1),
        kicks: kicks!(0,0; -1,0; -1,-1; 0,2; -1,2),
    },
    Entry {
        pair: (3, 2),
        kicks: kicks!(0,0; -1,0; -1,1; 0,-2; -1,-2),
    },
    Entry {
        pair: (0, 3),
        kicks: kicks!(0,0; 1,0; 1,-1; 0,2; 1,2),
    },
];

/// I — SRS wall kicks (`y` already flipped).
const I: &[Entry] = &[
    Entry {
        pair: (0, 1),
        kicks: kicks!(0,0; -2,0; 1,0; -2,1; 1,-2),
    },
    Entry {
        pair: (1, 2),
        kicks: kicks!(0,0; -1,0; 2,0; -1,-2; 2,1),
    },
    Entry {
        pair: (2, 3),
        kicks: kicks!(0,0; 2,0; -1,0; 2,-1; -1,2),
    },
    Entry {
        pair: (3, 0),
        kicks: kicks!(0,0; 1,0; -2,0; 1,2; -2,-1),
    },
    Entry {
        pair: (1, 0),
        kicks: kicks!(0,0; 2,0; -1,0; 2,-1; -1,2),
    },
    Entry {
        pair: (2, 1),
        kicks: kicks!(0,0; 1,0; -2,0; 1,2; -2,-1),
    },
    Entry {
        pair: (3, 2),
        kicks: kicks!(0,0; -2,0; 1,0; -2,1; 1,-2),
    },
    Entry {
        pair: (0, 3),
        kicks: kicks!(0,0; -1,0; 2,0; -1,-2; 2,1),
    },
];

/// Our 180° kicks (SRS has none). Symmetric by construction: the table for `a→b` reversed is the
/// table for `b→a`, asserted in `flip_tables_are_symmetric`.
const FLIP_UP: &[Kick] = kicks!(0,0; 0,-1; -1,0; 1,0; 0,1);
/// Exactly the negation of [`FLIP_UP`], asserted by `flip_tables_are_symmetric`.
const FLIP_DOWN: &[Kick] = kicks!(0,0; 0,1; 1,0; -1,0; 0,-1);

const FLIP_180: &[Entry] = &[
    Entry {
        pair: (0, 2),
        kicks: FLIP_UP,
    },
    Entry {
        pair: (2, 0),
        kicks: FLIP_DOWN,
    },
    Entry {
        pair: (1, 3),
        kicks: FLIP_UP,
    },
    Entry {
        pair: (3, 1),
        kicks: FLIP_DOWN,
    },
];

/// Every table lookup falls back to this: a rotation always has at least the no-op.
const NO_KICK: &[Kick] = &[Kick::new(0, 0)];

fn lookup(table: &[Entry], from: Rotation, to: Rotation) -> &'static [Kick] {
    let pair = (from.index(), to.index());
    for e in table {
        if e.pair == pair {
            return e.kicks;
        }
    }
    &[]
}

/// Candidate offsets for rotating `kind` from `from` to `to`, in application order.
///
/// Always non-empty for legal transitions, and always starts with the no-op.
pub fn kick_attempts(kind: Tetromino, from: Rotation, to: Rotation) -> &'static [Kick] {
    if from.index() == to.index() {
        return NO_KICK;
    }
    if kind == Tetromino::O {
        // Single orientation: rotation is the identity, so reporting a kick would be a lie.
        return NO_KICK;
    }
    let d = (to.index() + 4 - from.index()) % 4;
    let table = if kind == Tetromino::I { I } else { JLSTZ };
    let direct = lookup(table, from, to);
    if !direct.is_empty() {
        return direct;
    }
    if d == 2 {
        return lookup(FLIP_180, from, to);
    }
    // Not reachable through the public API (RotateDir only produces ±1 or ±2).
    NO_KICK
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_transition_starts_with_the_noop_and_is_non_empty() {
        for kind in Tetromino::ALL {
            for from in Rotation::ALL {
                for to in Rotation::ALL {
                    let k = kick_attempts(kind, from, to);
                    assert!(!k.is_empty(), "{kind} {from:?}->{to:?} has no attempts");
                    assert!(
                        k[0].is_none(),
                        "{kind} {from:?}->{to:?} first attempt is not the no-op"
                    );
                }
            }
        }
    }

    #[test]
    fn quarter_turns_have_five_attempts() {
        for kind in [Tetromino::J, Tetromino::T, Tetromino::I] {
            for from in Rotation::ALL {
                for dir in [Rotation::Cw, Rotation::Ccw] {
                    let to = if dir == Rotation::Cw {
                        from.cw()
                    } else {
                        from.ccw()
                    };
                    if kind == Tetromino::O {
                        continue;
                    }
                    assert_eq!(
                        kick_attempts(kind, from, to).len(),
                        5,
                        "{kind} {from:?}->{to:?}"
                    );
                }
            }
        }
    }

    /// Transcribed SRS reference: J spawn→R and I spawn→R.
    #[test]
    fn matches_reference_tables() {
        assert_eq!(
            kick_attempts(Tetromino::T, Rotation::Spawn, Rotation::Cw),
            &[
                Kick::new(0, 0),
                Kick::new(-1, 0),
                Kick::new(-1, -1),
                Kick::new(0, 2),
                Kick::new(-1, 2)
            ]
        );
        assert_eq!(
            kick_attempts(Tetromino::I, Rotation::Spawn, Rotation::Cw),
            &[
                Kick::new(0, 0),
                Kick::new(-2, 0),
                Kick::new(1, 0),
                Kick::new(-2, 1),
                Kick::new(1, -2)
            ]
        );
        // I R->2, published as (0,0) (-1,0) (+2,0) (-1,+2) (+2,-1) in y-up.
        assert_eq!(
            kick_attempts(Tetromino::I, Rotation::Cw, Rotation::Flip),
            &[
                Kick::new(0, 0),
                Kick::new(-1, 0),
                Kick::new(2, 0),
                Kick::new(-1, -2),
                Kick::new(2, 1)
            ]
        );
    }

    #[test]
    fn opposite_quarter_turns_are_negated() {
        // SRS property: kicks(a->b) == -kicks(b->a) component-wise.
        for kind in [
            Tetromino::J,
            Tetromino::L,
            Tetromino::S,
            Tetromino::T,
            Tetromino::Z,
            Tetromino::I,
        ] {
            for from in Rotation::ALL {
                let cw = from.cw();
                let forward = kick_attempts(kind, from, cw);
                let backward = kick_attempts(kind, cw, from);
                assert_eq!(forward.len(), backward.len());
                for (f, b) in forward.iter().zip(backward) {
                    assert_eq!(
                        (f.dc, f.dr),
                        (-b.dc, -b.dr),
                        "{kind} {from:?}<->{cw:?} asymmetric"
                    );
                }
            }
        }
    }

    #[test]
    fn flip_tables_are_symmetric() {
        for (a, b) in [
            (Rotation::Spawn, Rotation::Flip),
            (Rotation::Cw, Rotation::Ccw),
        ] {
            let fwd = kick_attempts(Tetromino::T, a, b);
            let back = kick_attempts(Tetromino::T, b, a);
            assert_eq!(fwd.len(), back.len());
            for (f, r) in fwd.iter().zip(back) {
                assert_eq!((f.dc, f.dr), (-r.dc, -r.dr));
            }
        }
    }

    #[test]
    fn o_piece_only_ever_gets_the_noop() {
        for from in Rotation::ALL {
            for to in Rotation::ALL {
                for k in kick_attempts(Tetromino::O, from, to) {
                    assert!(k.is_none(), "O kicked with {k:?}");
                }
            }
        }
    }

    #[test]
    fn unknown_transition_falls_back_to_noop() {
        // A same-state "rotation" is legal and must be the no-op only.
        assert_eq!(
            kick_attempts(Tetromino::I, Rotation::Cw, Rotation::Cw),
            &[Kick::new(0, 0)]
        );
    }
}
