use pfx_geom::mesh::Mesh;
use pfx_gpu::{Gpu, OffscreenTarget, wgpu};
use pfx_live::frame::{
    Camera, Instance, Matrix, MeshData, MeshHandle, Scene, SceneWind, Sun, multiply, transform,
};
use pfx_live::glass;
use pfx_live::lights::LocalLight;
use pfx_live::renderer::{Effects, Exposure, Finish, Renderer, Text};
use pfx_materials::Material;
use pfx_post::Chain;
use pfx_post::color::srgb_channel;

const SIZE: u32 = 256;
const SLAB: [f32; 3] = [0.5, 0.5, 0.2];
const SLAB_AT: [f32; 3] = [-0.8, 0.0, 0.0];
const FLOOR_TOP: f32 = -0.9;
const LIGHT_A: [f32; 3] = [0.6, 0.0, 1.2];
const LIGHT_B: [f32; 3] = [-0.8, 2.0, 1.2];
const NO_CLIP: [[f32; 4]; 2] = [[0.0; 4]; 2];

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

fn perspective() -> Matrix {
    let near = 0.1;
    let far = 100.0;
    let f = 1.0 / (40.0_f32.to_radians() * 0.5).tan();
    [
        [f, 0.0, 0.0, 0.0],
        [0.0, f, 0.0, 0.0],
        [0.0, 0.0, far / (near - far), -1.0],
        [0.0, 0.0, far * near / (near - far), 0.0],
    ]
}

fn camera() -> Camera {
    let mut view = identity();
    view[3][2] = -4.0;
    Camera {
        view,
        projection: perspective(),
        previous_view_projection: multiply(perspective(), view),
        position: [0.0, 0.0, 4.0],
    }
}

fn pixel(world: [f32; 3]) -> usize {
    let camera = camera();
    let clip = transform(
        multiply(camera.projection, camera.view),
        [world[0], world[1], world[2], 1.0],
    );
    let x = ((clip[0] / clip[3] * 0.5 + 0.5) * SIZE as f32) as usize;
    let y = ((0.5 - clip[1] / clip[3] * 0.5) * SIZE as f32) as usize;
    y * SIZE as usize + x
}

fn box_mesh(half: [f32; 3]) -> Mesh {
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
    Mesh::new(positions, normals, Vec::new(), uvs, indices)
}

fn upload(renderer: &mut Renderer, mesh: &Mesh) -> MeshHandle {
    renderer
        .upload_mesh(MeshData {
            positions: &mesh.positions,
            normals: &mesh.normals,
            tangents: &mesh.tangents,
            uvs: &mesh.uvs,
            uvs1: None,
            alpha: None,
            indices: &mesh.indices,
        })
        .unwrap()
}

fn clear_glass() -> Material {
    Material {
        base: [0.5, 0.62, 0.95],
        roughness: 0.05,
        transmission: 0.9,
        ior: 1.5,
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

fn light(position: [f32; 3]) -> LocalLight {
    LocalLight {
        position,
        colour: [1.0; 3],
        intensity: 2.5,
        radius: 0.05,
        range: 8.0,
        shadow: true,
    }
}

#[derive(Clone, Copy)]
struct Part {
    half: [f32; 3],
    at: [f32; 3],
    shadow_only: bool,
    casts_shadow: bool,
    clip: [[f32; 4]; 2],
}

impl Part {
    fn slab() -> Self {
        Self {
            half: SLAB,
            at: SLAB_AT,
            shadow_only: false,
            casts_shadow: true,
            clip: NO_CLIP,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Twin {
    None,
    Tinted,
    Opaque,
}

struct Shot<'a> {
    glass: Option<Part>,
    copy: Twin,
    lights: &'a [LocalLight],
    sun: f32,
    size: [u32; 2],
    frames: usize,
    drift: f32,
}

impl Default for Shot<'_> {
    fn default() -> Self {
        Self {
            glass: None,
            copy: Twin::None,
            lights: &[],
            sun: 2.0,
            size: [SIZE, SIZE],
            frames: 6,
            drift: 0.0,
        }
    }
}

struct Taken {
    raw: Vec<u16>,
    passes: Vec<&'static str>,
    timings: Vec<(String, f64)>,
}

impl Taken {
    fn encoded(&self, at: usize) -> [f32; 3] {
        [0, 1, 2].map(|k| half::f16::from_bits(self.raw[at * 4 + k]).to_f32())
    }

    fn linear(&self, at: usize) -> [f32; 3] {
        self.encoded(at).map(srgb_channel)
    }

    fn mean(&self, label: &str) -> f64 {
        let values: Vec<f64> = self
            .timings
            .iter()
            .filter(|(name, _)| name == label)
            .map(|(_, ms)| *ms)
            .collect();
        if values.is_empty() {
            return 0.0;
        }
        values.iter().sum::<f64>() / values.len() as f64
    }
}

fn output(gpu: &Gpu, size: [u32; 2]) -> OffscreenTarget {
    let format = wgpu::TextureFormat::Rgba16Float;
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("glass cast output"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
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
        width: size[0],
        height: size[1],
    }
}

fn take(shot: &Shot<'_>) -> Taken {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut turns = pfx_gpu::pace::Turns::default();
    let mut renderer = turns.cpu(|| Renderer::new(gpu, shot.size[0], shot.size[1]).unwrap());
    renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
    renderer.set_finish(Chain::new());
    renderer.set_lights(shot.lights).unwrap();
    let floor = upload(&mut renderer, &box_mesh([3.0, 3.0, 0.1]));
    let mut instances = vec![Instance::new(
        floor,
        translated([0.0, 0.0, FLOOR_TOP - 0.1]),
        0,
        1,
    )];
    let slab = Part::slab();
    if shot.copy != Twin::None {
        let copy = upload(&mut renderer, &box_mesh(slab.half));
        let mut instance = Instance::new(copy, translated(slab.at), 1, 2);
        instance.shadow_only = true;
        instances.push(instance);
        if shot.copy == Twin::Tinted {
            renderer
                .set_transmission(&[None, Some(glass::cast_tint(&clear_glass()))])
                .unwrap();
        }
    }
    let surfaces: Vec<glass::Surface> = shot
        .glass
        .iter()
        .map(|part| glass::Surface {
            mesh: renderer.upload_glass(&box_mesh(part.half)).unwrap(),
            model: translated(part.at),
            material: clear_glass(),
            liquid: false,
            fluid_height: 0.0,
            ripple_height: 0.0,
            caustic_strength: 0.0,
            tinted: None,
            id: 0,
            casts_shadow: part.casts_shadow,
            shadow_only: part.shadow_only,
            clip: part.clip,
        })
        .collect();
    let materials = [floor_material(), Material::default()];
    let effects = Effects {
        glass: &surfaces,
        ..Default::default()
    };
    let target = output(renderer.gpu(), shot.size);
    let mut timings = Vec::new();
    for number in 0..shot.frames {
        turns.poll();
        if shot.drift != 0.0 {
            let moved: Vec<LocalLight> = shot
                .lights
                .iter()
                .map(|light| LocalLight {
                    position: [
                        light.position[0] + shot.drift * number as f32,
                        light.position[1],
                        light.position[2],
                    ],
                    ..*light
                })
                .collect();
            renderer.set_lights(&moved).unwrap();
        }
        let scene = Scene {
            camera: camera(),
            time: number as f32 / 60.0,
            seed: 5,
            sun: Sun {
                direction: [0.3, 0.5, 0.8],
                colour: [1.0; 3],
                intensity: shot.sun,
            },
            instances: &instances,
            materials: &materials,
            deformers: &[],
            wind: SceneWind::default(),
        };
        let passes = renderer
            .render(
                &scene,
                &Text::default(),
                &effects,
                Finish::Standard,
                &target.view,
            )
            .unwrap();
        turns.add(pfx_gpu::pace::gpu_ms(&passes).unwrap_or(f64::NAN));
        if number >= 4 {
            timings.extend(
                passes
                    .into_iter()
                    .map(|pass| (pass.label, pass.milliseconds)),
            );
        }
    }
    Taken {
        raw: renderer.gpu().readback_rgba16(&target).unwrap(),
        passes: renderer.last_pass_order().to_vec(),
        timings,
    }
}

fn worst(a: &Taken, b: &Taken) -> (f32, usize) {
    let mut worst = 0.0f32;
    let mut differing = 0;
    for (x, y) in a.raw.chunks_exact(4).zip(b.raw.chunks_exact(4)) {
        let gap = (0..3)
            .map(|k| {
                (half::f16::from_bits(x[k]).to_f32() - half::f16::from_bits(y[k]).to_f32()).abs()
            })
            .fold(0.0, f32::max);
        worst = worst.max(gap);
        differing += usize::from(gap > 2.0 / 255.0);
    }
    (worst, differing)
}

fn close(a: [f32; 3], b: [f32; 3], tolerance: f32) -> bool {
    (0..3).all(|k| (a[k] - b[k]).abs() <= tolerance)
}

fn floor_under(light: [f32; 3]) -> [f32; 3] {
    let t = (FLOOR_TOP - light[2]) / (SLAB_AT[2] - light[2]);
    [
        light[0] + (SLAB_AT[0] - light[0]) * t,
        light[1] + (SLAB_AT[1] - light[1]) * t,
        FLOOR_TOP,
    ]
}

#[test]
fn a_box_mesh_faces_out_with_counter_clockwise_triangles() {
    let mesh = box_mesh([1.0, 2.0, 3.0]);
    assert_eq!(mesh.positions.len(), 24);
    for triangle in mesh.indices.chunks_exact(3) {
        let [a, b, c] = [0, 1, 2].map(|k| mesh.positions[triangle[k] as usize]);
        let e1: [f32; 3] = std::array::from_fn(|k| b[k] - a[k]);
        let e2: [f32; 3] = std::array::from_fn(|k| c[k] - a[k]);
        let cross = [
            e1[1] * e2[2] - e1[2] * e2[1],
            e1[2] * e2[0] - e1[0] * e2[2],
            e1[0] * e2[1] - e1[1] * e2[0],
        ];
        let normal = mesh.normals[triangle[0] as usize];
        let along: f32 = (0..3).map(|k| cross[k] * normal[k]).sum();
        assert!(along > 0.0);
        let outward: f32 = (0..3).map(|k| a[k] * normal[k]).sum();
        assert!(outward > 0.0);
    }
}

#[test]
fn the_shadow_points_lie_in_one_light_shadow_each_and_off_the_slab_on_screen() {
    let a = floor_under(LIGHT_A);
    let b = floor_under(LIGHT_B);
    assert!((a[0] + 1.85).abs() < 1e-5 && a[1].abs() < 1e-5);
    assert!((b[0] + 0.8).abs() < 1e-5 && (b[1] + 1.5).abs() < 1e-5);
    let slab_rows = pixel([SLAB_AT[0], SLAB_AT[1] - SLAB[1], SLAB[2]]) / SIZE as usize;
    assert!(pixel(b) / SIZE as usize > slab_rows + 4);
    let slab_left = pixel([SLAB_AT[0] - SLAB[0], 0.0, SLAB[2]]) % SIZE as usize;
    assert!(pixel(a) % (SIZE as usize) + 4 < slab_left);
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn shadow_only_glass_casts_the_tinted_sun_shadow_of_drawn_glass_and_draws_nothing() {
    let nothing = take(&Shot::default());
    let copy = take(&Shot {
        copy: Twin::Tinted,
        ..Default::default()
    });
    let only = take(&Shot {
        glass: Some(Part {
            shadow_only: true,
            ..Part::slab()
        }),
        ..Default::default()
    });
    let drawn = take(&Shot {
        glass: Some(Part::slab()),
        ..Default::default()
    });
    assert!(only.passes.contains(&"shadow transmission"));
    assert!(!only.passes.contains(&"glass"));
    assert!(drawn.passes.contains(&"glass"));
    let (gap, differing) = worst(&copy, &only);
    println!("shadow-only glass against its shadow-only copy: worst {gap:.5}, {differing} pixels");
    assert!(gap <= 1.0 / 255.0, "worst {gap}");
    let shade = pixel([-1.1, -0.9, FLOOR_TOP]);
    let open = pixel([1.5, -0.9, FLOOR_TOP]);
    let ratio = |shot: &Taken| {
        let (lit, shaded) = (shot.linear(open), shot.linear(shade));
        [0, 1, 2].map(|k| shaded[k] / lit[k])
    };
    println!(
        "under the slab / open floor: nothing {:?}, shadow only {:?}, drawn {:?}",
        ratio(&nothing),
        ratio(&only),
        ratio(&drawn)
    );
    let tinted = ratio(&only);
    assert!(tinted[2] > tinted[0] + 0.1, "the shadow is not tinted");
    assert!(tinted[0] < ratio(&nothing)[0] - 0.1, "no shadow");
    for at in [shade, open] {
        assert!(
            close(only.encoded(at), drawn.encoded(at), 1.0 / 255.0),
            "{:?} against {:?}",
            only.encoded(at),
            drawn.encoded(at)
        );
    }
    let centre = pixel(SLAB_AT);
    assert!(!close(
        only.encoded(centre),
        drawn.encoded(centre),
        4.0 / 255.0
    ));
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn glass_tints_local_light_shadows_with_its_cast_tint() {
    let both = [light(LIGHT_A), light(LIGHT_B)];
    let base = Shot {
        sun: 0.0,
        ..Default::default()
    };
    let dark = take(&base);
    let only_a = take(&Shot {
        lights: &both[..1],
        ..base
    });
    let only_b = take(&Shot {
        lights: &both[1..],
        ..base
    });
    let lit = Shot {
        lights: &both,
        ..base
    };
    let nothing = take(&lit);
    let opaque = take(&Shot {
        copy: Twin::Opaque,
        ..lit
    });
    let shadow_only = take(&Shot {
        glass: Some(Part {
            shadow_only: true,
            ..Part::slab()
        }),
        ..lit
    });
    let drawn = take(&Shot {
        glass: Some(Part::slab()),
        ..lit
    });
    assert!(shadow_only.passes.contains(&"light transmission"));
    assert!(!nothing.passes.contains(&"light transmission"));
    let tint = glass::cast_tint(&clear_glass());
    for (name, at, own, other) in [
        ("A", pixel(floor_under(LIGHT_A)), &only_a, &only_b),
        ("B", pixel(floor_under(LIGHT_B)), &only_b, &only_a),
    ] {
        let floor = dark.linear(at);
        let mine = own.linear(at);
        let share: [f32; 3] = std::array::from_fn(|k| {
            let a = mine[k] - floor[k];
            let b = other.linear(at)[k] - floor[k];
            a / (a + b)
        });
        let ratio = |shot: &Taken| -> [f32; 3] {
            std::array::from_fn(|k| {
                (shot.linear(at)[k] - floor[k]) / (nothing.linear(at)[k] - floor[k])
            })
        };
        let expected = |passed: [f32; 3]| -> [f32; 3] {
            std::array::from_fn(|k| 1.0 - share[k] * (1.0 - passed[k]))
        };
        println!(
            "light {name}: share {share:?}; opaque {:?} against {:?}; glass {:?} against {:?}",
            ratio(&opaque),
            expected([0.0; 3]),
            ratio(&shadow_only),
            expected(tint)
        );
        assert!(share.iter().all(|value| *value > 0.2));
        assert!(close(ratio(&opaque), expected([0.0; 3]), 0.02));
        assert!(close(ratio(&shadow_only), expected(tint), 0.02));
        assert!(
            close(shadow_only.encoded(at), drawn.encoded(at), 1.0 / 255.0),
            "{:?} against {:?}",
            shadow_only.encoded(at),
            drawn.encoded(at)
        );
    }
    let open = pixel([1.5, 1.5, FLOOR_TOP]);
    assert!(close(
        shadow_only.encoded(open),
        nothing.encoded(open),
        1.0 / 255.0
    ));
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn clipped_glass_draws_and_casts_like_the_part_it_keeps() {
    let lights = [light(LIGHT_A)];
    let kept = Part {
        half: [0.25, SLAB[1], SLAB[2]],
        at: [SLAB_AT[0] + 0.25, SLAB_AT[1], SLAB_AT[2]],
        ..Part::slab()
    };
    let shot = |part: Part| {
        take(&Shot {
            glass: Some(part),
            lights: &lights,
            ..Default::default()
        })
    };
    let clipped = shot(Part {
        clip: [[-1.0, 0.0, 0.0, -SLAB_AT[0]], [0.0; 4]],
        ..Part::slab()
    });
    let second = shot(Part {
        clip: [[0.0; 4], [-1.0, 0.0, 0.0, -SLAB_AT[0]]],
        ..Part::slab()
    });
    let cut = shot(kept);
    let whole = shot(Part::slab());
    let (gap, differing) = worst(&clipped, &cut);
    let (_, apart) = worst(&whole, &cut);
    let (second_gap, second_differing) = worst(&second, &cut);
    println!(
        "clipped against the kept part: worst {gap:.5}, {differing} pixels over 2/255; second plane worst {second_gap:.5}, {second_differing}; the whole slab differs in {apart}"
    );
    assert!(apart > 1500);
    assert!(differing * 100 <= apart, "{differing} of {apart}");
    assert!(second_differing * 100 <= apart);
    let shade_sun = pixel([-0.3, -0.9, FLOOR_TOP]);
    let shade_light = pixel(floor_under(LIGHT_A));
    for at in [shade_sun, shade_light] {
        assert!(
            close(clipped.encoded(at), cut.encoded(at), 2.0 / 255.0),
            "{:?} against {:?}",
            clipped.encoded(at),
            cut.encoded(at)
        );
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn glass_in_two_local_lights_costs() {
    let both = [light(LIGHT_A), light(LIGHT_B)];
    for drift in [0.0, 1.0e-4] {
        for (name, glass) in [
            ("lights only", None),
            (
                "lights and shadow-only glass",
                Some(Part {
                    shadow_only: true,
                    ..Part::slab()
                }),
            ),
            ("lights and drawn glass", Some(Part::slab())),
        ] {
            let shot = take(&Shot {
                glass,
                lights: &both,
                size: [1920, 1080],
                frames: 68,
                drift,
                ..Default::default()
            });
            println!(
                "{name}, {}: light shadows {:.4} ms, light transmission {:.4} ms, shadow transmission {:.4} ms, glass front {:.4} ms",
                if drift == 0.0 {
                    "lights still"
                } else {
                    "lights moving"
                },
                shot.mean("light shadows"),
                shot.mean("light transmission"),
                shot.mean("shadow transmission"),
                shot.mean("glass front"),
            );
        }
    }
}
