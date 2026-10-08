use pfx_geom::mesh::Mesh as GeomMesh;
use pfx_gpu::pace::{Pacer, Turns};
use pfx_gpu::{Gpu, OffscreenTarget, wgpu};
use pfx_live::frame::{
    Camera as LiveCamera, Instance, Matrix, MeshData, Scene as LiveScene, SceneWind,
    Sun as LiveSun, multiply, transform,
};
use pfx_live::glass;
use pfx_live::lights::LocalLight as LiveLight;
use pfx_live::renderer::{Effects, Exposure, Finish, Renderer, Text};
use pfx_load::Sky;
use pfx_materials::Material;
use pfx_post::Chain;
use pfx_post::color::srgb_channel;
use pfx_trace::detail::{Bounces, Lens};
use pfx_trace::lights::LocalLight;
use pfx_trace::stage::{Mesh, Placement, Stage};
use pfx_trace::{Camera, Output, Projection, Sun};

const SIZE: u32 = 128;
const FOV_Y: f32 = 40.0;
const EYE: f32 = 4.0;
const SLAB: [f32; 3] = [0.5, 0.5, 0.2];
const FLOOR_TOP: f32 = -0.9;
const SUN: [f32; 3] = [1.0, 0.0, 0.8];
const LIGHT: [f32; 3] = [1.5, 0.0, 1.0];
const SUN_SHADE: [f32; 3] = [-1.125, 0.0, FLOOR_TOP];
const LIGHT_SHADE: [f32; 3] = [-1.35, 0.0, FLOOR_TOP];
const SAMPLES: u32 = 32;
const BOUNCED_SAMPLES: u32 = 128;
const DIRECT_TOLERANCE: f32 = 0.02;
const BOUNCED_ABOVE: f32 = 0.15;
const OFF_BYTES: [u64; 3] = [
    0xec54_cf52_6c87_4a86,
    0x41ef_cdf1_2868_1a39,
    0xb632_fda5_6681_ecfc,
];

fn identity() -> Matrix {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

fn translated(at: [f32; 3]) -> Matrix {
    let mut model = identity();
    model[3] = [at[0], at[1], at[2], 1.0];
    model
}

fn sun_direction() -> [f32; 3] {
    let length = SUN.iter().map(|value| value * value).sum::<f32>().sqrt();
    SUN.map(|value| value / length)
}

fn box_mesh(half: [f32; 3]) -> GeomMesh {
    let faces: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
        ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
        ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
        ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
        ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ([0.0, 0.0, -1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]),
    ];
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();
    for (normal, u, v) in faces {
        let first = positions.len() as u32;
        for (a, b) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            positions.push(std::array::from_fn(|k| {
                (normal[k] + u[k] * a + v[k] * b) * half[k]
            }));
            normals.push(normal);
            uvs.push([(a + 1.0) * 0.5, (b + 1.0) * 0.5]);
        }
        indices.extend([first, first + 1, first + 2, first, first + 2, first + 3]);
    }
    GeomMesh::new(positions, normals, Vec::new(), uvs, indices)
}

fn staged(mesh: &GeomMesh) -> Mesh<'_> {
    Mesh {
        positions: &mesh.positions,
        normals: &mesh.normals,
        tangents: &mesh.tangents,
        uvs: &mesh.uvs,
        alpha: None,
        indices: &mesh.indices,
    }
}

fn tinted_glass() -> Material {
    Material {
        base: [0.5, 0.62, 0.95],
        roughness: 0.05,
        transmission: 0.9,
        ior: 1.5,
        ..Material::default()
    }
}

fn absorbing_glass() -> Material {
    Material {
        base: [0.85, 0.7, 0.6],
        roughness: 0.05,
        transmission: 0.95,
        ior: 1.5,
        thickness: 0.4,
        absorption: 2.0,
        ..Material::default()
    }
}

fn floor_material() -> Material {
    Material {
        base: [0.6; 3],
        roughness: 0.6,
        ..Material::default()
    }
}

fn black_sky() -> Sky {
    Sky {
        width: 1,
        height: 1,
        texels: vec![[0.0, 0.0, 0.0, 1.0]],
    }
}

fn perspective() -> Matrix {
    let near = 0.1;
    let far = 100.0;
    let f = 1.0 / (FOV_Y.to_radians() * 0.5).tan();
    [
        [f, 0.0, 0.0, 0.0],
        [0.0, f, 0.0, 0.0],
        [0.0, 0.0, far / (near - far), -1.0],
        [0.0, 0.0, far * near / (near - far), 0.0],
    ]
}

fn live_camera() -> LiveCamera {
    let mut view = identity();
    view[3][2] = -EYE;
    LiveCamera {
        view,
        projection: perspective(),
        previous_view_projection: multiply(perspective(), view),
        position: [0.0, 0.0, EYE],
    }
}

fn trace_camera() -> Camera {
    let tan = (FOV_Y.to_radians() * 0.5).tan();
    Camera {
        origin: [0.0, 0.0, EYE],
        forward: [0.0, 0.0, -1.0],
        right: [tan, 0.0, 0.0],
        up: [0.0, tan, 0.0],
    }
}

fn pixel(world: [f32; 3]) -> (usize, usize) {
    let camera = live_camera();
    let clip = transform(
        multiply(camera.projection, camera.view),
        [world[0], world[1], world[2], 1.0],
    );
    let x = ((clip[0] / clip[3] * 0.5 + 0.5) * SIZE as f32) as usize;
    let y = ((0.5 - clip[1] / clip[3] * 0.5) * SIZE as f32) as usize;
    (x, y)
}

fn around(pixels: &[[f32; 3]], world: [f32; 3]) -> [f32; 3] {
    let (cx, cy) = pixel(world);
    let mut sum = [0.0; 3];
    let mut count = 0.0;
    for y in cy - 2..=cy + 2 {
        for x in cx - 2..=cx + 2 {
            let value = pixels[y * SIZE as usize + x];
            for k in 0..3 {
                sum[k] += value[k];
            }
            count += 1.0;
        }
    }
    sum.map(|value| value / count)
}

#[derive(Clone, Copy)]
struct Shot {
    glass: Option<Material>,
    sun: f32,
    light: bool,
}

impl Shot {
    fn trace_light(&self) -> Vec<LocalLight> {
        if !self.light {
            return Vec::new();
        }
        vec![LocalLight {
            radius: 0.05,
            ..LocalLight::point(LIGHT, [1.0; 3], 2.5, 8.0)
        }]
    }

    fn live_light(&self) -> Vec<LiveLight> {
        if !self.light {
            return Vec::new();
        }
        vec![LiveLight {
            position: LIGHT,
            colour: [1.0; 3],
            intensity: 2.5,
            radius: 0.05,
            range: 8.0,
            shadow: true,
        }]
    }
}

fn output(gpu: &Gpu) -> OffscreenTarget {
    let format = wgpu::TextureFormat::Rgba16Float;
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("glass shadow output"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    OffscreenTarget {
        texture,
        view,
        format,
        width: SIZE,
        height: SIZE,
    }
}

fn live(shot: Shot, turns: &mut Turns) -> Vec<[f32; 3]> {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut renderer = Renderer::new(gpu, SIZE, SIZE).unwrap();
    renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
    renderer.set_finish(Chain::new());
    renderer.set_lights(&shot.live_light()).unwrap();
    let floor = box_mesh([3.0, 3.0, 0.1]);
    let floor = renderer
        .upload_mesh(MeshData {
            positions: &floor.positions,
            normals: &floor.normals,
            tangents: &floor.tangents,
            uvs: &floor.uvs,
            uvs1: None,
            alpha: None,
            indices: &floor.indices,
        })
        .unwrap();
    let instances = [Instance::new(
        floor,
        translated([0.0, 0.0, FLOOR_TOP - 0.1]),
        0,
        1,
    )];
    let surfaces: Vec<glass::Surface> = shot
        .glass
        .iter()
        .map(|material| glass::Surface {
            mesh: renderer.upload_glass(&box_mesh(SLAB)).unwrap(),
            model: identity(),
            material: *material,
            liquid: false,
            fluid_height: 0.0,
            ripple_height: 0.0,
            caustic_strength: 0.0,
            tinted: None,
            id: 0,
            casts_shadow: true,
            shadow_only: false,
            clip: [[0.0; 4]; 2],
        })
        .collect();
    let materials = [floor_material()];
    let effects = Effects {
        glass: &surfaces,
        ..Default::default()
    };
    let target = output(renderer.gpu());
    for number in 0..6 {
        let scene = LiveScene {
            camera: live_camera(),
            time: number as f32 / 60.0,
            seed: 5,
            sun: LiveSun {
                direction: sun_direction(),
                colour: [1.0; 3],
                intensity: shot.sun,
            },
            instances: &instances,
            materials: &materials,
            deformers: &[],
            wind: SceneWind::default(),
        };
        turns.poll();
        let timings = renderer
            .render(
                &scene,
                &Text::default(),
                &effects,
                Finish::Standard,
                &target.view,
            )
            .unwrap();
        turns.add(pfx_gpu::pace::gpu_ms(&timings).unwrap_or(f64::NAN));
    }
    turns
        .cpu(|| renderer.gpu().readback_rgba16(&target))
        .unwrap()
        .chunks_exact(4)
        .map(|texel| std::array::from_fn(|k| srgb_channel(half::f16::from_bits(texel[k]).to_f32())))
        .collect()
}

struct Traced {
    pixels: Vec<[f32; 3]>,
    output: Output,
    per_sample_ms: f64,
    longest_ms: f64,
}

fn traced(
    gpu: &Gpu,
    shot: Shot,
    transmissive: bool,
    bounces: Bounces,
    samples: u32,
    turns: &mut Turns,
) -> Traced {
    let floor = box_mesh([3.0, 3.0, 0.1]);
    let slab = box_mesh(SLAB);
    let meshes = [staged(&floor), staged(&slab)];
    let mut placements = vec![Placement::new(
        0,
        translated([0.0, 0.0, FLOOR_TOP - 0.1]),
        0,
    )];
    let mut materials = vec![floor_material()];
    if let Some(glass) = shot.glass {
        materials.push(glass);
        placements.push(Placement::new(1, identity(), 1));
    }
    let mut staged = Stage {
        meshes: &meshes,
        instances: &placements,
        materials: &materials,
        sky: black_sky(),
        sun: Sun {
            direction: sun_direction(),
            color: [1.0; 3],
            intensity: shot.sun,
        },
        camera: trace_camera(),
        projection: Projection::Perspective,
        lens: Lens::default(),
    }
    .build()
    .unwrap();
    staged.detail.lights = shot.trace_light();
    staged.detail.transmissive_shadows = transmissive;
    staged.detail.bounces = bounces;
    let mut trace = staged.trace(gpu, SIZE, SIZE).unwrap();
    let mut pacer = Pacer::default();
    trace
        .sample_paced(gpu, 1, 11, &mut pacer, |ms| turns.add(ms))
        .unwrap();
    let stats = trace
        .sample_paced(gpu, samples - 1, 12, &mut pacer, |ms| turns.add(ms))
        .unwrap();
    turns.turn();
    let output = trace.readback(gpu).unwrap();
    let pixels = output
        .color
        .chunks_exact(16)
        .map(|texel| {
            std::array::from_fn(|k| f32::from_le_bytes(texel[k * 4..k * 4 + 4].try_into().unwrap()))
        })
        .collect();
    Traced {
        pixels,
        output,
        per_sample_ms: stats.milliseconds.iter().sum::<f64>() / f64::from(samples - 1),
        longest_ms: stats.longest_ms(),
    }
}

fn passed(with: [f32; 3], without: [f32; 3], dark: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|k| (with[k] - dark[k]) / (without[k] - dark[k]))
}

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

#[test]
fn the_shade_points_lie_in_the_slab_shadow_and_off_the_slab_on_screen() {
    let sun = sun_direction();
    let rise = (-SLAB[2] - FLOOR_TOP) / sun[2];
    let entry = [SUN_SHADE[0] + sun[0] * rise, SUN_SHADE[1] + sun[1] * rise];
    assert!(entry[0].abs() < SLAB[0] - 0.15 && entry[1].abs() < SLAB[1] - 0.15);
    let t = (FLOOR_TOP - LIGHT[2]) / (-LIGHT[2]);
    let under = [LIGHT[0] * (1.0 - t), LIGHT[1] * (1.0 - t)];
    assert!((under[0] - LIGHT_SHADE[0]).abs() < 0.01 && under[1].abs() < 1e-6);
    let slab_left = pixel([-SLAB[0], 0.0, SLAB[2]]).0;
    for shade in [SUN_SHADE, LIGHT_SHADE] {
        assert!(pixel(shade).0 + 6 < slab_left);
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_option_off_keeps_the_opaque_glass_shadow_bytes() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut turns = Turns::default();
    let shot = Shot {
        glass: Some(tinted_glass()),
        sun: 2.0,
        light: true,
    };
    let off = traced(&gpu, shot, false, Bounces::REFERENCE, 4, &mut turns);
    let hashes = [
        fnv(&off.output.color),
        fnv(&off.output.albedo),
        fnv(&off.output.normal),
    ];
    println!("option off: {hashes:x?}");
    assert_eq!(hashes, OFF_BYTES);
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_glass_pane_casts_live_s_tinted_shadow_under_the_sun_and_a_local_light() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut turns = Turns::default();
    let dark = live(
        Shot {
            glass: None,
            sun: 0.0,
            light: false,
        },
        &mut turns,
    );
    for (name, glass) in [("tinted", tinted_glass()), ("absorbing", absorbing_glass())] {
        let tint = glass::cast_tint(&glass);
        for (lit, shade, sun, light) in [
            ("sun", SUN_SHADE, 2.0, false),
            ("local light", LIGHT_SHADE, 0.0, true),
        ] {
            let with = Shot {
                glass: Some(glass),
                sun,
                light,
            };
            let without = Shot {
                glass: None,
                ..with
            };
            let live_passed = passed(
                around(&live(with, &mut turns), shade),
                around(&live(without, &mut turns), shade),
                around(&dark, shade),
            );
            for (budget, bounces, samples, above) in [
                ("direct light", Bounces::deep(0), SAMPLES, DIRECT_TOLERANCE),
                (
                    "reference bounces",
                    Bounces::REFERENCE,
                    BOUNCED_SAMPLES,
                    BOUNCED_ABOVE,
                ),
            ] {
                let mut run = |shot, transmissive| {
                    traced(&gpu, shot, transmissive, bounces, samples, &mut turns)
                };
                let open = run(without, true);
                let on = run(with, true);
                let off = run(with, false);
                let floor = around(&open.pixels, shade);
                let traced_passed = passed(around(&on.pixels, shade), floor, [0.0; 3]);
                let opaque = passed(around(&off.pixels, shade), floor, [0.0; 3]);
                println!(
                    "{name} glass, {lit}, {budget}: cast_tint {tint:.3?}, live {live_passed:.3?}, traced {traced_passed:.3?}, option off {opaque:.3?}; open floor traced {floor:.4?}"
                );
                assert!(on.longest_ms < 300.0 && off.longest_ms < 300.0);
                for k in 0..3 {
                    assert!(
                        (-DIRECT_TOLERANCE..=above).contains(&(traced_passed[k] - live_passed[k])),
                        "{name} glass, {lit}, {budget}, channel {k}: traced {traced_passed:?} against live {live_passed:?}"
                    );
                    assert!(
                        opaque[k] < above,
                        "{name} glass, {lit}, {budget}: the option off passes {opaque:?}"
                    );
                }
            }
        }
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn transmissive_shadows_cost_on_the_pane() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut turns = Turns::default();
    let shot = Shot {
        glass: Some(tinted_glass()),
        sun: 2.0,
        light: true,
    };
    for round in 0..2 {
        for transmissive in [false, true] {
            let run = traced(&gpu, shot, transmissive, Bounces::REFERENCE, 17, &mut turns);
            println!(
                "pane {SIZE}x{SIZE}, round {round}, option {}: {:.3} ms per sample, longest submission {:.2} ms",
                if transmissive { "on" } else { "off" },
                run.per_sample_ms,
                run.longest_ms
            );
            assert!(run.longest_ms < 300.0);
        }
    }
}
