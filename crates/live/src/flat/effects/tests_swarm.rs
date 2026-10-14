use super::super::tests::test_curve;
use super::super::{FlatIcons, FlatScene, Icon, Icons, icons, scaling, translation};
use super::*;

fn percentile(values: &mut [f64], at: usize) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() * at / 100]
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_swarm_of_shots_and_particles_fits_a_120_hz_frame_on_the_gpu_at_4k() {
    use crate::renderer::Renderer;
    use pfx_gpu::{Gpu, wgpu};
    let size = [3840u32, 2160u32];
    let layout = [3840.0f32, 2160.0];
    let icons = Icons::bake(&[icons::tick()]).unwrap();
    let curve = test_curve();
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
    let mut turns = pfx_gpu::pace::Turns::default();
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut renderer = turns.cpu(|| Renderer::new(gpu, size[0], size[1]).unwrap());
    let target = renderer
        .gpu()
        .offscreen(size[0], size[1], wgpu::TextureFormat::Rgba16Float)
        .unwrap();
    let flat_icons = FlatIcons::new(&renderer.gpu().device, &renderer.gpu().queue, icons);
    let mut solo = super::super::FlatPass::new(&renderer.gpu().device, &renderer.gpu().queue);
    let mut report = String::new();
    let mut worst = 0.0f64;
    for (variant, [bursting, shooting]) in [
        ("warm-up", [true, true]),
        ("2,000 icon shots", [false, true]),
        ("20,000 particles", [true, false]),
        ("2,000 icon shots and 20,000 particles", [true, true]),
    ] {
        turns.poll();
        let mut effects = Effects::new(EffectsDesc {
            particles: 22_000,
            ..EffectsDesc::new(layout, 21)
        });
        let ids: Vec<_> = recipes
            .iter()
            .map(|r| effects.recipe(*r).unwrap())
            .collect();
        let mut clock = Clock::new(60.0).unwrap();
        let mut game = Vec::new();
        let mut gpu_ms = Vec::new();
        let mut cpu_ms = Vec::new();
        let mut parts = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
        let (mut live, mut drawn) = (0, 0);
        for frame in 0..240u32 {
            turns.poll();
            let started = std::time::Instant::now();
            let tick = clock.advance(1.0 / 60.0);
            if bursting {
                for set in 0..4u32 {
                    for (k, id) in ids.iter().enumerate() {
                        let k = k as u32;
                        let x = 200.0 + ((frame * 53 + set * 911 + k * 131) % 3400) as f32;
                        let y = 200.0 + ((frame * 29 + set * 487 + k * 71) % 1700) as f32;
                        effects.burst(*id, [x, y]);
                    }
                }
            }
            let stepping = std::time::Instant::now();
            effects.step(&tick);
            let stepped = stepping.elapsed().as_secs_f64() * 1000.0;
            game.clear();
            if shooting {
                let t = clock.time() as f32;
                for s in 0..2000u32 {
                    let x = 40.0 + ((s * 97) % 3760) as f32;
                    let y = 2120.0 - ((s as f32 * 0.618_034) + t * 0.8).fract() * 2080.0;
                    game.push(
                        Draw::new(Shape::Icon(Icon(0)))
                            .transform(multiply(translation(x, y), scaling(0.35, 0.35)))
                            .fill(colours[s as usize % 5])
                            .elevation(1.0)
                            .shadow(Shadow::None)
                            .unpicked(),
                    );
                }
            }
            live = effects.live();
            let composing = std::time::Instant::now();
            let (draws, light) = effects.compose(&game, Light::default());
            drawn = draws.len();
            let composed = composing.elapsed().as_secs_f64() * 1000.0;
            let cpu = started.elapsed().as_secs_f64() * 1000.0;
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
            let preparing = std::time::Instant::now();
            solo.prepare(&renderer.gpu().device, &renderer.gpu().queue, &scene, size)
                .unwrap();
            let prepared = preparing.elapsed().as_secs_f64() * 1000.0;
            turns.poll();
            let rendering = std::time::Instant::now();
            let timings = renderer.render_flat(&scene, &target.view).unwrap();
            let rendered = rendering.elapsed().as_secs_f64() * 1000.0;
            let total = timings.iter().map(|t| t.milliseconds).sum::<f64>();
            turns.add(total);
            if frame >= 120 && !timings.is_empty() {
                gpu_ms.push(total);
                cpu_ms.push(cpu);
                parts[0].push(stepped);
                parts[1].push(composed);
                parts[2].push(rendered);
                parts[3].push(prepared);
            }
        }
        if variant == "warm-up" {
            continue;
        }
        assert!(gpu_ms.len() >= 100, "GPU timestamps are unavailable");
        let (gpu_median, cpu_median) = (percentile(&mut gpu_ms, 50), percentile(&mut cpu_ms, 50));
        let (gpu_p99, cpu_p99) = (percentile(&mut gpu_ms, 99), percentile(&mut cpu_ms, 99));
        worst = worst.max(gpu_p99);
        let [step, compose, render, prepare] = parts.each_mut().map(|part| percentile(part, 50));
        report += &format!(
            "{variant}: step {step:.2} ms, compose {compose:.2} ms, render_flat wall {render:.2} ms, of which prepare {prepare:.2} ms (medians)\n"
        );
        report += &format!(
            "{variant}: GPU {gpu_median:.2} ms (p99 {gpu_p99:.2}), effects and compose on the CPU {cpu_median:.2} ms (p99 {cpu_p99:.2}); {drawn} draws, {live} particles\n"
        );
        if bursting {
            assert!(live >= 18_000, "{live} particles");
        }
    }
    eprintln!("{report}");
    assert!(worst < 8.3, "GPU p99 {worst:.2} ms over 8.3\n{report}");
}
