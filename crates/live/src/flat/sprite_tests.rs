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
