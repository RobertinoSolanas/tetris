//! Binary entry point: parse the command line, then either render and exit or open a window.

use std::process::ExitCode;

use tetris_app::app;
use tetris_app::cli::{self, Options};

fn main() -> ExitCode {
    let options = match Options::parse(std::env::args()) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("tetris: {message}");
            eprintln!("try --help");
            return ExitCode::from(2);
        }
    };

    if options.help {
        print!("{}", cli::HELP);
        return ExitCode::SUCCESS;
    }

    if let Some(path) = &options.dump_audio {
        return match dump_audio(path) {
            Ok(()) => {
                println!("wrote {}", path.display());
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("tetris: {e}");
                ExitCode::from(1)
            }
        };
    }

    if options.snapshot.is_some() {
        return match app::snapshot(&options) {
            Ok(paths) => {
                for path in paths {
                    println!("wrote {}", path.display());
                }
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("tetris: {e}");
                ExitCode::from(1)
            }
        };
    }

    match app::run(options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("tetris: {e}");
            ExitCode::from(1)
        }
    }
}

/// Write a short demo of the sound bank so the synthesis can be inspected without playing.
fn dump_audio(path: &std::path::Path) -> Result<(), String> {
    use tetris_audio::{Cue, Sink, WavSink};
    let mut sink = WavSink::new(path);
    let sequence = [
        Cue::Move,
        Cue::Rotate,
        Cue::Lock,
        Cue::Clear(2),
        Cue::Tetris,
        Cue::LevelUp,
        Cue::GameOver,
    ];
    for cue in sequence {
        sink.play(cue);
        for _ in 0..12 {
            sink.tick();
        }
    }
    sink.finish()
        .map_err(|e| format!("write {}: {e}", path.display()))
}
