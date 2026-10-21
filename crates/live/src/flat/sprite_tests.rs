use pfx_core::anim::{Atlas, Playback, SpriteClip, SpritePlayer, SpriteSheet};
use pfx_load::fixture::{atlas_colour, atlas_rgba};

use super::cpu;
use super::tests::{gpu, gpu_render};
use super::*;

const COLUMNS: u32 = 4;
const ROWS: u32 = 2;
const CELL: u32 = 8;

fn image(filter: SpriteFilter) -> SpriteImage {
    SpriteImage::new(
        COLUMNS * CELL,
        ROWS * CELL,
        atlas_rgba(COLUMNS, ROWS, CELL),
        filter,
    )
    .unwrap()
}

fn atlas() -> Atlas {
    Atlas::grid([COLUMNS * CELL, ROWS * CELL], [CELL, CELL], None).unwrap()
}

fn encoded(index: u32) -> [f32; 4] {
    atlas_colour(index).map(|byte| f32::from(byte) / 255.0)
}

fn scene<'a>(
    draws: &'a [Draw],
    curve: &'a ShadowCurve,
    sprites: Option<&'a FlatSprites>,
) -> FlatScene<'a> {
    FlatScene {
        layout: [160.0, 90.0],
        clear: Some(Srgba::hex(0x101418)),
        curve,
        light: Light::default(),
        draws,
        groups: &[],
        text: &[],
        icons: None,
        sprites,
        environment: None,
        post: false,
        frame: 0,
        seed: 0,
    }
}

fn frames() -> Vec<Draw> {
    let atlas = atlas();
    (0..4)
        .map(|index| {
            Draw::sprite(
                [25.0 + 36.0 * index as f32, 30.0],
                [32.0, 32.0],
                atlas.uv(index * 2).unwrap(),
            )
            .id(10 + index as u32)
        })
        .chain([Draw::rect([80.0, 72.0], [60.0, 24.0], 6.0)
            .fill(Srgba::hex(0x445566))
            .id(20)])
        .collect()
}

#[test]
fn a_sprite_draw_packs_its_frame_into_the_shapes_uvs() {
    let fit = Fit::new([100.0, 100.0], [100, 100]).unwrap();
    let draw = Draw::sprite([50.0, 50.0], [20.0, 40.0], [0.25, 0.5, 0.5, 1.0]);
    let packed = pack_shape(&draw, &fit, None).unwrap();
    assert_eq!(packed.info[0] & FILL_MASK, FILL_IMAGE);
    assert_eq!(packed.icon, [0.375, 0.75, 0.25 / 20.0, 0.5 / 40.0]);
    let styled = Draw {
        style: Some(Style::Outline(Outline::solid(2.0))),
        ..draw
    };
    let styled = pack_shape(&styled, &fit, None).unwrap();
    assert_ne!(styled.info[0] & FILL_MASK, FILL_IMAGE);
    let patterned = draw.pattern(Pattern::new(PatternKind::Dots, 4.0, 0.0, 0.5, Srgba::WHITE));
    let patterned = pack_shape(&patterned, &fit, None).unwrap();
    assert_eq!(patterned.info[0] & PATTERN_MASK, 0);
    assert_eq!(patterned.icon, packed.icon);
}

#[test]
fn the_cpu_twin_paints_each_frame_of_the_atlas() {
    let fit = Fit::new([160.0, 90.0], [160, 90]).unwrap();
    for filter in [SpriteFilter::Nearest, SpriteFilter::Linear] {
        let atlas_image = image(filter);
        for (index, draw) in frames().iter().take(4).enumerate() {
            let instance = pack_shape(draw, &fit, None).unwrap();
            let centre = instance.to_local([25.0 + 36.0 * index as f32, 30.0]);
            let paint = cpu::paint_sprite(&instance, None, None, Some(&atlas_image), centre);
            let expected = encoded(index as u32 * 2);
            for channel in 0..3 {
                assert!(
                    (paint.colour[channel] - expected[channel]).abs() < 1e-5,
                    "{filter:?} frame {index}: {:?} against {expected:?}",
                    paint.colour
                );
            }
        }
    }
}

#[test]
fn a_tinted_sprite_multiplies_its_texels() {
    let fit = Fit::new([10.0, 10.0], [10, 10]).unwrap();
    let mut draw = Draw::sprite([5.0, 5.0], [8.0, 8.0], atlas().uv(3).unwrap());
    draw.fill = Fill::Image(SpriteFill {
        frame: atlas().uv(3).unwrap(),
        tint: Srgba([1.0, 0.5, 0.0, 0.5]),
    });
    let instance = pack_shape(&draw, &fit, None).unwrap();
    let paint = cpu::paint_sprite(
        &instance,
        None,
        None,
        Some(&image(SpriteFilter::Nearest)),
        instance.to_local([5.0, 5.0]),
    );
    let texel = encoded(3);
    let expected = [texel[0] * 0.5, texel[1] * 0.5 * 0.5, 0.0, 0.5];
    for (got, want) in paint.colour.iter().zip(expected) {
        assert!((got - want).abs() < 1e-5);
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_gpu_draws_sprite_frames_like_the_twin_and_picks_them() {
    let gpu = gpu();
    let mut pass = FlatPass::new(&gpu.device, &gpu.queue);
    let curve = ShadowCurve::none();
    let draws = frames();
    for filter in [SpriteFilter::Nearest, SpriteFilter::Linear] {
        let sprites = FlatSprites::new(&gpu.device, &gpu.queue, image(filter));
        let scene = scene(&draws, &curve, Some(&sprites));
        let size = [320, 180];
        let (pixels, ids) = gpu_render(&gpu, &mut pass, &scene, size);
        let twin = cpu::render(&scene, None, size).unwrap();
        let mut worst = 0.0f32;
        for (ours, theirs) in pixels.iter().zip(&twin) {
            for channel in 0..4 {
                worst = worst.max((ours[channel] - theirs[channel]).abs());
            }
        }
        println!("{filter:?}: worst against the twin {}", worst * 255.0);
        assert!(worst <= 2.0 / 255.0, "{filter:?}: worst {}", worst * 255.0);
        for index in 0..4u32 {
            let x = ((25.0 + 36.0 * index as f32) * 2.0) as usize;
            let at = 60 * size[0] as usize + x;
            assert_eq!(ids[at], 10 + index);
            let expected = encoded(index * 2);
            for channel in 0..3 {
                assert!((pixels[at][channel] - expected[channel]).abs() <= 1.0 / 255.0);
            }
        }
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_sprite_clip_steps_a_flat_draw_through_its_frames_by_tick() {
    let gpu = gpu();
    let mut pass = FlatPass::new(&gpu.device, &gpu.queue);
    let curve = ShadowCurve::none();
    let sprites = FlatSprites::new(&gpu.device, &gpu.queue, image(SpriteFilter::Nearest));
    let walk = SpriteClip::new("walk", vec![4, 5, 6, 7], 12.0, Playback::Loop).unwrap();
    let sheet = std::sync::Arc::new(SpriteSheet::new(atlas(), vec![walk]).unwrap());
    let mut player = SpritePlayer::new(sheet.clone(), 60);
    player.play("walk", 0).unwrap();
    for (tick, frame) in [(0u64, 4u32), (5, 5), (10, 6), (15, 7), (20, 4)] {
        let draws = [Draw::sprite(
            [80.0, 45.0],
            [40.0, 40.0],
            player.uv(tick).unwrap(),
        )];
        let scene = scene(&draws, &curve, Some(&sprites));
        let (pixels, _) = gpu_render(&gpu, &mut pass, &scene, [160, 90]);
        let centre = pixels[45 * 160 + 80];
        let expected = encoded(frame);
        for channel in 0..3 {
            assert!(
                (centre[channel] - expected[channel]).abs() <= 1.0 / 255.0,
                "tick {tick}: {centre:?} against frame {frame}"
            );
        }
    }
}

#[test]
fn a_baked_group_samples_back_as_its_premultiplied_pixels_without_a_dark_edge() {
    let curve = ShadowCurve::none();
    let draws = [
        Draw::circle([16.0, 16.0], 12.0)
            .fill(Srgba::hex(0xf0c040))
            .shadow(Shadow::None)
            .group(0),
        Draw::circle([12.0, 13.0], 2.0)
            .fill(Srgba::hex(0x202020))
            .shadow(Shadow::None)
            .group(0),
    ];
    let groups = [Group { opacity: 0.8 }];
    let face = FlatScene {
        layout: [32.0, 32.0],
        clear: None,
        groups: &groups,
        ..scene(&draws, &curve, None)
    };
    let size = 32;
    let pixels = cpu::render(&face, None, [size, size]).unwrap();
    let sprite =
        SpriteImage::from_premultiplied(size, size, &pixels, SpriteFilter::Linear).unwrap();
    let at = |x: i64, y: i64| pixels[(y.clamp(0, 31) * 32 + x.clamp(0, 31)) as usize];
    let premultiplied = |c: [f32; 4]| [c[0] * c[3], c[1] * c[3], c[2] * c[3], c[3]];
    for y in 0..size as i64 {
        for x in 0..size as i64 {
            let centre = [(x as f32 + 0.5) / 32.0, (y as f32 + 0.5) / 32.0];
            let got = premultiplied(sprite.sample(centre));
            let between = premultiplied(sprite.sample([(x as f32 + 1.0) / 32.0, centre[1]]));
            let mean: [f32; 4] = std::array::from_fn(|k| (at(x, y)[k] + at(x + 1, y)[k]) * 0.5);
            for k in 0..4 {
                assert!(
                    (got[k] - at(x, y)[k]).abs() < 1.5 / 255.0,
                    "{x},{y}: {got:?}"
                );
                assert!(
                    (between[k] - mean[k]).abs() < 1.5 / 255.0,
                    "{x}.5,{y}: {between:?} vs {mean:?}"
                );
            }
        }
    }
}

const STREAK_SIZE: [u32; 2] = [240, 120];

fn streak_atlas(filter: SpriteFilter) -> SpriteImage {
    streak_atlas_with(filter, [0, 255, 0, 255])
}

fn streak_atlas_with(filter: SpriteFilter, right: [u8; 4]) -> SpriteImage {
    let mut rgba = Vec::with_capacity(24 * 8 * 4);
    for y in 0..8 {
        for x in 0..24 {
            let texel = match (x / 8, x % 8 < 4, y < 4) {
                (1, true, true) => [0, 0, 255, 255],
                (1, false, true) => right,
                (1, _, false) => [0, 0, 0, 255],
                _ => [255, 0, 0, 255],
            };
            rgba.extend_from_slice(&texel);
        }
    }
    SpriteImage::new(24, 8, rgba, filter).unwrap()
}

fn streak_draw(blur: [f32; 2]) -> Draw {
    Draw::sprite(
        [60.0, 30.0],
        [40.0, 40.0],
        [8.0 / 24.0, 0.0, 16.0 / 24.0, 1.0],
    )
    .blur(blur)
    .id(7)
}

fn streak_scene<'a>(
    draws: &'a [Draw],
    curve: &'a ShadowCurve,
    sprites: Option<&'a FlatSprites>,
) -> FlatScene<'a> {
    FlatScene {
        layout: [120.0, 60.0],
        clear: None,
        ..scene(draws, curve, sprites)
    }
}

fn streak_at(pixels: &[[f32; 4]], layout: [f32; 2]) -> [f32; 4] {
    let x = (layout[0] * 2.0) as usize;
    let y = (layout[1] * 2.0) as usize;
    pixels[y * STREAK_SIZE[0] as usize + x]
}

fn assert_streaked(pixels: &[[f32; 4]], label: &str) {
    for (index, pixel) in pixels.iter().enumerate() {
        assert!(
            pixel[0] < 1.0 / 255.0,
            "{label}: the neighbouring frame bled into pixel {index}: {pixel:?}"
        );
    }
    let top = streak_at(pixels, [60.0, 20.0]);
    assert!(top[1] > 0.2 && top[2] > 0.2, "{label}: top {top:?}");
    let bottom = streak_at(pixels, [60.0, 40.0]);
    assert!(bottom[3] > 0.9, "{label}: bottom {bottom:?}");
    assert!(
        bottom[1] < 0.02 && bottom[2] < 0.02,
        "{label}: bottom {bottom:?}"
    );
    let right = streak_at(pixels, [88.0, 20.0]);
    assert!(
        right[3] > 0.05 && right[1] > 0.05,
        "{label}: right {right:?}"
    );
    assert!(right[2] < 2.0 / 255.0, "{label}: right {right:?}");
    let left = streak_at(pixels, [32.0, 20.0]);
    assert!(left[3] > 0.05 && left[2] > 0.05, "{label}: left {left:?}");
    assert!(left[1] < 2.0 / 255.0, "{label}: left {left:?}");
    let above = streak_at(pixels, [60.0, 6.0]);
    assert!(above[3] < 1.0 / 255.0, "{label}: above {above:?}");
}

#[test]
fn a_blurred_sprite_keeps_its_image_fill_and_frame() {
    let fit = Fit::new([120.0, 60.0], [240, 120]).unwrap();
    let still = pack_shape(&streak_draw([0.0, 0.0]), &fit, None).unwrap();
    assert_eq!(
        pack_shape(&streak_draw([0.1, 0.0]), &fit, None),
        Some(still)
    );
    for broken in [
        [f32::NAN, 9.0],
        [f32::INFINITY, 0.0],
        [0.0, f32::NEG_INFINITY],
    ] {
        assert_eq!(pack_shape(&streak_draw(broken), &fit, None), Some(still));
    }
    let moving = pack_shape(&streak_draw([30.0, 0.0]), &fit, None).unwrap();
    assert_eq!(moving.info[0] & FILL_MASK, FILL_IMAGE);
    assert_eq!(moving.bounds[2], still.bounds[2] + 15.0);
    assert_eq!(moving.bounds[3], still.bounds[3]);
    let falling = pack_shape(&streak_draw([0.0, 30.0]), &fit, None).unwrap();
    assert_eq!(falling.bounds[3], still.bounds[3] + 15.0);
    assert_eq!(falling.icon, still.icon);
    assert_eq!(moving.info[0] & BLUR, BLUR);
    assert_eq!(moving.icon, still.icon);
    assert_eq!(moving.fill_top, still.fill_top);
}

fn twin(draw: &Draw, atlas: Option<&SpriteImage>) -> Vec<[f32; 4]> {
    let fit = Fit::new([120.0, 60.0], STREAK_SIZE).unwrap();
    let instance = pack_shape(draw, &fit, None).unwrap();
    (0..STREAK_SIZE[1])
        .flat_map(|y| (0..STREAK_SIZE[0]).map(move |x| (x, y)))
        .map(|(x, y)| {
            let at = instance.to_local([x as f32 + 0.5, y as f32 + 0.5]);
            cpu::paint_sprite(&instance, None, None, atlas, at).colour
        })
        .collect()
}

fn assert_no_bleed(pixels: &[[f32; 4]], label: &str) {
    for (index, pixel) in pixels.iter().enumerate() {
        assert!(
            pixel[0] < 1.0 / 255.0,
            "{label}: the neighbouring frame bled into pixel {index}: {pixel:?}"
        );
    }
}

fn filters() -> [SpriteFilter; 2] {
    [SpriteFilter::Nearest, SpriteFilter::Linear]
}

fn flipped(blur: [f32; 2]) -> Draw {
    Draw::sprite(
        [60.0, 30.0],
        [40.0, 40.0],
        [16.0 / 24.0, 0.0, 8.0 / 24.0, 1.0],
    )
    .blur(blur)
}

fn disc(blur: [f32; 2]) -> Draw {
    let mut draw = Draw::circle([60.0, 30.0], 20.0).blur(blur);
    draw.fill = Fill::Image(SpriteFill::new([8.0 / 24.0, 0.0, 16.0 / 24.0, 1.0]));
    draw
}

fn tinted(blur: [f32; 2]) -> Draw {
    let mut draw = streak_draw(blur);
    draw.fill = Fill::Image(SpriteFill {
        frame: [8.0 / 24.0, 0.0, 16.0 / 24.0, 1.0],
        tint: Srgba([1.0, 0.5, 0.0, 0.5]),
    });
    draw
}

const OPAQUE_RIGHT: [u8; 4] = [0, 255, 0, 255];
const CLEAR_RIGHT: [u8; 4] = [0, 0, 0, 0];

fn story_cases() -> Vec<(&'static str, Draw, [u8; 4])> {
    vec![
        ("sideways", streak_draw([30.0, 0.0]), OPAQUE_RIGHT),
        ("flipped", flipped([30.0, 0.0]), OPAQUE_RIGHT),
        ("vertical", streak_draw([0.0, 30.0]), OPAQUE_RIGHT),
        ("diagonal", streak_draw([-24.0, 18.0]), OPAQUE_RIGHT),
        ("past the taps", streak_draw([100.0, 0.0]), OPAQUE_RIGHT),
        ("disc", disc([30.0, 0.0]), OPAQUE_RIGHT),
        ("tinted", tinted([30.0, 0.0]), OPAQUE_RIGHT),
        ("half clear", streak_draw([30.0, 0.0]), CLEAR_RIGHT),
    ]
}

#[test]
fn the_twin_streaks_a_blurred_sprite_inside_its_own_frame() {
    let fit = Fit::new([120.0, 60.0], STREAK_SIZE).unwrap();
    let still = pack_shape(&streak_draw([0.0, 0.0]), &fit, None).unwrap();
    for filter in filters() {
        let atlas = streak_atlas(filter);
        assert_streaked(
            &twin(&streak_draw([30.0, 0.0]), Some(&atlas)),
            &format!("{filter:?} twin"),
        );
        let at = still.to_local([176.5, 40.5]);
        assert_eq!(
            cpu::paint_sprite(&still, None, None, Some(&atlas), at).colour[3],
            0.0
        );
    }
}

#[test]
fn a_flipped_blurred_frame_streaks_mirrored_inside_its_own_cell() {
    for filter in filters() {
        let pixels = twin(&flipped([30.0, 0.0]), Some(&streak_atlas(filter)));
        let label = format!("{filter:?}");
        assert_no_bleed(&pixels, &label);
        let left = streak_at(&pixels, [32.0, 20.0]);
        assert!(left[1] > 0.05 && left[2] < 2.0 / 255.0, "{label}: {left:?}");
        let right = streak_at(&pixels, [88.0, 20.0]);
        assert!(
            right[2] > 0.05 && right[1] < 2.0 / 255.0,
            "{label}: {right:?}"
        );
    }
}

#[test]
fn a_vertical_blur_streaks_along_y_and_never_across() {
    for filter in filters() {
        let pixels = twin(&streak_draw([0.0, 30.0]), Some(&streak_atlas(filter)));
        let label = format!("{filter:?}");
        assert_no_bleed(&pixels, &label);
        let left = streak_at(&pixels, [50.0, 30.0]);
        assert!(left[2] > 0.05 && left[1] < 2.0 / 255.0, "{label}: {left:?}");
        let right = streak_at(&pixels, [70.0, 30.0]);
        assert!(
            right[1] > 0.05 && right[2] < 2.0 / 255.0,
            "{label}: {right:?}"
        );
        assert!(streak_at(&pixels, [60.0, 4.0])[3] > 0.05, "{label}: above");
        assert!(streak_at(&pixels, [60.0, 56.0])[3] > 0.05, "{label}: below");
        assert!(
            streak_at(&pixels, [36.0, 30.0])[3] < 1.0 / 255.0,
            "{label}: beside"
        );
    }
}

#[test]
fn a_blur_longer_than_the_taps_still_stays_in_its_frame() {
    for filter in filters() {
        let pixels = twin(&streak_draw([100.0, 0.0]), Some(&streak_atlas(filter)));
        let label = format!("{filter:?}");
        assert_no_bleed(&pixels, &label);
        let top = streak_at(&pixels, [60.0, 20.0]);
        assert!(top[1] > 0.05 && top[2] > 0.05, "{label}: {top:?}");
        assert!(
            streak_at(&pixels, [100.0, 20.0])[3] > 0.02,
            "{label}: right"
        );
        assert!(streak_at(&pixels, [20.0, 20.0])[3] > 0.02, "{label}: left");
    }
}

#[test]
fn a_blurred_disc_sprite_streaks_inside_its_disc() {
    for filter in filters() {
        let pixels = twin(&disc([30.0, 0.0]), Some(&streak_atlas(filter)));
        let label = format!("{filter:?}");
        assert_no_bleed(&pixels, &label);
        let top = streak_at(&pixels, [60.0, 20.0]);
        assert!(top[1] > 0.05 && top[2] > 0.05, "{label}: {top:?}");
        assert!(
            streak_at(&pixels, [60.0, 6.0])[3] < 1.0 / 255.0,
            "{label}: above"
        );
        assert!(streak_at(&pixels, [88.0, 30.0])[3] > 0.02, "{label}: right");
    }
}

#[test]
fn a_tinted_blurred_sprite_multiplies_its_streak() {
    let pixels = twin(
        &tinted([30.0, 0.0]),
        Some(&streak_atlas(SpriteFilter::Linear)),
    );
    for pixel in &pixels {
        assert!(pixel[2] < 1e-6, "{pixel:?}");
        assert!(pixel[3] <= 0.5 + 1e-5, "{pixel:?}");
    }
    let top = streak_at(&pixels, [60.0, 20.0]);
    assert!(top[1] > 0.05 && top[3] > 0.4, "{top:?}");
}

#[test]
fn transparent_texels_thin_the_streak() {
    let atlas = streak_atlas_with(SpriteFilter::Linear, CLEAR_RIGHT);
    let pixels = twin(&streak_draw([30.0, 0.0]), Some(&atlas));
    assert_no_bleed(&pixels, "half clear");
    let top = streak_at(&pixels, [60.0, 20.0]);
    assert!(top[3] > 0.3 && top[3] < 0.7, "{top:?}");
    assert!(streak_at(&pixels, [88.0, 20.0])[3] < 2.0 / 255.0);
    assert!(streak_at(&pixels, [32.0, 20.0])[2] > 0.05);
}

#[test]
fn a_blurred_sprite_without_an_atlas_streaks_its_tint() {
    let pixels = twin(&streak_draw([30.0, 0.0]), None);
    let top = streak_at(&pixels, [60.0, 20.0]);
    assert!(top.iter().all(|&channel| channel > 0.99), "{top:?}");
    let right = streak_at(&pixels, [88.0, 20.0]);
    assert!(right[3] > 0.05, "{right:?}");
    for channel in 0..3 {
        assert!((right[channel] - right[3]).abs() < 1e-5, "{right:?}");
    }
}

#[test]
fn every_blur_story_paints_inside_its_bounds() {
    let fit = Fit::new([120.0, 60.0], STREAK_SIZE).unwrap();
    for (name, draw, right) in story_cases() {
        let instance = pack_shape(&draw, &fit, None).unwrap();
        assert_eq!(instance.info[0] & FILL_MASK, FILL_IMAGE, "{name}");
        assert_eq!(instance.info[0] & BLUR, BLUR, "{name}");
        let atlas = streak_atlas_with(SpriteFilter::Linear, right);
        let [cx, cy, hx, hy] = instance.bounds;
        let margin = instance.margin();
        for (index, pixel) in twin(&draw, Some(&atlas)).iter().enumerate() {
            let x = (index % STREAK_SIZE[0] as usize) as f32 + 0.5;
            let y = (index / STREAK_SIZE[0] as usize) as f32 + 0.5;
            let local = instance.to_local([x, y]);
            if (local[0] - cx).abs() > hx + margin[0] || (local[1] - cy).abs() > hy + margin[1] {
                assert!(pixel[3] < 1.0 / 255.0, "{name}: {x},{y} {pixel:?}");
            }
            assert!(pixel.iter().all(|value| value.is_finite()), "{name}");
        }
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_gpu_streaks_a_blurred_sprite_like_the_twin_without_bleeding() {
    let gpu = gpu();
    let mut pass = FlatPass::new(&gpu.device, &gpu.queue);
    let curve = ShadowCurve::none();
    let draws = [streak_draw([30.0, 0.0])];
    for filter in [SpriteFilter::Nearest, SpriteFilter::Linear] {
        let sprites = FlatSprites::new(&gpu.device, &gpu.queue, streak_atlas(filter));
        let scene = streak_scene(&draws, &curve, Some(&sprites));
        let (pixels, ids) = gpu_render(&gpu, &mut pass, &scene, STREAK_SIZE);
        let twin = cpu::render(&scene, None, STREAK_SIZE).unwrap();
        let label = format!("{filter:?}");
        assert_streaked(&twin, &format!("{label} twin"));
        assert_streaked(&pixels, &label);
        let mut worst = 0.0f32;
        for (ours, theirs) in pixels.iter().zip(&twin) {
            for channel in 0..4 {
                worst = worst.max((ours[channel] - theirs[channel]).abs());
            }
        }
        println!("{label}: worst against the twin {}", worst * 255.0);
        assert!(worst <= 2.0 / 255.0, "{label}: worst {}", worst * 255.0);
        assert_eq!(ids[40 * STREAK_SIZE[0] as usize + 120], 7);
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_sprite_under_the_blur_threshold_draws_the_same_bytes_as_a_still_one() {
    let gpu = gpu();
    let mut pass = FlatPass::new(&gpu.device, &gpu.queue);
    let curve = ShadowCurve::none();
    let sprites = FlatSprites::new(&gpu.device, &gpu.queue, streak_atlas(SpriteFilter::Linear));
    let still = [streak_draw([0.0, 0.0])];
    let slow = [streak_draw([0.1, 0.0])];
    let (a, _) = gpu_render(
        &gpu,
        &mut pass,
        &streak_scene(&still, &curve, Some(&sprites)),
        STREAK_SIZE,
    );
    let (b, _) = gpu_render(
        &gpu,
        &mut pass,
        &streak_scene(&slow, &curve, Some(&sprites)),
        STREAK_SIZE,
    );
    assert_eq!(a, b);
    let twin = cpu::render(
        &streak_scene(&still, &curve, Some(&sprites)),
        None,
        STREAK_SIZE,
    )
    .unwrap();
    for (ours, theirs) in a.iter().zip(&twin) {
        for channel in 0..4 {
            assert!((ours[channel] - theirs[channel]).abs() <= 2.0 / 255.0);
        }
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_gpu_draws_every_blur_story_like_the_twin_without_bleeding() {
    let gpu = gpu();
    let mut pass = FlatPass::new(&gpu.device, &gpu.queue);
    let curve = ShadowCurve::none();
    for filter in filters() {
        for (name, draw, right) in story_cases() {
            let sprites =
                FlatSprites::new(&gpu.device, &gpu.queue, streak_atlas_with(filter, right));
            let draws = [draw.id(9)];
            let scene = streak_scene(&draws, &curve, Some(&sprites));
            let (pixels, ids) = gpu_render(&gpu, &mut pass, &scene, STREAK_SIZE);
            let twin = cpu::render(&scene, None, STREAK_SIZE).unwrap();
            let label = format!("{filter:?} {name}");
            assert_no_bleed(&pixels, &label);
            let mut worst = 0.0f32;
            for (ours, theirs) in pixels.iter().zip(&twin) {
                for channel in 0..4 {
                    worst = worst.max((ours[channel] - theirs[channel]).abs());
                }
            }
            println!("{label}: worst against the twin {}", worst * 255.0);
            assert!(worst <= 2.0 / 255.0, "{label}: worst {}", worst * 255.0);
            assert_eq!(ids[40 * STREAK_SIZE[0] as usize + 120], 9, "{label}");
        }
    }
}
