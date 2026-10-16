use pfx_geom::mesh::Mesh;
use pfx_gpu::{Gpu, OffscreenTarget, wgpu};
use pfx_live::frame::{
    Camera, Instance, Matrix, MeshData, MeshHandle, Scene, SceneWind, Sun, multiply, transform,
};
use pfx_live::glass;
use pfx_live::renderer::{Effects, Exposure, Finish, Renderer, Text};
use pfx_materials::{Family, Material};
use pfx_post::Chain;

const EYE: f32 = 6.0;
const LENS_AT: [f32; 3] = [0.0, 0.0, 0.8];
const LENS_TURN: f32 = 35.0;
const LENS_HALF: f32 = 0.4;
const WALL: f32 = -1.0;
const CELL: f32 = 0.2;

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

fn turned(at: [f32; 3], degrees: f32) -> Matrix {
    let (s, c) = degrees.to_radians().sin_cos();
    [
        [c, 0.0, -s, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [s, 0.0, c, 0.0],
        [at[0], at[1], at[2], 1.0],
    ]
}

fn perspective() -> Matrix {
    let near = 0.1;
    let far = 100.0;
    let f = 1.0 / (30.0_f32.to_radians() * 0.5).tan();
    [
        [f, 0.0, 0.0, 0.0],
        [0.0, f, 0.0, 0.0],
        [0.0, 0.0, far / (near - far), -1.0],
        [0.0, 0.0, far * near / (near - far), 0.0],
    ]
}

fn camera() -> Camera {
    let mut view = identity();
    view[3][2] = -EYE;
    Camera {
        view,
        projection: perspective(),
        previous_view_projection: multiply(perspective(), view),
        position: [0.0, 0.0, EYE],
    }
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

fn sheet_mesh(half: f32) -> Mesh {
    let full = box_mesh([half, half, 0.0]);
    Mesh::new(
        full.positions[16..20].to_vec(),
        full.normals[16..20].to_vec(),
        Vec::new(),
        full.uvs[16..20].to_vec(),
        vec![0, 1, 2, 0, 2, 3],
    )
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

fn tile(level: f32) -> Material {
    Material {
        base: [0.0; 3],
        roughness: 1.0,
        specular: 0.0,
        emission: [level; 3],
        ..Material::default()
    }
}

fn yellow(family: Family, transmission: f32, clearcoat: f32) -> Material {
    Material {
        family,
        base: [0.9, 0.8, 0.05],
        roughness: 0.1,
        transmission,
        clearcoat,
        clearcoat_roughness: 0.05,
        ior: 1.5,
        ..Material::default()
    }
}

#[derive(Clone, Copy)]
enum Lens {
    None,
    Thin(Material),
    Sheet(Material),
}

struct Taken {
    raw: Vec<u16>,
    size: u32,
    glass_ms: f64,
}

impl Taken {
    fn at(&self, pixel: [usize; 2]) -> [f32; 3] {
        let i = (pixel[1] * self.size as usize + pixel[0]) * 4;
        [0, 1, 2].map(|k| half::f16::from_bits(self.raw[i + k]).to_f32())
    }
}

fn output(gpu: &Gpu, size: [u32; 2]) -> OffscreenTarget {
    let format = wgpu::TextureFormat::Rgba16Float;
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("tinted lens output"),
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

fn take(lens: Lens, size: [u32; 2], frames: usize) -> Taken {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut turns = pfx_gpu::pace::Turns::default();
    let mut renderer = turns.cpu(|| Renderer::new(gpu, size[0], size[1]).unwrap());
    renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
    renderer.set_finish(Chain::new());
    let tile_mesh = upload(&mut renderer, &box_mesh([CELL * 0.5, CELL * 0.5, 0.02]));
    let mut instances = Vec::new();
    for row in 0..12 {
        for column in 0..12 {
            let at = [
                (column as f32 - 5.5) * CELL,
                (row as f32 - 5.5) * CELL,
                WALL,
            ];
            instances.push(Instance::new(
                tile_mesh,
                translated(at),
                ((row + column) % 2) as u32,
                1,
            ));
        }
    }
    let lens_model = turned(LENS_AT, LENS_TURN);
    let surfaces: Vec<glass::Surface> = match lens {
        Lens::None => Vec::new(),
        Lens::Thin(material) | Lens::Sheet(material) => {
            let mesh = match lens {
                Lens::Sheet(_) => sheet_mesh(LENS_HALF),
                _ => box_mesh([LENS_HALF, LENS_HALF, 0.001]),
            };
            vec![glass::Surface {
                mesh: renderer.upload_glass(&mesh).unwrap(),
                model: lens_model,
                material,
                liquid: false,
                fluid_height: 0.0,
                ripple_height: 0.0,
                caustic_strength: 0.0,
                tinted: None,
                id: 0,
                casts_shadow: false,
                shadow_only: false,
                clip: [[0.0; 4]; 2],
            }]
        }
    };
    let materials = [tile(0.03), tile(0.9)];
    let effects = Effects {
        glass: &surfaces,
        ..Default::default()
    };
    let target = output(renderer.gpu(), size);
    let mut glass_ms = Vec::new();
    for number in 0..frames {
        turns.poll();
        let scene = Scene {
            camera: camera(),
            time: number as f32 / 60.0,
            seed: 5,
            sun: Sun {
                direction: [0.3, 0.5, 0.8],
                colour: [1.0; 3],
                intensity: 2.0,
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
            glass_ms.push(
                passes
                    .iter()
                    .filter(|pass| pass.label.starts_with("glass "))
                    .map(|pass| pass.milliseconds)
                    .sum::<f64>(),
            );
        }
    }
    glass_ms.sort_by(f64::total_cmp);
    Taken {
        raw: renderer.gpu().readback_rgba16(&target).unwrap(),
        size: size[0],
        glass_ms: glass_ms.get(glass_ms.len() / 2).copied().unwrap_or(0.0),
    }
}

fn lens_pixels(size: u32) -> Vec<[usize; 2]> {
    let camera = camera();
    let view_projection = multiply(camera.projection, camera.view);
    let model = turned(LENS_AT, LENS_TURN);
    let mut pixels = Vec::new();
    let steps = 40;
    for i in 0..=steps {
        for j in 0..=steps {
            let u = (i as f32 / steps as f32 * 2.0 - 1.0) * LENS_HALF * 0.85;
            let v = (j as f32 / steps as f32 * 2.0 - 1.0) * LENS_HALF * 0.85;
            let world = transform(model, [u, v, 0.001, 1.0]);
            let clip = transform(view_projection, world);
            let x = ((clip[0] / clip[3] * 0.5 + 0.5) * size as f32) as usize;
            let y = ((0.5 - clip[1] / clip[3] * 0.5) * size as f32) as usize;
            pixels.push([x, y]);
        }
    }
    pixels.sort();
    pixels.dedup();
    pixels
}

struct Look {
    dropped: usize,
    light: [f32; 3],
    dark: [f32; 3],
    seen: usize,
}

fn look(bare: &Taken, lensed: &Taken) -> Look {
    let mut dropped = 0;
    let mut light = [0.0; 3];
    let mut dark = [0.0; 3];
    let (mut lights, mut darks) = (0, 0);
    let pixels = lens_pixels(bare.size);
    for &pixel in &pixels {
        let behind = bare.at(pixel);
        let through = lensed.at(pixel);
        if behind == through || through[2] > through[0] * 0.6 {
            dropped += 1;
        }
        let luma = behind.iter().sum::<f32>() / 3.0;
        if luma > 0.6 {
            lights += 1;
            (0..3).for_each(|k| light[k] += through[k]);
        } else if luma < 0.3 {
            darks += 1;
            (0..3).for_each(|k| dark[k] += through[k]);
        }
    }
    Look {
        dropped,
        light: light.map(|sum| sum / lights.max(1) as f32),
        dark: dark.map(|sum| sum / darks.max(1) as f32),
        seen: pixels.len(),
    }
}

fn assert_tinted_see_through(name: &str, look: &Look) {
    println!(
        "{name}: {} of {} lens pixels dropped, over light {:?}, over dark {:?}",
        look.dropped, look.seen, look.light, look.dark
    );
    assert_eq!(look.dropped, 0, "{name}: the lens drops pixels (streaks)");
    let contrast = (0..3)
        .map(|k| look.light[k] - look.dark[k])
        .collect::<Vec<_>>();
    assert!(
        contrast[0] > 0.15 && contrast[1] > 0.15,
        "{name}: the pattern does not show through: {contrast:?}"
    );
    assert!(
        look.light[2] < look.light[0] * 0.5 && look.light[2] < look.light[1] * 0.5,
        "{name}: the light tiles are not tinted yellow: {:?}",
        look.light
    );
}

const SIZE: u32 = 384;

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_thin_yellow_lens_shows_the_pattern_behind_it_tinted_with_no_streaks() {
    let bare = take(Lens::None, [SIZE, SIZE], 6);
    for (name, lens) in [
        (
            "2 mm plain t 0.6",
            Lens::Thin(yellow(Family::Plain, 0.6, 0.0)),
        ),
        (
            "2 mm lacquer t 0.6 coat",
            Lens::Thin(yellow(Family::Lacquer, 0.6, 1.0)),
        ),
        (
            "2 mm glass t 0.9",
            Lens::Thin(yellow(Family::Glass, 0.9, 0.0)),
        ),
        (
            "open sheet t 0.6",
            Lens::Sheet(yellow(Family::Plain, 0.6, 0.0)),
        ),
        (
            "open sheet t 0.4 coat",
            Lens::Sheet(yellow(Family::Plain, 0.4, 1.0)),
        ),
    ] {
        let lensed = take(lens, [SIZE, SIZE], 6);
        assert_tinted_see_through(name, &look(&bare, &lensed));
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_lens_with_no_transmission_hides_the_pattern() {
    let bare = take(Lens::None, [SIZE, SIZE], 6);
    let lensed = take(
        Lens::Sheet(yellow(Family::Plain, 0.0, 0.0)),
        [SIZE, SIZE],
        6,
    );
    let look = look(&bare, &lensed);
    println!(
        "transmission 0: {} dropped, over light {:?}, over dark {:?}",
        look.dropped, look.light, look.dark
    );
    assert_eq!(look.dropped, 0);
    for k in 0..3 {
        assert!(
            (look.light[k] - look.dark[k]).abs() < 0.01,
            "the pattern shows through an opaque lens: {:?} against {:?}",
            look.light,
            look.dark
        );
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu --class bench"]
fn measure_the_tinted_lens_cost() {
    for (name, size) in [("head 600", [600, 600]), ("2160x3840", [2160, 3840])] {
        let lensed = take(Lens::Thin(yellow(Family::Plain, 0.6, 1.0)), size, 24);
        println!(
            "{name}: glass front, back and compose {:.3} ms (median of 20 frames)",
            lensed.glass_ms
        );
    }
}
