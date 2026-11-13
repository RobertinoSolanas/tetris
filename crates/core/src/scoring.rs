//! Scoring & progression: what a clear is worth, and how fast the pieces fall.
//!
//! Every number in the game lives here (Guideline-flavoured, data-driven). The session calls
//! [`ScoreState::apply_clear`] / [`Progression::gravity`] and never does arithmetic of its own, so
//! rebalancing the game is a change to this file plus its tests.

use std::time::Duration;

/// How many rows a single lock cleared.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ClearKind {
    None,
    Single,
    Double,
    Triple,
    Tetris,
}

impl ClearKind {
    pub fn from_rows(rows: usize) -> Self {
        match rows {
            0 => ClearKind::None,
            1 => ClearKind::Single,
            2 => ClearKind::Double,
            3 => ClearKind::Triple,
            _ => ClearKind::Tetris,
        }
    }

    pub const fn rows(self) -> usize {
        match self {
            ClearKind::None => 0,
            ClearKind::Single => 1,
            ClearKind::Double => 2,
            ClearKind::Triple => 3,
            ClearKind::Tetris => 4,
        }
    }

    /// A "hard clear" keeps a back-to-back chain alive: a Tetris, or any T-Spin that cleared rows.
    pub const fn is_hard(self) -> bool {
        matches!(self, ClearKind::Tetris)
    }

    pub const fn label(self) -> &'static str {
        match self {
            ClearKind::None => "",
            ClearKind::Single => "SINGLE",
            ClearKind::Double => "DOUBLE",
            ClearKind::Triple => "TRIPLE",
            ClearKind::Tetris => "TETRIS",
        }
    }
}

/// T-spin classification of a lock.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SpinKind {
    None,
    Mini,
    Full,
}

impl SpinKind {
    pub const fn is_spin(self) -> bool {
        !matches!(self, SpinKind::None)
    }

    pub const fn label(self) -> &'static str {
        match self {
            SpinKind::None => "",
            SpinKind::Mini => "T-SPIN MINI",
            SpinKind::Full => "T-SPIN",
        }
    }
}

/// What paid for a [`crate::GameEvent::Scored`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ScoreReason {
    SoftDrop,
    HardDrop,
    Clear {
        kind: ClearKind,
        spin: SpinKind,
        combo: u32,
        back_to_back: u32,
        perfect: bool,
    },
}

/// Itemised score for one clear, so the UI can show *why* the number moved.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct ScoreBreakdown {
    /// Clear/T-spin value after the back-to-back multiplier, before the level multiplier.
    pub base: u64,
    /// Level multiplier applied to `base` and `perfect`.
    pub level: u32,
    /// Bonus from the combo chain (already level-scaled).
    pub combo_bonus: u64,
    /// Perfect-clear bonus (already level-scaled).
    pub perfect_bonus: u64,
    /// Everything summed up.
    pub total: u64,
}

/// Pure data: the value of every outcome.
#[derive(Clone, Debug, PartialEq)]
pub struct ScoreTable {
    pub single: u64,
    pub double: u64,
    pub triple: u64,
    pub tetris: u64,
    pub tspin: u64,
    pub tspin_single: u64,
    pub tspin_double: u64,
    pub tspin_triple: u64,
    pub tspin_mini: u64,
    pub tspin_mini_single: u64,
    /// Multiplier applied to the base value while a back-to-back chain is alive.
    pub back_to_back_multiplier: f64,
    /// Per chained clearing piece, times level (Guideline: 50).
    pub combo_base: u64,
    pub perfect_single: u64,
    pub perfect_double: u64,
    pub perfect_triple: u64,
    pub perfect_tetris: u64,
    /// A perfect clear off a back-to-back tetris is the top prize.
    pub perfect_b2b_tetris: u64,
    pub soft_drop_per_cell: u64,
    pub hard_drop_per_cell: u64,
}

impl Default for ScoreTable {
    fn default() -> Self {
        Self::guideline()
    }
}

impl ScoreTable {
    /// The values from the Tetris Guideline, with T-spin triple added.
    pub const fn guideline() -> Self {
        Self {
            single: 100,
            double: 300,
            triple: 500,
            tetris: 800,
            tspin: 400,
            tspin_single: 800,
            tspin_double: 1200,
            tspin_triple: 1600,
            tspin_mini: 100,
            tspin_mini_single: 200,
            back_to_back_multiplier: 1.5,
            combo_base: 50,
            perfect_single: 800,
            perfect_double: 1200,
            perfect_triple: 1800,
            perfect_tetris: 2000,
            perfect_b2b_tetris: 3200,
            soft_drop_per_cell: 1,
            hard_drop_per_cell: 2,
        }
    }

    /// Base value of a (clear, spin) pair, before b2b/level/combo.
    pub fn base_value(&self, clear: ClearKind, spin: SpinKind) -> u64 {
        match (clear, spin) {
            (ClearKind::None, SpinKind::None) => 0,
            (ClearKind::None, SpinKind::Mini) => self.tspin_mini,
            (ClearKind::None, SpinKind::Full) => self.tspin,
            (ClearKind::Single, SpinKind::None) => self.single,
            (ClearKind::Double, SpinKind::None) => self.double,
            (ClearKind::Triple, SpinKind::None) => self.triple,
            (ClearKind::Tetris, SpinKind::None)
            | (ClearKind::Tetris, SpinKind::Mini)
            | (ClearKind::Tetris, SpinKind::Full) => self.tetris,
            (ClearKind::Single, SpinKind::Mini) => self.tspin_mini_single,
            (ClearKind::Single, SpinKind::Full) => self.tspin_single,
            (ClearKind::Double, SpinKind::Mini) => self.tspin_mini_single,
            (ClearKind::Double, SpinKind::Full) => self.tspin_double,
            (ClearKind::Triple, SpinKind::Mini) => self.tspin_mini_single,
            (ClearKind::Triple, SpinKind::Full) => self.tspin_triple,
            // Unreachable in play: a T piece cannot fill four rows on its own. Scored as a plain
            // tetris so the function stays total instead of hiding an `unreachable!`.
        }
    }

    fn perfect_value(&self, clear: ClearKind, b2b: bool) -> u64 {
        match (clear, b2b) {
            (ClearKind::Tetris, true) => self.perfect_b2b_tetris,
            (ClearKind::Tetris, false) => self.perfect_tetris,
            (ClearKind::Triple, _) => self.perfect_triple,
            (ClearKind::Double, _) => self.perfect_double,
            (ClearKind::Single, _) => self.perfect_single,
            (ClearKind::None, _) => 0,
        }
    }

    /// Value of a drop (`rows` cells, soft or hard). Not level-multiplied, per Guideline.
    pub fn drop_value(&self, rows: u32, soft: bool) -> u64 {
        let per_cell = if soft {
            self.soft_drop_per_cell
        } else {
            self.hard_drop_per_cell
        };
        per_cell.saturating_mul(rows as u64)
    }
}

/// Chain state across locks: combo and back-to-back.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScoreState {
    /// Total score.
    pub total: u64,
    /// Consecutive locks that cleared at least one row, including the current one.
    pub combo: u32,
    /// Consecutive hard clears, including the current one.
    pub b2b_chain: u32,
    /// Best combo seen (for the results screen).
    pub max_combo: u32,
    /// Best back-to-back chain seen.
    pub max_b2b: u32,
}

/// Result of applying one lock to [`ScoreState`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ClearOutcome {
    pub breakdown: ScoreBreakdown,
    /// 0 when no back-to-back bonus was paid, otherwise the 1-based chain index.
    pub back_to_back: u32,
    /// 0 when no combo bonus was paid, otherwise the 1-based chain index.
    pub combo: u32,
}

impl ScoreState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset chains *and* total (new match).
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Add drop points and return how many were added.
    pub fn apply_drop(&mut self, table: &ScoreTable, rows: u32, soft: bool) -> u64 {
        let amount = table.drop_value(rows, soft);
        self.total = self.total.saturating_add(amount);
        amount
    }

    /// Fold one lock into the score, updating combo and back-to-back chains.
    ///
    /// Rules (Guideline):
    /// * combo counts consecutive locks that cleared ≥ 1 row; a lock that clears nothing resets it;
    /// * the bonus is `combo_base × (combo − 1) × level`, i.e. paid from the second chained clear;
    /// * back-to-back counts consecutive *hard* clears; a non-hard clear breaks it, a lock that
    ///   clears nothing leaves it alone;
    /// * the b2b multiplier is applied to the base value before the level multiplier.
    pub fn apply_clear(
        &mut self,
        table: &ScoreTable,
        clear: ClearKind,
        spin: SpinKind,
        perfect: bool,
        level: u32,
    ) -> ClearOutcome {
        let rows = clear.rows();
        let hard_clear = rows > 0 && (clear.is_hard() || spin.is_spin());

        if rows == 0 && !spin.is_spin() {
            // Nothing cleared, no spin: combo breaks, b2b is untouched.
            self.combo = 0;
            return ClearOutcome {
                breakdown: ScoreBreakdown::default(),
                back_to_back: 0,
                combo: 0,
            };
        }

        // Combo counts locks that cleared rows only; a silent T-spin breaks it but still pays.
        let combo_index = if rows > 0 {
            self.combo += 1;
            self.max_combo = self.max_combo.max(self.combo);
            self.combo.saturating_sub(1)
        } else {
            self.combo = 0;
            0
        };

        let b2b_index = if hard_clear {
            self.b2b_chain += 1;
            self.max_b2b = self.max_b2b.max(self.b2b_chain);
            self.b2b_chain.saturating_sub(1)
        } else if rows > 0 {
            // A clear that is not "hard" breaks the chain.
            self.b2b_chain = 0;
            0
        } else {
            // Silent spin: neither extends nor breaks.
            0
        };

        let base_raw = table.base_value(clear, spin);
        let b2b_eligible = b2b_index > 0 && rows > 0;
        let with_b2b = if b2b_eligible {
            (base_raw as f64 * table.back_to_back_multiplier).floor() as u64
        } else {
            base_raw
        };
        let level = level.max(1);
        let base = with_b2b.saturating_mul(level as u64);
        let combo_bonus = table
            .combo_base
            .saturating_mul(combo_index as u64)
            .saturating_mul(level as u64);
        let perfect_bonus = if perfect {
            table
                .perfect_value(clear, b2b_index > 0)
                .saturating_mul(level as u64)
        } else {
            0
        };

        let breakdown = ScoreBreakdown {
            base,
            level,
            combo_bonus,
            perfect_bonus,
            total: base
                .saturating_add(combo_bonus)
                .saturating_add(perfect_bonus),
        };
        self.total = self.total.saturating_add(breakdown.total);

        ClearOutcome {
            breakdown,
            back_to_back: if b2b_eligible { b2b_index } else { 0 },
            combo: combo_index,
        }
    }
}

/// Level curve and gravity.
#[derive(Clone, Debug, PartialEq)]
pub struct Progression {
    /// Lines needed per level.
    pub lines_per_level: u32,
    /// Level the match starts at (≥ 1).
    pub start_level: u32,
    /// Floor on gravity, in ms per row. Defaults to one 120 Hz tick.
    pub min_gravity_ms: f64,
}

impl Default for Progression {
    fn default() -> Self {
        Self::guideline()
    }
}

impl Progression {
    /// 10 lines per level, start at 1, gravity floor of one tick (~8.33 ms = 20 G).
    pub const fn guideline() -> Self {
        Self {
            lines_per_level: 10,
            start_level: 1,
            min_gravity_ms: 1000.0 / 120.0,
        }
    }

    /// Level implied by a lifetime line count.
    pub fn level_for(&self, lines: u32) -> u32 {
        let per = self.lines_per_level.max(1);
        self.start_level.max(1) + lines / per
    }

    /// Lines still needed to reach `level + 1`.
    pub fn lines_to_next_level(&self, lines: u32) -> u32 {
        let per = self.lines_per_level.max(1);
        per - (lines % per)
    }

    /// Milliseconds per natural drop step at `level`.
    ///
    /// The Guideline curve `(0.8 − (level − 1) × 0.007)^(level − 1)` seconds, clamped from below at
    /// [`Progression::min_gravity_ms`] so the game stays playable and tick-quantised.
    pub fn gravity_ms(&self, level: u32) -> f64 {
        let l = level.max(1);
        let exp = (l - 1) as f64;
        let base = 0.8 - (l - 1) as f64 * 0.007;
        let seconds = if base <= 0.0 { 0.0 } else { base.powf(exp) };
        (seconds * 1000.0).max(self.min_gravity_ms.max(1.0))
    }

    /// [`Progression::gravity_ms`] as a [`Duration`].
    pub fn gravity(&self, level: u32) -> Duration {
        Duration::from_secs_f64(self.gravity_ms(level) / 1000.0)
    }

    /// Soft-drop gravity: natural gravity divided by the soft-drop factor, never slower than
    /// natural gravity and never faster than the floor.
    pub fn soft_gravity(&self, level: u32, soft_drop_factor: f64) -> Duration {
        let natural = self.gravity_ms(level);
        let factor = if soft_drop_factor.is_finite() {
            soft_drop_factor.max(1.0)
        } else {
            1.0
        };
        let ms = (natural / factor).max(self.min_gravity_ms.max(1.0));
        Duration::from_secs_f64(ms / 1000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(clear: ClearKind, spin: SpinKind, perfect: bool, level: u32) -> (u64, u32, u32) {
        let t = ScoreTable::guideline();
        let mut s = ScoreState::new();
        // One isolated clear: no combo, no b2b.
        let o = s.apply_clear(&t, clear, spin, perfect, level);
        (o.breakdown.total, o.back_to_back, o.combo)
    }

    #[test]
    fn isolated_clear_values_at_level_one() {
        assert_eq!(outcome(ClearKind::Single, SpinKind::None, false, 1).0, 100);
        assert_eq!(outcome(ClearKind::Double, SpinKind::None, false, 1).0, 300);
        assert_eq!(outcome(ClearKind::Triple, SpinKind::None, false, 1).0, 500);
        assert_eq!(outcome(ClearKind::Tetris, SpinKind::None, false, 1).0, 800);
    }

    #[test]
    fn spin_values_at_level_one() {
        assert_eq!(outcome(ClearKind::None, SpinKind::Full, false, 1).0, 400);
        assert_eq!(outcome(ClearKind::None, SpinKind::Mini, false, 1).0, 100);
        assert_eq!(outcome(ClearKind::Single, SpinKind::Full, false, 1).0, 800);
        assert_eq!(outcome(ClearKind::Single, SpinKind::Mini, false, 1).0, 200);
        assert_eq!(outcome(ClearKind::Double, SpinKind::Full, false, 1).0, 1200);
        assert_eq!(outcome(ClearKind::Triple, SpinKind::Full, false, 1).0, 1600);
    }

    #[test]
    fn level_multiplies_the_base_value() {
        assert_eq!(outcome(ClearKind::Tetris, SpinKind::None, false, 5).0, 4000);
        assert_eq!(outcome(ClearKind::Single, SpinKind::None, false, 3).0, 300);
    }

    #[test]
    fn back_to_back_tetris_is_one_and_a_half() {
        let t = ScoreTable::guideline();
        let mut s = ScoreState::new();
        let first = s.apply_clear(&t, ClearKind::Tetris, SpinKind::None, false, 1);
        assert_eq!(first.breakdown.total, 800);
        assert_eq!(first.back_to_back, 0);

        let second = s.apply_clear(&t, ClearKind::Tetris, SpinKind::None, false, 1);
        assert_eq!(second.back_to_back, 1, "second hard clear starts b2b");
        // 800 * 1.5 = 1200, plus combo 50 for the second chained clear.
        assert_eq!(second.breakdown.total, 1250);
        assert_eq!(second.combo, 1);

        let third = s.apply_clear(&t, ClearKind::Tetris, SpinKind::None, false, 1);
        assert_eq!(third.back_to_back, 2);
        assert_eq!(third.breakdown.total, 1200 + 100);
    }

    #[test]
    fn t_spin_keeps_back_to_back_alive() {
        let t = ScoreTable::guideline();
        let mut s = ScoreState::new();
        s.apply_clear(&t, ClearKind::Tetris, SpinKind::None, false, 1);
        let ts = s.apply_clear(&t, ClearKind::Double, SpinKind::Full, false, 1);
        assert_eq!(ts.back_to_back, 1, "T-spin double is a hard clear");
        // 1200 * 1.5 = 1800 + 50 combo
        assert_eq!(ts.breakdown.total, 1850);
    }

    #[test]
    fn a_plain_clear_breaks_back_to_back_but_not_the_chain_of_combos() {
        let t = ScoreTable::guideline();
        let mut s = ScoreState::new();
        s.apply_clear(&t, ClearKind::Tetris, SpinKind::None, false, 1);
        let plain = s.apply_clear(&t, ClearKind::Single, SpinKind::None, false, 1);
        assert_eq!(plain.back_to_back, 0, "b2b broken");
        assert_eq!(plain.combo, 1, "combo still alive");
        assert_eq!(plain.breakdown.total, 150);

        // And the next tetris is no longer b2b.
        let next = s.apply_clear(&t, ClearKind::Tetris, SpinKind::None, false, 1);
        assert_eq!(next.back_to_back, 0);
        assert_eq!(next.breakdown.total, 800 + 100);
    }

    #[test]
    fn a_lock_that_clears_nothing_breaks_the_combo_only() {
        let t = ScoreTable::guideline();
        let mut s = ScoreState::new();
        s.apply_clear(&t, ClearKind::Tetris, SpinKind::None, false, 1);
        let nothing = s.apply_clear(&t, ClearKind::None, SpinKind::None, false, 1);
        assert_eq!(nothing.breakdown.total, 0);
        assert_eq!(s.combo, 0);
        assert_eq!(s.b2b_chain, 1, "b2b survives a silent lock");

        let t2 = s.apply_clear(&t, ClearKind::Tetris, SpinKind::None, false, 1);
        assert_eq!(t2.back_to_back, 1, "chain was preserved");
        assert_eq!(t2.combo, 0, "combo restarted, so no combo bonus");
        assert_eq!(t2.breakdown.total, 1200);
    }

    #[test]
    fn silent_t_spin_scores_without_breaking_b2b() {
        let t = ScoreTable::guideline();
        let mut s = ScoreState::new();
        s.apply_clear(&t, ClearKind::Tetris, SpinKind::None, false, 1);
        // A spin with no clear goes through apply_clear too: it pays the spin value.
        let spin_only = s.apply_clear(&t, ClearKind::None, SpinKind::Full, false, 1);
        assert_eq!(spin_only.breakdown.total, 400, "a silent T-spin still pays");
        assert_eq!(spin_only.back_to_back, 0, "but does not extend the chain");
        assert_eq!(s.b2b_chain, 1, "chain survives");
        assert_eq!(s.combo, 0, "combo needs rows");
    }

    #[test]
    fn perfect_clear_bonus_is_level_scaled() {
        assert_eq!(
            outcome(ClearKind::Single, SpinKind::None, true, 1).0,
            100 + 800
        );
        assert_eq!(
            outcome(ClearKind::Tetris, SpinKind::None, true, 2).0,
            (800 + 2000) * 2
        );
    }

    #[test]
    fn perfect_b2b_tetris_is_the_top_prize() {
        let t = ScoreTable::guideline();
        let mut s = ScoreState::new();
        s.apply_clear(&t, ClearKind::Tetris, SpinKind::None, false, 1);
        let big = s.apply_clear(&t, ClearKind::Tetris, SpinKind::None, true, 1);
        assert_eq!(big.back_to_back, 1);
        // 1200 (b2b tetris) + 50 (combo) + 3200 (perfect b2b tetris)
        assert_eq!(big.breakdown.total, 1200 + 50 + 3200);
    }

    #[test]
    fn drop_points_are_not_level_scaled() {
        let t = ScoreTable::guideline();
        let mut s = ScoreState::new();
        assert_eq!(s.apply_drop(&t, 4, true), 4);
        assert_eq!(s.apply_drop(&t, 20, false), 40);
        assert_eq!(s.total, 44);
    }

    #[test]
    fn score_total_accumulates_and_saturates() {
        let t = ScoreTable::guideline();
        let mut s = ScoreState::new();
        for _ in 0..10 {
            s.apply_clear(&t, ClearKind::Tetris, SpinKind::None, false, 1);
        }
        assert!(s.total > 8000);
        s.total = u64::MAX - 1;
        s.apply_clear(&t, ClearKind::Tetris, SpinKind::None, false, 1);
        assert_eq!(s.total, u64::MAX, "score must saturate, never wrap");
    }

    #[test]
    fn best_chains_are_tracked() {
        let t = ScoreTable::guideline();
        let mut s = ScoreState::new();
        for _ in 0..4 {
            s.apply_clear(&t, ClearKind::Tetris, SpinKind::None, false, 1);
        }
        assert_eq!(s.max_combo, 4);
        assert_eq!(s.max_b2b, 4);
        s.apply_clear(&t, ClearKind::None, SpinKind::None, false, 1);
        assert_eq!(s.max_combo, 4, "records survive the reset");
    }

    #[test]
    fn clear_kind_from_rows() {
        assert_eq!(ClearKind::from_rows(0), ClearKind::None);
        assert_eq!(ClearKind::from_rows(1), ClearKind::Single);
        assert_eq!(ClearKind::from_rows(4), ClearKind::Tetris);
        assert_eq!(ClearKind::from_rows(9), ClearKind::Tetris, "clamped");
        assert_eq!(ClearKind::Tetris.rows(), 4);
        assert!(ClearKind::Tetris.is_hard());
        assert!(!ClearKind::Triple.is_hard());
    }

    #[test]
    fn levels_come_every_ten_lines() {
        let p = Progression::guideline();
        assert_eq!(p.level_for(0), 1);
        assert_eq!(p.level_for(9), 1);
        assert_eq!(p.level_for(10), 2);
        assert_eq!(p.level_for(19), 2);
        assert_eq!(p.level_for(20), 3);
        assert_eq!(p.level_for(120), 13);
        assert_eq!(p.lines_to_next_level(0), 10);
        assert_eq!(p.lines_to_next_level(7), 3);
        assert_eq!(p.lines_to_next_level(10), 10);
    }

    #[test]
    fn custom_level_size_is_respected_and_never_zero() {
        let p = Progression {
            lines_per_level: 5,
            ..Progression::guideline()
        };
        assert_eq!(p.level_for(5), 2);
        assert_eq!(p.level_for(9), 2);
        let z = Progression {
            lines_per_level: 0,
            ..Progression::guideline()
        };
        assert_eq!(z.level_for(0), 1);
        assert_eq!(z.level_for(1), 2);
    }

    #[test]
    fn gravity_monotonically_speeds_up_and_is_clamped() {
        let p = Progression::guideline();
        let mut previous = f64::INFINITY;
        for level in 1..=25 {
            let g = p.gravity_ms(level);
            assert!(g.is_finite() && g > 0.0, "level {level}: {g}");
            assert!(g <= previous, "level {level} got slower: {g} > {previous}");
            assert!(g >= p.min_gravity_ms - f64::EPSILON, "level {level}: {g}");
            previous = g;
        }
        // Guideline landmarks.
        assert!((p.gravity_ms(1) - 1000.0).abs() < 1.0);
        // (0.8 − 4·0.007)^4 = 0.772^4 ≈ 0.355 s
        assert!((p.gravity_ms(5) - 355.0).abs() < 6.0);
        assert!((p.gravity_ms(10) - 65.0).abs() < 6.0);
        assert!(p.gravity_ms(20) <= p.min_gravity_ms * 1.001);
    }

    #[test]
    fn soft_gravity_is_faster_but_bounded() {
        let p = Progression::guideline();
        let natural = p.gravity(3);
        let soft = p.soft_gravity(3, 20.0);
        assert!(soft < natural);
        assert!(soft >= Duration::from_secs_f64(p.min_gravity_ms / 1000.0));
        // factor 1 means "no boost", and nonsense factors must not panic or invert.
        assert_eq!(p.soft_gravity(3, 1.0), natural);
        let weird = p.soft_gravity(3, f64::NAN);
        assert!(weird.as_secs_f64() > 0.0);
    }
}
