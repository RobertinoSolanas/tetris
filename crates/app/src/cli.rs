//! Command-line options. Everything here is also exercised by the test suite, because a game that
//! cannot be driven from a script cannot be verified by one either.

use std::path::PathBuf;

/// Parsed options.
#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    /// Open a window, run a few frames with a scripted player, exit 0.
    pub smoke_test: bool,
    /// Frames to run in smoke-test mode.
    pub frames: u32,
    /// Render one gameplay frame offscreen into this PNG and exit.
    pub snapshot: Option<PathBuf>,
    /// Seed for the match (`None` = derive one).
    pub seed: Option<u64>,
    /// Window size in physical pixels.
    pub width: u32,
    pub height: u32,
    /// Post effects off.
    pub no_crt: bool,
    /// Audio off.
    pub no_sound: bool,
    /// Dump the audio of a few cues into a WAV file (diagnostics).
    pub dump_audio: Option<PathBuf>,
    pub help: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            smoke_test: false,
            frames: 120,
            snapshot: None,
            seed: None,
            width: 1280,
            height: 800,
            no_crt: false,
            no_sound: false,
            dump_audio: None,
            help: false,
        }
    }
}

pub const HELP: &str = "\
pi-tetris — a neon Tetris in Rust

USAGE:
    tetris [OPTIONS]

OPTIONS:
    --seed <N>          Deterministic piece sequence
    --window <WxH>      Window size, e.g. --window 1600x900
    --no-crt            Disable scanlines/curvature/bloom
    --no-sound          Do not spawn an audio player
    --snapshot <FILE>   Render gameplay + title frames to PNG and exit
    --smoke-test        Open a window, play a scripted match, exit 0
    --frames <N>        Frames for --smoke-test (default 120)
    --dump-audio <FILE> Write a few synthesised cues to a WAV file and exit
    -h, --help          Print this help

KEYS:
    ←/→ move   ↓ soft drop   SPACE hard drop
    ↑/X rotate cw   Z rotate ccw   A rotate 180
    C/SHIFT hold    P/ESC pause    R restart
    F2 crt   M mute   F3 debug info   Q quit at the title
";

impl Options {
    /// Parse arguments. The first item is expected to be the program name (like `std::env::args`).
    pub fn parse<I, S>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let args: Vec<String> = args.into_iter().map(Into::into).collect();
        let mut out = Options::default();
        let mut i = 1; // skip argv[0]
        while i < args.len() {
            let arg = args[i].as_str();
            let mut take_value = |name: &str| -> Result<String, String> {
                i += 1;
                args.get(i)
                    .cloned()
                    .ok_or_else(|| format!("{name} needs a value"))
            };
            match arg {
                "--help" | "-h" => out.help = true,
                "--smoke-test" => out.smoke_test = true,
                "--no-crt" => out.no_crt = true,
                "--no-sound" => out.no_sound = true,
                "--frames" => {
                    let value = take_value("--frames")?;
                    out.frames = value
                        .parse::<u32>()
                        .map_err(|_| format!("--frames: '{value}' is not a number"))?
                        .clamp(1, 100_000);
                }
                "--seed" => {
                    let value = take_value("--seed")?;
                    out.seed = Some(
                        value
                            .parse::<u64>()
                            .map_err(|_| format!("--seed: '{value}' is not a number"))?,
                    );
                }
                "--window" => {
                    let value = take_value("--window")?;
                    let (w, h) = parse_size(&value)?;
                    out.width = w;
                    out.height = h;
                }
                "--snapshot" => out.snapshot = Some(PathBuf::from(take_value("--snapshot")?)),
                "--dump-audio" => out.dump_audio = Some(PathBuf::from(take_value("--dump-audio")?)),
                other if other.starts_with("--window=") => {
                    let value = other.trim_start_matches("--window=").to_string();
                    let (w, h) = parse_size(&value)?;
                    out.width = w;
                    out.height = h;
                }
                other => return Err(format!("unknown option '{other}' (try --help)")),
            }
            i += 1;
        }
        if out.width < 320 || out.height < 240 {
            return Err(format!(
                "window size {}×{} is too small (minimum 320×240)",
                out.width, out.height
            ));
        }
        Ok(out)
    }

    /// Options for the tests / snapshot runs: no window, deterministic.
    pub fn snapshot_defaults(path: PathBuf) -> Self {
        Self {
            snapshot: Some(path),
            seed: Some(1234),
            ..Default::default()
        }
    }
}

fn parse_size(text: &str) -> Result<(u32, u32), String> {
    let Some((w, h)) = text.split_once(['x', 'X']) else {
        return Err(format!("--window: expected WxH, got '{text}'"));
    };
    let w = w
        .trim()
        .parse::<u32>()
        .map_err(|_| format!("--window: '{w}' is not a number"))?;
    let h = h
        .trim()
        .parse::<u32>()
        .map_err(|_| format!("--window: '{h}' is not a number"))?;
    Ok((w, h))
}
