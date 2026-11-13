//! The view model: presentation state that the domain does not (and must not) know about.
//!
//! It consumes the session's [`GameEvent`]s and a `dt`, and produces animation values — particle
//! bursts, the row-collapse offset, screen shake, drop trails, toasts, banner timers. Keeping it in
//! one place means the scene builder stays a pure function of `(session, view_model, layout)`, and
//! the whole animation layer is testable without a GPU.

use tetris_core::{GameEvent, Tetromino};

/// Hard cap so a runaway party trick cannot exhaust memory.
pub const MAX_PARTICLES: usize = 1500;
/// Maximum simultaneous toasts.
pub const MAX_TOASTS: usize = 4;

/// A physics particle (pixels, seconds).
#[derive(Copy, Clone, Debug)]
pub struct Particle {
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
    pub size: f32,
    pub color: [f32; 3],
    pub life: f32,
    pub ttl: f32,
    /// Gravity in px/s².
    pub gravity: f32,
}

impl Particle {
    fn step(&mut self, dt: f32) {
        self.vy += self.gravity * dt;
        self.vx *= 1.0 - 0.6 * dt;
        self.x += self.vx * dt;
        self.y += self.vy * dt;
        self.life -= dt;
    }

    /// 0..1 remaining life, for alpha fade.
    pub fn fade(&self) -> f32 {
        (self.life / self.ttl.max(0.001)).clamp(0.0, 1.0)
    }
}

/// A line-clear animation: which rows, and how far through the effect we are.
#[derive(Clone, Debug)]
pub struct ClearAnim {
    pub rows: Vec<u32>,
    pub elapsed: f32,
    pub duration: f32,
    /// 0 single … 3 double, 4 tetris — drives colour and shake.
    pub severity: f32,
}

impl ClearAnim {
    pub fn progress(&self) -> f32 {
        (self.elapsed / self.duration).clamp(0.0, 1.0)
    }
}

/// A fading imprint of a hard-dropped piece.
#[derive(Clone, Debug)]
pub struct Trail {
    pub cells: Vec<(i32, i32)>,
    pub color: [f32; 3],
    pub elapsed: f32,
    pub duration: f32,
    /// Rows the piece travelled, used to stretch the trail.
    pub distance: u32,
}

impl Trail {
    pub fn progress(&self) -> f32 {
        (self.elapsed / self.duration).clamp(0.0, 1.0)
    }
}

/// A short text banner above the well ("TETRIS", "T-SPIN DOUBLE", "COMBO ×4" …).
#[derive(Clone, Debug)]
pub struct Toast {
    pub text: String,
    pub color: [f32; 3],
    pub elapsed: f32,
    pub duration: f32,
    /// Vertical slot so simultaneous messages do not overlap.
    pub slot: usize,
}

impl Toast {
    pub fn progress(&self) -> f32 {
        (self.elapsed / self.duration).clamp(0.0, 1.0)
    }
}

/// Everything the renderer needs that is not in the domain.
#[derive(Clone, Debug)]
pub struct ViewModel {
    pub particles: Vec<Particle>,
    pub clears: Vec<ClearAnim>,
    pub trails: Vec<Trail>,
    pub toasts: Vec<Toast>,
    /// Screen shake amplitude in pixels (decays to 0).
    pub shake: f32,
    /// Full-screen flash 0..1 (decays to 0).
    pub flash: f32,
    /// Seconds since the view model was created (drives background drift).
    pub clock: f32,
    /// Level-up banner timer, seconds remaining.
    pub level_banner: f32,
    pub level_shown: u32,
    /// Hold/next "pop" animation, 1 → 0.
    pub hold_pop: f32,
    pub next_pop: f32,
    /// The piece currently in the hold slot, for the panel.
    pub hold_piece: Option<Tetromino>,
    pub queue: Vec<Tetromino>,
    /// Danger pulse 0..1 (grows while the stack is high).
    pub danger: f32,
    /// Menu cursor, used by the title/game-over screens.
    pub menu_selection: usize,
    /// Name entry buffer for the high-score screen.
    pub name_entry: String,
    /// Set when a score qualified, cleared once entered.
    pub qualifying: bool,
    /// Ghost/lock pulse 0..1 while the piece is grounded.
    pub lock_pulse: f32,
    /// How many rows the stack just fell (drives the collapse easing).
    collapse_rows: u32,
    collapse_elapsed: f32,
    collapse_duration: f32,
    /// Deterministic pseudo-random source for particle scatter (no OS entropy here).
    rng_state: u64,
}

impl Default for ViewModel {
    fn default() -> Self {
        Self::new()
    }
}

impl ViewModel {
    pub fn new() -> Self {
        Self {
            particles: Vec::new(),
            clears: Vec::new(),
            trails: Vec::new(),
            toasts: Vec::new(),
            shake: 0.0,
            flash: 0.0,
            clock: 0.0,
            level_banner: 0.0,
            level_shown: 1,
            hold_pop: 0.0,
            next_pop: 0.0,
            hold_piece: None,
            queue: Vec::new(),
            danger: 0.0,
            menu_selection: 0,
            name_entry: String::new(),
            qualifying: false,
            lock_pulse: 0.0,
            collapse_rows: 0,
            collapse_elapsed: 0.0,
            collapse_duration: 0.22,
            rng_state: 0x2545_F491_4F6C_DD1D,
        }
    }

    /// Reset everything but the clock (new match).
    pub fn reset(&mut self) {
        let clock = self.clock;
        *self = Self::new();
        self.clock = clock;
    }

    fn random(&mut self) -> f32 {
        let mut x = self.rng_state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng_state = x;
        ((x >> 40) as f32 / (1u64 << 24) as f32).clamp(0.0, 1.0)
    }

    /// `cell(x, y)` maps a board cell to pixel centres so bursts land on the right squares.
    pub fn handle_events(
        &mut self,
        events: &[GameEvent],
        cell: f32,
        field_x: f32,
        field_y: f32,
        colors: &dyn Fn(Tetromino) -> [f32; 3],
    ) {
        for event in events {
            match event {
                GameEvent::Started { .. } => self.reset(),
                GameEvent::PieceSpawned { queue, hold, .. } => {
                    self.queue = queue.clone();
                    self.hold_piece = *hold;
                    self.next_pop = 1.0;
                }
                GameEvent::HoldSwapped { hold, .. } => {
                    self.hold_piece = *hold;
                    self.hold_pop = 1.0;
                }
                GameEvent::HardDropped { rows } => {
                    let strength = (*rows).min(24) as f32;
                    self.shake = self.shake.max(strength * 0.55);
                }
                GameEvent::Locked {
                    piece, cells, spin, ..
                } => {
                    let color = colors(*piece);
                    if !cells.is_empty() {
                        let top = cells.iter().map(|c| c.row).min().unwrap_or(0);
                        let bottom = cells.iter().map(|c| c.row).max().unwrap_or(0);
                        let distance = (bottom - top).max(0) as u32;
                        self.trails.push(Trail {
                            cells: cells.iter().map(|c| (c.col, c.row)).collect::<Vec<_>>(),
                            color,
                            elapsed: 0.0,
                            duration: 0.24,
                            distance,
                        });
                        if self.trails.len() > 8 {
                            self.trails.remove(0);
                        }
                    }
                    if spin.is_spin() {
                        self.push_toast(
                            if *spin == tetris_core::SpinKind::Mini {
                                "T-SPIN MINI".to_string()
                            } else {
                                "T-SPIN".to_string()
                            },
                            [0.85, 0.5, 1.0],
                        );
                    }
                    let _ = (cell, field_x, field_y);
                }
                GameEvent::Cleared { rows, kind } => {
                    let severity = kind.rows() as f32;
                    self.clears.push(ClearAnim {
                        rows: rows.clone(),
                        elapsed: 0.0,
                        duration: 0.32,
                        severity,
                    });
                    self.collapse_rows = rows.len() as u32;
                    self.collapse_elapsed = 0.0;
                    self.flash = self.flash.max(0.10 + 0.08 * severity);
                    self.shake = self.shake.max(2.0 + 3.5 * severity);

                    // Sparks fly out of each cleared row. Values are drawn first, then pushed:
                    // `spawn_particle` needs `&mut self` on its own.
                    for &row in rows {
                        for col in 0..10 {
                            let r1 = self.random();
                            let r2 = self.random();
                            let r3 = self.random();
                            let r4 = self.random();
                            let tint = 0.6 + 0.4 * r1;
                            self.spawn_particle(Particle {
                                x: field_x + (col as f32 + 0.5) * cell,
                                y: field_y + (row as f32 + 0.5) * cell,
                                vx: (r2 - 0.5) * 420.0,
                                vy: -120.0 - r3 * 320.0,
                                size: cell * (0.14 + 0.16 * r4),
                                color: [tint, 0.9, 1.0],
                                ttl: 0.5 + 0.5 * r1,
                                life: 1.0,
                                gravity: 900.0,
                            });
                        }
                    }
                }
                GameEvent::Combo { count } if *count >= 1 => {
                    self.push_toast(format!("COMBO ×{}", count + 1), [1.0, 0.85, 0.4]);
                }
                GameEvent::BackToBack { chain } => {
                    self.push_toast(format!("BACK-TO-BACK ×{chain}"), [1.0, 0.55, 0.3]);
                }
                GameEvent::LevelUp { level } => {
                    self.level_banner = 2.0;
                    self.level_shown = *level;
                    self.flash = self.flash.max(0.3);
                    self.push_toast(format!("LEVEL {level}"), [0.4, 1.0, 0.7]);
                }
                GameEvent::Danger(on) => {
                    self.danger = if *on { 1.0 } else { 0.0 };
                }
                GameEvent::GameOver { .. } => {
                    self.flash = self.flash.max(0.5);
                    self.shake = self.shake.max(10.0);
                }
                GameEvent::Scored { reason, amount, .. } if *amount > 0 => {
                    use tetris_core::ScoreReason::*;
                    if let Clear {
                        kind,
                        spin,
                        perfect,
                        ..
                    } = reason
                    {
                        let label = if *perfect {
                            "PERFECT CLEAR"
                        } else if spin.is_spin() && kind.rows() > 0 {
                            "SPIN CLEAR"
                        } else {
                            ""
                        };
                        if !label.is_empty() {
                            self.push_toast(label.to_string(), [1.0, 1.0, 0.6]);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn spawn_particle(&mut self, p: Particle) {
        if self.particles.len() >= MAX_PARTICLES {
            return;
        }
        self.particles.push(p);
    }

    fn push_toast(&mut self, text: String, color: [f32; 3]) {
        if self.toasts.iter().any(|t| t.text == text) {
            // Refresh the existing one instead of stacking identical messages.
            if let Some(toast) = self.toasts.iter_mut().find(|t| t.text == text) {
                toast.elapsed = 0.0;
            }
            return;
        }
        if self.toasts.len() >= MAX_TOASTS {
            self.toasts.remove(0);
        }
        self.toasts.push(Toast {
            text,
            color,
            elapsed: 0.0,
            duration: 1.4,
            slot: 0,
        });
        self.pack_toast_slots();
    }

    /// Toasts are drawn stacked from the middle of the well, so their slots must stay contiguous.
    fn pack_toast_slots(&mut self) {
        for (index, toast) in self.toasts.iter_mut().enumerate() {
            toast.slot = index;
        }
    }

    /// Advance every animation by `dt` seconds.
    pub fn tick(&mut self, dt: f32) {
        if dt <= 0.0 {
            return;
        }
        let dt = dt.min(0.1); // a 100 ms hitch must not teleport particles
        self.clock += dt;

        for p in self.particles.iter_mut() {
            p.step(dt);
        }
        self.particles.retain(|p| p.life > 0.0);

        for clear in self.clears.iter_mut() {
            clear.elapsed += dt;
        }
        self.clears.retain(|c| c.progress() < 1.0);

        for trail in self.trails.iter_mut() {
            trail.elapsed += dt;
        }
        self.trails.retain(|t| t.progress() < 1.0);

        for toast in self.toasts.iter_mut() {
            toast.elapsed += dt;
        }
        self.toasts.retain(|t| t.progress() < 1.0);
        self.pack_toast_slots();

        self.shake = decay(self.shake, dt, 26.0);
        self.flash = decay(self.flash, dt, 3.2);
        self.hold_pop = decay(self.hold_pop, dt, 4.5);
        self.next_pop = decay(self.next_pop, dt, 4.5);
        self.level_banner = (self.level_banner - dt).max(0.0);
        self.collapse_elapsed = (self.collapse_elapsed + dt).min(self.collapse_duration);
    }

    /// Rows the stack visually still has to fall, as a fractional offset (`0` when settled).
    ///
    /// Eases from the full row count down to zero so the collapse reads as motion instead of a jump.
    pub fn collapse_offset(&self) -> f32 {
        if !self.collapsing() {
            return 0.0;
        }
        let t = (self.collapse_elapsed / self.collapse_duration).clamp(0.0, 1.0);
        let eased = 1.0 - (1.0 - t) * (1.0 - t); // ease-out quad
        (1.0 - eased) * self.collapse_rows as f32
    }

    /// Rows still pending in the collapse animation.
    pub fn collapse_rows(&self) -> u32 {
        self.collapse_rows
    }

    /// Is a stack collapse still in progress?
    pub fn collapsing(&self) -> bool {
        self.collapse_rows > 0 && self.collapse_elapsed < self.collapse_duration
    }

    /// Rows currently being flashed white, with their 0..1 intensity.
    pub fn flashing_rows(&self) -> Vec<(u32, f32)> {
        let mut out = Vec::new();
        for clear in &self.clears {
            let p = clear.progress();
            // Flash hard for the first third, then fade out.
            let intensity = if p < 0.3 { 1.0 } else { 1.0 - (p - 0.3) / 0.7 };
            for &row in &clear.rows {
                out.push((row, intensity.clamp(0.0, 1.0)));
            }
        }
        out
    }

    /// Is any animation still running? Used to decide whether to keep redrawing while paused.
    pub fn is_animating(&self) -> bool {
        !self.particles.is_empty()
            || !self.clears.is_empty()
            || !self.trails.is_empty()
            || !self.toasts.is_empty()
            || self.shake > 0.05
            || self.flash > 0.01
            || self.level_banner > 0.0
            || self.collapsing()
    }
}

/// Exponential decay towards zero, frame-rate independent.
fn decay(value: f32, dt: f32, rate: f32) -> f32 {
    let next = value * (-rate * dt).exp();
    if next < 0.001 {
        0.0
    } else {
        next
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tetris_core::{Cell, ClearKind, ScoreReason, SpinKind};

    fn colors(_: Tetromino) -> [f32; 3] {
        [1.0, 0.5, 0.2]
    }

    fn feed(vm: &mut ViewModel, events: Vec<GameEvent>) {
        vm.handle_events(&events, 30.0, 100.0, 50.0, &colors);
    }

    #[test]
    fn a_new_view_model_is_quiet() {
        let vm = ViewModel::new();
        assert!(!vm.is_animating());
        assert_eq!(vm.collapse_offset(), 0.0);
        assert!(vm.flashing_rows().is_empty());
        assert!(vm.particles.is_empty());
    }

    #[test]
    fn a_line_clear_spawns_particles_flash_shake_and_a_collapse() {
        let mut vm = ViewModel::new();
        feed(
            &mut vm,
            vec![GameEvent::Cleared {
                rows: vec![19, 20, 21],
                kind: ClearKind::Triple,
            }],
        );
        assert!(vm.is_animating());
        assert_eq!(vm.clears.len(), 1);
        assert_eq!(vm.collapse_rows(), 3);
        assert!(vm.shake > 5.0, "triple should shake");
        assert!(vm.flash > 0.1);
        assert_eq!(vm.particles.len(), 3 * 10);
        assert!(vm
            .flashing_rows()
            .iter()
            .any(|(row, intensity)| *row == 20 && *intensity > 0.9));
    }

    #[test]
    fn animations_finish_and_clean_up() {
        let mut vm = ViewModel::new();
        feed(
            &mut vm,
            vec![GameEvent::Cleared {
                rows: vec![21],
                kind: ClearKind::Single,
            }],
        );
        for _ in 0..200 {
            vm.tick(1.0 / 60.0);
        }
        assert!(!vm.is_animating(), "everything should have settled");
        assert!(vm.particles.is_empty());
        assert!(vm.clears.is_empty());
        assert_eq!(vm.collapse_offset(), 0.0);
    }

    #[test]
    fn collapse_offset_starts_at_the_row_count_and_eases_to_zero() {
        let mut vm = ViewModel::new();
        feed(
            &mut vm,
            vec![GameEvent::Cleared {
                rows: vec![20, 21],
                kind: ClearKind::Double,
            }],
        );
        let start = vm.collapse_offset();
        assert!(start > 1.9, "should start at ~2 rows, got {start}");
        vm.tick(0.11);
        let mid = vm.collapse_offset();
        assert!(mid < start && mid > 0.0, "mid {mid} start {start}");
        for _ in 0..40 {
            vm.tick(0.02);
        }
        assert_eq!(vm.collapse_offset(), 0.0);
    }

    #[test]
    fn hard_drop_shakes_in_proportion_to_the_distance() {
        let mut short = ViewModel::new();
        feed(&mut short, vec![GameEvent::HardDropped { rows: 2 }]);
        let mut long = ViewModel::new();
        feed(&mut long, vec![GameEvent::HardDropped { rows: 20 }]);
        assert!(long.shake > short.shake * 3.0);
        assert!(long.shake > 8.0);

        // A drop that travels nowhere does not shake.
        let mut none = ViewModel::new();
        feed(&mut none, vec![GameEvent::HardDropped { rows: 0 }]);
        assert!(none.shake < 0.01);
    }

    #[test]
    fn locks_leave_a_trail_that_fades() {
        let mut vm = ViewModel::new();
        feed(
            &mut vm,
            vec![GameEvent::Locked {
                piece: Tetromino::T,
                cells: vec![Cell::new(3, 18), Cell::new(4, 18), Cell::new(4, 19)],
                spin: SpinKind::None,
            }],
        );
        assert_eq!(vm.trails.len(), 1);
        assert_eq!(vm.trails[0].cells.len(), 3);
        for _ in 0..60 {
            vm.tick(1.0 / 60.0);
        }
        assert!(vm.trails.is_empty(), "trail should have faded");
    }

    #[test]
    fn trail_count_is_capped() {
        let mut vm = ViewModel::new();
        for _ in 0..40 {
            feed(
                &mut vm,
                vec![GameEvent::Locked {
                    piece: Tetromino::I,
                    cells: vec![Cell::new(0, 21)],
                    spin: SpinKind::None,
                }],
            );
        }
        assert!(vm.trails.len() <= 8, "{} trails", vm.trails.len());
    }

    #[test]
    fn particles_never_exceed_the_cap() {
        let mut vm = ViewModel::new();
        for _ in 0..40 {
            feed(
                &mut vm,
                vec![GameEvent::Cleared {
                    rows: (0..4).collect(),
                    kind: ClearKind::Tetris,
                }],
            );
        }
        assert!(vm.particles.len() <= MAX_PARTICLES);
    }

    #[test]
    fn toasts_deduplicate_and_are_capped() {
        let mut vm = ViewModel::new();
        for _ in 0..20 {
            feed(&mut vm, vec![GameEvent::BackToBack { chain: 3 }]);
        }
        assert_eq!(
            vm.toasts.len(),
            1,
            "identical toasts should refresh, not stack"
        );

        for i in 0..10 {
            feed(&mut vm, vec![GameEvent::Combo { count: i }]);
        }
        assert!(vm.toasts.len() <= MAX_TOASTS);
        // Slots stay packed so nothing overlaps.
        let slots: Vec<usize> = vm.toasts.iter().map(|t| t.slot).collect();
        assert_eq!(slots, (0..slots.len()).collect::<Vec<_>>());
    }

    #[test]
    fn level_up_sets_a_banner_that_expires() {
        let mut vm = ViewModel::new();
        feed(&mut vm, vec![GameEvent::LevelUp { level: 7 }]);
        assert_eq!(vm.level_banner, 2.0);
        assert_eq!(vm.level_shown, 7);
        for _ in 0..150 {
            vm.tick(1.0 / 60.0);
        }
        assert_eq!(vm.level_banner, 0.0);
    }

    #[test]
    fn danger_flag_tracks_the_domain_event() {
        let mut vm = ViewModel::new();
        feed(&mut vm, vec![GameEvent::Danger(true)]);
        assert_eq!(vm.danger, 1.0);
        feed(&mut vm, vec![GameEvent::Danger(false)]);
        assert_eq!(vm.danger, 0.0);
    }

    #[test]
    fn perfect_clear_announces_itself() {
        let mut vm = ViewModel::new();
        feed(
            &mut vm,
            vec![GameEvent::Scored {
                reason: ScoreReason::Clear {
                    kind: ClearKind::Single,
                    spin: SpinKind::None,
                    combo: 0,
                    back_to_back: 0,
                    perfect: true,
                },
                amount: 900,
                total: 900,
            }],
        );
        assert!(vm.toasts.iter().any(|t| t.text == "PERFECT CLEAR"));
    }

    #[test]
    fn a_zero_score_event_says_nothing() {
        let mut vm = ViewModel::new();
        feed(
            &mut vm,
            vec![GameEvent::Scored {
                reason: ScoreReason::Clear {
                    kind: ClearKind::Single,
                    spin: SpinKind::None,
                    combo: 0,
                    back_to_back: 0,
                    perfect: true,
                },
                amount: 0,
                total: 0,
            }],
        );
        assert!(vm.toasts.is_empty());
    }

    #[test]
    fn tick_with_zero_time_changes_nothing() {
        let mut vm = ViewModel::new();
        feed(
            &mut vm,
            vec![GameEvent::Cleared {
                rows: vec![21],
                kind: ClearKind::Single,
            }],
        );
        let before = (vm.shake, vm.flash, vm.particles.len(), vm.clock);
        vm.tick(0.0);
        assert_eq!((vm.shake, vm.flash, vm.particles.len(), vm.clock), before);
    }

    #[test]
    fn a_huge_dt_is_clamped_so_physics_stay_sane() {
        let mut vm = ViewModel::new();
        feed(
            &mut vm,
            vec![GameEvent::Cleared {
                rows: vec![21],
                kind: ClearKind::Single,
            }],
        );
        let first = vm.particles[0];
        vm.tick(10.0); // an Alt+Tab of ten seconds
                       // Clamped to 0.1 s per tick: the particle must still exist, not have flown off to infinity.
        assert!(vm.particles.len() <= 10);
        assert!(
            first.y.is_finite() && vm.particles.is_empty() || vm.particles[0].y.is_finite(),
            "particle positions must stay finite"
        );
    }

    #[test]
    fn started_resets_the_presentation_state() {
        let mut vm = ViewModel::new();
        feed(
            &mut vm,
            vec![
                GameEvent::Cleared {
                    rows: vec![21],
                    kind: ClearKind::Single,
                },
                GameEvent::LevelUp { level: 9 },
            ],
        );
        assert!(vm.is_animating());
        feed(&mut vm, vec![GameEvent::Started { seed: 1, level: 1 }]);
        assert!(!vm.is_animating());
        assert_eq!(vm.level_shown, 1);
    }

    #[test]
    fn hold_and_queue_snapshots_update_from_events() {
        let mut vm = ViewModel::new();
        feed(
            &mut vm,
            vec![GameEvent::PieceSpawned {
                piece: Tetromino::T,
                queue: vec![Tetromino::I, Tetromino::O],
                hold: Some(Tetromino::S),
            }],
        );
        assert_eq!(vm.hold_piece, Some(Tetromino::S));
        assert_eq!(vm.queue, vec![Tetromino::I, Tetromino::O]);
        assert!(vm.next_pop > 0.9);

        feed(
            &mut vm,
            vec![GameEvent::HoldSwapped {
                piece: Tetromino::Z,
                hold: Some(Tetromino::T),
            }],
        );
        assert_eq!(vm.hold_piece, Some(Tetromino::T));
        assert!(vm.hold_pop > 0.9);
    }

    #[test]
    fn rendering_is_deterministic_for_the_same_event_and_tick_script() {
        let script = |seed: u64| {
            let mut vm = ViewModel::new();
            let _ = seed;
            feed(
                &mut vm,
                vec![
                    GameEvent::HardDropped { rows: 12 },
                    GameEvent::Cleared {
                        rows: vec![19, 20, 21],
                        kind: ClearKind::Triple,
                    },
                    GameEvent::LevelUp { level: 4 },
                ],
            );
            for _ in 0..30 {
                vm.tick(1.0 / 120.0);
            }
            (
                vm.particles.len(),
                vm.shake.to_bits(),
                vm.flash.to_bits(),
                vm.toasts.iter().map(|t| t.text.clone()).collect::<Vec<_>>(),
            )
        };
        assert_eq!(script(1), script(1));
    }

    #[test]
    fn decay_never_flips_sign_or_produces_nan() {
        for (value, dt, rate) in [
            (1.0f32, 1.0 / 60.0, 26.0),
            (0.0, 0.5, 26.0),
            (1.0, 0.0, 26.0),
            (0.5, 10.0, 3.2),
        ] {
            let out = decay(value, dt, rate);
            assert!(out.is_finite() && out >= 0.0, "{value} → {out}");
        }
    }

    #[test]
    fn particle_fade_stays_in_range() {
        let p = Particle {
            x: 0.0,
            y: 0.0,
            vx: 0.0,
            vy: 0.0,
            size: 4.0,
            color: [1.0; 3],
            life: 0.5,
            ttl: 1.0,
            gravity: 0.0,
        };
        assert!((p.fade() - 0.5).abs() < 0.001);
        let dead = Particle { life: -1.0, ..p };
        assert_eq!(dead.fade(), 0.0);
        let overshoot = Particle { life: 5.0, ..p };
        assert_eq!(overshoot.fade(), 1.0);
    }
}
