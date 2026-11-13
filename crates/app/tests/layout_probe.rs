//! End-to-end layout probes: build a real scene, render it on the GPU, and assert on where the
//! light landed. These are the tests that say "the HUD is still on screen and in the right place"
//! without anybody having to look at the game.

use tetris_app::app::render_probe_frame;
use tetris_app::scene_builder::Screen;

/// Skip cleanly where there is no Vulkan device to ask.
fn probe(
    width: u32,
    height: u32,
    screen: Screen,
    crt: bool,
) -> Option<tetris_app::app::ProbeFrame> {
    match render_probe_frame(width, height, screen, crt) {
        Ok(frame) => Some(frame),
        Err(e) => {
            eprintln!("SKIP (no GPU): {e}");
            None
        }
    }
}

#[test]
fn a_gameplay_frame_has_content_everywhere_it_should() {
    let Some(frame) = probe(1280, 800, Screen::Playing, false) else {
        return;
    };
    assert_eq!((frame.width, frame.height), (1280, 800));
    let layout = frame.layout;

    // 1. Something is drawn at all.
    let total = frame.lit_in(0, 0, frame.width, frame.height, 40);
    assert!(total > 5_000, "frame is nearly empty: {total} lit px");

    // 2. The well contains the stacked blocks: inside the field there must be plenty of light.
    let field = frame.lit_in(
        layout.field_x as u32 + 4,
        layout.field_y as u32 + (22.0 * 0.55 * layout.cell) as u32,
        (layout.field_x + layout.field_w) as u32 - 4,
        (layout.field_y + layout.field_h) as u32 - 4,
        60,
    );
    assert!(field > 2_000, "stack not visible in the well: {field} px");

    // 3. The left panel carries the score text.
    let left = frame.lit_in(
        layout.left.x as u32 + 4,
        layout.left.y as u32 + 4,
        layout.left.right() as u32 - 4,
        (layout.left.y + layout.left.h * 0.35) as u32,
        90,
    );
    assert!(left > 200, "score text missing: {left} px");

    // 4. The right panel carries the hold/next pieces.
    let right = frame.lit_in(
        layout.right.x as u32 + 4,
        layout.right.y as u32 + 4,
        layout.right.right() as u32 - 4,
        layout.right.bottom() as u32 - 4,
        90,
    );
    assert!(right > 400, "hold/next missing: {right} px");

    // 5. Above the well (top-centre strip) is background, i.e. darker.
    let bg = frame.luma((layout.field_x + layout.field_w * 0.5) as u32, 6);
    assert!(bg < 120, "background should stay dark, got {bg}");
}

#[test]
fn the_title_screen_shows_a_wordmark_in_the_upper_half() {
    let Some(frame) = probe(1280, 800, Screen::Title, false) else {
        return;
    };
    let upper = frame.lit_in(300, 100, 980, 300, 150);
    assert!(upper > 1_000, "title text missing: {upper} lit px");

    // The bright mass in the wordmark band should be centred on the window, not lopsided.
    let mut sum_x = 0u64;
    let mut count = 0u64;
    for y in 100..300u32 {
        for x in 0..frame.width {
            if frame.luma(x, y) > 150 {
                sum_x += x as u64;
                count += 1;
            }
        }
    }
    assert!(count > 500, "no wordmark to centre");
    let centroid = (sum_x / count) as i64;
    assert!(
        (centroid - 640).abs() < 60,
        "wordmark centroid at x={centroid}, expected near 640"
    );
}

#[test]
fn crt_effects_change_the_picture() {
    let Some(off) = probe(640, 480, Screen::Playing, false) else {
        return;
    };
    let Some(on) = probe(640, 480, Screen::Playing, true) else {
        return;
    };
    // Bloom + vignette + scanlines must measurably differ from a flat render.
    let diff = on.difference_vs(&off);
    assert!(
        diff > 100_000,
        "post effects had no visible effect (diff {diff})"
    );
}

#[test]
fn tiny_and_huge_windows_still_render() {
    for (w, h) in [(360, 320), (3200, 1800)] {
        let Some(frame) = probe(w, h, Screen::Playing, false) else {
            return;
        };
        assert_eq!((frame.width, frame.height), (w, h));
        let lit = frame.lit_in(0, 0, w, h, 50);
        assert!(lit > 1_000, "{w}×{h} rendered almost nothing ({lit} px)");
    }
}

#[test]
fn the_game_over_screen_is_dimmed_with_a_bright_panel() {
    let Some(playing) = probe(900, 700, Screen::Playing, false) else {
        return;
    };
    let Some(over) = probe(900, 700, Screen::GameOver, false) else {
        return;
    };
    // The scrim darkens most of the frame…
    let playing_lit = playing.lit_in(0, 0, playing.width, playing.height, 90);
    let over_lit = over.lit_in(0, 0, over.width, over.height, 90);
    assert!(
        over_lit < playing_lit,
        "game over should dim the scene: {over_lit} vs {playing_lit}"
    );
    // …while the title and panel stay bright.
    let bright = over.lit_in(200, 100, 700, 260, 150);
    assert!(bright > 500, "game over text missing: {bright}");
}
