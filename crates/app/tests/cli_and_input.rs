//! Command-line parsing and key mapping — the two places where the outside world talks to the game.

use winit::event::ElementState;
use winit::keyboard::{Key, NamedKey};

use tetris_app::cli::Options;
use tetris_app::input::{translate, Action};
use tetris_app::scene_builder::Screen;
use tetris_core::{Command, Direction, RotateDir};

fn opts(args: &[&str]) -> Result<Options, String> {
    Options::parse(std::iter::once("tetris".to_string()).chain(args.iter().map(|s| s.to_string())))
}

// ------------------------------------------------------------------------------- CLI

#[test]
fn defaults_open_a_window_at_a_sensible_size() {
    let options = opts(&[]).unwrap();
    assert_eq!((options.width, options.height), (1280, 800));
    assert!(!options.smoke_test);
    assert!(!options.no_crt);
    assert!(!options.no_sound);
    assert!(options.snapshot.is_none());
    assert!(options.seed.is_none());
    assert!(!options.help);
}

#[test]
fn every_flag_is_understood() {
    let options = opts(&[
        "--seed",
        "4242",
        "--window",
        "1024x768",
        "--no-crt",
        "--no-sound",
        "--smoke-test",
        "--frames",
        "30",
    ])
    .unwrap();
    assert_eq!(options.seed, Some(4242));
    assert_eq!((options.width, options.height), (1024, 768));
    assert!(options.no_crt && options.no_sound && options.smoke_test);
    assert_eq!(options.frames, 30);
}

#[test]
fn help_flag_short_circuits() {
    assert!(opts(&["--help"]).unwrap().help);
    assert!(opts(&["-h"]).unwrap().help);
}

#[test]
fn window_accepts_the_equals_form_and_uppercase_x() {
    assert_eq!(
        opts(&["--window=800X600"]).unwrap().height,
        600,
        "uppercase X and `=` syntax should both work"
    );
}

#[test]
fn unknown_options_are_rejected_with_guidance() {
    let err = opts(&["--turbo"]).unwrap_err();
    assert!(err.contains("unknown option"), "{err}");
    assert!(err.contains("--help"), "{err}");
}

#[test]
fn missing_values_are_reported_not_panicking() {
    for args in [
        vec!["--seed"],
        vec!["--frames"],
        vec!["--window"],
        vec!["--snapshot"],
        vec!["--dump-audio"],
    ] {
        let err = opts(&args).unwrap_err();
        assert!(err.contains("needs a value"), "{args:?} → {err}");
    }
}

#[test]
fn malformed_values_are_reported() {
    assert!(opts(&["--seed", "abc"]).is_err());
    assert!(
        opts(&["--seed", "-5"]).is_err(),
        "a u64 seed cannot be negative"
    );
    assert!(opts(&["--frames", "many"]).is_err());
    assert!(opts(&["--window", "1024"]).is_err());
    assert!(opts(&["--window", "axb"]).is_err());
}

#[test]
fn a_window_too_small_to_draw_in_is_refused() {
    assert!(opts(&["--window", "100x100"]).is_err());
    assert!(
        opts(&["--window", "320x240"]).is_ok(),
        "the minimum is allowed"
    );
}

// ------------------------------------------------------------------------------ keys

fn actions(logical: Key, state: ElementState, screen: Screen) -> Vec<Action> {
    translate(&logical, state, false, screen)
}

fn pressed(logical: Key, screen: Screen) -> Vec<Action> {
    actions(logical, ElementState::Pressed, screen)
}

fn letter(c: char, screen: Screen) -> Vec<Action> {
    pressed(Key::Character(c.to_string().into()), screen)
}

#[test]
fn arrows_and_letters_become_movement_commands() {
    assert_eq!(
        pressed(Key::Named(NamedKey::ArrowLeft), Screen::Playing),
        vec![Action::Game(Command::Move(Direction::Left))]
    );
    assert_eq!(
        pressed(Key::Named(NamedKey::ArrowRight), Screen::Playing),
        vec![Action::Game(Command::Move(Direction::Right))]
    );
    // H and L move where the arrows move, for left-hand players.
    assert_eq!(
        letter('h', Screen::Playing),
        vec![Action::Game(Command::Move(Direction::Left))]
    );
    assert_eq!(
        letter('L', Screen::Playing),
        vec![Action::Game(Command::Move(Direction::Right))]
    );
}

#[test]
fn releasing_a_direction_key_stops_the_auto_shift() {
    let released = actions(
        Key::Named(NamedKey::ArrowLeft),
        ElementState::Released,
        Screen::Playing,
    );
    assert_eq!(
        released,
        vec![Action::Game(Command::StopMove(Direction::Left))]
    );
}

#[test]
fn rotation_keys_map_to_the_three_directions() {
    assert_eq!(
        letter('x', Screen::Playing),
        vec![Action::Game(Command::Rotate(RotateDir::Cw))]
    );
    assert_eq!(
        letter('z', Screen::Playing),
        vec![Action::Game(Command::Rotate(RotateDir::Ccw))]
    );
    assert_eq!(
        letter('a', Screen::Playing),
        vec![Action::Game(Command::Rotate(RotateDir::Flip))]
    );
    assert_eq!(
        pressed(Key::Named(NamedKey::ArrowUp), Screen::Playing),
        vec![Action::Game(Command::Rotate(RotateDir::Cw))]
    );
}

#[test]
fn space_drops_and_down_toggles_soft_drop() {
    assert_eq!(
        pressed(Key::Named(NamedKey::Space), Screen::Playing),
        vec![Action::Game(Command::HardDrop)]
    );
    assert_eq!(
        pressed(Key::Named(NamedKey::ArrowDown), Screen::Playing),
        vec![Action::Game(Command::SoftDrop(true))]
    );
    assert_eq!(
        actions(
            Key::Named(NamedKey::ArrowDown),
            ElementState::Released,
            Screen::Playing
        ),
        vec![Action::Game(Command::SoftDrop(false))]
    );
}

#[test]
fn hold_is_c_or_shift_and_rejects_are_reported_by_the_domain() {
    assert_eq!(
        letter('c', Screen::Playing),
        vec![Action::Game(Command::Hold)]
    );
    assert_eq!(
        letter('C', Screen::Playing),
        vec![Action::Game(Command::Hold)],
        "shifted capitals must work too"
    );
}

#[test]
fn system_toggles_work_on_every_screen() {
    for screen in [
        Screen::Title,
        Screen::Playing,
        Screen::Paused,
        Screen::GameOver,
    ] {
        assert!(
            pressed(Key::Named(NamedKey::F2), screen).contains(&Action::ToggleCrt),
            "{screen:?} F2"
        );
        assert!(
            pressed(Key::Named(NamedKey::F3), screen).contains(&Action::ToggleDebug),
            "{screen:?} F3"
        );
        assert!(
            letter('m', screen).contains(&Action::ToggleMute),
            "{screen:?} M"
        );
    }
}

#[test]
fn escape_and_enter_mean_different_things_per_screen() {
    // Title: Enter starts, Q quits, Escape does nothing harmful.
    let enter = pressed(Key::Named(NamedKey::Enter), Screen::Title);
    assert!(enter.contains(&Action::Confirm), "{enter:?}");
    let q = letter('q', Screen::Title);
    assert!(q.contains(&Action::Quit), "{q:?}");

    // Playing: Escape pauses rather than quitting the app.
    let esc = pressed(Key::Named(NamedKey::Escape), Screen::Playing);
    assert_eq!(esc, vec![Action::Back]);

    // Game over: Enter confirms the name / restarts, letters feed the name field.
    let over_enter = pressed(Key::Named(NamedKey::Enter), Screen::GameOver);
    assert!(over_enter.contains(&Action::Confirm), "{over_enter:?}");
    let typed = letter('j', Screen::GameOver);
    assert!(typed.contains(&Action::NameChar('j')), "{typed:?}");
    let back = pressed(Key::Named(NamedKey::Backspace), Screen::GameOver);
    assert!(back.contains(&Action::NameBackspace), "{back:?}");
}

#[test]
fn autorepeat_only_matters_for_movement() {
    // A held arrow must not re-trigger the DAS press, but the release still matters.
    let actions = translate(
        &Key::Named(NamedKey::ArrowLeft),
        ElementState::Pressed,
        true,
        Screen::Playing,
    );
    // Movement keys are the one case where a repeat is filtered by the domain, so it is fine to
    // forward it; the state machine ignores a re-press of a held direction.
    assert_eq!(actions, vec![Action::Game(Command::Move(Direction::Left))]);

    assert_eq!(
        translate(
            &Key::Character("x".into()),
            ElementState::Pressed,
            true,
            Screen::Playing
        ),
        vec![Action::Ignored],
        "auto-repeated rotation would spam the rotate sound"
    );
}

#[test]
fn unknown_keys_are_ignored_not_misread() {
    assert_eq!(
        pressed(Key::Named(NamedKey::F7), Screen::Playing),
        vec![Action::Ignored]
    );
}

#[test]
fn a_release_of_an_unmapped_key_is_ignored() {
    assert_eq!(
        actions(
            Key::Named(NamedKey::F7),
            ElementState::Released,
            Screen::Playing
        ),
        vec![Action::Ignored]
    );
}
