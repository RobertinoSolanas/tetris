//! The shell: window, event loop, frame scheduling, and the wiring between contexts.
//!
//! Two run modes live here:
//! * [`run`] — the real game, a winit `ApplicationHandler` driving a fixed-timestep simulation.
//! * [`snapshot`] — render two frames (gameplay and title) offscreen to PNG and exit, which is how
//!   the look is verified without a human at the keyboard (docs/adr/0010).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tetris_core::{
    ActivePiece, Cell, ClearKind, Command, GameEvent, HandlingConfig, Progression, RotateDir,
    Rotation, ScoreReason, Session, SessionConfig, Stats, Tetromino,
};
use tetris_render::{Font, Renderer};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

use crate::cli::Options;
use crate::input::{translate, Action};
use crate::layout::Layout;
use crate::persistence::{ScoreBoard, ScoreEntry, Settings, Store};
use crate::scene_builder::{self, BuildContext, FrameInfo, Screen};
use crate::view_model::ViewModel;

/// Seconds per simulation step; 120 Hz keeps DAS/ARR and 20 G gravity well quantised.
const SIM_TICK: Duration = Duration::from_nanos(1_000_000_000 / 120);
/// Never simulate more than this many steps in one frame (an Alt+Tab guard).
const MAX_STEPS_PER_FRAME: usize = 8;

/// Map one domain event onto a sound, if it deserves one.
pub fn cue_for(event: &GameEvent) -> Option<tetris_audio::Cue> {
    use tetris_audio::Cue;
    match event {
        GameEvent::Started { .. } => Some(Cue::MenuSelect),
        GameEvent::Moved { .. } => Some(Cue::Move),
        GameEvent::Rotated { .. } => Some(Cue::Rotate),
        GameEvent::RotateBlocked { .. } => None,
        GameEvent::HardDropped { rows } if *rows > 0 => Some(Cue::Lock),
        GameEvent::Locked { spin, .. } if spin.is_spin() => Some(Cue::Spin),
        GameEvent::Locked { .. } => Some(Cue::Lock),
        GameEvent::Cleared { kind, .. } => Some(match kind.rows() {
            4 => Cue::Tetris,
            _ => Cue::Clear(kind.rows() as u32),
        }),
        GameEvent::HoldSwapped { .. } => Some(Cue::Hold),
        GameEvent::HoldRejected => Some(Cue::HoldDenied),
        GameEvent::Combo { count } if *count >= 2 => Some(Cue::Combo),
        GameEvent::BackToBack { .. } => Some(Cue::BackToBack),
        GameEvent::LevelUp { .. } => Some(Cue::LevelUp),
        GameEvent::Danger(true) => Some(Cue::Danger),
        GameEvent::Paused => Some(Cue::Pause),
        GameEvent::Resumed => Some(Cue::Resume),
        GameEvent::GameOver { .. } => Some(Cue::GameOver),
        _ => None,
    }
}

/// A fresh session built from the settings.
fn make_session(settings: &Settings, seed: u64) -> Session {
    let config = SessionConfig {
        handling: HandlingConfig::standard(),
        progression: Progression {
            lines_per_level: settings.lines_per_level.max(1),
            ..Progression::guideline()
        },
        ..SessionConfig::default()
    };
    Session::new(seed, config)
}

fn seed_from_clock() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0xC0FFEE)
}

/// The game application state.
pub struct App {
    options: Options,
    store: Store,
    settings: Settings,
    scores: ScoreBoard,
    session: Session,
    vm: ViewModel,
    layout: Layout,
    scene: tetris_render::Scene,
    font: Font,
    renderer: Option<Renderer>,
    window: Option<Arc<Window>>,
    screen: Screen,
    audio: Box<dyn tetris_audio::Sink>,
    last_frame: Option<Instant>,
    accumulator: Duration,
    sim_ms: f32,
    fps: f32,
    frames: u32,
    time: f32,
    quit: bool,
    /// Consecutive frames where the swapchain refused to give us an image. A minimise produces a
    /// handful of these; only a sustained failure means the window is really gone.
    surface_failures: u32,
    /// Has the window ever actually held the input focus? Some backends (X11 behind a compositor)
    /// deliver a focus-out before the first focus-in, and honouring that would pause a match the
    /// player has not started looking at.
    had_focus: bool,
    /// Matches started by the scripted smoke-test player.
    smoke_matches: u32,
    /// Pieces locked and lines cleared across every match of this process (the smoke player
    /// restarts itself after a game over).
    smoke_pieces: u32,
    smoke_lines: u32,
    best_score: u64,
}

impl App {
    pub fn new(options: Options) -> Self {
        Self::new_with(options, Store::from_env())
    }

    /// Build an app on top of a specific data directory: tests, and anyone embedding the game.
    pub fn new_with(options: Options, store: Store) -> Self {
        let mut settings = store.load_settings();
        if options.no_crt {
            settings.crt = false;
        }
        let scores = store.load_scores();
        let seed = options.seed.unwrap_or_else(seed_from_clock);
        settings.last_seed = Some(seed);
        let session = make_session(&settings, seed);
        let font = Font::build();
        let layout = Layout::new(options.width as f32, options.height as f32, 1.0);
        let scene = tetris_render::Scene::new(layout.width, layout.height);
        let audio: Box<dyn tetris_audio::Sink> = if options.no_sound {
            Box::new(tetris_audio::SilentSink::default())
        } else {
            tetris_audio::default_sink(settings.muted)
        };
        println!(
            "audio: {}",
            if options.no_sound {
                "disabled (--no-sound)"
            } else {
                tetris_audio::describe_backend()
            }
        );
        Self {
            options,
            store,
            settings,
            scores,
            session,
            vm: ViewModel::new(),
            layout,
            scene,
            font,
            renderer: None,
            window: None,
            screen: Screen::Title,
            audio,
            last_frame: None,
            accumulator: Duration::ZERO,
            sim_ms: 0.0,
            fps: 0.0,
            frames: 0,
            time: 0.0,
            quit: false,
            surface_failures: 0,
            had_focus: false,
            smoke_matches: 0,
            smoke_pieces: 0,
            smoke_lines: 0,
            best_score: 0,
        }
    }

    // Read-only views, so the whole app can be driven and inspected from an integration test
    // without ever opening a window.
    pub fn screen(&self) -> Screen {
        self.screen
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    pub fn view_model(&self) -> &ViewModel {
        &self.vm
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn scores(&self) -> &ScoreBoard {
        &self.scores
    }

    /// Where this app persists settings and scores.
    pub fn store_dir(&self) -> std::path::PathBuf {
        self.store.dir().to_path_buf()
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    pub fn scene(&self) -> &tetris_render::Scene {
        &self.scene
    }

    /// Drive the app with one action (windowless: this is what the event loop does per key).
    pub fn apply_action(&mut self, action: Action) {
        self.handle_action(action);
    }

    /// Advance the simulation and animation by `dt` without drawing (headless stepping).
    pub fn step(&mut self, dt: Duration) {
        self.session.tick(dt);
        let events = self.session.drain_events();
        self.consume_events(&events);
        self.vm.tick(dt.as_secs_f32());
    }

    /// The scene for the current state — exposed so tests can assert on layout without a window.
    pub fn build_scene(&mut self) {
        let frame = FrameInfo {
            fps: self.fps,
            backend: self
                .renderer
                .as_ref()
                .map(|r| r.backend())
                .unwrap_or_else(|| "headless".to_string()),
            sim_ms: self.sim_ms,
        };
        let ctx = BuildContext {
            session: &self.session,
            vm: &self.vm,
            layout: &self.layout,
            screen: self.screen,
            settings: &self.settings,
            board: &self.scores,
            frame: &frame,
            font: &self.font,
        };
        scene_builder::build(&mut self.scene, &ctx);
    }

    fn leave_match(&mut self) {
        self.session.apply(Command::QuitToTitle);
        let events = self.session.drain_events();
        self.consume_events(&events);
        self.screen = Screen::Title;
        if self.options.smoke_test {
            self.quit = true;
        }
    }

    fn start_match(&mut self) {
        self.remember_progress();
        self.smoke_matches += 1;
        let seed = self.options.seed.unwrap_or_else(seed_from_clock);
        self.settings.last_seed = Some(seed);
        self.session = make_session(&self.settings, seed);
        self.vm.reset();
        self.session.start(seed);
        self.screen = Screen::Playing;
        let events = self.session.drain_events();
        self.consume_events(&events);
    }

    fn save_to_disk(&self) {
        // Failures here are not worth interrupting a game over: the score is still on screen.
        let _ = self.store.save_settings(&self.settings);
        let _ = self.store.save_scores(&self.scores);
    }

    fn consume_events(&mut self, events: &[GameEvent]) {
        if events.is_empty() {
            return;
        }
        self.vm.handle_events(
            events,
            self.layout.cell,
            self.layout.field_x,
            self.layout.field_y,
            &|piece: Tetromino| piece.color(),
        );
        for event in events {
            if let Some(cue) = cue_for(event) {
                self.audio.play(cue);
            }
            if let GameEvent::GameOver { stats, .. } = event {
                self.on_game_over(stats.clone());
            }
        }
    }

    fn on_game_over(&mut self, stats: Stats) {
        self.screen = Screen::GameOver;
        if self.scores.rank_of(stats.score).is_some() {
            self.vm.qualifying = true;
            self.vm.name_entry = String::new();
        }
    }

    fn commit_score(&mut self) {
        let stats = self.session.stats();
        if stats.score == 0 {
            self.vm.qualifying = false;
            return;
        }
        let name = if self.vm.name_entry.trim().is_empty() {
            self.settings.player.clone()
        } else {
            self.vm.name_entry.clone()
        };
        let entry = ScoreEntry {
            name: name.clone(),
            score: stats.score,
            lines: stats.lines,
            level: stats.level,
            pieces: stats.pieces,
            seconds: stats.time.as_secs() as u32,
        };
        if self.scores.submit(entry).is_some() {
            self.settings.player = name;
            self.save_to_disk();
        }
        self.vm.qualifying = false;
        self.vm.name_entry.clear();
    }

    fn handle_action(&mut self, action: Action) {
        match action {
            Action::Game(command) => {
                if self.screen == Screen::Playing {
                    let events = self.step_command(command);
                    self.consume_events(&events);
                }
            }
            Action::Confirm => match self.screen {
                Screen::Title => self.start_match(),
                Screen::GameOver => {
                    if self.vm.qualifying {
                        self.commit_score();
                    }
                    self.start_match();
                }
                Screen::Paused => {
                    let events = self.step_command(Command::Resume);
                    self.screen = Screen::Playing;
                    self.consume_events(&events);
                }
                Screen::Playing => {}
            },
            Action::Back => match self.screen {
                Screen::Playing => {
                    let events = self.step_command(Command::Pause);
                    self.screen = Screen::Paused;
                    self.consume_events(&events);
                }
                // Escape on the pause menu abandons the match; Enter resumes it.
                Screen::Paused => self.leave_match(),
                Screen::GameOver | Screen::Title => self.leave_match(),
            },
            Action::Restart => self.start_match(),
            Action::Quit => self.quit = true,
            Action::TogglePause => match self.screen {
                Screen::Playing => {
                    let events = self.step_command(Command::Pause);
                    self.screen = Screen::Paused;
                    self.consume_events(&events);
                }
                Screen::Paused => {
                    let events = self.step_command(Command::Resume);
                    self.screen = Screen::Playing;
                    self.consume_events(&events);
                }
                _ => {}
            },
            Action::ToggleCrt => {
                self.settings.crt = !self.settings.crt;
                self.save_to_disk();
            }
            Action::ToggleMute => {
                self.settings.muted = !self.settings.muted;
                self.audio.set_muted(self.settings.muted);
                self.save_to_disk();
            }
            Action::ToggleDebug => {
                self.settings.show_debug = !self.settings.show_debug;
                self.save_to_disk();
            }
            Action::NameChar(ch) if self.vm.qualifying => {
                if self.vm.name_entry.len() < 12 {
                    self.vm.name_entry.push(ch.to_ascii_uppercase());
                }
            }
            Action::NameBackspace if self.vm.qualifying => {
                self.vm.name_entry.pop();
            }
            Action::NameSubmit => {
                if self.vm.qualifying {
                    self.commit_score();
                }
            }
            Action::Ignored | Action::NameChar(_) | Action::NameBackspace => {}
        }
    }

    /// Apply a command and advance one tick so keyboard-driven play feels immediate.
    fn step_command(&mut self, command: Command) -> Vec<GameEvent> {
        self.session.apply(command);
        self.session.tick(SIM_TICK);
        self.session.drain_events()
    }

    fn resize(&mut self, width: u32, height: u32) {
        let scale = self
            .window
            .as_ref()
            .map(|w| w.scale_factor() as f32)
            .unwrap_or(1.0);
        self.layout = Layout::new(width as f32, height as f32, scale);
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.resize(width, height);
        }
    }

    /// One frame: simulate, animate, build the scene, draw.
    fn frame(&mut self, event_loop: Option<&ActiveEventLoop>) {
        let now = Instant::now();
        let dt = self
            .last_frame
            .map(|last| now.duration_since(last))
            .unwrap_or(SIM_TICK);
        self.last_frame = Some(now);
        let dt = dt.min(Duration::from_millis(250));
        self.time += dt.as_secs_f32();

        // 1. Simulation at a fixed timestep (docs/adr/0005).
        let sim_start = Instant::now();
        if self.session.phase() == tetris_core::Phase::Playing || self.screen == Screen::Playing {
            self.accumulator += dt;
            let mut steps = 0;
            while self.accumulator >= SIM_TICK && steps < MAX_STEPS_PER_FRAME {
                self.accumulator -= SIM_TICK;
                steps += 1;
                self.session.tick(SIM_TICK);
                let events = self.session.drain_events();
                if !events.is_empty() {
                    self.consume_events(&events);
                }
            }
            if steps == MAX_STEPS_PER_FRAME {
                // Drop the backlog rather than spiralling.
                self.accumulator = Duration::ZERO;
            }
        }
        self.sim_ms = sim_start.elapsed().as_secs_f64() as f32 * 1000.0;

        // 2. Presentation animation runs even while paused (overlays keep breathing).
        self.vm.tick(dt.as_secs_f32());

        // 3. Scene + draw.
        self.build_scene();
        if let Some(renderer) = self.renderer.as_mut() {
            if let Some(surface) = renderer.surface() {
                match surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(frame)
                    | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                        let (tw, th) = (frame.texture.width(), frame.texture.height());
                        let view = frame
                            .texture
                            .create_view(&wgpu::TextureViewDescriptor::default());
                        renderer.render(&self.scene, self.time, &view);
                        if renderer.size() != (tw, th) {
                            renderer.resize(tw, th);
                        }
                        renderer.present(frame);
                        self.surface_failures = 0;
                    }
                    // The window changed shape: reconfigure and try again next frame.
                    wgpu::CurrentSurfaceTexture::Outdated => {
                        if let Some(window) = self.window.as_ref() {
                            let size: PhysicalSize<u32> = window.inner_size();
                            self.resize(size.width, size.height);
                        }
                    }
                    // Minimising or occluding the window is not an error: skip the frame and let
                    // the compositor bring it back. Only a sustained failure gives up.
                    other => {
                        self.surface_failures += 1;
                        if self.surface_failures == 1 {
                            eprintln!("surface unavailable ({other:?}), retrying");
                        }
                        if self.surface_failures > 240 {
                            eprintln!("surface never came back, closing");
                            self.quit = true;
                        }
                        if let Some(window) = self.window.as_ref() {
                            let size = window.inner_size();
                            if size.width > 0 && size.height > 0 {
                                self.resize(size.width, size.height);
                            }
                        }
                    }
                }
            }
        }

        // 4. Audio pump.
        self.audio.tick();

        // 5. Frame-rate bookkeeping and the smoke-test exit.
        if dt.as_secs_f64() > 0.0 {
            let instant = 1.0 / dt.as_secs_f64() as f32;
            self.fps = if self.fps == 0.0 {
                instant
            } else {
                self.fps * 0.9 + instant * 0.1
            };
        }
        self.frames += 1;

        if self.options.smoke_test && self.frames >= self.options.frames.max(1) {
            let stats = self.session.stats();
            println!(
                "smoke test: {} frames at {:.0} fps | {} match(es) | {} pieces locked | \
                 {} lines cleared | best score {} | final score {} | screen {:?} | {} live particles",
                self.frames,
                self.fps,
                self.smoke_matches,
                self.smoke_pieces + stats.pieces,
                self.smoke_lines + stats.lines,
                self.best_score.max(stats.score),
                stats.score,
                self.screen,
                self.vm.particles.len()
            );
            // The scripted player must actually have played, or the smoke test proves nothing.
            assert!(
                self.smoke_pieces + stats.pieces > 0,
                "smoke test ran {} frames without locking a single piece",
                self.frames
            );
            self.save_to_disk();
            self.quit = true;
        }
        if self.quit {
            self.save_to_disk();
            if let Some(event_loop) = event_loop {
                event_loop.exit();
            }
        } else if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }

    /// One step of the scripted smoke-test player, public so `cargo test` covers the same path the
    /// `--smoke-test` run takes.
    pub fn smoke_step_once(&mut self) {
        self.smoke_step();
    }

    /// Fold the current match's numbers into the process totals before it is thrown away.
    fn remember_progress(&mut self) {
        let stats = self.session.stats();
        self.smoke_pieces += stats.pieces;
        self.smoke_lines += stats.lines;
        self.best_score = self.best_score.max(stats.score);
    }

    /// Scripted player for `--smoke-test`: search a placement, then walk the active piece into it
    /// one key-press per frame. This exercises rotation, kicks, walking, locks, line clears, the
    /// scoring chain, particles and game-over → restart, on the real frame loop.
    fn smoke_step(&mut self) {
        if self.screen == Screen::GameOver {
            self.vm.qualifying = false;
            self.start_match();
            return;
        }
        let Some(active) = self.session.active() else {
            return;
        };
        let Some(plan) = plan_for(&self.session) else {
            // Nothing to place (the very first frame): nudge and wait.
            let events = self.step_command(Command::Move(tetris_core::Direction::Right));
            self.consume_events(&events);
            return;
        };

        if active.rot != plan.rot {
            let dir = if active.rot.cw() == plan.rot {
                RotateDir::Cw
            } else if active.rot.ccw() == plan.rot {
                RotateDir::Ccw
            } else {
                RotateDir::Flip
            };
            let events = self.step_command(Command::Rotate(dir));
            self.consume_events(&events);
            return;
        }

        if active.col != plan.col {
            let direction = if active.col < plan.col {
                tetris_core::Direction::Right
            } else {
                tetris_core::Direction::Left
            };
            let events = self.step_command(Command::Move(direction));
            self.consume_events(&events);
            if let Some(now) = self.session.active() {
                if now.col == active.col {
                    // Blocked sideways: take the drop where we are.
                    let events = self.step_command(Command::HardDrop);
                    self.consume_events(&events);
                }
            }
            let events = self.step_command(Command::StopMove(direction));
            self.consume_events(&events);
            return;
        }

        let events = self.step_command(Command::HardDrop);
        self.consume_events(&events);
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let attributes = Window::default_attributes()
            .with_title(format!("pi-tetris {} — neon edition", tetris_core::VERSION))
            .with_inner_size(LogicalSize::new(
                self.options.width as f64 * 0.5,
                self.options.height as f64 * 0.5,
            ))
            .with_min_inner_size(LogicalSize::new(360.0, 320.0));

        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(e) => {
                eprintln!("could not open a window: {e}");
                event_loop.exit();
                return;
            }
        };

        let size = window.inner_size();
        match Renderer::for_window(window.clone(), size.width, size.height) {
            Ok(renderer) => {
                println!(
                    "renderer: {} · surface {:?}",
                    renderer.adapter_info(),
                    renderer.output_format()
                );
                self.renderer = Some(renderer);
            }
            Err(e) => {
                eprintln!("{e}");
                event_loop.exit();
                return;
            }
        }

        self.window = Some(window);
        self.resize(size.width, size.height);
        self.last_frame = Some(Instant::now());

        if self.options.smoke_test {
            // Straight into a match so the smoke test exercises gameplay, not just menus.
            self.start_match();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                self.save_to_disk();
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                if size.width > 0 && size.height > 0 {
                    self.resize(size.width, size.height);
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                // Release events are meaningful (auto-shift), so do not filter them out wholesale.
                if matches!(event.state, ElementState::Released) && event.repeat {
                    return;
                }
                let actions = translate(&event.logical_key, event.state, event.repeat, self.screen);
                for action in actions {
                    self.handle_action(action);
                }
                if self.quit {
                    self.save_to_disk();
                    event_loop.exit();
                }
            }
            WindowEvent::RedrawRequested => {
                if self.options.smoke_test {
                    // The scripted player handles every screen, including restarting after a game
                    // over, so a long smoke run keeps exercising spawn/clear/score paths.
                    self.smoke_step();
                }
                self.frame(Some(event_loop));
            }
            WindowEvent::Focused(focused) => {
                if focused {
                    self.had_focus = true;
                } else if self.had_focus
                    && !self.options.smoke_test
                    && self.screen == Screen::Playing
                {
                    // Losing focus mid-match pauses it; nothing is lost, P resumes.
                    let events = self.step_command(Command::Pause);
                    self.screen = Screen::Paused;
                    self.consume_events(&events);
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        // Poll-driven; request a redraw so we keep going even without input.
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }
}

/// Run the game.
pub fn run(options: Options) -> Result<(), String> {
    let event_loop = EventLoop::new().map_err(|e| format!("event loop: {e}"))?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App::new(options);
    event_loop
        .run_app(&mut app)
        .map_err(|e| format!("event loop ended: {e}"))
}

// ------------------------------------------------------------- scripted smoke-test player
//
// `--smoke-test` needs a player that actually plays, otherwise the smoke run only proves the
// window opens. This is a textbook one-ply search: try every (rotation, column), drop the piece,
// score the resulting board, pick the best. It is deliberately simple and completely deterministic.

/// Where the scripted player wants the next piece: bounding-box column and rotation.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    /// Bounding-box column the piece should be walked to.
    pub col: i32,
    /// Rotation the piece should be turned into first.
    pub rot: Rotation,
}

/// Score a candidate board. Cleared rows dominate; then come aggregate height, bumpiness and holes.
pub fn evaluate(board: &tetris_core::Board) -> i32 {
    let mut heights = [0i32; 10];
    let mut holes = 0i32;
    for col in 0..10i32 {
        let mut seen_block = false;
        for row in 0..tetris_core::BOARD_ROWS as i32 {
            if board.color_at(col, row).is_some() {
                if !seen_block {
                    heights[col as usize] = tetris_core::BOARD_ROWS as i32 - row;
                    seen_block = true;
                }
            } else if seen_block {
                holes += 1;
            }
        }
    }
    let cleared = board.full_rows().len() as i32;
    let aggregate: i32 = heights.iter().sum();
    let bumpiness: i32 = heights.windows(2).map(|w| (w[0] - w[1]).abs()).sum();
    let max_height = heights.iter().copied().max().unwrap_or(0);
    cleared * 400 - aggregate - bumpiness * 4 - max_height * 2 - holes * 12
}

/// Best (rotation, column) for the piece in flight, or `None` if nothing fits.
pub fn plan_for(session: &Session) -> Option<Plan> {
    let active = session.active()?;
    let last_col = tetris_core::BOARD_COLS as i32 - active.kind.box_size();
    let mut best: Option<(i32, Plan)> = None;
    for rot in Rotation::ALL {
        for col in 0..=last_col {
            let candidate = ActivePiece {
                kind: active.kind,
                rot,
                col,
                row: 0,
            };
            let mut probe = session.board().clone();
            if !probe.fits_piece(&candidate) {
                continue;
            }
            let landed = probe.ghost(candidate);
            if probe
                .lock(&landed.cells(), active.kind.color_index() as u8)
                .is_err()
            {
                continue;
            }
            let score = evaluate(&probe);
            if best.as_ref().is_none_or(|(s, _)| score > *s) {
                best = Some((score, Plan { col, rot }));
            }
        }
    }
    best.map(|(_, plan)| plan)
}

// --------------------------------------------------------------------------- snapshot mode

/// Build a session that looks like real play: a jagged stack, a piece in flight, a fresh clear.
pub fn showcase_session(seed: u64) -> Session {
    let mut session = make_session(&Settings::default(), seed);
    session.start(seed);
    session.advance_to(1, SIM_TICK);

    // A stack with a four-row gap in the right-hand column.
    for row in 17..22 {
        for col in 0..9 {
            let pattern = (row + col) % 5 != 0;
            if pattern || row >= 18 {
                let piece = Tetromino::ALL[((row * 3 + col) % 7) as usize];
                let _ = session
                    .board_mut()
                    .lock(&[Cell::new(col, row)], piece.color_index() as u8);
            }
        }
    }
    // A couple of lone blocks higher up for texture.
    let _ = session
        .board_mut()
        .lock(&[Cell::new(2, 15)], Tetromino::L.color_index() as u8);
    let _ = session
        .board_mut()
        .lock(&[Cell::new(6, 14)], Tetromino::S.color_index() as u8);

    let mut piece = ActivePiece::spawn(Tetromino::I);
    piece.rot = Rotation::Cw;
    piece.col = 7;
    piece.row = 2;
    session.set_active(piece);
    session
}

/// Render two representative frames to `<path>` and `<path stem>-title.png`.
pub fn snapshot(options: &Options) -> Result<Vec<PathBuf>, String> {
    let path = options
        .snapshot
        .clone()
        .ok_or_else(|| "snapshot mode needs --snapshot <file>".to_string())?;
    let mut renderer =
        Renderer::headless(options.width, options.height).map_err(|e| e.to_string())?;
    let font = Font::build();
    let layout = Layout::new(options.width as f32, options.height as f32, 1.0);
    let settings = Settings {
        show_debug: false,
        ..Settings::default()
    };
    let scores = {
        let mut board = ScoreBoard::new();
        for (i, score) in [184_320, 152_100, 128_400, 96_700, 61_200]
            .iter()
            .enumerate()
        {
            board.submit(ScoreEntry {
                name: ["NEON", "ADA", "PIPE", "IVY", "ECHO"][i].to_string(),
                score: *score,
                lines: 120 - i as u32 * 17,
                level: 9 - i as u32,
                pieces: 300 - i as u32 * 40,
                seconds: 600 + i as u32 * 30,
            });
        }
        board
    };

    let mut written = Vec::new();

    // 1. Gameplay, mid-clear.
    let mut session = showcase_session(options.seed.unwrap_or(1234));
    let mut vm = ViewModel::new();
    let events = [
        GameEvent::HardDropped { rows: 16 },
        GameEvent::Locked {
            piece: Tetromino::I,
            cells: vec![
                Cell::new(9, 18),
                Cell::new(9, 19),
                Cell::new(9, 20),
                Cell::new(9, 21),
            ],
            spin: tetris_core::SpinKind::None,
        },
        GameEvent::Cleared {
            rows: vec![18, 19, 20, 21],
            kind: ClearKind::Tetris,
        },
        GameEvent::Scored {
            reason: ScoreReason::Clear {
                kind: ClearKind::Tetris,
                spin: tetris_core::SpinKind::None,
                combo: 1,
                back_to_back: 1,
                perfect: false,
            },
            amount: 1250,
            total: 12_480,
        },
        GameEvent::BackToBack { chain: 2 },
        GameEvent::Combo { count: 2 },
    ];
    vm.handle_events(&events, layout.cell, layout.field_x, layout.field_y, &|p| {
        p.color()
    });
    // Let the burst develop a moment so the still shows motion, then spawn the next piece.
    for _ in 0..6 {
        vm.tick(1.0 / 60.0);
    }
    session.advance_to(6, SIM_TICK);

    let mut scene = tetris_render::Scene::new(layout.width, layout.height);
    let frame = FrameInfo {
        fps: 60.0,
        backend: "Vulkan (snapshot)".into(),
        sim_ms: 0.35,
    };
    scene_builder::build(
        &mut scene,
        &BuildContext {
            session: &session,
            vm: &vm,
            layout: &layout,
            screen: Screen::Playing,
            settings: &settings,
            board: &scores,
            frame: &frame,
            font: &font,
        },
    );
    let (w, h, pixels) = renderer.render_to_image(&scene, 0.0);
    tetris_render::write_rgba(&path, w, h, &pixels)
        .map_err(|e| format!("write {}: {e}", path.display()))?;
    written.push(path.clone());

    // 2. Title screen.
    let title_session = make_session(&settings, options.seed.unwrap_or(1234));
    let mut title_vm = ViewModel::new();
    title_vm.clock = 1.4;
    let mut title_scene = tetris_render::Scene::new(layout.width, layout.height);
    scene_builder::build(
        &mut title_scene,
        &BuildContext {
            session: &title_session,
            vm: &title_vm,
            layout: &layout,
            screen: Screen::Title,
            settings: &settings,
            board: &scores,
            frame: &frame,
            font: &font,
        },
    );
    let title_path = with_suffix(&path, "-title");
    let (w, h, pixels) = renderer.render_to_image(&title_scene, 0.0);
    tetris_render::write_rgba(&title_path, w, h, &pixels)
        .map_err(|e| format!("write {}: {e}", title_path.display()))?;
    written.push(title_path);

    Ok(written)
}

/// Everything about a probe frame: the pixels *and* the layout they were drawn into, so tests can
/// assert "the well is here" rather than "some pixel is bright somewhere".
pub struct ProbeFrame {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    pub layout: Layout,
}

impl ProbeFrame {
    /// RGBA of one pixel.
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.width + x) as usize) * 4;
        [
            self.pixels[i],
            self.pixels[i + 1],
            self.pixels[i + 2],
            self.pixels[i + 3],
        ]
    }

    /// Rec.601 luma of one pixel, 0..255.
    pub fn luma(&self, x: u32, y: u32) -> u32 {
        let p = self.pixel(x, y);
        (p[0] as u32 * 299 + p[1] as u32 * 587 + p[2] as u32 * 114) / 1000
    }

    /// Brightest pixel inside a rectangle: `(x, y, luma)`.
    pub fn brightest_in(&self, x0: u32, y0: u32, x1: u32, y1: u32) -> (u32, u32, u32) {
        let mut best = (0u32, 0u32, 0u32);
        for y in y0.min(self.height)..y1.min(self.height) {
            for x in x0.min(self.width)..x1.min(self.width) {
                let l = self.luma(x, y);
                if l > best.2 {
                    best = (x, y, l);
                }
            }
        }
        best
    }

    /// How many pixels in a rectangle are brighter than `threshold`.
    pub fn lit_in(&self, x0: u32, y0: u32, x1: u32, y1: u32, threshold: u32) -> u32 {
        let mut count = 0;
        for y in y0.min(self.height)..y1.min(self.height) {
            for x in x0.min(self.width)..x1.min(self.width) {
                if self.luma(x, y) > threshold {
                    count += 1;
                }
            }
        }
        count
    }

    /// Mean absolute difference against another frame of the same size (sampled every 7 px).
    pub fn difference_vs(&self, other: &ProbeFrame) -> u64 {
        if (self.width, self.height) != (other.width, other.height) {
            return u64::MAX;
        }
        self.pixels
            .iter()
            .zip(other.pixels.iter())
            .step_by(7)
            .map(|(a, b)| (i16::from(*a) - i16::from(*b)).unsigned_abs() as u64)
            .sum()
    }
}

/// Render one frame offscreen for the layout-probe tests.
pub fn render_probe_frame(
    width: u32,
    height: u32,
    screen: Screen,
    crt: bool,
) -> Result<ProbeFrame, String> {
    let mut renderer = Renderer::headless(width, height).map_err(|e| e.to_string())?;
    let font = Font::build();
    let layout = Layout::new(width as f32, height as f32, 1.0);
    let settings = Settings {
        crt,
        ..Settings::default()
    };
    let session = if screen == Screen::Title {
        make_session(&settings, 4242)
    } else {
        showcase_session(4242)
    };
    let mut vm = ViewModel::new();
    vm.clock = 1.0;
    if screen == Screen::GameOver {
        vm.qualifying = true;
        vm.name_entry = "ADA".into();
    }
    let scores = ScoreBoard::new();
    let frame = FrameInfo::default();
    let mut scene = tetris_render::Scene::new(width as f32, height as f32);
    scene_builder::build(
        &mut scene,
        &BuildContext {
            session: &session,
            vm: &vm,
            layout: &layout,
            screen,
            settings: &settings,
            board: &scores,
            frame: &frame,
            font: &font,
        },
    );
    let (w, h, pixels) = renderer.render_to_image(&scene, 0.0);
    Ok(ProbeFrame {
        width: w,
        height: h,
        pixels,
        layout,
    })
}

fn with_suffix(path: &std::path::Path, suffix: &str) -> PathBuf {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "snapshot".to_string());
    let parent = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    parent.join(format!("{stem}{suffix}.png"))
}
