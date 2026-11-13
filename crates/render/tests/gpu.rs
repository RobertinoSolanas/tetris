//! Headless GPU tests (docs/adr/0010): real Vulkan device, no window, pixels read back and
//! asserted. Creating the renderer already compiles and validates every WGSL shader in the crate.

use tetris_render::{build_instances, Font, Layer, PostFx, Quad, Renderer, Scene, Text};

/// Skip (green) on machines without a usable adapter rather than failing for an environmental
/// reason we cannot fix from here.
fn renderer(width: u32, height: u32) -> Option<Renderer> {
    match Renderer::headless(width, height) {
        Ok(r) => Some(r),
        Err(e) => {
            eprintln!("SKIP: no GPU adapter: {e}");
            None
        }
    }
}

fn plain_scene() -> Scene {
    let mut scene = Scene::new(128.0, 128.0);
    scene.post = PostFx::off();
    scene.clear_color = [0.0, 0.0, 0.0, 1.0];
    scene
}

fn channel(image: &[u8], width: u32, x: u32, y: u32, c: usize) -> u8 {
    let i = ((y * width + x) as usize) * 4 + c;
    image[i]
}

#[test]
fn shaders_compile_and_a_frame_renders() {
    let Some(mut r) = renderer(128, 128) else {
        return;
    };
    let scene = plain_scene();
    let (w, h, image) = r.render_to_image(&scene, 0.0);
    assert_eq!((w, h), (128, 128));
    assert_eq!(image.len(), 128 * 128 * 4);
    // Every pixel is opaque and nothing is NaN-ish garbage.
    assert!(image.iter().skip(3).step_by(4).all(|a| *a > 250));
}

#[test]
fn a_solid_quad_lands_where_the_scene_says() {
    let Some(mut r) = renderer(128, 128) else {
        return;
    };
    let mut scene = plain_scene();
    scene.push(
        Layer::Playfield,
        Quad::solid(48.0, 48.0, 32.0, 32.0, [1.0, 0.0, 0.0, 1.0]),
    );
    let (w, _, image) = r.render_to_image(&scene, 0.0);

    // Centre of the quad: red.
    assert!(
        channel(&image, w, 64, 64, 0) > 200,
        "expected red at the quad centre, got {:?}",
        &image[(64 * w as usize + 64) * 4..(64 * w as usize + 64) * 4 + 4]
    );
    assert!(channel(&image, w, 64, 64, 1) < 60, "green bled in");
    assert!(channel(&image, w, 64, 64, 2) < 60, "blue bled in");

    // Well outside it: the clear colour (black).
    assert!(
        channel(&image, w, 8, 8, 0) < 24,
        "corner should be background, got {}",
        channel(&image, w, 8, 8, 0)
    );
}

#[test]
fn layers_draw_back_to_front() {
    let Some(mut r) = renderer(64, 64) else {
        return;
    };
    let mut scene = plain_scene();
    scene.push(
        Layer::Background,
        Quad::solid(0.0, 0.0, 64.0, 64.0, [1.0, 0.0, 0.0, 1.0]),
    );
    scene.push(
        Layer::Overlay,
        Quad::solid(16.0, 16.0, 32.0, 32.0, [0.0, 0.0, 1.0, 1.0]),
    );
    let (w, _, image) = r.render_to_image(&scene, 0.0);
    assert!(channel(&image, w, 32, 32, 2) > 200, "overlay should win");
    assert!(
        channel(&image, w, 4, 4, 0) > 200,
        "background visible outside"
    );
}

#[test]
fn text_renders_ink_at_the_requested_place() {
    let Some(mut r) = renderer(128, 64) else {
        return;
    };
    let mut scene = plain_scene();
    scene.text(Text::new("0", 20.0, 20.0, 4.0, [1.0, 1.0, 1.0, 1.0]));
    let (w, _, image) = r.render_to_image(&scene, 0.0);

    // The digit occupies 5×7 px at scale 4 → 20×28 px starting at (20, 20).
    let mut lit = 0;
    for y in 20..48u32 {
        for x in 20..40u32 {
            if channel(&image, w, x, y, 0) > 120 {
                lit += 1;
            }
        }
    }
    assert!(
        lit > 40,
        "expected glyph ink in the box, found {lit} lit px"
    );

    // And nothing lit outside it.
    let mut outside = 0;
    for y in 1u32..18 {
        for x in 1u32..120 {
            if channel(&image, w, x, y, 0) > 120 {
                outside += 1;
            }
        }
    }
    assert_eq!(outside, 0, "text leaked above its box");
}

#[test]
fn bloom_spreads_light_beyond_the_quad() {
    let Some(mut r) = renderer(128, 128) else {
        return;
    };

    let with_bloom = |renderer: &mut Renderer, post: PostFx| {
        let mut scene = plain_scene();
        scene.post = post;
        scene.push(
            Layer::Effects,
            Quad::solid(60.0, 60.0, 8.0, 8.0, [1.0, 1.0, 1.0, 1.0]).with_glow(2.0),
        );
        let (_, _, image) = renderer.render_to_image(&scene, 0.0);
        // Sample 14 px to the left of the quad: inside the glow halo, outside the quad itself.
        image[(64 * 128 + 46) * 4] as u32
    };

    let off = with_bloom(&mut r, PostFx::off());
    let on = with_bloom(&mut r, PostFx::arcade());
    assert!(
        on > off + 4,
        "bloom should brighten the surround: off={off} on={on}"
    );
}

#[test]
fn the_flash_setting_lifts_the_whole_frame() {
    let Some(mut r) = renderer(64, 64) else {
        return;
    };
    let mut scene = plain_scene();
    scene.flash = 0.25;
    let (w, _, image) = r.render_to_image(&scene, 0.0);
    assert!(
        channel(&image, w, 10, 10, 0) > 24,
        "flash should lift the background"
    );
}

#[test]
fn resize_rebuilds_targets_and_renders() {
    let Some(mut r) = renderer(64, 48) else {
        return;
    };
    let mut scene = plain_scene();
    scene.resize(64.0, 48.0);
    scene.push(
        Layer::Pieces,
        Quad::solid(0.0, 0.0, 64.0, 48.0, [0.0, 1.0, 0.0, 1.0]),
    );
    let (w, h, image) = r.render_to_image(&scene, 0.0);
    assert_eq!((w, h), (64, 48));
    assert!(channel(&image, w, 32, 24, 1) > 180);

    r.resize(96, 72);
    let mut big = plain_scene();
    big.resize(96.0, 72.0);
    big.push(
        Layer::Pieces,
        Quad::solid(0.0, 0.0, 96.0, 72.0, [0.0, 0.0, 1.0, 1.0]),
    );
    let (w2, h2, image2) = r.render_to_image(&big, 0.0);
    assert_eq!((w2, h2), (96, 72));
    assert!(channel(&image2, w2, 90, 70, 2) > 180);
}

#[test]
fn a_frame_with_many_instances_survives_the_buffer_growth() {
    let Some(mut r) = renderer(64, 64) else {
        return;
    };
    let mut scene = plain_scene();
    // Far more quads than the initial 2048-instance buffer.
    for i in 0..5000 {
        scene.push(
            Layer::Effects,
            Quad::solid(
                (i % 64) as f32,
                (i / 64) as f32,
                1.0,
                1.0,
                [1.0, 1.0, 1.0, 1.0],
            ),
        );
    }
    let (_, _, image) = r.render_to_image(&scene, 0.0);
    assert_eq!(image.len(), 64 * 64 * 4);
}

#[test]
fn a_zero_size_scene_does_not_panic() {
    let Some(mut r) = renderer(16, 16) else {
        return;
    };
    let mut scene = plain_scene();
    scene.resize(0.0, 0.0);
    let (_, _, image) = r.render_to_image(&scene, 0.0);
    assert!(!image.is_empty());
}

#[test]
fn instance_building_is_pure_and_matches_the_scene() {
    let font = Font::build();
    let mut scene = plain_scene();
    scene.push(
        Layer::Playfield,
        Quad::solid(1.0, 2.0, 3.0, 4.0, [0.5, 0.25, 0.0, 1.0]),
    );
    scene.text(Text::new("HI", 0.0, 0.0, 1.0, [1.0; 4]));
    scene.particle(tetris_render::Particle::new(10.0, 10.0, 2.0, [1.0; 4]));

    let first = build_instances(&scene, &font);
    let second = build_instances(&scene, &font);
    assert_eq!(first, second, "instance building must be deterministic");
    assert_eq!(first[0].pos(), [1.0, 2.0]);
    assert_eq!(first[0].color(), [0.5, 0.25, 0.0, 1.0]);
    // One quad + two glyphs + one particle.
    assert_eq!(first.len(), 4);
}

#[test]
fn shake_moves_the_world_but_not_the_hud() {
    let font = Font::build();
    let mut scene = plain_scene();
    scene.push(Layer::Pieces, Quad::solid(10.0, 10.0, 4.0, 4.0, [1.0; 4]));
    scene.push(Layer::Hud, Quad::solid(10.0, 10.0, 4.0, 4.0, [1.0; 4]));
    scene.shake = 6.0;
    let instances = build_instances(&scene, &font);
    assert_eq!(instances.len(), 2);
    assert_ne!(instances[0].pos(), [10.0, 10.0], "playfield should shake");
    assert_eq!(instances[1].pos(), [10.0, 10.0], "HUD must not shake");
}
