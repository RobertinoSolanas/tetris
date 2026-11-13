//! `tetris-audio` — the sound context: short synthesised cues and a sink to play them.
//!
//! No audio files and no C bindings (docs/adr/0007, docs/adr/0008): [`SoundBank`] renders each
//! [`Cue`] into PCM, [`Mixer`] layers them, and a [`Sink`] plays the stream — by piping raw S16LE
//! into `pw-play`/`aplay` when they exist, and by doing nothing at all when they do not.

use std::io::Write;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread::JoinHandle;

/// Sample rate everything here is rendered at.
pub const SAMPLE_RATE: u32 = 44_100;

/// A game sound. Kept deliberately coarse: the app maps domain events onto these.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Cue {
    /// Piece slid one column.
    Move,
    /// Piece rotated.
    Rotate,
    /// Piece came to rest.
    Lock,
    /// Hold accepted.
    Hold,
    /// Hold refused (already used this piece).
    HoldDenied,
    /// 1–3 rows cleared; the number is the pitch of the arpeggio.
    Clear(u32),
    /// Four rows at once.
    Tetris,
    /// T-spin (any rows).
    Spin,
    /// Level goal reached.
    LevelUp,
    /// Combo step above 1.
    Combo,
    /// Back-to-back step.
    BackToBack,
    /// The stack reached the danger zone.
    Danger,
    Pause,
    Resume,
    /// Menu cursor moved.
    MenuMove,
    /// Menu confirm.
    MenuSelect,
    /// Match over.
    GameOver,
}

impl Cue {
    /// Length in seconds, for tests and for the (unused) pre-load path.
    pub fn duration(self) -> f32 {
        match self {
            Cue::Move => 0.05,
            Cue::Rotate => 0.07,
            Cue::Lock => 0.11,
            Cue::Hold => 0.09,
            Cue::HoldDenied => 0.12,
            Cue::Clear(_) => 0.24,
            Cue::Tetris => 0.45,
            Cue::Spin => 0.34,
            Cue::LevelUp => 0.5,
            Cue::Combo => 0.12,
            Cue::BackToBack => 0.16,
            Cue::Danger => 0.3,
            Cue::Pause | Cue::Resume => 0.07,
            Cue::MenuMove => 0.05,
            Cue::MenuSelect => 0.16,
            Cue::GameOver => 0.9,
        }
    }
}

/// One note in a cue: frequency, start offset, length, level, waveform.
#[derive(Copy, Clone, Debug)]
struct Note {
    freq: f32,
    start: f32,
    length: f32,
    gain: f32,
    shape: Shape,
}

#[derive(Copy, Clone, Debug, PartialEq)]
enum Shape {
    Sine,
    Square,
    Triangle,
    Noise,
}

/// Renders cues into mono `f32` samples in `-1.0..=1.0`.
#[derive(Copy, Clone, Debug)]
pub struct SoundBank {
    sample_rate: u32,
}

impl Default for SoundBank {
    fn default() -> Self {
        Self::new(SAMPLE_RATE)
    }
}

impl SoundBank {
    pub const fn new(sample_rate: u32) -> Self {
        Self { sample_rate }
    }

    pub const fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Program the cue: a short list of notes. Data, not code, so the sound design is readable.
    fn notes(cue: Cue) -> Vec<Note> {
        let n = |freq, start, length, gain, shape| Note {
            freq,
            start,
            length,
            gain,
            shape,
        };
        use Shape::*;
        match cue {
            Cue::Move => vec![n(180.0, 0.0, 0.04, 0.5, Square)],
            Cue::Rotate => vec![
                n(520.0, 0.0, 0.05, 0.45, Square),
                n(780.0, 0.02, 0.04, 0.3, Square),
            ],
            Cue::Lock => vec![
                n(110.0, 0.0, 0.1, 0.9, Sine),
                n(70.0, 0.0, 0.08, 0.6, Noise),
            ],
            Cue::Hold => vec![
                n(440.0, 0.0, 0.06, 0.5, Triangle),
                n(660.0, 0.03, 0.06, 0.35, Triangle),
            ],
            Cue::HoldDenied => vec![
                n(140.0, 0.0, 0.09, 0.5, Square),
                n(120.0, 0.06, 0.06, 0.4, Square),
            ],
            Cue::Clear(rows) => {
                let base = 420.0;
                let steps = rows.clamp(1, 3) as usize;
                let mut notes = Vec::new();
                for i in 0..steps {
                    notes.push(n(
                        base * (1.0 + 0.26 * i as f32),
                        i as f32 * 0.05,
                        0.12,
                        0.5,
                        Triangle,
                    ));
                }
                notes.push(n(base * 2.0, steps as f32 * 0.05, 0.1, 0.35, Sine));
                notes
            }
            Cue::Tetris => vec![
                n(392.0, 0.00, 0.12, 0.55, Square),
                n(523.0, 0.07, 0.12, 0.55, Square),
                n(659.0, 0.14, 0.12, 0.55, Square),
                n(784.0, 0.21, 0.22, 0.6, Square),
                n(1568.0, 0.21, 0.18, 0.22, Sine),
                n(90.0, 0.0, 0.4, 0.35, Noise),
            ],
            Cue::Spin => vec![
                n(330.0, 0.0, 0.1, 0.5, Triangle),
                n(494.0, 0.06, 0.1, 0.5, Triangle),
                n(659.0, 0.12, 0.18, 0.5, Triangle),
            ],
            Cue::LevelUp => {
                let mut notes = Vec::new();
                for i in 0..5 {
                    notes.push(n(
                        440.0 * (1.0 + 0.12 * i as f32),
                        i as f32 * 0.07,
                        0.16,
                        0.45,
                        Sine,
                    ));
                }
                notes
            }
            Cue::Combo => vec![n(880.0, 0.0, 0.07, 0.4, Sine)],
            Cue::BackToBack => vec![
                n(660.0, 0.0, 0.07, 0.4, Square),
                n(990.0, 0.05, 0.09, 0.4, Square),
            ],
            Cue::Danger => vec![
                n(220.0, 0.0, 0.08, 0.4, Square),
                n(220.0, 0.12, 0.08, 0.4, Square),
                n(220.0, 0.24, 0.06, 0.3, Square),
            ],
            Cue::Pause => vec![n(300.0, 0.0, 0.06, 0.35, Sine)],
            Cue::Resume => vec![n(420.0, 0.0, 0.06, 0.35, Sine)],
            Cue::MenuMove => vec![n(600.0, 0.0, 0.03, 0.28, Square)],
            Cue::MenuSelect => vec![
                n(523.0, 0.0, 0.07, 0.4, Triangle),
                n(784.0, 0.05, 0.1, 0.4, Triangle),
            ],
            Cue::GameOver => {
                let mut notes = Vec::new();
                for i in 0..6 {
                    notes.push(n(
                        440.0 * 0.9_f32.powi(i),
                        i as f32 * 0.12,
                        0.3,
                        0.5,
                        Triangle,
                    ));
                }
                notes.push(n(55.0, 0.0, 0.7, 0.4, Sine));
                notes
            }
        }
    }

    fn wave(shape: Shape, phase: f32, noise_seed: u32) -> f32 {
        match shape {
            Shape::Sine => (phase * std::f32::consts::TAU).sin(),
            Shape::Square => {
                if phase.fract() < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            Shape::Triangle => {
                // -0.5 → +0.5 → -0.5 over one period, centred like the other shapes.
                let p = phase - phase.floor();
                if p < 0.5 {
                    4.0 * p - 1.0
                } else {
                    3.0 - 4.0 * p
                }
            }
            Shape::Noise => {
                // Cheap deterministic hash noise: no RNG state, so rendering a cue twice is equal.
                let mut h = noise_seed.wrapping_mul(2_654_435_761);
                h ^= h >> 13;
                h = h.wrapping_mul(1_274_126_177);
                ((h >> 8) as f32 / (u32::MAX >> 8) as f32) * 2.0 - 1.0
            }
        }
    }

    /// Envelope: fast attack, exponential-ish decay. `t` in `0..=1` of the note.
    fn envelope(t: f32) -> f32 {
        if !(0.0..=1.0).contains(&t) {
            return 0.0;
        }
        let attack = (t / 0.06).min(1.0);
        let release = 1.0 - t;
        attack * release * release
    }

    /// Render a cue to mono samples, peak-normalised to 0.8 (silent cues stay silent).
    pub fn render(&self, cue: Cue) -> Vec<f32> {
        let sr = self.sample_rate.max(1) as f32;
        let notes = Self::notes(cue);
        let total = cue.duration().max(
            notes
                .iter()
                .map(|x| x.start + x.length)
                .fold(0.0f32, f32::max),
        );
        let frames = (total * sr).ceil() as usize;
        let mut out = vec![0.0f32; frames];

        for note in &notes {
            let start = (note.start * sr) as usize;
            let len = (note.length * sr) as usize;
            for i in 0..len {
                let frame = start + i;
                if frame >= frames {
                    break;
                }
                let t = i as f32 / len.max(1) as f32;
                let phase = note.freq * (i as f32 / sr);
                let sample =
                    Self::wave(note.shape, phase, frame as u32) * Self::envelope(t) * note.gain;
                out[frame] += sample;
            }
        }

        // Normalise so every cue sits at the same level and cannot clip in the mixer.
        let peak = out.iter().fold(0.0f32, |acc, s| acc.max(s.abs()));
        if peak > 1e-6 {
            let scale = 0.8 / peak;
            for s in out.iter_mut() {
                *s *= scale;
            }
        }
        for s in out.iter_mut() {
            if !s.is_finite() {
                *s = 0.0;
            }
        }
        out
    }
}

/// One playing cue.
#[derive(Debug)]
struct Voice {
    samples: Vec<f32>,
    position: usize,
}

impl Voice {
    fn finished(&self) -> bool {
        self.position >= self.samples.len()
    }
}

/// Layers cue playbacks into a stereo interleaved stream.
#[derive(Debug)]
pub struct Mixer {
    bank: SoundBank,
    voices: Vec<Voice>,
    volume: f32,
    muted: bool,
    dropped: u64,
}

impl Mixer {
    pub fn new(bank: SoundBank) -> Self {
        Self {
            bank,
            voices: Vec::new(),
            volume: 0.8,
            muted: false,
            dropped: 0,
        }
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume.clamp(0.0, 1.0);
    }

    pub fn volume(&self) -> f32 {
        self.volume
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
    }

    pub fn muted(&self) -> bool {
        self.muted
    }

    /// How many frames have been produced (for tests and diagnostics).
    pub fn dropped_voices(&self) -> u64 {
        self.dropped
    }

    /// Start a cue. Long tails are trimmed so a stuck output can't grow memory without bound.
    pub fn push(&mut self, cue: Cue) {
        if self.muted || self.volume <= 0.0 {
            return;
        }
        if self.voices.len() >= 24 {
            self.voices.remove(0);
            self.dropped += 1;
        }
        let samples = self.bank.render(cue);
        if !samples.is_empty() {
            self.voices.push(Voice {
                samples,
                position: 0,
            });
        }
    }

    /// Number of live voices (decays to 0 between sounds).
    pub fn active_voices(&self) -> usize {
        self.voices.len()
    }

    /// Fill `out` with interleaved stereo frames and advance every voice.
    pub fn fill(&mut self, out: &mut [i16]) {
        let frames = out.len() / 2;
        for frame in 0..frames {
            let mut mix = 0.0f32;
            for voice in self.voices.iter_mut() {
                if !voice.finished() {
                    mix += voice.samples[voice.position];
                    voice.position += 1;
                }
            }
            // Drop finished voices after the frame so nothing truncates mid-sample.
            self.voices.retain(|voice| !voice.finished());

            let scaled = (mix.clamp(-1.0, 1.0) * self.volume * 32767.0).round();
            let sample = scaled.clamp(-32768.0, 32767.0) as i16;
            out[frame * 2] = sample;
            out[frame * 2 + 1] = sample;
        }
    }
}

/// Where the sound goes.
pub trait Sink: Send {
    /// Queue a cue (never blocks).
    fn play(&mut self, cue: Cue);
    fn set_volume(&mut self, volume: f32);
    fn set_muted(&mut self, muted: bool);
    /// Called once per rendered frame so the sink can drain/mix at the game's pace.
    fn tick(&mut self) {}
}

/// Does nothing. The default when no player binary is available or sound is switched off.
#[derive(Debug, Default)]
pub struct SilentSink {
    volume: f32,
    muted: bool,
    /// Cues that *would* have played — handy in tests.
    pub heard: Vec<Cue>,
}

impl Sink for SilentSink {
    fn play(&mut self, cue: Cue) {
        if !self.muted {
            self.heard.push(cue);
        }
    }
    fn set_volume(&mut self, volume: f32) {
        self.volume = volume;
    }
    fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
    }
}

impl SilentSink {
    pub fn volume(&self) -> f32 {
        self.volume
    }
}

/// Writes the mixed stream to a WAV file (diagnostics and tests).
pub struct WavSink {
    mixer: Mixer,
    samples: Vec<i16>,
    frame_budget: usize,
    path: Option<std::path::PathBuf>,
}

impl WavSink {
    pub fn new(path: impl Into<std::path::PathBuf>) -> Self {
        Self {
            mixer: Mixer::new(SoundBank::default()),
            samples: Vec::new(),
            frame_budget: SAMPLE_RATE as usize * 30,
            path: Some(path.into()),
        }
    }

    /// The samples captured so far, interleaved stereo.
    pub fn samples(&self) -> &[i16] {
        &self.samples
    }

    /// Finish and write the file.
    pub fn finish(mut self) -> std::io::Result<()> {
        self.drain();
        let mut file = std::fs::File::create(
            self.path
                .as_ref()
                .ok_or_else(|| std::io::Error::other("no path for the wav sink"))?,
        )?;
        file.write_all(&wav_bytes(&self.samples))?;
        file.flush()
    }

    fn drain(&mut self) {
        while self.mixer.active_voices() > 0 && self.samples.len() / 2 < self.frame_budget {
            let mut buf = vec![0i16; 512 * 2];
            self.mixer.fill(&mut buf);
            self.samples.extend_from_slice(&buf);
        }
    }
}

impl Sink for WavSink {
    fn play(&mut self, cue: Cue) {
        self.mixer.push(cue);
    }
    fn set_volume(&mut self, volume: f32) {
        self.mixer.set_volume(volume);
    }
    fn set_muted(&mut self, muted: bool) {
        self.mixer.set_muted(muted);
    }
    fn tick(&mut self) {
        self.drain();
    }
}

/// Minimal RIFF/WAVE header + PCM data, so `--dump-audio out.wav` needs no crate.
pub fn wav_bytes(samples: &[i16]) -> Vec<u8> {
    let data_bytes = samples.len() * 2;
    let mut out = Vec::with_capacity(44 + data_bytes);
    let u32le = |out: &mut Vec<u8>, v: u32| out.extend_from_slice(&v.to_le_bytes());
    let u16le = |out: &mut Vec<u8>, v: u16| out.extend_from_slice(&v.to_le_bytes());

    out.extend_from_slice(b"RIFF");
    u32le(&mut out, 36 + data_bytes as u32);
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    u32le(&mut out, 16);
    u16le(&mut out, 1); // PCM
    u16le(&mut out, 2); // stereo
    u32le(&mut out, SAMPLE_RATE);
    u32le(&mut out, SAMPLE_RATE * 4);
    u16le(&mut out, 4);
    u16le(&mut out, 16);
    out.extend_from_slice(b"data");
    u32le(&mut out, data_bytes as u32);
    for s in samples {
        u16le(&mut out, (*s) as u16);
    }
    out
}

/// Which player binary to spawn, in order of preference.
const PLAYERS: [&str; 2] = ["pw-play", "aplay"];

fn player_args(binary: &str) -> Vec<String> {
    match binary {
        "pw-play" => vec![
            "--quiet".into(),
            "--rate".into(),
            SAMPLE_RATE.to_string(),
            "--channels".into(),
            "2".into(),
            "--format".into(),
            "s16-le".into(),
            "-".into(),
        ],
        _ => vec![
            "-q".into(),
            "-r".into(),
            SAMPLE_RATE.to_string(),
            "-f".into(),
            "S16_LE".into(),
            "-c".into(),
            "2".into(),
            "-t".into(),
            "raw".into(),
            "-".into(),
        ],
    }
}

/// Streams the mix into a `pw-play`/`aplay` subprocess.
///
/// A bounded channel sits between the game thread and the writer thread: if the audio server stalls,
/// cues are dropped rather than stalling the frame loop.
pub struct SubprocessSink {
    mixer: Mixer,
    tx: Option<SyncSender<Vec<i16>>>,
    child: Option<Child>,
    writer: Option<JoinHandle<()>>,
    dropped: Arc<AtomicU32>,
    alive: Arc<AtomicBool>,
}

impl SubprocessSink {
    /// Spawn a player. `Err` means "no usable player binary", and the caller should fall back to
    /// [`SilentSink`].
    pub fn spawn() -> Result<Self, String> {
        let mut last = String::from("no player binary found");
        for binary in PLAYERS {
            match Command::new(binary)
                .args(player_args(binary))
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                Ok(mut child) => {
                    let stdin: ChildStdin = child.stdin.take().expect("stdin was piped");
                    let (tx, rx) = mpsc::sync_channel::<Vec<i16>>(8);
                    let dropped = Arc::new(AtomicU32::new(0));
                    let alive = Arc::new(AtomicBool::new(true));
                    let alive_thread = alive.clone();
                    let writer = std::thread::Builder::new()
                        .name("tetris-audio".into())
                        .spawn(move || {
                            let mut stdin = stdin;
                            while let Ok(chunk) = rx.recv() {
                                let bytes = bytemuck_bytes(&chunk);
                                if stdin.write_all(&bytes).is_err() {
                                    alive_thread.store(false, Ordering::Relaxed);
                                    break;
                                }
                                let _ = stdin.flush();
                            }
                            let _ = stdin.flush();
                        })
                        .map_err(|e| format!("spawn audio writer thread: {e}"))?;
                    return Ok(Self {
                        mixer: Mixer::new(SoundBank::default()),
                        tx: Some(tx),
                        child: Some(child),
                        writer: Some(writer),
                        dropped,
                        alive,
                    });
                }
                Err(e) => last = format!("{binary}: {e}"),
            }
        }
        Err(last)
    }

    /// Cues discarded because the audio path could not keep up.
    pub fn dropped(&self) -> u32 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// False once the player process has gone away.
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }
}

impl Sink for SubprocessSink {
    fn play(&mut self, cue: Cue) {
        self.mixer.push(cue);
    }

    fn set_volume(&mut self, volume: f32) {
        self.mixer.set_volume(volume);
    }

    fn set_muted(&mut self, muted: bool) {
        self.mixer.set_muted(muted);
    }

    /// Called once per frame: renders one frame's worth of audio and hands it to the writer thread.
    fn tick(&mut self) {
        if !self.is_alive() {
            return;
        }
        // 1/60 s of stereo audio keeps latency low without spinning the thread.
        let frames = (SAMPLE_RATE / 60).max(64) as usize;
        let mut buf = vec![0i16; frames * 2];
        self.mixer.fill(&mut buf);
        let Some(tx) = self.tx.as_ref() else {
            return;
        };
        match tx.try_send(buf) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
            Err(TrySendError::Disconnected(_)) => {
                self.alive.store(false, Ordering::Relaxed);
            }
        }
    }
}

impl Drop for SubprocessSink {
    fn drop(&mut self) {
        // Dropping the last sender ends the writer thread, which closes stdin, which ends the
        // player process. Take the sender *before* joining or we would wait for ourselves.
        drop(self.tx.take());
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// i16 slice → little-endian bytes (no bytemuck dependency in this crate).
fn bytemuck_bytes(samples: &[i16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(samples.len() * 2);
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// Which playback backend *would* be used, for the startup log.
pub fn describe_backend() -> &'static str {
    PLAYERS
        .iter()
        .find(|binary| which_exists(binary))
        .map(|binary| *binary as &str)
        .unwrap_or("silent (no player binary found)")
}

/// Cheap `which`: is the binary executable and on `PATH`?
fn which_exists(binary: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| dir.join(binary).is_file())
}

/// Convenience: build the best available sink.
pub fn default_sink(muted: bool) -> Box<dyn Sink> {
    if muted {
        return Box::new(SilentSink::default());
    }
    match SubprocessSink::spawn() {
        Ok(sink) => Box::new(sink),
        Err(_) => Box::new(SilentSink::default()),
    }
}
