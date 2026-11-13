//! Records & settings: the only place that touches the filesystem.
//!
//! Storage is JSON written by our own tiny encoder (`crate::json`), atomic (write a temp file, then
//! rename), and *lossy by design*: anything unreadable falls back to defaults rather than stopping
//! the game. Losing a settings file must never prevent somebody from playing.

use std::io;
use std::path::{Path, PathBuf};

use crate::json::{self, Value};

/// How many high scores we keep.
pub const MAX_SCORES: usize = 10;

/// Player-facing options, mirrored to `settings.json`.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// CRT/scanline/curvature post effects.
    pub crt: bool,
    /// Bloom strength 0..1.
    pub bloom: f32,
    /// Master volume 0..1.
    pub volume: f32,
    pub muted: bool,
    /// F3 overlay with frame time and scene statistics.
    pub show_debug: bool,
    /// Lines per level.
    pub lines_per_level: u32,
    /// Preferred seed for the next match (`None` = pick one).
    pub last_seed: Option<u64>,
    /// Name used for a new high score.
    pub player: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            crt: true,
            bloom: 0.85,
            volume: 0.8,
            muted: false,
            show_debug: false,
            lines_per_level: 10,
            last_seed: None,
            player: "PLAYER".to_string(),
        }
    }
}

impl Settings {
    fn to_value(&self) -> Value {
        let mut v = Value::object();
        v.set("crt", Value::Bool(self.crt));
        v.set("bloom", Value::Number(self.bloom as f64));
        v.set("volume", Value::Number(self.volume as f64));
        v.set("muted", Value::Bool(self.muted));
        v.set("show_debug", Value::Bool(self.show_debug));
        v.set(
            "lines_per_level",
            Value::Number(self.lines_per_level as f64),
        );
        v.set(
            "last_seed",
            match self.last_seed {
                Some(seed) => Value::Number(seed as f64),
                None => Value::Null,
            },
        );
        v.set("player", Value::String(self.player.clone()));
        v
    }

    fn from_value(v: &Value) -> Self {
        let defaults = Self::default();
        Self {
            crt: v.bool_or("crt", defaults.crt),
            bloom: clamp01(v.f64_or("bloom", defaults.bloom as f64) as f32),
            volume: clamp01(v.f64_or("volume", defaults.volume as f64) as f32),
            muted: v.bool_or("muted", defaults.muted),
            show_debug: v.bool_or("show_debug", defaults.show_debug),
            lines_per_level: v.u32_or("lines_per_level", defaults.lines_per_level).max(1),
            last_seed: v.get("last_seed").and_then(Value::as_u64),
            player: sanitize_name(&v.str_or("player", &defaults.player)),
        }
    }
}

/// One row of the high-score table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScoreEntry {
    pub name: String,
    pub score: u64,
    pub lines: u32,
    pub level: u32,
    pub pieces: u32,
    pub seconds: u32,
}

impl ScoreEntry {
    fn to_value(&self) -> Value {
        let mut v = Value::object();
        v.set("name", Value::String(self.name.clone()));
        v.set("score", Value::Number(self.score as f64));
        v.set("lines", Value::Number(self.lines as f64));
        v.set("level", Value::Number(self.level as f64));
        v.set("pieces", Value::Number(self.pieces as f64));
        v.set("seconds", Value::Number(self.seconds as f64));
        v
    }

    fn from_value(v: &Value) -> Self {
        Self {
            name: sanitize_name(&v.str_or("name", "PLAYER")),
            score: v.u64_or("score", 0),
            lines: v.u32_or("lines", 0),
            level: v.u32_or("level", 1),
            pieces: v.u32_or("pieces", 0),
            seconds: v.u32_or("seconds", 0),
        }
    }
}

/// Top-`MAX_SCORES` table, always sorted best-first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScoreBoard {
    pub entries: Vec<ScoreEntry>,
}

impl ScoreBoard {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a score. Returns its 1-based rank, or `None` if it did not make the table.
    pub fn submit(&mut self, entry: ScoreEntry) -> Option<usize> {
        let entry = ScoreEntry {
            name: sanitize_name(&entry.name),
            ..entry
        };
        if self.entries.len() >= MAX_SCORES
            && entry.score <= self.entries.last().map(|e| e.score).unwrap_or(0)
        {
            return None;
        }
        let position = self
            .entries
            .iter()
            .position(|existing| entry.score > existing.score)
            .unwrap_or(self.entries.len());
        self.entries.insert(position, entry);
        self.entries.truncate(MAX_SCORES);
        Some(position + 1)
    }

    /// 1-based rank of `score` if it would place, else `None`.
    pub fn rank_of(&self, score: u64) -> Option<usize> {
        if self.entries.len() < MAX_SCORES {
            return Some(
                self.entries
                    .iter()
                    .position(|e| score > e.score)
                    .map(|i| i + 1)
                    .unwrap_or(self.entries.len() + 1),
            );
        }
        self.entries
            .iter()
            .position(|e| score > e.score)
            .map(|i| i + 1)
    }

    pub fn best(&self) -> Option<u64> {
        self.entries.first().map(|e| e.score)
    }

    fn to_value(&self) -> Value {
        Value::Object(vec![(
            "scores".to_string(),
            Value::Array(self.entries.iter().map(ScoreEntry::to_value).collect()),
        )])
    }

    fn from_value(v: &Value) -> Self {
        let entries = v
            .get("scores")
            .and_then(Value::as_array)
            .map(|items| {
                let mut list: Vec<ScoreEntry> = items.iter().map(ScoreEntry::from_value).collect();
                list.sort_by_key(|entry| std::cmp::Reverse(entry.score));
                list.truncate(MAX_SCORES);
                list
            })
            .unwrap_or_default();
        Self { entries }
    }
}

/// Where the files live.
#[derive(Clone, Debug)]
pub struct Store {
    dir: PathBuf,
}

impl Store {
    /// `$XDG_DATA_HOME/pi-tetris`, or `$HOME/.local/share/pi-tetris`.
    pub fn from_env() -> Self {
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            })
            .unwrap_or_else(|| PathBuf::from("."));
        Self {
            dir: base.join("pi-tetris"),
        }
    }

    pub fn in_dir(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    /// Read settings; anything wrong on disk yields the defaults.
    pub fn load_settings(&self) -> Settings {
        self.read_json("settings.json")
            .map(|v| Settings::from_value(&v))
            .unwrap_or_default()
    }

    pub fn save_settings(&self, settings: &Settings) -> io::Result<()> {
        self.write_json("settings.json", &settings.to_value())
    }

    /// Read the high-score table; anything wrong on disk yields an empty table.
    pub fn load_scores(&self) -> ScoreBoard {
        self.read_json("scores.json")
            .map(|v| ScoreBoard::from_value(&v))
            .unwrap_or_default()
    }

    pub fn save_scores(&self, board: &ScoreBoard) -> io::Result<()> {
        self.write_json("scores.json", &board.to_value())
    }

    fn read_json(&self, name: &str) -> Result<Value, String> {
        let text = std::fs::read_to_string(self.path(name)).map_err(|e| e.to_string())?;
        json::parse(&text)
    }

    /// Write `tmp` then rename, so a crash mid-write cannot leave half a file behind.
    fn write_json(&self, name: &str, value: &Value) -> io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let final_path = self.path(name);
        let tmp_path = final_path.with_extension("json.tmp");
        let mut text = value.to_pretty();
        text.push('\n');
        std::fs::write(&tmp_path, text)?;
        std::fs::rename(&tmp_path, &final_path)
    }
}

fn clamp01(v: f32) -> f32 {
    if !v.is_finite() {
        0.0
    } else {
        v.clamp(0.0, 1.0)
    }
}

/// Keep names short, printable and non-empty (they are drawn with a pixel font).
fn sanitize_name(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|c| !c.is_control())
        .take(12)
        .map(|c| {
            if c.is_ascii_lowercase() {
                c.to_ascii_uppercase()
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        "PLAYER".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("pi-tetris-store-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn settings_round_trip() {
        let dir = temp_dir("settings");
        let store = Store::in_dir(&dir);
        let settings = Settings {
            crt: false,
            bloom: 0.4,
            volume: 0.25,
            muted: true,
            show_debug: true,
            lines_per_level: 15,
            last_seed: Some(4242),
            player: "ANIKA".into(),
        };
        store.save_settings(&settings).unwrap();
        assert_eq!(store.load_settings(), settings);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_yields_defaults() {
        let dir = temp_dir("missing");
        let store = Store::in_dir(&dir);
        assert_eq!(store.load_settings(), Settings::default());
        assert_eq!(store.load_scores(), ScoreBoard::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_files_yield_defaults_instead_of_failing() {
        let dir = temp_dir("corrupt");
        let store = Store::in_dir(&dir);
        std::fs::write(store.path("settings.json"), "{ this is not json").unwrap();
        std::fs::write(store.path("scores.json"), "[[[[").unwrap();
        assert_eq!(store.load_settings(), Settings::default());
        assert!(store.load_scores().entries.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn out_of_range_values_are_clamped_not_trusted() {
        let dir = temp_dir("clamps");
        let store = Store::in_dir(&dir);
        std::fs::write(
            store.path("settings.json"),
            r#"{"crt": true, "bloom": 99.0, "volume": -5.0, "lines_per_level": 0, "player": "\u0001\u0002"}"#,
        )
        .unwrap();
        let loaded = store.load_settings();
        assert_eq!(loaded.bloom, 1.0);
        assert_eq!(loaded.volume, 0.0);
        assert_eq!(loaded.lines_per_level, 1, "never zero");
        assert_eq!(loaded.player, "PLAYER", "control characters are dropped");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scores_are_sorted_truncated_and_ranked() {
        let mut board = ScoreBoard::new();
        let entry = |score: u64| ScoreEntry {
            name: "PIP".into(),
            score,
            lines: 1,
            level: 1,
            pieces: 2,
            seconds: 3,
        };
        for score in [100, 900, 500, 700, 300, 800, 1000, 200, 600, 400, 50] {
            board.submit(entry(score));
        }
        assert_eq!(board.entries.len(), MAX_SCORES);
        assert_eq!(board.best(), Some(1000));
        assert!(board.entries.windows(2).all(|w| w[0].score >= w[1].score));
        assert_eq!(
            board.entries.last().unwrap().score,
            100,
            "the 11th submission (50) should not have placed"
        );

        assert_eq!(board.rank_of(10_000), Some(1));
        assert_eq!(board.rank_of(950), Some(2));
        assert_eq!(board.rank_of(1), None, "below the table");
    }

    #[test]
    fn an_empty_table_accepts_anything() {
        let mut board = ScoreBoard::new();
        assert_eq!(board.rank_of(0), Some(1));
        assert_eq!(
            board.submit(ScoreEntry {
                name: "ZERO".into(),
                score: 0,
                lines: 0,
                level: 1,
                pieces: 0,
                seconds: 0
            }),
            Some(1)
        );
    }

    #[test]
    fn scoreboard_round_trip() {
        let dir = temp_dir("board");
        let store = Store::in_dir(&dir);
        let mut board = ScoreBoard::new();
        board.submit(ScoreEntry {
            name: "Ada".into(),
            score: 123_456,
            lines: 42,
            level: 6,
            pieces: 111,
            seconds: 321,
        });
        board.submit(ScoreEntry {
            name: "BOB".into(),
            score: 999,
            lines: 3,
            level: 1,
            pieces: 9,
            seconds: 30,
        });
        store.save_scores(&board).unwrap();
        let loaded = store.load_scores();
        assert_eq!(loaded.entries.len(), 2);
        assert_eq!(loaded.entries[0].name, "ADA", "names normalise on load");
        assert_eq!(loaded.entries[0].score, 123_456);
        assert_eq!(loaded, board);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn saving_is_atomic_enough_to_leave_no_temp_file() {
        let dir = temp_dir("atomic");
        let store = Store::in_dir(&dir);
        store.save_settings(&Settings::default()).unwrap();
        store.save_scores(&ScoreBoard::new()).unwrap();
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "temp files left behind: {leftovers:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn store_directory_follows_xdg() {
        let store = Store::in_dir("/tmp/example-data");
        assert_eq!(store.dir(), Path::new("/tmp/example-data"));
        // from_env must never panic regardless of the environment.
        let _ = Store::from_env();
    }

    #[test]
    fn names_are_sanitised() {
        assert_eq!(sanitize_name("  hello  "), "HELLO");
        assert_eq!(sanitize_name(""), "PLAYER");
        assert_eq!(sanitize_name("   "), "PLAYER");
        assert_eq!(sanitize_name("\u{1}\u{2}"), "PLAYER");
        assert_eq!(sanitize_name("verylongnameindeed").len(), 12);
    }
}
