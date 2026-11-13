//! Tests written against `tetris-core` *from the outside*, the way the shell crate sees it.
//!
//! These exist to catch "it only works from inside the module" mistakes and to pin down the
//! guarantees the app layer relies on: a public way to drive a match, a replayable log, and a
//! session that can be moved into a render loop.

use std::time::Duration;

use tetris_core::{
    ActivePiece, Board, Cell, Command, Direction, EndReason, GameEvent, Phase, RotateDir, Rotation,
    ScoreReason, Session, SessionConfig, Tetromino, BOARD_ROWS,
};

const TICK: Duration = Duration::from_nanos(1_000_000_000 / 120);

#[test]
fn session_is_send_so_the_shell_can_own_it() {
    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}
    assert_send::<Session>();
    assert_sync::<Session>();
    assert_send::<Board>();
}

/// Walk the active piece sideways one column per command pair (press, release).
fn slide_towards(s: &mut Session, target_left_col: i32) {
    for _ in 0..12 {
        let Some(piece) = s.active() else { return };
        let left = piece.cells().iter().map(|c| c.col).min().unwrap_or(0);
        if left == target_left_col {
            return;
        }
        let dir = if left < target_left_col {
            Direction::Right
        } else {
            Direction::Left
        };
        s.apply(Command::StopMove(dir));
        s.apply(Command::Move(dir));
        if s.active()
            .map(|p| p.cells().iter().map(|c| c.col).min().unwrap_or(0))
            == Some(left)
        {
            return; // blocked by a wall
        }
    }
}

/// Column whose stack is highest up the well, i.e. where the next piece should *not* go; the
/// returned value is a legal bounding-box column for the next piece.
fn emptiest_box_column(s: &Session) -> i32 {
    let mut heights = [0i32; 10];
    for (col, height) in heights.iter_mut().enumerate() {
        for row in 0..BOARD_ROWS as i32 {
            if s.board().color_at(col as i32, row).is_some() {
                *height = BOARD_ROWS as i32 - row;
                break;
            }
        }
    }
    let mut best = 0i32;
    let mut best_h = i32::MAX;
    for box_col in 0..=7 {
        let span = heights[box_col as usize] + heights[(box_col + 1) as usize];
        if span < best_h {
            best_h = span;
            best = box_col;
        }
    }
    best
}

#[test]
fn a_greedy_bot_locks_many_pieces_through_the_public_api() {
    // Always aim for the emptiest pair of columns: enough strategy to keep the well playable for a
    // long time, simple enough to stay deterministic and readable.
    let mut s = Session::with_seed(4242);
    s.start(4242);
    let mut cleared = 0;
    for i in 0..6000u64 {
        if s.phase() != Phase::Playing {
            break;
        }
        s.advance_to(i + 1, TICK);
        if s.active().is_none() {
            s.advance_to(i + 40, TICK);
            continue;
        }
        let target = emptiest_box_column(&s);
        slide_towards(&mut s, target);
        s.apply(Command::HardDrop);
        for event in s.drain_events() {
            if let GameEvent::Cleared { rows, .. } = event {
                cleared += rows.len();
            }
        }
    }
    // The well holds 220 cells; placing 30+ pieces without clearing much means the command
    // vocabulary really is enough to play the game from outside the crate.
    assert!(
        s.stats().pieces >= 30,
        "the bot only placed {} pieces (cleared {cleared})",
        s.stats().pieces
    );
}

#[test]
fn a_recorded_run_can_be_replayed_through_the_public_api() {
    let mut s = Session::with_seed(31337);
    s.start(31337);
    let mut target = 5;
    for i in 0..250 {
        s.advance_to(i + 1, TICK);
        if s.active().is_none() {
            s.advance_to(i + 20, TICK);
        }
        match i % 5 {
            0 => s.apply(Command::Rotate(RotateDir::Cw)),
            1 => s.apply(Command::Rotate(RotateDir::Ccw)),
            2 => s.apply(Command::Rotate(RotateDir::Flip)),
            3 => s.apply(Command::Hold),
            _ => {}
        }
        slide_towards(&mut s, target);
        target = (target + 3) % 8;
        s.apply(Command::HardDrop);
        let _ = s.drain_events();
    }

    let log: Vec<(u64, Command)> = s.input_log().to_vec();
    let expected = s.fingerprint();
    let end = s.tick_index();

    let mut replay = Session::replay(31337, SessionConfig::default(), &log);
    replay.advance_to(end, TICK);
    assert_eq!(expected, replay.fingerprint());
    assert_eq!(replay.stats().lines, s.stats().lines);
    assert_eq!(replay.stats().score, s.stats().score);
}

#[test]
fn a_tetris_cleared_via_the_public_api_reports_every_event_in_order() {
    let mut s = Session::with_seed(2024);
    s.start(2024);
    s.advance_to(1, TICK);

    for row in 18..22 {
        for col in 0..9 {
            let _ = s.board_mut().lock(&[Cell::new(col, row)], 0);
        }
    }
    let mut piece = ActivePiece::spawn(Tetromino::I);
    piece.rot = Rotation::Cw;
    piece.col = 7; // the vertical I occupies box column + 2 → column 9
    piece.row = 0;
    s.set_active(piece);

    let _ = s.drain_events(); // discard the opening spawn events
    s.apply(Command::HardDrop);
    let mut events = s.drain_events();
    // The next piece only appears once the line-clear delay has elapsed.
    s.advance_to(40, TICK);
    events.extend(s.drain_events());

    let kinds: Vec<&str> = events
        .iter()
        .map(|e| match e {
            GameEvent::HardDropped { .. } => "hard",
            GameEvent::Locked { .. } => "lock",
            GameEvent::Cleared { .. } => "clear",
            GameEvent::Scored { .. } => "score",
            GameEvent::PieceSpawned { .. } => "spawn",
            _ => "other",
        })
        .collect();

    let position = |needle: &str| kinds.iter().position(|k| *k == needle);
    let hard = position("hard").expect("hard drop event");
    let lock = position("lock").expect("lock event");
    let clear = position("clear").expect("clear event");
    // The clear's own payout, not the earlier hard-drop points.
    let clear_score = events
        .iter()
        .position(|e| {
            matches!(
                e,
                GameEvent::Scored {
                    reason: ScoreReason::Clear { .. },
                    ..
                }
            )
        })
        .expect("a clear should pay");
    assert!(
        hard < lock && lock < clear && clear < clear_score,
        "events out of order: {kinds:?}"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, GameEvent::PieceSpawned { .. })),
        "a new piece should be dealt; kinds {kinds:?} raw {events:?}"
    );
}

#[test]
fn quitting_mid_match_produces_a_quit_result() {
    let mut s = Session::with_seed(7);
    s.start(7);
    s.advance_to(10, TICK);
    s.quit();
    assert_eq!(s.phase(), Phase::GameOver);
    let events = s.drain_events();
    assert!(matches!(
        events.last(),
        Some(GameEvent::GameOver {
            reason: EndReason::Quit,
            ..
        })
    ));
}

#[test]
fn the_board_and_the_session_agree_on_the_ghost() {
    let mut s = Session::with_seed(11);
    s.start(11);
    s.advance_to(1, TICK);
    let piece = s.active().expect("a piece");
    let ghost = s.ghost().expect("a ghost");
    assert_eq!(ghost.kind, piece.kind);
    assert_eq!(ghost.col, piece.col);
    assert!(ghost.row >= piece.row);
    assert!(
        !s.board().fits_piece(&ghost.moved(0, 1)),
        "the ghost must be resting on something"
    );
}

#[test]
fn every_piece_can_be_placed_somewhere_on_an_empty_well() {
    let board = Board::new();
    for kind in Tetromino::ALL {
        for rot in Rotation::ALL {
            for col in 0..8 {
                let piece = ActivePiece {
                    kind,
                    rot,
                    col,
                    row: 0,
                };
                let _ = board.fits_piece(&piece);
            }
        }
        assert!(board.fits_piece(&ActivePiece::spawn(kind)), "{kind} spawn");
    }
}
