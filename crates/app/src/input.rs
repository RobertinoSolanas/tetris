//! Keyboard → intent.
//!
//! The mapping table is data so it can be printed by `--help`, asserted in tests, and kept in sync
//! with the on-screen control list.

use tetris_core::{Command, Direction, RotateDir};
use winit::event::ElementState;
use winit::keyboard::{Key, NamedKey};

use crate::scene_builder::Screen;

/// Anything the app can do in response to a key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// A gameplay command.
    Game(Command),
    /// Enter / start.
    Confirm,
    /// Escape: pause, or back to the title.
    Back,
    Restart,
    Quit,
    TogglePause,
    ToggleCrt,
    ToggleMute,
    ToggleDebug,
    /// High-score name entry.
    NameChar(char),
    NameBackspace,
    NameSubmit,
    /// Nothing this app does.
    Ignored,
}

/// Translate one key transition for the current screen.
///
/// The arguments are the plain pieces of a `winit::event::KeyEvent` rather than the struct itself:
/// its `platform_specific` field differs between the X11 and Wayland backends, which would make
/// this mapping untestable on whichever backend the test binary was not built for.
pub fn translate(
    logical_key: &Key,
    state: ElementState,
    repeat: bool,
    screen: Screen,
) -> Vec<Action> {
    let pressed = matches!(state, ElementState::Pressed);
    let key = match logical_key {
        Key::Named(named) => Some(*named),
        Key::Character(_) => None,
        _ => None,
    };

    let mut out = Vec::new();

    // ---- global keys, every screen
    match key {
        Some(NamedKey::Escape) if pressed => out.push(Action::Back),
        Some(NamedKey::F2) if pressed => out.push(Action::ToggleCrt),
        Some(NamedKey::F3) if pressed => out.push(Action::ToggleDebug),
        _ => {}
    }
    if pressed && !repeat {
        if let Key::Character(text) = logical_key {
            match text.as_str() {
                "m" | "M" => out.push(Action::ToggleMute),
                "r" | "R" if screen != Screen::Title => out.push(Action::Restart),
                "q" | "Q" if screen == Screen::Title => out.push(Action::Quit),
                _ => {}
            }
        }
    }

    match screen {
        Screen::Title => {
            if pressed && !repeat {
                match key {
                    Some(NamedKey::Enter) | Some(NamedKey::Space) => out.push(Action::Confirm),
                    _ => {}
                }
            }
        }
        Screen::Playing => {
            // Release events matter: the domain owns the auto-shift state machine.
            if let Some(dir) = horizontal(logical_key) {
                out.push(if pressed {
                    Action::Game(Command::Move(dir))
                } else {
                    Action::Game(Command::StopMove(dir))
                });
            }
            if pressed && !repeat {
                if let Some(dir) = rotation(logical_key) {
                    out.push(Action::Game(Command::Rotate(dir)));
                }
                match key {
                    Some(NamedKey::Space) => out.push(Action::Game(Command::HardDrop)),
                    Some(NamedKey::ArrowDown) => out.push(Action::Game(Command::SoftDrop(true))),
                    Some(NamedKey::Pause) => out.push(Action::TogglePause),
                    _ => {}
                }
                if let Key::Character(text) = logical_key {
                    match text.as_str() {
                        "c" | "C" | "Shift" => out.push(Action::Game(Command::Hold)),
                        "p" | "P" => out.push(Action::TogglePause),
                        _ => {}
                    }
                }
            }
            if !pressed {
                if let Some(NamedKey::ArrowDown) = key {
                    out.push(Action::Game(Command::SoftDrop(false)))
                }
            }
        }
        Screen::Paused => {
            if pressed && !repeat && key == Some(NamedKey::Enter) {
                out.push(Action::TogglePause);
            }
        }
        Screen::GameOver => {
            if pressed && !repeat {
                if let Some(NamedKey::Enter) = key {
                    out.push(Action::Confirm)
                }
                if let Key::Character(text) = logical_key {
                    if let Some(ch) = text.chars().next() {
                        if !ch.is_control() {
                            out.push(Action::NameChar(ch));
                        }
                    }
                }
            }
            if pressed && key == Some(NamedKey::Backspace) {
                out.push(Action::NameBackspace);
            }
        }
    }

    if out.is_empty() {
        out.push(Action::Ignored);
    }
    out
}

fn horizontal(key: &Key) -> Option<Direction> {
    match key {
        Key::Named(NamedKey::ArrowLeft) => Some(Direction::Left),
        Key::Named(NamedKey::ArrowRight) => Some(Direction::Right),
        Key::Character(c) if c == "h" || c == "H" => Some(Direction::Left),
        Key::Character(c) if c == "l" || c == "L" => Some(Direction::Right),
        _ => None,
    }
}

fn rotation(key: &Key) -> Option<RotateDir> {
    match key {
        Key::Named(NamedKey::ArrowUp) => Some(RotateDir::Cw),
        Key::Character(c) if c == "x" || c == "X" => Some(RotateDir::Cw),
        Key::Character(c) if c == "z" || c == "Z" => Some(RotateDir::Ccw),
        Key::Character(c) if c == "a" || c == "A" => Some(RotateDir::Flip),
        _ => None,
    }
}
