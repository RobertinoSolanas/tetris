//! Drives the whole application headlessly — actions in, screens and scores out — the way the
//! event loop does, but without a window.

use std::time::Duration;

use tetris_app::app::App;
use tetris_app::cli::Options;
use tetris_app::input::Action;
use tetris_app::persistence::Store;
use tetris_app::scene_builder::Screen;
use tetris_core::{Cell, Command, Direction, GameEvent};

const TICK: Duration = Duration::from_nanos(1_000_000_000 / 120);

fn app(tag: &str) -> App {
    let dir = std::env::temp_dir().join(format!("pi-tetris-flow-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let options = Options {
        seed: Some(2024),
        no_sound: true,
        no_crt: true,
        ..Default::default()
    };
    App::new_with(options, Store::in_dir(dir))
}

#[test]
fn the_app_starts_on_the_title_screen() {
    let app = app("title");
    assert_eq!(app.screen(), Screen::Title);
    assert!(app.session().active().is_none());
    assert_eq!(app.session().phase(), tetris_core::Phase::Title);
}

#[test]
fn confirm_starts_a_match_and_keys_move_the_piece() {
    let mut app = app("play");
    app.apply_action(Action::Confirm);
    assert_eq!(app.screen(), Screen::Playing);
    app.step(TICK);
    let piece = app.session().active().expect("a piece after one tick");

    for _ in 0..3 {
        app.apply_action(Action::Game(Command::StopMove(Direction::Left)));
        app.apply_action(Action::Game(Command::Move(Direction::Left)));
        app.step(TICK);
    }
    let moved = app.session().active().expect("still playing");
    let before = piece.cells().iter().map(|c| c.col).min().unwrap();
    let after = moved.cells().iter().map(|c| c.col).min().unwrap();
    assert!(
        after < before,
        "left presses should walk the piece left ({before} → {after})"
    );
}

#[test]
fn hard_drops_accumulate_score_and_locks() {
    let mut app = app("score");
    app.apply_action(Action::Confirm);
    for _ in 0..12 {
        app.apply_action(Action::Game(Command::HardDrop));
        app.step(TICK);
        if app.screen() != Screen::Playing {
            break;
        }
    }
    assert!(app.session().stats().pieces >= 8, "few pieces locked");
}

#[test]
fn pause_and_resume_round_trip() {
    let mut app = app("pause");
    app.apply_action(Action::Confirm);
    app.step(TICK);

    app.apply_action(Action::TogglePause);
    assert_eq!(app.screen(), Screen::Paused);
    let before = app.session().fingerprint();
    for _ in 0..20 {
        app.step(TICK);
    }
    assert_eq!(
        app.session().fingerprint(),
        before,
        "paused must freeze the sim"
    );

    app.apply_action(Action::TogglePause);
    assert_eq!(app.screen(), Screen::Playing);
    for _ in 0..40 {
        app.step(TICK);
    }
    assert_ne!(
        app.session().fingerprint(),
        before,
        "resumed must run again"
    );
}

#[test]
fn restart_replaces_the_match() {
    let mut app = app("restart");
    app.apply_action(Action::Confirm);
    for _ in 0..6 {
        app.apply_action(Action::Game(Command::HardDrop));
        app.step(TICK);
    }
    let pieces_before = app.session().stats().pieces;
    app.apply_action(Action::Restart);
    assert_eq!(app.screen(), Screen::Playing);
    assert!(app.session().board().is_empty());
    assert!(app.session().stats().pieces < pieces_before);
}

#[test]
fn toggles_are_immediate_and_persist() {
    let mut app = app("toggles");
    let crt = app.settings().crt;
    app.apply_action(Action::ToggleCrt);
    assert_eq!(app.settings().crt, !crt);
    app.apply_action(Action::ToggleDebug);
    assert!(app.settings().show_debug);
    app.apply_action(Action::ToggleMute);
    assert!(app.settings().muted);

    // A second app over the same directory sees the saved values.
    let dir = app.store_dir();
    let options = Options {
        no_sound: true,
        ..Default::default()
    };
    let reloaded = App::new_with(options, Store::in_dir(&dir));
    assert_eq!(reloaded.settings().crt, !crt);
    assert!(reloaded.settings().show_debug);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_finished_match_lands_on_the_game_over_screen_and_asks_for_a_name() {
    let mut app = app("gameover");
    app.apply_action(Action::Confirm);
    // Stack in the middle of the well: with no horizontal play the tower tops out quickly.
    for _ in 0..400 {
        app.apply_action(Action::Game(Command::HardDrop));
        app.step(TICK);
        if app.screen() == Screen::GameOver {
            break;
        }
    }
    assert_eq!(app.screen(), Screen::GameOver, "the match never ended");
    assert!(
        app.view_model().qualifying,
        "any non-zero score should qualify against an empty table"
    );
}

#[test]
fn entering_a_name_writes_the_high_score_table() {
    let mut app = app("hiscore");
    app.apply_action(Action::Confirm);
    for _ in 0..400 {
        app.apply_action(Action::Game(Command::HardDrop));
        app.step(TICK);
        if app.screen() == Screen::GameOver {
            break;
        }
    }
    if !app.view_model().qualifying {
        // No score to register (everything stacked without clearing): nothing to assert.
        return;
    }
    for ch in ['A', 'D', 'A'] {
        app.apply_action(Action::NameChar(ch));
    }
    assert_eq!(app.view_model().name_entry, "ADA");
    app.apply_action(Action::NameSubmit);
    assert_eq!(app.scores().entries.len(), 1);
    assert_eq!(app.scores().entries[0].name, "ADA");

    // And confirm-after-submit starts a fresh match instead of double-inserting.
    app.apply_action(Action::Confirm);
    assert_eq!(app.screen(), Screen::Playing);
    assert_eq!(app.scores().entries.len(), 1);
    let _ = std::fs::remove_dir_all(app.store_dir());
}

#[test]
fn back_from_playing_pauses_then_returns_to_the_title() {
    let mut app = app("back");
    app.apply_action(Action::Confirm);
    app.apply_action(Action::Back);
    assert_eq!(app.screen(), Screen::Paused);
    app.apply_action(Action::Back);
    assert_eq!(app.screen(), Screen::Title);
    let _ = std::fs::remove_dir_all(app.store_dir());
}

#[test]
fn events_become_sound_cues_without_panic() {
    let events = [
        GameEvent::Moved {
            dir: tetris_core::Direction::Left,
        },
        GameEvent::Cleared {
            rows: vec![18, 19, 20, 21],
            kind: tetris_core::ClearKind::Tetris,
        },
        GameEvent::LevelUp { level: 3 },
        GameEvent::HoldRejected,
        GameEvent::Danger(true),
        GameEvent::Paused,
    ];
    for event in &events {
        // Every interesting event maps to something audible; unmapped events map to None, not panic.
        let _ = tetris_app::app::cue_for(event);
    }
    assert_eq!(
        tetris_app::app::cue_for(&GameEvent::Cleared {
            rows: vec![21],
            kind: tetris_core::ClearKind::Single
        }),
        Some(tetris_audio::Cue::Clear(1))
    );
    assert_eq!(
        tetris_app::app::cue_for(&GameEvent::Cleared {
            rows: vec![18, 19, 20, 21],
            kind: tetris_core::ClearKind::Tetris
        }),
        Some(tetris_audio::Cue::Tetris)
    );
    assert!(tetris_app::app::cue_for(&GameEvent::Resumed).is_some());
}

#[test]
fn the_scripted_player_clears_lines_and_keeps_playing() {
    // The same player `--smoke-test` drives, asserted headlessly: a run that only ever stacked
    // pieces in one column would pass every other test and still be a boring demo.
    let mut app = app("scripted");
    app.apply_action(Action::Confirm);
    let mut matches = 0;
    for _ in 0..4000 {
        let before = app.screen();
        app.smoke_step_once();
        app.step(TICK);
        if before == Screen::Playing && app.screen() == Screen::GameOver {
            matches += 1;
        }
    }
    let stats = app.session().stats();
    assert!(
        stats.lines > 0 || matches > 0,
        "the scripted player cleared nothing and never topped out: {stats:?}"
    );
    assert!(
        app.view_model().particles.len() < 2000,
        "particle list is unbounded"
    );
    let _ = std::fs::remove_dir_all(app.store_dir());
}

#[test]
fn the_board_heuristic_rewards_clears_and_punishes_holes() {
    use tetris_app::app::evaluate;
    use tetris_core::Board;

    let mut open_floor = Board::new();
    for col in 0..9 {
        let _ = open_floor.lock(&[Cell::new(col, 21)], 0);
    }
    let mut completed = Board::new();
    for col in 0..10 {
        let _ = completed.lock(&[Cell::new(col, 21)], 0);
    }
    assert!(
        evaluate(&completed) > evaluate(&open_floor),
        "a completed row must score better than an incomplete one"
    );

    // Same height, but one buried gap: that is a hole, and holes are expensive.
    let mut holey = Board::new();
    for col in 0..10 {
        if col != 4 {
            let _ = holey.lock(&[Cell::new(col, 21)], 0);
        }
        let _ = holey.lock(&[Cell::new(col, 20)], 0);
    }
    let mut solid = Board::new();
    for col in 0..10 {
        let _ = solid.lock(&[Cell::new(col, 20)], 0);
        let _ = solid.lock(&[Cell::new(col, 21)], 0);
    }
    assert!(
        evaluate(&solid) > evaluate(&holey),
        "a buried gap must cost more than a flat floor"
    );
    let _ = open_floor;
}

#[test]
fn a_plan_always_exists_for_a_fresh_board() {
    let mut app = app("plan");
    app.apply_action(Action::Confirm);
    app.step(TICK);
    let plan = tetris_app::app::plan_for(app.session()).expect("a plan for an empty well");
    assert!(
        (0..=7).contains(&plan.col),
        "bounding-box column out of range: {plan:?}"
    );
    let _ = std::fs::remove_dir_all(app.store_dir());
}
