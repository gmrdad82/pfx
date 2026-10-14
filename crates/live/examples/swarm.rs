use std::path::PathBuf;

use pfx_core::clock::Clock;
use pfx_core::ease::Ease;
use pfx_gpu::{Gpu, wgpu};
use pfx_live::flat::effects::{Burst, Curve, Effects, EffectsDesc};
use pfx_live::flat::{
    CurvePoint, Draw, Fill, FlatIcons, FlatScene, Icon, Icons, Light, Shadow, ShadowCurve, Shape,
    Srgba, Stroke, icons, multiply, scaling, translation,
};
use pfx_live::renderer::Renderer;

fn arg(index: usize, default: u32) -> u32 {
    std::env::args()
        .nth(index)
        .map(|value| value.parse().expect("frames, width and height are numbers"))
        .unwrap_or(default)
}

fn main() {
    let out = PathBuf::from(
        std::env::args()
            .nth(1)
            .expect("usage: swarm <dir> [frames] [width] [height] [shot scale]"),
    );
    let frames = arg(2, 360);
    let size = [arg(3, 1920), arg(4, 1080)];
    let scale: f32 = std::env::args()
        .nth(5)
        .map(|value| value.parse().expect("the shot scale is a number"))
        .unwrap_or(0.35);
    std::fs::create_dir_all(&out).unwrap();
    let layout = [3840.0f32, 2160.0];
    let curve = ShadowCurve::new(
        Srgba::hex(0x2b3442),
        vec![CurvePoint {
            elevation: 1.0,
            offset: 4.0,
            sigma: 5.0,
            alpha: 0.22,
            contact: None,
        }],
    )
    .unwrap();
    let colours = [0xe2614c, 0x8e57b0, 0xf0b429, 0x138f8a, 0x3d8fd8].map(Srgba::hex);
    let recipes = [
        Burst {
            count: 40,
            speed: [150.0, 520.0],
            life: [0.8, 1.2],
            drag: 0.8,
            size: [0.6, 1.4],
            scale: Curve::new(1.0, 0.4, Ease::QuadIn),
            ..Burst::new(Shape::Circle { radius: 4.0 }, Fill::Solid(colours[2]))
        },
        Burst {
            count: 30,
            speed: [100.0, 420.0],
            life: [0.8, 1.2],
            drag: 0.5,
            spin: [2.0, 12.0],
            size: [0.8, 1.6],
            ..Burst::new(
                Shape::Rect {
                    half: [5.0, 3.0],
                    radii: [1.0; 4],
                },
                Fill::Solid(colours[0]),
            )
        },
        Burst {
            count: 15,
            speed: [60.0, 200.0],
            life: [0.8, 1.2],
            size: [0.8, 1.2],
            additive: true,
            ..Burst::new(
                Shape::Ring {
                    radius: 5.0,
                    width: 1.5,
                },
                Fill::Solid(colours[4]),
            )
        },
    ];
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut renderer = Renderer::new(gpu, size[0], size[1]).unwrap();
    let target = renderer
        .gpu()
        .offscreen(size[0], size[1], wgpu::TextureFormat::Rgba16Float)
        .unwrap();
    let flat_icons = FlatIcons::new(
        &renderer.gpu().device,
        &renderer.gpu().queue,
        Icons::bake(&[icons::tick()]).unwrap(),
    );
    let mut effects = Effects::new(EffectsDesc {
        particles: 22_000,
        ..EffectsDesc::new(layout, 21)
    });
    let ids: Vec<_> = recipes
        .iter()
        .map(|recipe| effects.recipe(*recipe).unwrap())
        .collect();
    let mut clock = Clock::new(60.0).unwrap();
    let mut game = Vec::new();
    let mut gpu_ms = Vec::new();
    for frame in 0..frames + 60 {
        let tick = clock.advance(1.0 / 60.0);
        for set in 0..4u32 {
            for (k, id) in ids.iter().enumerate() {
                let k = k as u32;
                let x = 200.0 + ((frame * 53 + set * 911 + k * 131) % 3400) as f32;
                let y = 200.0 + ((frame * 29 + set * 487 + k * 71) % 1700) as f32;
                effects.burst(*id, [x, y]);
            }
        }
        effects.step(&tick);
        game.clear();
        let t = clock.time() as f32;
        for s in 0..2000u32 {
            let x = 40.0 + ((s * 97) % 3760) as f32;
            let y = 2120.0 - ((s as f32 * 0.618_034) + t * 0.8).fract() * 2080.0;
            game.push(
                Draw::new(Shape::Icon(Icon(0)))
                    .transform(multiply(translation(x, y), scaling(scale, scale)))
                    .stroke(Stroke::solid(colours[s as usize % 5], 7.0))
                    .elevation(1.0)
                    .shadow(Shadow::None)
                    .unpicked(),
            );
        }
        let live = effects.live();
        let (draws, light) = effects.compose(&game, Light::default());
        let drawn = draws.len();
        let scene = FlatScene {
            layout,
            clear: Some(Srgba::hex(0x101820)),
            curve: &curve,
            light,
            draws,
            groups: &[],
            text: &[],
            icons: Some(&flat_icons),
            sprites: None,
            environment: None,
            post: false,
            frame,
            seed: 0,
        };
        let timings = renderer.render_flat(&scene, &target.view).unwrap();
        if frame < 60 {
            continue;
        }
        gpu_ms.push(timings.iter().map(|t| t.milliseconds).sum::<f64>());
        let pixels = renderer.gpu().readback_rgba16(&target).unwrap();
        let image: Vec<u8> = pixels
            .chunks_exact(4)
            .flat_map(|p| {
                let channel = |c: usize| {
                    (half::f16::from_bits(p[c]).to_f32().clamp(0.0, 1.0) * 255.0).round() as u8
                };
                [channel(0), channel(1), channel(2), 255]
            })
            .collect();
        let file = std::fs::File::create(out.join(format!("{:04}.png", frame - 60))).unwrap();
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), size[0], size[1]);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&image)
            .unwrap();
        if frame == frames + 59 {
            println!("{drawn} draws, {live} particles on the last frame");
        }
    }
    gpu_ms.sort_by(f64::total_cmp);
    println!(
        "{frames} frames of {}x{} in {}; GPU p50 {:.3} ms, max {:.3} ms",
        size[0],
        size[1],
        out.display(),
        gpu_ms[gpu_ms.len() / 2],
        gpu_ms[gpu_ms.len() - 1]
    );
}
