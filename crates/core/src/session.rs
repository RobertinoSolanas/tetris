//! Match lifecycle — the aggregate root of the gameplay domain.
//!
//! `Session` owns the board, the piece supply, the score chains and every timer, and is the only
//! place where the other contexts are allowed to meet. The outside world talks to it in [`Command`]s
//! and hears back [`GameEvent`]s; it never reaches into the board to mutate it.

use std::time::Duration;

use crate::actions::{Command, Direction, RotateDir};
use crate::board::Board;
use crate::events::{EndReason, GameEvent};
use crate::handling::{AutoShift, DropTimer, HandlingConfig, InputBuffer, LockTimer};
use crate::kicks::kick_attempts;
use crate::piece::{ActivePiece, Cell, Rotation, Tetromino};
use crate::rng::Fnv;
use crate::scoring::{
    ClearKind, ClearOutcome, Progression, ScoreReason, ScoreState, ScoreTable, SpinKind,
};
use crate::supply::PieceSupply;

/// Where the match is in its lifecycle.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Nothing simulated yet; the title screen animates.
    Title,
    Playing,
    Paused,
    GameOver,
}

impl Phase {
    pub const fn is_playing(self) -> bool {
        matches!(self, Phase::Playing)
    }
}

/// Everything a session can be configured with.
#[derive(Clone, Debug)]
pub struct SessionConfig {
    pub handling: HandlingConfig,
    pub progression: Progression,
    pub score_table: ScoreTable,
    /// How many upcoming pieces the HUD shows.
    pub preview_size: usize,
    /// Stack fraction at which the danger warning fires.
    pub danger_threshold: f32,
    /// Length of one simulation tick, used for replaying input logs.
    pub tick: Duration,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            handling: HandlingConfig::standard(),
            progression: Progression::guideline(),
            score_table: ScoreTable::guideline(),
            preview_size: 5,
            danger_threshold: 0.8,
            tick: Duration::from_nanos(1_000_000_000 / 120),
        }
    }
}

/// Final/aggregate numbers of a match.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stats {
    pub score: u64,
    pub lines: u32,
    pub level: u32,
    pub pieces: u32,
    pub tetrises: u32,
    pub spins: u32,
    pub perfect_clears: u32,
    pub max_combo: u32,
    pub max_back_to_back: u32,
    pub time: Duration,
}

/// The match. See the module docs.
pub struct Session {
    config: SessionConfig,
    phase: Phase,
    seed: u64,

    board: Board,
    supply: PieceSupply,
    active: Option<ActivePiece>,
    hold: Option<Tetromino>,
    hold_used: bool,

    score: ScoreState,
    lines: u32,
    level: u32,
    pieces: u32,
    tetrises: u32,
    spins: u32,
    perfect_clears: u32,
    elapsed: Duration,
    danger: bool,

    // Timers and input state machines.
    drop: DropTimer,
    lock: LockTimer,
    shift: AutoShift,
    soft_dropping: bool,
    /// Counts down between a lock/clear and the next spawn (Guideline ARE).
    entry: Duration,
    /// A piece is waiting to be dealt when `entry` reaches zero.
    pending_spawn: bool,

    // T-spin bookkeeping.
    last_action_was_rotation: bool,
    last_kick: Option<(i32, i32)>,

    tick_index: u64,
    input_log: Vec<(u64, Command)>,
    events: Vec<GameEvent>,

    /// One intent pressed during the entry delay, replayed when the next piece appears.
    input_buffer: InputBuffer,
}

impl Session {
    /// A session in [`Phase::Title`] with a seeded supply already primed.
    pub fn new(seed: u64, config: SessionConfig) -> Self {
        let input_buffer = InputBuffer::new(config.handling.input_buffer, config.tick);
        Self {
            config,
            phase: Phase::Title,
            seed,
            board: Board::new(),
            supply: PieceSupply::new(seed),
            active: None,
            hold: None,
            hold_used: false,
            score: ScoreState::new(),
            lines: 0,
            level: 1,
            pieces: 0,
            tetrises: 0,
            spins: 0,
            perfect_clears: 0,
            elapsed: Duration::ZERO,
            danger: false,
            drop: DropTimer::new(),
            lock: LockTimer::new(),
            shift: AutoShift::new(),
            soft_dropping: false,
            entry: Duration::ZERO,
            pending_spawn: false,
            last_action_was_rotation: false,
            last_kick: None,
            tick_index: 0,
            input_log: Vec::new(),
            events: Vec::new(),
            input_buffer,
        }
    }

    /// A default-configured session.
    pub fn with_seed(seed: u64) -> Self {
        Self::new(seed, SessionConfig::default())
    }

    pub const fn config(&self) -> &SessionConfig {
        &self.config
    }

    // ---------------------------------------------------------------- getters

    pub const fn phase(&self) -> Phase {
        self.phase
    }

    pub const fn seed(&self) -> u64 {
        self.seed
    }

    pub const fn board(&self) -> &Board {
        &self.board
    }

    pub const fn active(&self) -> Option<ActivePiece> {
        self.active
    }

    pub const fn hold(&self) -> Option<Tetromino> {
        self.hold
    }

    pub const fn hold_used(&self) -> bool {
        self.hold_used
    }

    pub const fn score(&self) -> u64 {
        self.score.total
    }

    pub const fn lines(&self) -> u32 {
        self.lines
    }

    pub const fn level(&self) -> u32 {
        self.level
    }

    pub const fn combo(&self) -> u32 {
        self.score.combo
    }

    pub const fn back_to_back(&self) -> u32 {
        self.score.b2b_chain
    }

    pub const fn elapsed(&self) -> Duration {
        self.elapsed
    }

    pub const fn tick_index(&self) -> u64 {
        self.tick_index
    }

    pub const fn soft_dropping(&self) -> bool {
        self.soft_dropping
    }

    /// `true` while a lock/clear delay is running and the next piece has not been dealt yet.
    pub const fn spawning(&self) -> bool {
        self.pending_spawn
    }

    /// Stack height as a fraction of the visible field, `0.0..=1.0`.
    pub fn danger_level(&self) -> f32 {
        self.board.stack_height_fraction()
    }

    /// Board-editor hook: replace the stack wholesale.
    ///
    /// Used by the test suite to build exact scenarios (and by practice drills later). It changes
    /// nothing else on purpose — no timers, no score — so callers own exactly what they touch.
    pub fn board_mut(&mut self) -> &mut Board {
        &mut self.board
    }

    /// Make `piece` the active piece, clearing any lock/drop state (same caveats as
    /// [`Session::board_mut`]).
    pub fn set_active(&mut self, piece: ActivePiece) {
        self.drop.reset();
        self.lock.disarm();
        self.last_action_was_rotation = false;
        self.last_kick = None;
        self.active = Some(piece);
    }

    /// Lock delay remaining for the active piece, if it is grounded.
    pub fn lock_delay_remaining(&self) -> Option<Duration> {
        if self.lock.is_armed() {
            Some(
                self.config
                    .handling
                    .lock_delay
                    .saturating_sub(self.lock.elapsed()),
            )
        } else {
            None
        }
    }

    pub fn preview(&self) -> Vec<Tetromino> {
        self.supply.preview(self.config.preview_size)
    }

    /// Where the active piece would land.
    pub fn ghost(&self) -> Option<ActivePiece> {
        self.active.map(|p| self.board.ghost(p))
    }

    pub fn stats(&self) -> Stats {
        Stats {
            score: self.score.total,
            lines: self.lines,
            level: self.level,
            pieces: self.pieces,
            tetrises: self.tetrises,
            spins: self.spins,
            perfect_clears: self.perfect_clears,
            max_combo: self.score.max_combo,
            max_back_to_back: self.score.max_b2b,
            time: self.elapsed,
        }
    }

    /// Commands received so far as `(tick, Command)`; enough to reproduce the match exactly.
    pub fn input_log(&self) -> &[(u64, Command)] {
        &self.input_log
    }

    /// Take the events produced since the last call.
    pub fn drain_events(&mut self) -> Vec<GameEvent> {
        std::mem::take(&mut self.events)
    }

    pub fn events(&self) -> &[GameEvent] {
        &self.events
    }

    /// Cheap equality fingerprint of everything a replay must reproduce.
    pub fn fingerprint(&self) -> u64 {
        let mut h = Fnv::new();
        self.board.fingerprint(&mut h);
        h.hash_u64(self.score.total);
        h.hash_u64(self.lines as u64);
        h.hash_u64(self.level as u64);
        h.hash_u64(self.pieces as u64);
        h.hash_u64(self.tetrises as u64);
        h.hash_u64(self.spins as u64);
        h.hash_u64(self.perfect_clears as u64);
        h.hash_u64(self.score.combo as u64);
        h.hash_u64(self.score.b2b_chain as u64);
        h.hash(match self.phase {
            Phase::Title => 0,
            Phase::Playing => 1,
            Phase::Paused => 2,
            Phase::GameOver => 3,
        });
        match self.active {
            None => {
                h.hash(0);
            }
            Some(p) => {
                h.hash(1);
                h.hash(p.kind as u8);
                h.hash(p.rot.index() as u8);
                h.hash_i32(p.col);
                h.hash_i32(p.row);
            }
        }
        h.hash(match self.hold {
            None => 0,
            Some(k) => k as u8 + 1,
        });
        h.hash_bool(self.hold_used);
        h.hash_bool(self.soft_dropping);
        h.finish()
    }

    // ------------------------------------------------------------ lifecycle

    /// Start (or restart) a match. Any previous state is discarded.
    pub fn start(&mut self, seed: u64) {
        self.seed = seed;
        self.board.clear();
        self.supply = PieceSupply::new(seed);
        self.active = None;
        self.hold = None;
        self.hold_used = false;
        self.score = ScoreState::new();
        self.lines = 0;
        self.level = self.config.progression.start_level.max(1);
        self.pieces = 0;
        self.tetrises = 0;
        self.spins = 0;
        self.perfect_clears = 0;
        self.elapsed = Duration::ZERO;
        self.danger = false;
        self.drop = DropTimer::new();
        self.lock = LockTimer::new();
        self.shift = AutoShift::new();
        self.soft_dropping = false;
        self.entry = self.config.handling.spawn_delay;
        self.pending_spawn = true;
        self.last_action_was_rotation = false;
        self.last_kick = None;
        self.tick_index = 0;
        self.events.clear();
        self.input_buffer.clear();
        // `input_log` deliberately survives: it is the shell's record of everything sent to this
        // session, and a restart is itself a logged command (see ADR-0009).
        self.phase = Phase::Playing;
        self.push(GameEvent::Started {
            seed,
            level: self.level,
        });
    }

    /// Rebuild a run from a recorded log, exactly as it happened.
    ///
    /// Ticks are driven by the stamps in the log. If the rebuilt simulation stops advancing (it
    /// finished or was paused and the recording did not say otherwise), replay stops waiting for
    /// ticks that will never come and simply feeds the rest of the log in, which is what the live
    /// session did with it.
    pub fn replay(seed: u64, config: SessionConfig, log: &[(u64, Command)]) -> Session {
        let tick = config.tick;
        let mut session = Session::new(seed, config);
        session.start(seed);
        for &(at, cmd) in log {
            while session.tick_index < at {
                let before = session.tick_index;
                session.tick(tick);
                if session.tick_index == before {
                    break;
                }
            }
            session.execute(cmd);
        }
        session
    }

    /// `true` when further ticks change nothing (paused or finished).
    pub const fn frozen(&self) -> bool {
        !self.phase.is_playing()
    }

    /// Advance to `target` tick by `tick` steps, stopping early if the simulation freezes.
    /// Returns the number of ticks actually simulated.
    pub fn advance_to(&mut self, target: u64, tick: Duration) -> u64 {
        let mut done = 0;
        while self.tick_index < target {
            let before = self.tick_index;
            self.tick(tick);
            if self.tick_index == before {
                break;
            }
            done += 1;
        }
        done
    }

    // ------------------------------------------------------------------ input

    /// Apply a player command. Events are queued; take them with [`Session::drain_events`].
    pub fn apply(&mut self, cmd: Command) {
        self.input_log.push((self.tick_index, cmd));
        self.execute(cmd);
    }

    /// Like [`Session::apply`] but without recording (used by replay internals and tests).
    fn execute(&mut self, cmd: Command) {
        match cmd {
            Command::Start { seed } => self.start(seed),
            Command::QuitToTitle => {
                if self.phase != Phase::Title {
                    self.phase = Phase::Title;
                    self.active = None;
                    self.shift.cancel();
                    self.lock.disarm();
                    self.push(GameEvent::ReturnedToTitle);
                }
            }
            Command::Pause => {
                if self.phase == Phase::Playing {
                    self.phase = Phase::Paused;
                    self.shift.cancel();
                    self.input_buffer.clear();
                    self.push(GameEvent::Paused);
                }
            }
            Command::Resume => {
                if self.phase == Phase::Paused {
                    self.phase = Phase::Playing;
                    self.push(GameEvent::Resumed);
                }
            }
            _ => {
                if self.phase != Phase::Playing {
                    return;
                }
                match cmd {
                    Command::Move(dir) => self.press_direction(dir),
                    Command::StopMove(dir) => {
                        self.shift.release(dir);
                    }
                    Command::SoftDrop(on) => {
                        if self.soft_dropping != on {
                            self.soft_dropping = on;
                            // Re-align the accumulator so the speed change is immediate.
                            self.drop.reset();
                        }
                    }
                    Command::Rotate(dir) => {
                        if self.active.is_some() {
                            self.rotate(dir);
                        }
                    }
                    Command::HardDrop | Command::Hold => {
                        if self.active.is_some() {
                            if matches!(cmd, Command::HardDrop) {
                                self.hard_drop();
                            } else {
                                self.hold_piece();
                            }
                        } else {
                            // Entry delay: remember it briefly so it lands on the next piece.
                            self.input_buffer.push(cmd, self.tick_index);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    fn press_direction(&mut self, dir: Direction) {
        if self.shift.press(dir, &self.config.handling) && self.active.is_some() {
            self.try_move(dir);
        }
    }

    // ------------------------------------------------------------------- tick

    /// Advance the simulation by `dt`. Does nothing unless [`Phase::Playing`].
    pub fn tick(&mut self, dt: Duration) {
        if self.phase != Phase::Playing || dt.is_zero() {
            return;
        }
        self.tick_index += 1;
        self.elapsed += dt;

        // 1. Entry delay (ARE) after a lock or a line clear.
        if self.active.is_none() {
            if self.pending_spawn {
                if self.entry <= dt {
                    self.entry = Duration::ZERO;
                    self.pending_spawn = false;
                    self.spawn_next();
                } else {
                    self.entry -= dt;
                }
            }
            return;
        }

        // 2. Horizontal auto-shift.
        let steps = self.shift.tick(dt, &self.config.handling);
        for _ in 0..steps {
            let Some(dir) = self.shift.held() else { break };
            if !self.try_move(dir) {
                // Running into a wall stops the repeat instead of hammering it.
                self.shift.cancel();
                break;
            }
        }

        let Some(piece) = self.active else {
            return;
        };

        // 3. Gravity (unless the piece is grounded and the lock timer is doing its job).
        let grounded = !self.board.fits_piece(&piece.moved(0, 1));
        if grounded {
            if !self.lock.is_armed() {
                self.lock.arm(&self.config.handling);
            }
            if self.lock.tick(dt, &self.config.handling) {
                self.lock_piece();
            }
        } else {
            self.lock.disarm();
            let interval = if self.soft_dropping {
                self.config
                    .progression
                    .soft_gravity(self.level, self.config.handling.soft_drop_factor)
            } else {
                self.config.progression.gravity(self.level)
            };
            let rows = self
                .drop
                .tick(dt, interval)
                .min(self.board.drop_distance(piece).max(0) as u32);
            if rows > 0 {
                self.active = Some(piece.moved(0, rows as i32));
                self.last_action_was_rotation = false;
                self.last_kick = None;
                if self.soft_dropping {
                    let amount = self.score.apply_drop(&self.config.score_table, rows, true);
                    if amount > 0 {
                        let total = self.score.total;
                        self.push(GameEvent::Scored {
                            reason: ScoreReason::SoftDrop,
                            amount,
                            total,
                        });
                    }
                }
                self.push(GameEvent::Dropped {
                    rows,
                    soft: self.soft_dropping,
                });
            }
        }

        self.update_danger();
    }

    // ------------------------------------------------------------- operations

    fn try_move(&mut self, dir: Direction) -> bool {
        let Some(piece) = self.active else {
            return false;
        };
        let moved = piece.moved(dir.delta_col(), 0);
        if !self.board.fits_piece(&moved) {
            self.push(GameEvent::MoveBlocked { dir });
            return false;
        }
        self.active = Some(moved);
        self.last_action_was_rotation = false;
        self.last_kick = None;
        self.push(GameEvent::Moved { dir });
        // A successful shift while grounded refreshes the lock delay (step reset).
        if self.lock.is_armed() {
            self.lock.touch(&self.config.handling);
            self.shift.step_reset(&self.config.handling);
        }
        true
    }

    fn rotate(&mut self, dir: RotateDir) {
        let Some(piece) = self.active else {
            return;
        };
        let target = piece.rotated(dir).rot;
        if piece.kind == Tetromino::O {
            // O has one orientation: accept it as a no-op rotation so spin/UI logic stays simple.
            self.last_action_was_rotation = true;
            self.last_kick = None;
            self.push(GameEvent::Rotated { dir, kick: None });
            if self.lock.is_armed() {
                self.lock.touch(&self.config.handling);
            }
            return;
        }
        for kick in kick_attempts(piece.kind, piece.rot, target) {
            let candidate = ActivePiece {
                kind: piece.kind,
                rot: target,
                col: piece.col + kick.dc,
                row: piece.row + kick.dr,
            };
            if self.board.fits_piece(&candidate) {
                let kick_used = if kick.is_none() {
                    None
                } else {
                    Some((kick.dc, kick.dr))
                };
                self.active = Some(candidate);
                self.last_action_was_rotation = true;
                self.last_kick = kick_used;
                self.push(GameEvent::Rotated {
                    dir,
                    kick: kick_used,
                });
                if self.lock.is_armed() {
                    self.lock.touch(&self.config.handling);
                }
                return;
            }
        }
        self.push(GameEvent::RotateBlocked { dir });
    }

    fn hard_drop(&mut self) {
        let Some(piece) = self.active else {
            return;
        };
        let distance = self.board.drop_distance(piece);
        self.active = Some(piece.moved(0, distance));
        if distance > 0 {
            let amount = self
                .score
                .apply_drop(&self.config.score_table, distance as u32, false);
            let total = self.score.total;
            self.push(GameEvent::HardDropped {
                rows: distance as u32,
            });
            if amount > 0 {
                self.push(GameEvent::Scored {
                    reason: ScoreReason::HardDrop,
                    amount,
                    total,
                });
            }
        } else {
            self.push(GameEvent::HardDropped { rows: 0 });
        }
        self.drop.reset();
        self.lock_piece();
    }

    fn hold_piece(&mut self) {
        let Some(piece) = self.active else {
            return;
        };
        if self.hold_used {
            self.push(GameEvent::HoldRejected);
            return;
        }
        let current = piece.kind;
        // An empty slot deals the next queued piece; otherwise the held piece comes out to play.
        let incoming = self.hold.unwrap_or_else(|| self.supply.deal());
        self.hold = Some(current);
        self.hold_used = true;
        self.after_piece_replaced(ActivePiece::spawn(incoming), true);
    }

    /// Common tail for hold-swap and spawn: reset per-piece timers and check for a top-out.
    fn after_piece_replaced(&mut self, piece: ActivePiece, from_hold: bool) {
        self.drop.reset();
        self.lock.disarm();
        self.last_action_was_rotation = false;
        self.last_kick = None;
        if self.board.spawn_blocked(piece) {
            self.active = Some(piece);
            self.game_over(EndReason::SpawnBlocked);
            return;
        }
        self.active = Some(piece);
        if from_hold {
            let held = piece.kind;
            self.push(GameEvent::HoldSwapped {
                piece: held,
                hold: self.hold,
            });
        } else {
            let queue = self.preview();
            self.push(GameEvent::PieceSpawned {
                piece: piece.kind,
                queue,
                hold: self.hold,
            });
            // A buffered intent (hard drop / hold pressed during the delay) applies right now.
            if let Some(buffered) = self.input_buffer.take(self.tick_index) {
                self.execute(buffered);
            }
        }
    }

    fn spawn_next(&mut self) {
        let piece = ActivePiece::spawn(self.supply.deal());
        self.after_piece_replaced(piece, false);
    }

    /// Lock the active piece into the stack, then score/clear/schedule the next spawn.
    fn lock_piece(&mut self) {
        let Some(piece) = self.active else {
            return;
        };
        let cells: Vec<Cell> = piece.cells().into_iter().collect();
        let spin = self.detect_spin(&piece);

        let lock_result = self.board.lock(&cells, piece.kind.color_index() as u8);

        self.pieces += 1;
        self.active = None;
        self.lock.disarm();
        self.drop.reset();
        self.hold_used = false;

        self.push(GameEvent::Locked {
            piece: piece.kind,
            cells,
            spin,
        });

        if spin.is_spin() {
            self.spins += 1;
        }

        let full = self.board.full_rows();
        let clear = ClearKind::from_rows(full.len());
        if clear.rows() > 0 {
            self.board.remove_rows(&full);
            if clear == ClearKind::Tetris {
                self.tetrises += 1;
            }
        }
        // A perfect clear is only meaningful when rows actually went away.
        let perfect = clear.rows() > 0 && self.board.is_empty();
        if perfect {
            self.perfect_clears += 1;
        }

        // Always fold the lock into the score: a silent T-spin pays, a plain no-clear lock breaks
        // the combo, and both cases live in `scoring` rather than here.
        let outcome: ClearOutcome =
            self.score
                .apply_clear(&self.config.score_table, clear, spin, perfect, self.level);

        let before = self.level;
        if clear.rows() > 0 {
            self.lines += clear.rows() as u32;
            self.level = self.config.progression.level_for(self.lines);
            self.push(GameEvent::Cleared {
                rows: full,
                kind: clear,
            });
        }
        if outcome.breakdown.total > 0 {
            let total = self.score.total;
            self.push(GameEvent::Scored {
                reason: ScoreReason::Clear {
                    kind: clear,
                    spin,
                    combo: outcome.combo,
                    back_to_back: outcome.back_to_back,
                    perfect,
                },
                amount: outcome.breakdown.total,
                total,
            });
        }
        if outcome.combo > 0 {
            self.push(GameEvent::Combo {
                count: outcome.combo,
            });
        }
        if outcome.back_to_back > 0 {
            self.push(GameEvent::BackToBack {
                chain: outcome.back_to_back,
            });
        }
        if self.level > before {
            self.push(GameEvent::LevelUp { level: self.level });
        }

        self.entry = if clear.rows() > 0 {
            self.config.handling.line_clear_delay
        } else {
            self.config.handling.spawn_delay
        };
        self.pending_spawn = true;

        if lock_result == Err(crate::board::LockError::AboveField) {
            self.game_over(EndReason::LockedOut);
            return;
        }

        // Zero entry delay: deal the next piece immediately so the timeline stays frame-rate free.
        if self.entry.is_zero() && self.pending_spawn && self.phase.is_playing() {
            self.pending_spawn = false;
            self.spawn_next();
        }
        self.update_danger();
    }

    /// 3-corner rule: a T lock whose last successful action was a rotation, with ≥ 3 filled corners
    /// around the T's centre, is a spin. Full when both front corners are filled or when a wall kick
    /// moved the piece; mini otherwise.
    fn detect_spin(&self, piece: &ActivePiece) -> SpinKind {
        if piece.kind != Tetromino::T || !self.last_action_was_rotation {
            return SpinKind::None;
        }
        // The T's centre in a 3×3 box.
        let centre = (piece.col + 1, piece.row + 1);
        let corners = [
            (centre.0 - 1, centre.1 - 1),
            (centre.0 + 1, centre.1 - 1),
            (centre.0 - 1, centre.1 + 1),
            (centre.0 + 1, centre.1 + 1),
        ];
        let filled: Vec<bool> = corners
            .iter()
            .map(|(c, r)| self.board.is_occupied(*c, *r))
            .collect();
        let count = filled.iter().filter(|&&f| f).count();
        if count < 3 {
            return SpinKind::None;
        }
        // Front corners per rotation (row grows downward):
        // Spawn points up  → top two; Cw → right two; Flip → bottom two; Ccw → left two.
        let front: [usize; 2] = match piece.rot {
            Rotation::Spawn => [0, 1],
            Rotation::Cw => [1, 3],
            Rotation::Flip => [2, 3],
            Rotation::Ccw => [0, 2],
        };
        let front_filled = filled[front[0]] && filled[front[1]];
        if front_filled || self.last_kick.is_some() {
            SpinKind::Full
        } else {
            SpinKind::Mini
        }
    }

    fn game_over(&mut self, reason: EndReason) {
        if self.phase == Phase::GameOver {
            return;
        }
        self.phase = Phase::GameOver;
        self.shift.cancel();
        self.lock.disarm();
        let stats = self.stats();
        self.push(GameEvent::GameOver { stats, reason });
    }

    fn update_danger(&mut self) {
        let now = self.board.stack_height_fraction() >= self.config.danger_threshold;
        if now != self.danger {
            self.danger = now;
            self.push(GameEvent::Danger(now));
        }
    }

    fn push(&mut self, event: GameEvent) {
        self.events.push(event);
    }

    /// Give up: end the match as if the player quit.
    pub fn quit(&mut self) {
        match self.phase {
            Phase::Title => {}
            Phase::GameOver => {}
            _ => self.game_over(EndReason::Quit),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(v: u64) -> Duration {
        Duration::from_millis(v)
    }

    /// One simulation tick of the default config (must match `SessionConfig::default().tick`).
    const TICK: Duration = Duration::from_nanos(1_000_000_000 / 120);

    /// A session already in play with its first piece dealt.
    fn playing(seed: u64) -> Session {
        let mut s = Session::with_seed(seed);
        s.start(seed);
        s.tick(ms(1));
        assert_eq!(s.phase(), Phase::Playing);
        assert!(s.active().is_some(), "first piece should be dealt");
        s
    }

    /// A session whose entry delay is a known 120 ms (so ordering can be asserted).
    fn playing_with_clear_delay(seed: u64, delay: Duration) -> Session {
        let config = SessionConfig {
            handling: HandlingConfig {
                line_clear_delay: delay,
                spawn_delay: delay,
                ..HandlingConfig::standard()
            },
            ..SessionConfig::default()
        };
        let mut s = Session::new(seed, config);
        s.start(seed);
        s.tick(ms(200));
        assert!(s.active().is_some());
        s
    }

    /// Fill `rows` of the board, leaving column `open` empty.
    fn fill_row_leaving(s: &mut Session, row: i32, open: i32) {
        for c in 0..10 {
            if c != open {
                // Cells may already be filled by leftovers from a previous drop.
                let _ = s.board_mut().lock(&[Cell::new(c, row)], 0);
            }
        }
    }

    /// Install a vertical I whose cells sit in `col`, then hard-drop it.
    fn drop_vertical_i(s: &mut Session, col: i32) {
        let mut piece = ActivePiece::spawn(Tetromino::I);
        piece.rot = Rotation::Cw; // occupies box column + 2
        piece.col = col - 2;
        piece.row = 0;
        s.set_active(piece);
        s.apply(Command::HardDrop);
    }

    /// Tick until the active piece can no longer fall (returns tick count).
    fn ground(s: &mut Session) -> u64 {
        let mut guard = 0;
        while let Some(p) = s.active() {
            if !s.board().fits_piece(&p.moved(0, 1)) {
                return guard;
            }
            s.tick(TICK);
            guard += 1;
            assert!(guard < 20_000, "piece never landed");
        }
        guard
    }

    fn find_scored(events: &[GameEvent], clear: ClearKind) -> Option<u64> {
        events.iter().find_map(|e| match e {
            GameEvent::Scored {
                reason: ScoreReason::Clear { kind, .. },
                amount,
                ..
            } if *kind == clear => Some(*amount),
            _ => None,
        })
    }

    // ------------------------------------------------------------- lifecycle

    #[test]
    fn title_becomes_playing_and_resets_state() {
        let mut s = Session::with_seed(1);
        assert_eq!(s.phase(), Phase::Title);
        assert!(s.active().is_none());

        s.start(7);
        assert_eq!(s.phase(), Phase::Playing);
        assert_eq!(s.seed(), 7);
        assert_eq!(s.score(), 0);
        assert_eq!(s.lines(), 0);
        assert_eq!(s.level(), 1);
        assert!(s.board().is_empty());
        let events = s.drain_events();
        assert!(matches!(&events[0], GameEvent::Started { seed: 7, .. }));
    }

    #[test]
    fn restart_starts_clean() {
        let mut s = playing(2);
        for _ in 0..20 {
            s.apply(Command::HardDrop);
            s.tick(ms(200));
            let _ = s.drain_events();
        }
        assert!(!s.board().is_empty());
        s.apply(Command::Start { seed: 99 });
        assert!(s.board().is_empty());
        assert_eq!(s.score(), 0);
        assert_eq!(s.lines(), 0);
        assert_eq!(s.seed(), 99);
        assert_eq!(s.phase(), Phase::Playing);
    }

    #[test]
    fn quit_to_title_resets_phase() {
        let mut s = playing(3);
        s.apply(Command::QuitToTitle);
        assert_eq!(s.phase(), Phase::Title);
        assert!(s.active().is_none());
        assert!(s.drain_events().contains(&GameEvent::ReturnedToTitle));
    }

    #[test]
    fn gameplay_commands_are_ignored_outside_playing() {
        let mut s = Session::with_seed(4);
        s.apply(Command::Move(Direction::Left));
        s.apply(Command::HardDrop);
        s.apply(Command::Hold);
        s.tick(ms(1000));
        assert!(s.active().is_none());
        assert!(s.board().is_empty());
        assert!(s.drain_events().is_empty());
    }

    // ----------------------------------------------------------------- falling

    #[test]
    fn gravity_drops_one_row_per_gravity_interval() {
        let mut s = playing(5);
        let before = s.active().unwrap().row;
        s.tick(ms(900));
        assert_eq!(
            s.active().unwrap().row,
            before,
            "level 1 gravity is 1 s/row"
        );
        s.tick(ms(200));
        assert_eq!(s.active().unwrap().row, before + 1);
        assert!(s.drain_events().contains(&GameEvent::Dropped {
            rows: 1,
            soft: false
        }));
    }

    #[test]
    fn soft_drop_is_the_soft_drop_factor_faster_and_pays() {
        let mut s = playing(6);
        let before = s.active().unwrap().row;
        s.apply(Command::SoftDrop(true));
        s.tick(ms(100));
        let after = s.active().unwrap().row;
        // SDF 20 ⇒ 50 ms per row ⇒ 2 rows in 100 ms.
        assert_eq!(after - before, 2, "soft drop should be 20× gravity");
        let events = s.drain_events();
        assert!(events.iter().any(|e| matches!(
            e,
            GameEvent::Scored { reason: ScoreReason::SoftDrop, amount, .. } if *amount == 2
        )));
        s.apply(Command::SoftDrop(false));
        assert!(!s.soft_dropping());
    }

    #[test]
    fn hard_drop_lands_locks_and_pays_two_per_cell() {
        let mut s = playing(7);
        let distance = (s.ghost().unwrap().row - s.active().unwrap().row) as u64;
        assert!(distance > 0);
        s.apply(Command::HardDrop);
        let events = s.drain_events();
        assert!(events.iter().any(|e| matches!(
            e,
            GameEvent::HardDropped { rows } if *rows as u64 == distance
        )));
        assert!(events.iter().any(|e| matches!(
            e,
            GameEvent::Scored { reason: ScoreReason::HardDrop, amount, total, .. }
                if *amount == distance * 2 && *total == distance * 2
        )));
        assert!(events.iter().any(|e| matches!(e, GameEvent::Locked { .. })));
        assert!(s.board().lowest_row().is_some());
        assert_eq!(s.stats().pieces, 1);
    }

    #[test]
    fn hard_drop_from_the_resting_position_pays_nothing() {
        let mut s = playing(8);
        ground(&mut s);
        let _ = s.drain_events();
        s.apply(Command::HardDrop);
        let events = s.drain_events();
        assert!(events
            .iter()
            .any(|e| matches!(e, GameEvent::HardDropped { rows: 0 })));
        assert!(!events.iter().any(|e| matches!(
            e,
            GameEvent::Scored {
                reason: ScoreReason::HardDrop,
                ..
            }
        )));
    }

    #[test]
    fn lock_delay_holds_a_grounding_piece_then_locks_it() {
        let mut s = playing(9);
        ground(&mut s);
        let pieces = s.stats().pieces;
        let _ = s.drain_events();
        s.tick(ms(400));
        assert_eq!(s.stats().pieces, pieces, "lock delay should keep it alive");
        assert!(
            s.lock_delay_remaining().is_some(),
            "and report the countdown"
        );
        s.tick(ms(150));
        assert_eq!(s.stats().pieces, pieces + 1, "it locked after the delay");
    }

    #[test]
    fn moving_while_grounded_refreshes_the_lock_delay() {
        let mut s = playing(10);
        ground(&mut s);
        let piece = s.active().unwrap();
        let dir = if s.board().fits_piece(&piece.moved(-1, 0)) {
            Direction::Left
        } else {
            Direction::Right
        };
        let pieces = s.stats().pieces;
        s.tick(ms(450));
        s.apply(Command::Move(dir));
        s.apply(Command::StopMove(dir));
        let _ = s.drain_events();
        s.tick(ms(450));
        assert_eq!(s.stats().pieces, pieces, "the touch bought another 500 ms");
        s.tick(ms(200));
        assert_eq!(s.stats().pieces, pieces + 1, "and then it locked");
    }

    #[test]
    fn reset_budget_is_bounded() {
        let mut s = playing(11);
        ground(&mut s);
        let budget = s.config().handling.max_lock_resets;
        for i in 0..budget + 6 {
            let piece = s.active().expect("still alive mid-budget");
            let dir = if s.board().fits_piece(&piece.moved(-1, 0)) {
                Direction::Left
            } else if s.board().fits_piece(&piece.moved(1, 0)) {
                Direction::Right
            } else {
                break;
            };
            s.apply(Command::Move(dir));
            s.apply(Command::StopMove(dir));
            let _ = s.drain_events();
            assert!(i < 32);
        }
        let pieces = s.stats().pieces;
        let mut ticks = 0;
        while s.stats().pieces == pieces && ticks < 100 {
            s.tick(ms(50));
            ticks += 1;
        }
        assert!(
            ticks < 20,
            "locked only after {ticks} ticks — the reset budget did not cap the hover"
        );
    }

    // ------------------------------------------------------------------ clears

    #[test]
    fn a_single_clear_scores_a_hundred() {
        let mut s = playing(12);
        fill_row_leaving(&mut s, 21, 9);
        let _ = s.drain_events();
        drop_vertical_i(&mut s, 9);
        let events = s.drain_events();
        assert!(events.iter().any(|e| matches!(
            e,
            GameEvent::Cleared { kind: ClearKind::Single, rows } if rows.len() == 1
        )));
        assert_eq!(find_scored(&events, ClearKind::Single), Some(100));
        assert_eq!(s.lines(), 1);
        assert!(s.board().full_rows().is_empty(), "no row may remain full");
    }

    #[test]
    fn a_tetris_scores_eight_hundred_and_is_counted() {
        let mut s = playing(13);
        // A stray cell outside the cleared rows keeps this from also being a perfect clear.
        s.board_mut().lock(&[Cell::new(0, 17)], 0).unwrap();
        for r in 18..22 {
            fill_row_leaving(&mut s, r, 9);
        }
        let _ = s.drain_events();
        drop_vertical_i(&mut s, 9);
        let events = s.drain_events();
        assert!(events.iter().any(|e| matches!(
            e,
            GameEvent::Cleared { kind: ClearKind::Tetris, rows } if rows.len() == 4
        )));
        assert_eq!(find_scored(&events, ClearKind::Tetris), Some(800));
        assert_eq!(s.lines(), 4);
        assert_eq!(s.stats().tetrises, 1);
        assert_eq!(
            s.stats().perfect_clears,
            0,
            "stray cell prevents a perfect clear"
        );
        // Hard-drop points ride on top of the clear value.
        assert!(s.score() > 800);
    }

    #[test]
    fn back_to_back_tetrises_pay_one_and_a_half() {
        let mut s = playing(14);
        for round in 0..2 {
            for r in 18..22 {
                fill_row_leaving(&mut s, r, 9);
            }
            let _ = s.drain_events();
            drop_vertical_i(&mut s, 9);
            let events = s.drain_events();
            // Each round empties the well, so the perfect-clear bonus is part of the number:
            // round 0: 800 + 2000;  round 1: 800×1.5 + 50 combo + 3200 (b2b perfect tetris).
            let want = if round == 0 {
                800 + 2000
            } else {
                1200 + 50 + 3200
            };
            assert_eq!(find_scored(&events, ClearKind::Tetris), Some(want));
            if round == 0 {
                assert!(!events
                    .iter()
                    .any(|e| matches!(e, GameEvent::BackToBack { .. })));
            } else {
                assert!(events.contains(&GameEvent::BackToBack { chain: 1 }));
                assert!(events.contains(&GameEvent::Combo { count: 1 }));
            }
            s.tick(ms(200));
            let _ = s.drain_events();
        }
        assert_eq!(s.lines(), 8);
        assert_eq!(s.back_to_back(), 2);
    }

    #[test]
    fn combo_bonus_grows_with_the_chain() {
        let mut s = playing(15);
        let mut expected = Vec::new();
        for i in 1..=3u64 {
            // A different open column each round: earlier rounds leave leftovers in the well.
            let open = (9 - (i - 1) as i32 + 10) % 10;
            fill_row_leaving(&mut s, 21, open);
            let _ = s.drain_events();
            drop_vertical_i(&mut s, open);
            let events = s.drain_events();
            let amount = find_scored(&events, ClearKind::Single)
                .unwrap_or_else(|| panic!("round {i}: no single clear\n{}", s.board().ascii()));
            expected.push(amount);
            // 100, then 100 + 50, then 100 + 100
            assert_eq!(amount, 100 + 50 * (i - 1), "chain position {i}");
            s.tick(ms(200));
            let _ = s.drain_events();
        }
        assert_eq!(expected.len(), 3);
        assert_eq!(s.combo(), 3);
    }

    #[test]
    fn a_lock_that_clears_nothing_breaks_the_combo() {
        let mut s = playing(16);
        fill_row_leaving(&mut s, 21, 9);
        drop_vertical_i(&mut s, 9);
        s.tick(ms(200));
        assert_eq!(s.combo(), 1);
        let _ = s.drain_events();

        // A piece dropped on empty floor clears nothing.
        ground(&mut s);
        s.apply(Command::HardDrop);
        let _ = s.drain_events();
        assert_eq!(s.combo(), 0);
    }

    #[test]
    fn a_perfect_clear_pays_the_bonus() {
        let mut s = playing(17);
        s.board_mut().clear();
        // Bottom row: six filled cells, four open in the middle for a horizontal I.
        for c in [0i32, 1, 2, 7, 8, 9] {
            s.board_mut().lock(&[Cell::new(c, 21)], 0).unwrap();
        }
        let mut piece = ActivePiece::spawn(Tetromino::I);
        piece.col = 3;
        piece.row = 20; // spawn cells land in row 21, columns 3..=6
        s.set_active(piece);
        let _ = s.drain_events();
        s.apply(Command::HardDrop);
        let events = s.drain_events();
        assert!(
            s.board().is_empty(),
            "board should be empty\n{}",
            s.board().ascii()
        );
        assert!(events.iter().any(|e| matches!(
            e,
            GameEvent::Scored {
                reason: ScoreReason::Clear { kind: ClearKind::Single, perfect: true, .. },
                amount,
                ..
            } if *amount == 100 + 800
        )));
        assert_eq!(s.stats().perfect_clears, 1);
    }

    #[test]
    fn level_up_happens_at_the_goal_and_speeds_gravity_up() {
        let mut s = playing(20);
        assert_eq!(s.level(), 1);
        let g0 = s.config().progression.gravity_ms(1);
        s.board_mut().lock(&[Cell::new(0, 17)], 0).unwrap();
        for round in 0..3 {
            for r in 18..22 {
                fill_row_leaving(&mut s, r, 9);
            }
            let _ = s.drain_events();
            drop_vertical_i(&mut s, 9);
            let events = s.drain_events();
            if round == 2 {
                assert!(
                    events.contains(&GameEvent::LevelUp { level: 2 }),
                    "{} lines should reach level 2, events: {events:?}",
                    s.lines()
                );
            } else {
                assert!(
                    !events
                        .iter()
                        .any(|e| matches!(e, GameEvent::LevelUp { .. })),
                    "no level change at {} lines: {events:?}",
                    s.lines()
                );
            }
            s.tick(ms(200));
            let _ = s.drain_events();
        }
        assert_eq!(s.lines(), 12);
        assert_eq!(s.level(), 2);
        assert!(s.config().progression.gravity_ms(2) < g0);
    }

    // ------------------------------------------------------------------- spins

    /// Put a T in Spawn orientation at box `(3, 19)` and rotate it 180° into the pocket below,
    /// so the lock's last action *is* a rotation (the precondition for any spin).
    fn rotate_t_into_pocket(s: &mut Session, corners: &[(i32, i32)]) {
        for (c, r) in corners {
            s.board_mut().lock(&[Cell::new(*c, *r)], 0).unwrap();
        }
        let mut piece = ActivePiece::spawn(Tetromino::T);
        piece.col = 3;
        piece.row = 19; // spawn cells: (4,19), (3..=5, 20)
        s.set_active(piece);
        s.apply(Command::Rotate(RotateDir::Flip));
        assert_eq!(s.active().unwrap().rot, Rotation::Flip);
    }

    #[test]
    fn a_full_t_spin_is_detected_and_paid() {
        let mut s = playing(21);
        // Flip pose occupies (3,20), (4,20), (5,20), (4,21); centre is (4,20).
        // Fill both *front* (lower) corners → full spin; (4,21) stays the pocket.
        // (3,19) is the third corner; the front pair (3,21)+(5,21) makes it a *full* spin.
        rotate_t_into_pocket(&mut s, &[(3, 19), (3, 21), (5, 21)]);
        let _ = s.drain_events();
        s.apply(Command::HardDrop);
        let events = s.drain_events();
        assert!(
            events.iter().any(|e| matches!(
                e,
                GameEvent::Locked {
                    piece: Tetromino::T,
                    spin: SpinKind::Full,
                    ..
                }
            )),
            "expected a full spin, got {:?}\n{}",
            events
                .iter()
                .find(|e| matches!(e, GameEvent::Locked { .. })),
            s.board().ascii()
        );
        assert_eq!(
            events.iter().find_map(|e| match e {
                GameEvent::Scored {
                    reason:
                        ScoreReason::Clear {
                            spin: SpinKind::Full,
                            ..
                        },
                    amount,
                    ..
                } => Some(*amount),
                _ => None,
            }),
            Some(400),
            "a silent T-spin pays 400 × level"
        );
        assert_eq!(s.stats().spins, 1);
    }

    #[test]
    fn a_mini_t_spin_is_detected_when_the_front_is_open() {
        let mut s = playing(22);
        // Three corners filled (both rear ones plus one front) → mini, not full.
        rotate_t_into_pocket(&mut s, &[(3, 19), (5, 19), (3, 21)]);
        let _ = s.drain_events();
        s.apply(Command::HardDrop);
        let events = s.drain_events();
        assert!(events.iter().any(|e| matches!(
            e,
            GameEvent::Locked {
                spin: SpinKind::Mini,
                ..
            }
        )));
    }

    #[test]
    fn a_plain_lock_is_not_a_spin() {
        let mut s = playing(23);
        s.apply(Command::HardDrop);
        let events = s.drain_events();
        assert!(events.iter().any(|e| matches!(
            e,
            GameEvent::Locked {
                spin: SpinKind::None,
                ..
            }
        )));
    }

    #[test]
    fn moving_after_rotating_cancels_the_spin() {
        let mut s = playing(24);
        // Mini setup: one front corner open, so the piece can slide right out of the pocket.
        rotate_t_into_pocket(&mut s, &[(3, 19), (5, 19), (3, 21)]);
        let piece = s.active().unwrap();
        assert!(
            s.board().fits_piece(&piece.moved(1, 0)),
            "setup should leave room to the right\n{}",
            s.board().ascii()
        );
        s.apply(Command::Move(Direction::Right));
        let _ = s.drain_events();
        s.apply(Command::HardDrop);
        let events = s.drain_events();
        assert!(events.iter().any(|e| matches!(
            e,
            GameEvent::Locked {
                spin: SpinKind::None,
                ..
            }
        )));
    }

    // ------------------------------------------------------------------ kicks

    #[test]
    fn rotation_uses_wall_kicks() {
        let mut s = playing(25);
        let mut piece = ActivePiece::spawn(Tetromino::I);
        piece.rot = Rotation::Cw; // cells sit in box column + 2
        piece.col = -2; // → board column 0
        piece.row = 5;
        s.set_active(piece);
        s.apply(Command::Rotate(RotateDir::Cw));
        let events = s.drain_events();
        let kick = events.iter().find_map(|e| match e {
            GameEvent::Rotated { kick, .. } => Some(*kick),
            _ => None,
        });
        assert!(kick.is_some(), "expected a wall kick, got {events:?}");
        assert_eq!(s.active().unwrap().rot, Rotation::Flip);
        assert!(s.board().fits_piece(&s.active().unwrap()));
    }

    #[test]
    fn blocked_rotation_reports_and_changes_nothing() {
        let mut s = playing(26);
        // A one-row tunnel: an I lying in row 20 between filled rows 19 and 21.
        {
            let board = s.board_mut();
            for c in 0..10 {
                board.lock(&[Cell::new(c, 19)], 0).unwrap();
                board.lock(&[Cell::new(c, 21)], 0).unwrap();
                if !(3..=6).contains(&c) {
                    board.lock(&[Cell::new(c, 20)], 0).unwrap();
                }
            }
        }
        let mut piece = ActivePiece::spawn(Tetromino::I);
        piece.col = 3;
        piece.row = 19; // spawn cells are box row + 1 → row 20, cols 3..=6
        s.set_active(piece);
        let before = s.active().unwrap();
        s.apply(Command::Rotate(RotateDir::Cw));
        let events = s.drain_events();
        assert!(events
            .iter()
            .any(|e| matches!(e, GameEvent::RotateBlocked { dir: RotateDir::Cw })));
        assert_eq!(s.active().unwrap(), before);
    }

    #[test]
    fn o_rotation_is_a_no_op_that_still_reports() {
        let mut s = playing(27);
        // Force an O by using set_active.
        s.set_active(ActivePiece::spawn(Tetromino::O));
        let before = s.active().unwrap();
        s.apply(Command::Rotate(RotateDir::Cw));
        let events = s.drain_events();
        assert!(events.iter().any(|e| matches!(
            e,
            GameEvent::Rotated {
                dir: RotateDir::Cw,
                kick: None
            }
        )));
        assert_eq!(s.active().unwrap(), before);
    }

    // ------------------------------------------------------------------- hold

    #[test]
    fn hold_swaps_once_then_rejects() {
        let mut s = playing(28);
        let first = s.active().unwrap().kind;
        let queued = s.preview();
        s.apply(Command::Hold);
        let events = s.drain_events();
        assert!(events
            .iter()
            .any(|e| matches!(e, GameEvent::HoldSwapped { .. })));
        assert_eq!(s.hold(), Some(first));
        let second = s.active().unwrap().kind;
        assert_eq!(second, queued[0], "empty hold deals from the queue");

        s.apply(Command::Hold);
        assert!(s.drain_events().contains(&GameEvent::HoldRejected));
        assert_eq!(s.active().unwrap().kind, second);
        assert_eq!(s.hold(), Some(first));
    }

    #[test]
    fn hold_can_be_used_again_after_the_next_lock() {
        let mut s = playing(29);
        s.apply(Command::Hold);
        let held = s.hold().unwrap();
        let _ = s.drain_events();

        ground(&mut s);
        s.apply(Command::HardDrop);
        let _ = s.drain_events();
        assert!(!s.hold_used(), "locking releases the hold");

        let current = s.active().unwrap().kind;
        s.apply(Command::Hold);
        assert_eq!(s.active().unwrap().kind, held);
        assert_eq!(s.hold(), Some(current));
    }

    #[test]
    fn hold_resets_rotation_and_position() {
        let mut s = playing(30);
        s.apply(Command::Rotate(RotateDir::Cw));
        s.apply(Command::Move(Direction::Left));
        let _ = s.drain_events();
        s.apply(Command::Hold);
        let p = s.active().unwrap();
        assert_eq!(p.rot, Rotation::Spawn);
        assert_eq!(p.row, p.kind.spawn_row());
        assert_eq!(p.col, p.kind.spawn_col());
    }

    // --------------------------------------------------------------- top out

    #[test]
    fn a_blocked_spawn_ends_the_match() {
        let mut s = playing(31);
        // Fill both spawn rows, leaving column 0 open so neither row counts as "complete"
        // (a complete row would clear itself and re-open the spawn area).
        {
            let board = s.board_mut();
            for r in 0..2 {
                for c in 1..10 {
                    let _ = board.lock(&[Cell::new(c, r)], 0);
                }
            }
        }
        // The piece is sitting in the spawn area; knock it out of the way and collect everything
        // the domain reports from the lock and the spawn that follows it.
        let mut events = Vec::new();
        s.apply(Command::HardDrop);
        events.extend(s.drain_events());
        s.tick(ms(200));
        events.extend(s.drain_events());
        assert!(
            events.iter().any(|e| matches!(
                e,
                GameEvent::GameOver {
                    reason: EndReason::SpawnBlocked | EndReason::LockedOut,
                    ..
                }
            )),
            "expected game over, got {events:?}"
        );
        assert_eq!(s.phase(), Phase::GameOver);
    }

    #[test]
    fn locking_entirely_above_the_field_is_a_lock_out() {
        let mut s = playing(32);
        // Fill every row: a piece that comes to rest with all cells above row 0 is a lock-out.
        {
            let board = s.board_mut();
            for r in 0..22 {
                for c in 0..10 {
                    let _ = board.lock(&[Cell::new(c, r)], 0);
                }
            }
        }
        let mut piece = ActivePiece::spawn(Tetromino::O);
        piece.col = 4;
        piece.row = -2; // cells occupy rows -2 and -1, resting on the filled row 0
        s.set_active(piece);
        let _ = s.drain_events();
        s.apply(Command::HardDrop);
        let events = s.drain_events();
        assert!(
            events.iter().any(|e| matches!(
                e,
                GameEvent::GameOver {
                    reason: EndReason::LockedOut,
                    ..
                }
            )),
            "expected a lock-out, got {events:?}"
        );
        assert_eq!(s.phase(), Phase::GameOver);
    }

    #[test]
    fn game_over_freezes_the_simulation() {
        let mut s = playing(33);
        s.quit();
        assert_eq!(s.phase(), Phase::GameOver);
        let f = s.fingerprint();
        for _ in 0..100 {
            s.tick(ms(50));
        }
        assert_eq!(s.fingerprint(), f, "a finished match must not move");
        s.apply(Command::Move(Direction::Left));
        s.apply(Command::HardDrop);
        assert_eq!(s.fingerprint(), f);
    }

    // ------------------------------------------------------------ pause gate

    #[test]
    fn pause_and_resume_gate_the_simulation() {
        let mut s = playing(34);
        s.apply(Command::Pause);
        assert_eq!(s.phase(), Phase::Paused);
        let f = s.fingerprint();
        for _ in 0..50 {
            s.tick(ms(50));
        }
        assert_eq!(s.fingerprint(), f, "paused time must not advance the game");
        assert!(s.drain_events().contains(&GameEvent::Paused));

        s.apply(Command::Resume);
        assert_eq!(s.phase(), Phase::Playing);
        for _ in 0..50 {
            s.tick(ms(50));
        }
        assert_ne!(s.fingerprint(), f, "a resumed match moves again");
        assert!(s.drain_events().contains(&GameEvent::Resumed));
    }

    #[test]
    fn pause_cancels_a_held_direction() {
        let mut s = playing(35);
        s.apply(Command::Move(Direction::Left));
        let _ = s.drain_events();
        s.apply(Command::Pause);
        s.apply(Command::Resume);
        let before = s.active().unwrap().col;
        for _ in 0..60 {
            s.tick(ms(20));
        }
        assert_eq!(
            s.active().unwrap().col,
            before,
            "DAS must not survive a pause"
        );
    }

    // ---------------------------------------------------------------- danger

    #[test]
    fn danger_fires_when_the_stack_crosses_the_threshold() {
        let mut s = playing(36);
        {
            let board = s.board_mut();
            for r in 4..22 {
                for c in 0..10 {
                    let _ = board.lock(&[Cell::new(c, r)], 0);
                }
            }
        }
        assert!(s.danger_level() >= s.config().danger_threshold);
        s.tick(TICK);
        assert!(s.drain_events().contains(&GameEvent::Danger(true)));
        // And clears once the stack comes back down.
        s.board_mut().clear();
        s.tick(TICK);
        assert!(s.drain_events().contains(&GameEvent::Danger(false)));
    }

    // ------------------------------------------------------------ entry delay

    #[test]
    fn the_next_piece_waits_for_the_entry_delay() {
        let mut s = playing_with_clear_delay(37, ms(120));
        for r in 18..22 {
            fill_row_leaving(&mut s, r, 9);
        }
        let _ = s.drain_events();
        drop_vertical_i(&mut s, 9);
        let events = s.drain_events();
        assert!(events.iter().any(|e| matches!(
            e,
            GameEvent::Cleared {
                kind: ClearKind::Tetris,
                ..
            }
        )));
        assert!(s.active().is_none(), "no piece during the delay");
        assert!(s.spawning());

        s.tick(ms(100));
        assert!(s.active().is_none(), "still waiting");
        s.tick(ms(30));
        assert!(s.active().is_some(), "dealt after the delay");
        assert!(s
            .drain_events()
            .iter()
            .any(|e| matches!(e, GameEvent::PieceSpawned { .. })));
    }

    #[test]
    fn a_hard_drop_pressed_during_the_delay_lands_on_the_new_piece() {
        let mut s = playing_with_clear_delay(38, ms(120));
        for r in 18..22 {
            fill_row_leaving(&mut s, r, 9);
        }
        drop_vertical_i(&mut s, 9);
        let pieces_after_first = s.stats().pieces;
        let _ = s.drain_events();

        s.apply(Command::HardDrop); // pressed while no piece is on screen
        assert!(s.active().is_none());
        s.tick(ms(150));
        assert!(
            s.stats().pieces > pieces_after_first,
            "the buffered drop should have locked the new piece"
        );
    }

    // ------------------------------------------------------------- determinism

    #[test]
    fn replay_reproduces_the_same_match_bit_for_bit() {
        let mut s = Session::with_seed(777);
        s.start(777);
        let mut rng = 0x1234_5678u64; // local, independent of the supply
        let mut restarts = 0u64;
        let mut locked_total = 0u64;
        let mut prev_pieces = 0u32;
        for i in 0..12_000u64 {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            // A dumb but busy policy: slide, spin, occasionally drop, hold now and then.
            match rng % 8 {
                0..=2 => s.apply(Command::Move(Direction::Left)),
                3..=5 => s.apply(Command::Move(Direction::Right)),
                6 => s.apply(Command::Rotate(RotateDir::Cw)),
                _ => {
                    if i % 3 == 0 {
                        s.apply(Command::HardDrop);
                    } else {
                        s.apply(Command::SoftDrop(i % 2 == 0));
                    }
                }
            }
            if i % 11 == 0 {
                s.apply(Command::Hold);
            }
            if i % 7 == 0 {
                s.apply(Command::StopMove(Direction::Left));
                s.apply(Command::StopMove(Direction::Right));
            }
            s.tick(TICK);
            let pieces = s.stats().pieces;
            if pieces > prev_pieces {
                locked_total += (pieces - prev_pieces) as u64;
            }
            prev_pieces = pieces;
            if s.phase() == Phase::GameOver {
                // Keep the soak going: a restart is just another logged command.
                restarts += 1;
                s.apply(Command::Start {
                    seed: 777 + restarts,
                });
                prev_pieces = 0;
            }
        }
        assert!(
            locked_total > 200,
            "soak locked only {locked_total} pieces over {restarts} matches — the policy is not \
             exercising the rules"
        );

        let expected = s.fingerprint();
        let end = s.tick_index();
        let replayed_final = |seed: u64| {
            let mut r = Session::replay(seed, SessionConfig::default(), s.input_log());
            // The live match ticked once after the final command; the replay must catch up.
            r.advance_to(end, TICK);
            r.fingerprint()
        };
        assert_eq!(expected, replayed_final(777), "replay diverged");
        assert!(
            restarts > 0,
            "the soak should have finished at least one match"
        );
        // NB: replaying under a *different* seed is deliberately not asserted here — the log
        // contains `Start { seed }` commands, so the runs converge by construction. Seed
        // sensitivity has its own test below.
    }

    #[test]
    fn the_seed_changes_the_game() {
        let run = |seed: u64| {
            let mut s = Session::with_seed(seed);
            s.start(seed);
            for _ in 0..900 {
                s.apply(Command::HardDrop);
                s.tick(TICK);
                s.tick(TICK);
                s.tick(TICK);
            }
            (s.fingerprint(), s.stats().lines, s.stats().score)
        };
        assert_ne!(run(1).0, run(2).0, "different seeds must play differently");
        assert_eq!(run(1).0, run(1).0, "and the same seed must not drift");
    }

    // ------------------------------------------------------------------ misc

    #[test]
    fn events_are_drained_exactly_once() {
        let mut s = playing(39);
        s.apply(Command::HardDrop);
        assert!(!s.drain_events().is_empty());
        assert!(s.drain_events().is_empty(), "second drain is empty");
    }

    #[test]
    fn preview_stays_the_configured_length() {
        let mut s = playing(40);
        assert_eq!(s.preview().len(), s.config().preview_size);
        for _ in 0..20 {
            s.apply(Command::HardDrop);
            s.tick(ms(200));
            let _ = s.drain_events();
            assert_eq!(s.preview().len(), s.config().preview_size);
        }
    }

    #[test]
    fn stats_report_the_match() {
        let mut s = playing(41);
        for r in 18..22 {
            fill_row_leaving(&mut s, r, 9);
        }
        drop_vertical_i(&mut s, 9);
        s.tick(ms(200));
        let stats = s.stats();
        assert_eq!(stats.lines, 4);
        assert_eq!(stats.tetrises, 1);
        assert!(stats.pieces >= 1);
        assert!(stats.score >= 800);
        assert!(stats.time > Duration::ZERO);
    }
}
