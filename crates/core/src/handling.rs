//! Handling & timing: the state machines behind game feel.
//!
//! Nothing here reads a clock — the session advances every machine with the `dt` it was given, which
//! is what makes "does holding left repeat at 20 ms?" a test instead of a vibe (docs/adr/0005).

use crate::actions::{Command, Direction};
use std::time::Duration;

/// Every tunable number about how input and gravity feel.
#[derive(Clone, Debug, PartialEq)]
pub struct HandlingConfig {
    /// Delayed Auto Shift: time from key-down to the first repeat.
    pub das: Duration,
    /// Auto Repeat Rate: time between repeats once DAS has fired.
    pub arr: Duration,
    /// Gravity divisor while soft drop is held (Guideline SDF).
    pub soft_drop_factor: f64,
    /// How long a grounded piece survives before locking.
    pub lock_delay: Duration,
    /// How many successful moves/rotations may refresh the lock delay (Guideline: 15).
    pub max_lock_resets: u32,
    /// Pause between a line clear and the next spawn, so the animation reads.
    pub line_clear_delay: Duration,
    /// Pause before a freshly spawned piece starts falling (Guideline "entry delay").
    pub spawn_delay: Duration,
    /// Window in which a command pressed during lock/clear delay is remembered and replayed.
    pub input_buffer: Duration,
}

impl Default for HandlingConfig {
    fn default() -> Self {
        Self::standard()
    }
}

impl HandlingConfig {
    /// A competitive-feeling setup: 133 ms DAS, 20 ms ARR, 500 ms lock delay with 15 resets.
    pub const fn standard() -> Self {
        Self {
            das: Duration::from_millis(133),
            arr: Duration::from_millis(20),
            soft_drop_factor: 20.0,
            lock_delay: Duration::from_millis(500),
            max_lock_resets: 15,
            line_clear_delay: Duration::from_millis(120),
            spawn_delay: Duration::ZERO,
            input_buffer: Duration::from_millis(100),
        }
    }

    /// Relaxed timings for showing the game to someone who has never played.
    pub const fn relaxed() -> Self {
        Self {
            das: Duration::from_millis(167),
            arr: Duration::from_millis(40),
            soft_drop_factor: 8.0,
            lock_delay: Duration::from_millis(500),
            max_lock_resets: 15,
            line_clear_delay: Duration::from_millis(220),
            spawn_delay: Duration::from_millis(60),
            input_buffer: Duration::from_millis(150),
        }
    }
}

/// Which phase of the tap → hold → repeat cycle we are in.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ShiftPhase {
    Idle,
    /// Waiting for DAS to elapse.
    Delay,
    /// Repeating at ARR.
    Repeat,
}

/// Horizontal auto-shift for one direction pair.
#[derive(Copy, Clone, Debug)]
pub struct AutoShift {
    held: Option<Direction>,
    phase: ShiftPhase,
    timer: Duration,
}

impl Default for AutoShift {
    fn default() -> Self {
        Self::new()
    }
}

impl AutoShift {
    pub const fn new() -> Self {
        Self {
            held: None,
            phase: ShiftPhase::Idle,
            timer: Duration::ZERO,
        }
    }

    pub const fn held(&self) -> Option<Direction> {
        self.held
    }

    pub const fn phase(&self) -> ShiftPhase {
        self.phase
    }

    /// Time until the next repeat is due (for tests and a potential UI indicator).
    pub const fn timer(&self) -> Duration {
        self.timer
    }

    /// A direction key went down. Returns `true` when the piece should step immediately.
    ///
    /// Ignored if the same direction is already held (auto-repeat is generated here, not by the OS).
    pub fn press(&mut self, dir: Direction, cfg: &HandlingConfig) -> bool {
        if self.held == Some(dir) {
            return false;
        }
        self.held = Some(dir);
        self.phase = ShiftPhase::Delay;
        self.timer = cfg.das;
        true
    }

    /// A direction key came up. Releases only if it is the held one (tap left while holding right).
    pub fn release(&mut self, dir: Direction) {
        if self.held == Some(dir) {
            self.held = None;
            self.phase = ShiftPhase::Idle;
            self.timer = Duration::ZERO;
        }
    }

    /// Advance by `dt`; returns how many repeat steps are due (0 most of the time).
    pub fn tick(&mut self, dt: Duration, cfg: &HandlingConfig) -> u32 {
        if self.held.is_none() || self.phase == ShiftPhase::Idle {
            return 0;
        }
        let arr = if cfg.arr.is_zero() {
            Duration::from_millis(1)
        } else {
            cfg.arr
        };
        if dt < self.timer {
            self.timer -= dt;
            return 0;
        }
        // A repeat is due; spend whatever time is left over on further ARR intervals, so a long
        // frame produces the same number of steps that many short frames would have.
        let mut overflow = dt - self.timer;
        let mut steps = 1u32;
        self.phase = ShiftPhase::Repeat;
        while overflow >= arr && steps < 8 {
            overflow -= arr;
            steps += 1;
        }
        // saturating: the repeat cap above can leave `overflow` larger than one ARR interval.
        self.timer = arr.saturating_sub(overflow);
        steps
    }

    /// "Step reset": after a step that actually moved the piece, restart the DAS countdown.
    ///
    /// This is the rule that makes a wall-hug slide feel snappy instead of running away.
    pub fn step_reset(&mut self, cfg: &HandlingConfig) {
        if self.held.is_some() {
            self.phase = ShiftPhase::Delay;
            self.timer = cfg.das;
        }
    }

    /// Forget everything (piece locked, pause, focus lost).
    pub fn cancel(&mut self) {
        *self = Self::new();
    }
}

/// Outcome of trying to refresh the lock delay.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Touch {
    /// Delay restarted; this many resets remain.
    Reset { resets_left: u32 },
    /// Budget exhausted: the delay keeps running and will lock on expiry.
    Exhausted,
    /// The piece was not grounded, nothing to refresh.
    NotArmed,
}

/// Lock delay with a reset budget (Guideline "lock delay" + "move reset").
#[derive(Clone, Debug, Default)]
pub struct LockTimer {
    armed: bool,
    elapsed: Duration,
    resets_left: u32,
}

impl LockTimer {
    pub const fn new() -> Self {
        Self {
            armed: false,
            elapsed: Duration::ZERO,
            resets_left: 0,
        }
    }

    pub const fn is_armed(&self) -> bool {
        self.armed
    }

    pub const fn elapsed(&self) -> Duration {
        self.elapsed
    }

    pub const fn resets_left(&self) -> u32 {
        self.resets_left
    }

    /// The piece just became grounded.
    pub fn arm(&mut self, cfg: &HandlingConfig) {
        self.armed = true;
        self.elapsed = Duration::ZERO;
        self.resets_left = cfg.max_lock_resets;
    }

    /// The piece is no longer grounded (it fell further): stop counting.
    pub fn disarm(&mut self) {
        *self = Self::new();
    }

    /// A successful move or rotation while grounded.
    pub fn touch(&mut self, _cfg: &HandlingConfig) -> Touch {
        if !self.armed {
            return Touch::NotArmed;
        }
        if self.resets_left == 0 {
            return Touch::Exhausted;
        }
        self.resets_left -= 1;
        self.elapsed = Duration::ZERO;
        Touch::Reset {
            resets_left: self.resets_left,
        }
    }

    /// Advance; `true` means "lock the piece now".
    pub fn tick(&mut self, dt: Duration, cfg: &HandlingConfig) -> bool {
        if !self.armed {
            return false;
        }
        self.elapsed += dt;
        if self.elapsed >= cfg.lock_delay {
            self.elapsed = cfg.lock_delay;
            return true;
        }
        false
    }
}

/// Gravity accumulator: converts `dt` into whole rows of falling.
#[derive(Clone, Debug, Default)]
pub struct DropTimer {
    elapsed: Duration,
}

impl DropTimer {
    pub const fn new() -> Self {
        Self {
            elapsed: Duration::ZERO,
        }
    }

    pub const fn elapsed(&self) -> Duration {
        self.elapsed
    }

    pub fn reset(&mut self) {
        self.elapsed = Duration::ZERO;
    }

    /// Rows that fell in this `dt` at `interval` per row; the remainder is carried over.
    pub fn tick(&mut self, dt: Duration, interval: Duration) -> u32 {
        if interval.is_zero() {
            // Infinity gravity: everything falls in one go and the caller hard-drops anyway.
            self.elapsed = Duration::ZERO;
            return u32::MAX;
        }
        self.elapsed += dt;
        let mut rows = 0u32;
        while self.elapsed >= interval && rows < u32::MAX {
            self.elapsed -= interval;
            rows += 1;
            if rows >= 4096 {
                break; // absurd dt (unpaused for a week?): cap, keep the remainder
            }
        }
        rows
    }
}

/// A single-slot buffer for commands pressed just before the piece locks or during the clear delay.
///
/// Time is counted in ticks so the buffer is deterministic (a tick is 1/120 s; see ADR-0005).
#[derive(Clone, Debug)]
pub struct InputBuffer {
    slot: Option<(Command, u64)>,
    /// How long a buffered command stays valid, in ticks.
    window_ticks: u64,
}

impl Default for InputBuffer {
    fn default() -> Self {
        Self::new(
            Duration::from_millis(100),
            Duration::from_millis(1000 / 120),
        )
    }
}

impl InputBuffer {
    pub fn new(window: Duration, tick: Duration) -> Self {
        let ticks = if tick.is_zero() {
            12
        } else {
            (window.as_secs_f64() / tick.as_secs_f64()).ceil() as u64
        };
        Self {
            slot: None,
            window_ticks: ticks.max(1),
        }
    }

    pub fn window_ticks(&self) -> u64 {
        self.window_ticks
    }

    pub fn is_empty(&self) -> bool {
        self.slot.is_none()
    }

    /// Remember `cmd` at tick `now`, replacing anything already buffered.
    pub fn push(&mut self, cmd: Command, now: u64) {
        self.slot = Some((cmd, now));
    }

    /// Take the buffered command if it has not expired.
    pub fn take(&mut self, now: u64) -> Option<Command> {
        match self.slot {
            None => None,
            Some((cmd, at)) => {
                self.slot = None;
                if now.saturating_sub(at) <= self.window_ticks {
                    Some(cmd)
                } else {
                    None
                }
            }
        }
    }

    /// Drop anything buffered (pause, lock of a different piece, restart).
    pub fn clear(&mut self) {
        self.slot = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TICK: Duration = Duration::from_millis(1000 / 120);

    fn ms(v: u64) -> Duration {
        Duration::from_millis(v)
    }

    fn cfg() -> HandlingConfig {
        HandlingConfig::standard()
    }

    #[test]
    fn tap_moves_exactly_once() {
        let c = cfg();
        let mut shift = AutoShift::new();
        assert!(shift.press(Direction::Left, &c), "immediate step");
        // Nothing more until DAS elapses.
        for _ in 0..10 {
            assert_eq!(shift.tick(ms(10), &c), 0);
        }
        assert_eq!(shift.tick(ms(10), &c), 0, "100 ms in, still nothing");
        shift.release(Direction::Left);
        assert_eq!(
            shift.tick(ms(100), &c),
            0,
            "released before DAS ⇒ never repeats"
        );
        assert_eq!(shift.held(), None);
    }

    #[test]
    fn holding_repeats_at_arr_after_das() {
        let c = cfg(); // das 133, arr 20
        let mut shift = AutoShift::new();
        shift.press(Direction::Right, &c);

        // Nothing at all until DAS elapses.
        assert_eq!(shift.tick(ms(132), &c), 0, "before DAS");
        // The DAS step fires, then exactly one step per ARR.
        assert_eq!(shift.tick(ms(1), &c), 1, "first repeat fires at DAS");
        assert_eq!(shift.tick(ms(20), &c), 1);
        assert_eq!(shift.tick(ms(20), &c), 1);
        assert_eq!(shift.tick(ms(19), &c), 0, "not a full ARR yet");
        assert_eq!(shift.tick(ms(1), &c), 1);
    }

    #[test]
    fn a_long_tick_can_produce_several_repeats() {
        let c = cfg();
        let mut shift = AutoShift::new();
        shift.press(Direction::Left, &c);
        // 133 ms DAS + 60 ms ⇒ 1 + 3 repeats in one giant frame.
        assert_eq!(shift.tick(ms(200), &c), 4);
    }

    #[test]
    fn pressing_the_opposite_direction_switches_immediately() {
        let c = cfg();
        let mut shift = AutoShift::new();
        shift.press(Direction::Left, &c);
        assert!(
            shift.press(Direction::Right, &c),
            "switch steps immediately"
        );
        assert_eq!(shift.held(), Some(Direction::Right));
        // Releasing the *old* key must not cancel the new direction.
        shift.release(Direction::Left);
        assert_eq!(shift.held(), Some(Direction::Right));
        shift.release(Direction::Right);
        assert_eq!(shift.held(), None);
    }

    #[test]
    fn duplicate_press_of_the_same_key_is_ignored() {
        let c = cfg();
        let mut shift = AutoShift::new();
        assert!(shift.press(Direction::Left, &c));
        assert!(
            !shift.press(Direction::Left, &c),
            "OS key-repeat must not double-step"
        );
    }

    #[test]
    fn step_reset_restarts_the_das_countdown() {
        let c = cfg();
        let mut shift = AutoShift::new();
        shift.press(Direction::Left, &c);
        assert_eq!(shift.tick(ms(120), &c), 0);
        shift.step_reset(&c);
        assert_eq!(shift.timer(), c.das, "DAS restarted");
        assert_eq!(
            shift.tick(ms(120), &c),
            0,
            "and we are 120 ms into a fresh DAS"
        );
        assert_eq!(shift.tick(ms(20), &c), 1);
    }

    #[test]
    fn cancel_clears_everything() {
        let c = cfg();
        let mut shift = AutoShift::new();
        shift.press(Direction::Left, &c);
        shift.cancel();
        assert_eq!(shift.held(), None);
        assert_eq!(shift.tick(ms(500), &c), 0);
    }

    #[test]
    fn zero_arr_does_not_spin_forever() {
        let c = HandlingConfig {
            arr: Duration::ZERO,
            ..HandlingConfig::standard()
        };
        let mut shift = AutoShift::new();
        shift.press(Direction::Left, &c);
        let steps = shift.tick(ms(500), &c);
        assert!((1..=8).contains(&steps), "clamped, got {steps}");
    }

    #[test]
    fn lock_delay_expires() {
        let c = cfg();
        let mut lock = LockTimer::new();
        assert!(!lock.is_armed());
        assert!(!lock.tick(ms(600), &c), "not armed ⇒ no lock");

        lock.arm(&c);
        assert!(lock.is_armed());
        assert_eq!(lock.resets_left(), 15);
        assert!(!lock.tick(ms(499), &c));
        assert!(lock.tick(ms(1), &c), "locks at exactly 500 ms");
    }

    #[test]
    fn touch_resets_the_delay_until_the_budget_runs_out() {
        let c = cfg();
        let mut lock = LockTimer::new();
        assert_eq!(lock.touch(&c), Touch::NotArmed, "needs arming first");

        lock.arm(&c);
        for i in 0..15 {
            assert!(!lock.tick(ms(400), &c));
            assert_eq!(
                lock.touch(&c),
                Touch::Reset {
                    resets_left: 14 - i
                },
                "reset {i}"
            );
            assert_eq!(lock.elapsed(), Duration::ZERO, "delay restarted");
        }
        assert_eq!(lock.resets_left(), 0);
        assert_eq!(lock.touch(&c), Touch::Exhausted);
        assert!(!lock.tick(ms(400), &c), "still has 100 ms of delay left");
        assert!(lock.tick(ms(100), &c), "and then it locks");
    }

    #[test]
    fn disarm_forgets_the_arming() {
        let c = cfg();
        let mut lock = LockTimer::new();
        lock.arm(&c);
        lock.tick(ms(400), &c);
        lock.disarm();
        assert!(!lock.is_armed());
        assert_eq!(lock.elapsed(), Duration::ZERO);
        lock.arm(&c);
        assert_eq!(lock.resets_left(), 15, "fresh piece gets a fresh budget");
    }

    #[test]
    fn drop_timer_counts_rows_and_keeps_the_remainder() {
        let mut drop = DropTimer::new();
        assert_eq!(drop.tick(ms(500), ms(1000)), 0);
        assert_eq!(drop.elapsed(), ms(500));
        assert_eq!(drop.tick(ms(500), ms(1000)), 1);
        assert_eq!(drop.elapsed(), Duration::ZERO);
        assert_eq!(drop.tick(ms(2500), ms(1000)), 2, "two rows, half carried");
        assert_eq!(drop.elapsed(), ms(500));
    }

    #[test]
    fn drop_timer_handles_gravity_faster_than_the_tick() {
        let mut drop = DropTimer::new();
        assert_eq!(
            drop.tick(TICK, ms(1)),
            8,
            "1 ms gravity, 8 ms tick ⇒ 8 rows"
        );
        let mut drop = DropTimer::new();
        assert_eq!(drop.tick(ms(100), ms(10)), 10);
        let mut drop = DropTimer::new();
        assert_eq!(
            drop.tick(ms(100), Duration::ZERO),
            u32::MAX,
            "infinity gravity"
        );
    }

    #[test]
    fn input_buffer_expires_after_its_window() {
        let mut buf = InputBuffer::new(ms(100), TICK); // ~12 ticks at 120 Hz
        assert!(buf.is_empty());
        assert_eq!(buf.take(0), None);

        buf.push(Command::Hold, 100);
        assert_eq!(buf.take(105), Some(Command::Hold));

        buf.push(Command::HardDrop, 1000);
        assert_eq!(buf.take(1012), Some(Command::HardDrop), "inside window");
        buf.push(Command::HardDrop, 1000);
        assert_eq!(buf.take(1200), None, "expired");

        buf.push(Command::Hold, 5);
        assert!(buf.take(900).is_none(), "stale entries never leak out");
    }

    #[test]
    fn input_buffer_holds_one_slot_and_can_be_cleared() {
        let mut buf = InputBuffer::new(ms(100), TICK);
        buf.push(Command::Hold, 0);
        buf.push(Command::Rotate(crate::RotateDir::Cw), 1);
        assert_eq!(buf.take(2), Some(Command::Rotate(crate::RotateDir::Cw)));
        assert!(buf.is_empty());

        buf.push(Command::Hold, 0);
        buf.clear();
        assert!(buf.is_empty());
        assert_eq!(buf.take(0), None);
    }

    #[test]
    fn input_buffer_window_rounding_is_sane() {
        assert!(InputBuffer::new(ms(100), TICK).window_ticks() >= 10);
        assert_eq!(InputBuffer::new(Duration::ZERO, TICK).window_ticks(), 1);
        assert_eq!(InputBuffer::new(ms(100), Duration::ZERO).window_ticks(), 12);
    }

    #[test]
    fn presets_are_ordered_and_sane() {
        let std = HandlingConfig::standard();
        let relax = HandlingConfig::relaxed();
        assert!(std.das < relax.das);
        assert!(std.arr < relax.arr);
        assert!(std.soft_drop_factor > relax.soft_drop_factor);
        assert_eq!(std.max_lock_resets, 15);
        assert!(std.lock_delay >= ms(500));
        assert!(!std.line_clear_delay.is_zero());
    }
}
