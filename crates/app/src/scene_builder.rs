//! Scene builder — the anti-corruption layer between the domain and the renderer.
//!
//! Everything in this module answers the same question: given what the game *is* (a `Session`) and
//! how it currently *looks mid-motion* (a `ViewModel`), what rectangles and strings go on screen?
//! The renderer never learns that Tetris exists (docs/adr/0006); all of that knowledge lives here,
//! as a pure function of `(session, view_model, layout, screen)`.

use tetris_core::{Phase, Session, Tetromino, BOARD_COLS, BOARD_ROWS, BUFFER_ROWS};
use tetris_render::{Font, Layer, Particle as SceneParticle, PostFx, Quad, Scene, Text};

use crate::layout::Layout;
use crate::persistence::{ScoreBoard, Settings};
use crate::rect::Rect;
use crate::theme as t;
use crate::view_model::ViewModel;

/// Which screen the player is looking at. Derived from [`Phase`] plus the app's own menu state.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Screen {
    Title,
    Playing,
    Paused,
    GameOver,
}

/// Per-frame diagnostics the debug overlay shows.
#[derive(Clone, Debug, Default)]
pub struct FrameInfo {
    pub fps: f32,
    pub backend: String,
    /// Milliseconds spent simulating last frame.
    pub sim_ms: f32,
}

/// Everything the builder needs. Borrowed, never mutated.
pub struct BuildContext<'a> {
    pub session: &'a Session,
    pub vm: &'a ViewModel,
    pub layout: &'a Layout,
    pub screen: Screen,
    pub settings: &'a Settings,
    pub board: &'a ScoreBoard,
    pub frame: &'a FrameInfo,
    pub font: &'a Font,
}

/// Rebuild `scene` for this frame.
pub fn build(scene: &mut Scene, ctx: &BuildContext<'_>) {
    scene.clear();
    scene.resize(ctx.layout.width, ctx.layout.height);
    scene.clear_color = t::SKY_BOTTOM;

    let bloom = if ctx.settings.crt {
        ctx.settings.bloom
    } else {
        0.0
    };
    scene.post = if ctx.settings.crt {
        PostFx {
            bloom,
            ..PostFx::arcade()
        }
    } else {
        PostFx {
            bloom,
            ..PostFx::off()
        }
    };
    scene.shake = ctx.vm.shake;
    scene.flash = ctx.vm.flash;

    background(scene, ctx);

    // The title screen is an attract page: no well, no HUD, just the wordmark, drifting pieces and
    // the honours board. Everything else draws the full game.
    if ctx.screen != Screen::Title {
        well(scene, ctx);
        stack(scene, ctx);
        ghost(scene, ctx);
        active_piece(scene, ctx);
        trails(scene, ctx);
        particles(scene, ctx);
        hud(scene, ctx);
        toasts(scene, ctx);
    }

    match ctx.screen {
        Screen::Title => title_overlay(scene, ctx),
        Screen::Paused => message_overlay(scene, ctx, "PAUSED", "PRESS P TO RESUME"),
        Screen::GameOver => game_over_overlay(scene, ctx),
        Screen::Playing => {}
    }

    if ctx.settings.show_debug {
        debug_overlay(scene, ctx);
    }
}

// --------------------------------------------------------------------------- background

fn background(scene: &mut Scene, ctx: &BuildContext<'_>) {
    let l = ctx.layout;
    scene.push(
        Layer::Background,
        Quad::gradient(0.0, 0.0, l.width, l.height, t::SKY_TOP),
    );

    // A slowly drifting grid gives the empty space some life without competing with the well.
    let drift = (ctx.vm.clock * 14.0) % 48.0;
    let grid = Quad::solid(0.0, 0.0, l.width, l.height, t::BG_GRID)
        .with_pattern(1.0)
        .with_radius(0.0);
    scene.push(Layer::Background, grid);

    // Two soft neon washes behind the panels.
    let pulse = 0.5 + 0.5 * (ctx.vm.clock * 0.6).sin();
    scene.push(
        Layer::Background,
        Quad::rounded(
            l.left.x - 20.0,
            l.left.y + l.left.h * 0.1,
            l.left.w + 40.0,
            l.left.h * 0.8,
            40.0,
            t::with_alpha([0.1, 0.5, 0.9, 1.0], 0.10 + 0.04 * pulse),
        )
        .with_glow(0.9),
    );
    scene.push(
        Layer::Background,
        Quad::rounded(
            l.right.x - 20.0,
            l.right.y + l.right.h * 0.1,
            l.right.w + 40.0,
            l.right.h * 0.8,
            40.0,
            t::with_alpha([0.9, 0.3, 0.8, 1.0], 0.08 + 0.04 * (1.0 - pulse)),
        )
        .with_glow(0.8),
    );
    let _ = drift;
}

// --------------------------------------------------------------------------------- well

fn well(scene: &mut Scene, ctx: &BuildContext<'_>) {
    let l = ctx.layout;
    let frame = Rect::new(l.field_x, l.field_y, l.field_w, l.field_h);

    scene.push(
        Layer::Playfield,
        Quad::rounded(
            frame.x - 6.0,
            frame.y - 6.0,
            frame.w + 12.0,
            frame.h + 12.0,
            12.0,
            t::WELL,
        )
        .with_glow(0.15),
    );
    scene.push(
        Layer::Playfield,
        Quad::solid(frame.x, frame.y, frame.w, frame.h, t::GRID).with_pattern(1.0),
    );

    // Danger: the top of the well glows red as the stack climbs.
    let danger = danger_amount(ctx);
    if danger > 0.01 {
        let pulse = 0.55 + 0.45 * (ctx.vm.clock * 6.0).sin();
        scene.push(
            Layer::Playfield,
            Quad::gradient(
                frame.x,
                frame.y,
                frame.w,
                frame.h * 0.30,
                t::with_alpha(t::DANGER, 0.30 * danger * pulse),
            ),
        );
    }

    // Frame: a bright outline whose colour follows the danger level.
    let edge = if danger > 0.01 {
        t::with_alpha([1.0, 0.7 - 0.55 * danger, 0.75 - 0.6 * danger, 0.9], 0.9)
    } else {
        t::WELL_EDGE
    };
    scene.push(
        Layer::Playfield,
        Quad::outline(
            frame.x - 6.0,
            frame.y - 6.0,
            frame.w + 12.0,
            frame.h + 12.0,
            12.0,
            edge,
        )
        .with_glow(0.8 + 0.8 * danger),
    );

    // Separator between the hidden buffer rows and the visible field.
    let buffer_line = frame.y + l.cell * BUFFER_ROWS as f32;
    scene.push(
        Layer::Playfield,
        Quad::solid(
            frame.x,
            buffer_line,
            frame.w,
            1.0,
            t::with_alpha(t::LABEL, 0.25),
        ),
    );
}

fn danger_amount(ctx: &BuildContext<'_>) -> f32 {
    let fraction = ctx.session.board().stack_height_fraction();
    let threshold = ctx.session.config().danger_threshold;
    if fraction <= threshold * 0.75 {
        0.0
    } else {
        ((fraction - threshold * 0.75) / (1.0 - threshold * 0.75)).clamp(0.0, 1.0)
    }
}

// -------------------------------------------------------------------------------- stack

fn stack(scene: &mut Scene, ctx: &BuildContext<'_>) {
    let l = ctx.layout;
    let board = ctx.session.board();
    let collapse = ctx.vm.collapse_offset();
    let flashing = ctx.vm.flashing_rows();

    for row in 0..BOARD_ROWS as i32 {
        for col in 0..BOARD_COLS as i32 {
            let Some(color_index) = board.color_at(col, row) else {
                continue;
            };
            let Some(piece) = Tetromino::ALL.get(color_index as usize) else {
                continue;
            };
            let mut rect = l.cell_rect(col, row);
            if collapse > 0.0 {
                rect.y -= collapse * l.cell;
            }
            let flash = flashing
                .iter()
                .find(|(r, _)| *r as i32 == row)
                .map(|(_, i)| *i)
                .unwrap_or(0.0);
            let color = if flash > 0.0 {
                t::tint_towards_white(with_piece_color(*piece, 1.0), flash)
            } else {
                with_piece_color(*piece, 1.0)
            };
            let glow = if flash > 0.0 { 1.6 * flash } else { 0.55 };
            scene.push(
                Layer::Pieces,
                Quad::block(
                    rect.x + rect.w * 0.06,
                    rect.y + rect.h * 0.06,
                    rect.w * 0.88,
                    color,
                )
                .with_glow(glow),
            );
        }
    }
}

fn with_piece_color(piece: Tetromino, alpha: f32) -> [f32; 4] {
    let [r, g, b] = piece.color();
    [r, g, b, alpha]
}

// --------------------------------------------------------------- ghost + active piece

fn ghost(scene: &mut Scene, ctx: &BuildContext<'_>) {
    let Some(ghost) = ctx.session.ghost() else {
        return;
    };
    if ctx.screen != Screen::Playing {
        return;
    }
    let Some(active) = ctx.session.active() else {
        return;
    };
    if ghost.row == active.row {
        return; // resting: the piece itself already shows where it lands
    }
    for cell in ghost.cells() {
        let rect = ctx.layout.cell_rect(cell.col, cell.row);
        scene.push(
            Layer::Playfield,
            Quad::outline(
                rect.x + rect.w * 0.16,
                rect.y + rect.h * 0.16,
                rect.w * 0.68,
                rect.h * 0.68,
                rect.w * 0.18,
                t::GHOST,
            )
            .with_glow(0.25),
        );
    }
}

fn active_piece(scene: &mut Scene, ctx: &BuildContext<'_>) {
    let Some(piece) = ctx.session.active() else {
        return;
    };
    if matches!(ctx.screen, Screen::Title) {
        return;
    }
    let grounded = ctx.session.lock_delay_remaining().is_some();
    let pulse = if grounded {
        // Blink faster as the lock delay runs out, so the player can feel the clock.
        let remaining = ctx
            .session
            .lock_delay_remaining()
            .map(|d| d.as_secs_f32())
            .unwrap_or(1.0);
        let rate = 4.0 + 18.0 * (1.0 - (remaining / 0.5).clamp(0.0, 1.0));
        0.5 + 0.5 * (ctx.vm.clock * rate).sin()
    } else {
        1.0
    };

    for cell in piece.cells() {
        let rect = ctx.layout.cell_rect(cell.col, cell.row);
        scene.push(
            Layer::Pieces,
            Quad::block(
                rect.x + rect.w * 0.05,
                rect.y + rect.h * 0.05,
                rect.w * 0.9,
                with_piece_color(piece.kind, 1.0),
            )
            .with_glow(0.75 + 0.5 * pulse),
        );
    }
}

// ------------------------------------------------------------------ trails + particles

fn trails(scene: &mut Scene, ctx: &BuildContext<'_>) {
    let l = ctx.layout;
    for trail in &ctx.vm.trails {
        let p = trail.progress();
        let alpha = (1.0 - p) * 0.5;
        if alpha <= 0.01 || trail.distance == 0 {
            continue;
        }
        for (col, row) in &trail.cells {
            // A vertical smear from where the piece came from down to where it landed.
            let rect = l.cell_rect(*col, *row);
            let height = l.cell * (1.0 + trail.distance.min(20) as f32 * (1.0 - p));
            scene.push(
                Layer::Effects,
                Quad::gradient(
                    rect.x + rect.w * 0.2,
                    rect.y - height,
                    rect.w * 0.6,
                    height,
                    [trail.color[0], trail.color[1], trail.color[2], alpha * 0.55],
                )
                .with_glow(1.1),
            );
        }
    }
}

fn particles(scene: &mut Scene, ctx: &BuildContext<'_>) {
    for p in ctx.vm.particles.iter() {
        scene.particle(SceneParticle {
            x: p.x,
            y: p.y,
            size: p.size * (0.4 + 0.6 * p.fade()),
            color: [p.color[0], p.color[1], p.color[2], p.fade()],
            glow: 1.4 * p.fade(),
        });
    }
}

// ---------------------------------------------------------------------------------- hud

fn hud(scene: &mut Scene, ctx: &BuildContext<'_>) {
    let l = ctx.layout;
    let stats = ctx.session.stats();

    // ---- left panel: score + progression
    let panel = l.left;
    scene.push(
        Layer::Hud,
        Quad::rounded(panel.x, panel.y, panel.w, panel.h, 14.0, t::PANEL),
    );
    scene.push(
        Layer::Hud,
        Quad::outline(panel.x, panel.y, panel.w, panel.h, 14.0, t::PANEL_EDGE).with_glow(0.5),
    );

    let mut rest = panel.inset(14.0);
    let (head, next) = rest.take_top(l.body() * 10.0);
    rest = next;
    scene.text(Text::new("SCORE", head.x, head.y, l.body(), t::LABEL).with_tracking(1.5 * l.scale));
    scene.text(
        Text::new(
            format_thousands(stats.score),
            head.x,
            head.y + l.body() * 2.6,
            l.score_size(),
            t::ACCENT,
        )
        .with_glow(1.1),
    );
    let best = ctx.board.best().unwrap_or(0).max(stats.score);
    scene.text(Text::new(
        format!("BEST {}", format_thousands(best)),
        head.x,
        head.y + l.body() * 5.4,
        l.body(),
        t::MUTED,
    ));

    let (rows, remainder) = rest.take_top(l.body() * 9.0);
    rest = remainder;
    let row_h = l.body() * 3.0;
    for (i, (label, value)) in [
        ("LEVEL", stats.level.to_string()),
        ("LINES", format_thousands(stats.lines as u64)),
        (
            "TIME",
            format!(
                "{}:{:02}",
                stats.time.as_secs() / 60,
                stats.time.as_secs() % 60
            ),
        ),
    ]
    .iter()
    .enumerate()
    {
        let y = rows.y + i as f32 * row_h;
        scene.text(Text::new(*label, rows.x, y, l.body(), t::LABEL));
        scene.text(Text::new(value.clone(), rows.right(), y, l.body(), t::TEXT).right_aligned());
    }

    // Progress to the next level.
    let to_next = ctx
        .session
        .config()
        .progression
        .lines_to_next_level(stats.lines);
    let per = ctx.session.config().progression.lines_per_level.max(1);
    let filled = 1.0 - (to_next as f32 / per as f32);
    let bar = rest.take_top(l.body() * 1.6).0;
    scene.push(
        Layer::Hud,
        Quad::rounded(
            bar.x,
            bar.y,
            bar.w,
            bar.h,
            bar.h * 0.5,
            t::with_alpha(t::GRID, 0.8),
        ),
    );
    if filled > 0.01 {
        scene.push(
            Layer::Hud,
            Quad::rounded(
                bar.x,
                bar.y,
                bar.w * filled,
                bar.h,
                bar.h * 0.5,
                t::with_alpha(t::ACCENT, 0.9),
            )
            .with_glow(0.9),
        );
    }

    // ---- right panel: hold + next
    let panel = l.right;
    scene.push(
        Layer::Hud,
        Quad::rounded(panel.x, panel.y, panel.w, panel.h, 14.0, t::PANEL),
    );
    scene.push(
        Layer::Hud,
        Quad::outline(panel.x, panel.y, panel.w, panel.h, 14.0, t::PANEL_EDGE).with_glow(0.5),
    );
    let inner = panel.inset(14.0);

    let hold_h = l.cell * 2.6 + l.body() * 2.0;
    scene.text(Text::new("HOLD", inner.x, inner.y, l.body(), t::LABEL));
    let hold_box = Rect::new(
        inner.x,
        inner.y + l.body() * 1.8,
        inner.w,
        hold_h - l.body() * 1.8,
    );
    scene.push(
        Layer::Hud,
        Quad::rounded(
            hold_box.x,
            hold_box.y,
            hold_box.w,
            hold_box.h,
            10.0,
            t::with_alpha(t::WELL, 0.9),
        ),
    );
    if let Some(held) = ctx.session.hold() {
        let pop = 1.0 + ctx.vm.hold_pop * 0.18;
        draw_mini_piece(
            scene,
            held,
            hold_box.centre_x(),
            hold_box.centre_y(),
            (hold_box.w / 4.2).min(hold_box.h / 2.6) * pop,
            if ctx.session.hold_used() { 0.45 } else { 1.0 },
        );
    }

    let next_y = hold_box.bottom() + l.body() * 2.0;
    scene.text(Text::new("NEXT", inner.x, next_y, l.body(), t::LABEL));
    let queue = ctx.session.preview();
    let count = queue.len().max(1);
    let slot_h = ((inner.bottom() - next_y - l.body() * 1.6) / count as f32).max(l.cell * 1.4);
    for (i, piece) in queue.iter().enumerate() {
        let centre_y = next_y + l.body() * 1.4 + slot_h * (i as f32 + 0.5);
        let emphasis = if i == 0 { 1.0 } else { 0.55 + 0.1 * i as f32 };
        let size = if i == 0 { slot_h * 0.42 } else { slot_h * 0.30 };
        draw_mini_piece(
            scene,
            *piece,
            inner.centre_x(),
            centre_y,
            size,
            emphasis.min(1.0),
        );
    }

    // Combo / back-to-back badges under the score bar.
    let combo = ctx.session.combo();
    if combo > 1 {
        scene.text(
            Text::new(
                format!("COMBO ×{}", combo - 1),
                l.field_x,
                l.field_y + l.field_h + 8.0 * l.scale,
                l.body() * 1.2,
                t::WARM,
            )
            .with_glow(1.0),
        );
    }
    if ctx.session.back_to_back() > 1 {
        scene.text(
            Text::new(
                format!("B2B ×{}", ctx.session.back_to_back() - 1),
                l.field_x + l.field_w,
                l.field_y + l.field_h + 8.0 * l.scale,
                l.body() * 1.2,
                t::WARM,
            )
            .with_glow(1.0)
            .right_aligned(),
        );
    }
}

/// Draw a piece centred on a point, used by the hold/next panels.
fn draw_mini_piece(scene: &mut Scene, piece: Tetromino, cx: f32, cy: f32, cell: f32, alpha: f32) {
    let cells = piece.cells_at(tetris_core::Rotation::Spawn, 0, 0);
    let min_col = cells.iter().map(|c| c.col).min().unwrap_or(0);
    let max_col = cells.iter().map(|c| c.col).max().unwrap_or(0);
    let min_row = cells.iter().map(|c| c.row).min().unwrap_or(0);
    let max_row = cells.iter().map(|c| c.row).max().unwrap_or(0);
    let w = (max_col - min_col + 1) as f32 * cell;
    let h = (max_row - min_row + 1) as f32 * cell;

    for c in &cells {
        let x = cx - w * 0.5 + (c.col - min_col) as f32 * cell;
        let y = cy - h * 0.5 + (c.row - min_row) as f32 * cell;
        scene.push(
            Layer::Hud,
            Quad::block(
                x + cell * 0.06,
                y + cell * 0.06,
                cell * 0.88,
                with_piece_color(piece, alpha),
            )
            .with_glow(0.6 * alpha),
        );
    }
}

// ------------------------------------------------------------------------------ toasts

fn toasts(scene: &mut Scene, ctx: &BuildContext<'_>) {
    let l = ctx.layout;
    for toast in &ctx.vm.toasts {
        let p = toast.progress();
        let in_ease = (p * 8.0).min(1.0);
        let out_ease = 1.0 - ((p - 0.75) / 0.25).max(0.0);
        let alpha = (in_ease * out_ease).clamp(0.0, 1.0);
        if alpha <= 0.02 {
            continue;
        }
        let y = l.field_y + l.field_h * 0.28 + toast.slot as f32 * l.body() * 3.2;
        scene.text(
            Text::new(
                toast.text.clone(),
                l.field_x + l.field_w * 0.5,
                y,
                l.body() * 1.4,
                [toast.color[0], toast.color[1], toast.color[2], alpha],
            )
            .centred()
            .with_glow(1.2),
        );
    }

    if ctx.vm.level_banner > 0.0 {
        let alpha = (ctx.vm.level_banner / 2.0).min(1.0);
        scene.text(
            Text::new(
                format!("LEVEL {}", ctx.vm.level_shown),
                l.field_x + l.field_w * 0.5,
                l.field_y - l.body() * 3.2,
                l.body() * 1.6,
                t::with_alpha(t::ACCENT, alpha),
            )
            .centred()
            .with_glow(1.4),
        );
    }
}

// --------------------------------------------------------------------------- overlays

fn wordmark(scene: &mut Scene, ctx: &BuildContext<'_>, title: &str, subtitle: &str) {
    let l = ctx.layout;
    let cx = l.width * 0.5;
    let bob = (ctx.vm.clock * 1.2).sin() * 3.0 * l.scale;
    scene.text(
        Text::new(title, cx, l.height * 0.20 + bob, l.title(), t::TEXT)
            .centred()
            .with_tracking(1.0)
            .with_glow(1.6),
    );
    if !subtitle.is_empty() {
        scene.text(
            Text::new(
                subtitle,
                cx,
                l.height * 0.20 + bob + l.title() * 1.4,
                l.body() * 1.1,
                t::ACCENT,
            )
            .centred()
            .with_glow(0.8),
        );
    }
}

/// Tetrominoes drifting down the title screen — the "attract mode".
fn attract_pieces(scene: &mut Scene, ctx: &BuildContext<'_>) {
    let l = ctx.layout;
    let count = 9;
    for i in 0..count {
        let seed = i as f32;
        let speed = 14.0 + (i % 4) as f32 * 6.0;
        let y = ((ctx.vm.clock * speed + seed * 137.0) % (l.height + 240.0)) - 160.0;
        let x = 40.0 + ((seed * 233.0) % (l.width - 80.0)).max(0.0);
        let piece = Tetromino::ALL[i % Tetromino::ALL.len()];
        let cell = (l.cell * (0.35 + 0.05 * (i % 3) as f32)).max(6.0);
        let rot = tetris_core::Rotation::from_index((i + (ctx.vm.clock * 0.2) as usize) & 3);
        for c in piece.cells_at(rot, 0, 0) {
            scene.push(
                Layer::Background,
                Quad::block(
                    x + c.col as f32 * cell,
                    y + c.row as f32 * cell,
                    cell * 0.9,
                    with_piece_color(piece, 0.32),
                )
                .with_glow(0.5),
            );
        }
    }
}

fn title_overlay(scene: &mut Scene, ctx: &BuildContext<'_>) {
    attract_pieces(scene, ctx);
    wordmark(scene, ctx, "TETRIS", "NEON EDITION");

    let l = ctx.layout;
    let cx = l.width * 0.5;
    let blink = if (ctx.vm.clock * 1.6).sin() > -0.2 {
        1.0
    } else {
        0.25
    };
    scene.text(
        Text::new(
            "PRESS ENTER TO PLAY",
            cx,
            l.height * 0.50,
            l.body() * 1.4,
            t::with_alpha(t::TEXT, blink),
        )
        .centred()
        .with_glow(1.0 * blink),
    );

    if l.compact() {
        // Small window: the wordmark and the prompt are the whole story.
        return;
    }

    // Controls, on the right.
    let panel_w = l.width * 0.30;
    let panel_h = l.height * 0.30;
    let controls = Rect::new(cx + l.width * 0.03, l.height * 0.60, panel_w, panel_h);
    panel(scene, controls);
    scene.text(Text::new(
        "CONTROLS",
        controls.x + 18.0 * l.scale,
        controls.y + 14.0 * l.scale,
        l.body(),
        t::LABEL,
    ));
    let rows: [(&str, &str); 7] = [
        ("MOVE", "LEFT / RIGHT"),
        ("SOFT DROP", "DOWN"),
        ("HARD DROP", "SPACE"),
        ("ROTATE", "UP / X  Z  A"),
        ("HOLD", "C / SHIFT"),
        ("PAUSE", "P OR ESC"),
        ("TOGGLES", "F2 CRT  M MUTE  F3 INFO"),
    ];
    for (i, (label, keys)) in rows.iter().enumerate() {
        let y = controls.y + 14.0 * l.scale + (i as f32 + 1.4) * l.body() * 2.2;
        scene.text(Text::new(
            *label,
            controls.x + 18.0 * l.scale,
            y,
            l.body(),
            t::MUTED,
        ));
        scene.text(
            Text::new(
                *keys,
                controls.right() - 18.0 * l.scale,
                y,
                l.body(),
                t::TEXT,
            )
            .right_aligned(),
        );
    }

    // Honours board, on the left — the persistence layer made visible.
    let scores = Rect::new(cx - l.width * 0.03 - panel_w, controls.y, panel_w, panel_h);
    panel(scene, scores);
    scene.text(Text::new(
        "HIGH SCORES",
        scores.x + 18.0 * l.scale,
        scores.y + 14.0 * l.scale,
        l.body(),
        t::LABEL,
    ));
    if ctx.board.entries.is_empty() {
        scene.text(Text::new(
            "NO SCORES YET",
            scores.x + 18.0 * l.scale,
            scores.y + 14.0 * l.scale + l.body() * 2.4,
            l.body(),
            t::MUTED,
        ));
    }
    for (i, entry) in ctx.board.entries.iter().take(6).enumerate() {
        let y = scores.y + 14.0 * l.scale + (i as f32 + 1.4) * l.body() * 2.2;
        scene.text(Text::new(
            format!("{}. {}", i + 1, entry.name),
            scores.x + 18.0 * l.scale,
            y,
            l.body(),
            if i == 0 { t::WARM } else { t::TEXT },
        ));
        scene.text(
            Text::new(
                format_thousands(entry.score),
                scores.right() - 18.0 * l.scale,
                y,
                l.body(),
                if i == 0 { t::WARM } else { t::MUTED },
            )
            .right_aligned(),
        );
    }
}

/// Standard panel: translucent body with a glowing outline.
fn panel(scene: &mut Scene, rect: Rect) {
    scene.push(
        Layer::Overlay,
        Quad::rounded(rect.x, rect.y, rect.w, rect.h, 14.0, t::PANEL),
    );
    scene.push(
        Layer::Overlay,
        Quad::outline(rect.x, rect.y, rect.w, rect.h, 14.0, t::PANEL_EDGE).with_glow(0.4),
    );
}

fn message_overlay(scene: &mut Scene, ctx: &BuildContext<'_>, title: &str, hint: &str) {
    let l = ctx.layout;
    scene.push(
        Layer::Overlay,
        Quad::solid(0.0, 0.0, l.width, l.height, t::SCRIM),
    );
    wordmark(scene, ctx, title, hint);
}

fn game_over_overlay(scene: &mut Scene, ctx: &BuildContext<'_>) {
    let l = ctx.layout;
    scene.push(
        Layer::Overlay,
        Quad::solid(0.0, 0.0, l.width, l.height, t::SCRIM),
    );
    wordmark(scene, ctx, "GAME OVER", "");

    let stats = ctx.session.stats();
    let cx = l.width * 0.5;
    let lines = [
        ("SCORE", format_thousands(stats.score)),
        ("LINES", format!("{}", stats.lines)),
        ("LEVEL", format!("{}", stats.level)),
        ("PIECES", format!("{}", stats.pieces)),
        ("TETRISES", format!("{}", stats.tetrises)),
        ("T-SPINS", format!("{}", stats.spins)),
        (
            "MAX COMBO",
            format!("{}", stats.max_combo.saturating_sub(1)),
        ),
        (
            "TIME",
            format!(
                "{}:{:02}",
                stats.time.as_secs() / 60,
                stats.time.as_secs() % 60
            ),
        ),
    ];
    let panel_h = l.body() * (lines.len() as f32 * 1.9 + 2.0);
    let panel = Rect::new(
        cx - l.width * 0.14,
        l.height * 0.34,
        (l.width * 0.28).min(420.0),
        panel_h,
    );
    scene.push(
        Layer::Overlay,
        Quad::rounded(panel.x, panel.y, panel.w, panel.h, 14.0, t::PANEL),
    );
    scene.push(
        Layer::Overlay,
        Quad::outline(panel.x, panel.y, panel.w, panel.h, 14.0, t::PANEL_EDGE).with_glow(0.4),
    );
    for (i, (label, value)) in lines.iter().enumerate() {
        let y = panel.y + 14.0 * l.scale + i as f32 * l.body() * 1.9;
        scene.text(Text::new(
            *label,
            panel.x + 16.0 * l.scale,
            y,
            l.body(),
            t::LABEL,
        ));
        scene.text(
            Text::new(
                value.clone(),
                panel.right() - 16.0 * l.scale,
                y,
                l.body(),
                t::TEXT,
            )
            .right_aligned(),
        );
    }

    if ctx.vm.qualifying {
        let y = panel.bottom() + l.body() * 2.2;
        let caret = if (ctx.vm.clock * 2.4).sin() > 0.0 {
            "_"
        } else {
            " "
        };
        scene.text(
            Text::new(
                format!("NEW HIGH SCORE: {}{}", ctx.vm.name_entry, caret),
                cx,
                y,
                l.body() * 1.2,
                t::WARM,
            )
            .centred()
            .with_glow(1.0),
        );
        scene.text(
            Text::new(
                "TYPE A NAME, THEN PRESS ENTER",
                cx,
                y + l.body() * 2.4,
                l.body(),
                t::MUTED,
            )
            .centred(),
        );
    } else {
        let blink = if (ctx.vm.clock * 1.6).sin() > -0.2 {
            1.0
        } else {
            0.25
        };
        scene.text(
            Text::new(
                "ENTER: PLAY AGAIN   ESC: MENU",
                cx,
                panel.bottom() + l.body() * 2.6,
                l.body() * 1.2,
                t::with_alpha(t::TEXT, blink),
            )
            .centred()
            .with_glow(0.8 * blink),
        );
    }
}

fn debug_overlay(scene: &mut Scene, ctx: &BuildContext<'_>) {
    let l = ctx.layout;
    let counts = scene.counts();
    let line = format!(
        "FPS {:.0}  SIM {:.2}MS  {}  {}",
        ctx.frame.fps, ctx.frame.sim_ms, ctx.frame.backend, counts,
    );
    scene.push(
        Layer::Overlay,
        Quad::rounded(
            8.0,
            8.0,
            l.body() * 34.0,
            l.body() * 2.2,
            6.0,
            t::with_alpha(t::PANEL, 0.9),
        ),
    );
    scene.text(Text::new(line, 16.0, 14.0, l.body(), t::ACCENT));
    let phase = match ctx.session.phase() {
        Phase::Title => "TITLE",
        Phase::Playing => "PLAYING",
        Phase::Paused => "PAUSED",
        Phase::GameOver => "GAME OVER",
    };
    scene.text(Text::new(
        format!("{phase}  SEED {}", ctx.session.seed()),
        16.0,
        14.0 + l.body() * 2.4,
        l.body(),
        t::MUTED,
    ));
}

// ---------------------------------------------------------------------------- helpers

/// Group thousands with commas so a seven-figure score is readable at a glance.
pub fn format_thousands(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}
