use pfx_gpu::Gpu;
use pfx_gpu::pace::{Pacer, Turns};
use pfx_live::frame::{
    Camera as LiveCamera, Frame, Instance as LiveInstance, Matrix, MeshData, Scene as LiveScene,
    SceneWind, Sun as LiveSun, multiply,
};
use pfx_live::sky::SkySource;
use pfx_load::Sky;
use pfx_materials::{Material, Wet, puddle};
use pfx_trace::detail::Lens;
use pfx_trace::stage::{Mesh, Placement, Stage};
use pfx_trace::{Camera, Projection, Sun};

const SIZE: u32 = 96;
const HEIGHT: f32 = 4.0;
const FOV: f32 = 50.0;
const SAMPLES: u32 = 16;

struct Floor {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    tangents: Vec<[f32; 4]>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
}

impl Floor {
    fn new() -> Self {
        Self {
            positions: vec![
                [-4.0, 0.0, 4.0],
                [4.0, 0.0, 4.0],
                [4.0, 0.0, -4.0],
                [-4.0, 0.0, -4.0],
            ],
            normals: vec![[0.0, 1.0, 0.0]; 4],
            tangents: vec![[1.0, 0.0, 0.0, 1.0]; 4],
            uvs: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            indices: vec![0, 1, 2, 0, 2, 3],
        }
    }

    fn mesh(&self) -> Mesh<'_> {
        Mesh {
            positions: &self.positions,
            normals: &self.normals,
            tangents: &self.tangents,
            uvs: &self.uvs,
            uvs1: None,
            alpha: None,
            indices: &self.indices,
        }
    }

    fn data(&self) -> MeshData<'_> {
        MeshData {
            positions: &self.positions,
            normals: &self.normals,
            tangents: &self.tangents,
            uvs: &self.uvs,
            uvs1: None,
            alpha: None,
            indices: &self.indices,
        }
    }
}

fn identity() -> Matrix {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

fn unit(v: [f32; 3]) -> [f32; 3] {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    v.map(|value| value / length)
}

fn black_sky() -> Sky {
    Sky {
        width: 1,
        height: 1,
        texels: vec![[0.0, 0.0, 0.0, 1.0]],
    }
}

fn asphalt(wet: Wet) -> Material {
    Material {
        base: [0.5, 0.5, 0.5],
        roughness: 0.9,
        wet,
        ..Material::default()
    }
}

fn floor_xz(px: u32, py: u32) -> [f32; 3] {
    let ndc_x = (px as f32 + 0.5) / SIZE as f32 * 2.0 - 1.0;
    let ndc_y = (py as f32 + 0.5) / SIZE as f32 * 2.0 - 1.0;
    let tan = (FOV.to_radians() * 0.5).tan();
    [HEIGHT * tan * ndc_x, 0.0, HEIGHT * tan * ndc_y]
}

fn grey(pixel: [f32; 3]) -> f32 {
    (pixel[0] + pixel[1] + pixel[2]) / 3.0
}

fn mean(pixels: &[[f32; 3]]) -> f32 {
    pixels.iter().copied().map(grey).sum::<f32>() / pixels.len() as f32
}

fn peak(pixels: &[[f32; 3]]) -> f32 {
    pixels.iter().copied().map(grey).fold(0.0, f32::max)
}

fn traced(gpu: &Gpu, material: Material, sun: [f32; 3]) -> Vec<[f32; 3]> {
    let floor = Floor::new();
    let meshes = [floor.mesh()];
    let placements = [Placement {
        two_sided: true,
        ..Placement::new(0, identity(), 0)
    }];
    let tan = (FOV.to_radians() * 0.5).tan();
    let staged = Stage {
        meshes: &meshes,
        instances: &placements,
        materials: &[material],
        sky: black_sky(),
        sun: Sun {
            direction: sun,
            color: [1.0; 3],
            intensity: 3.0,
        },
        camera: Camera {
            origin: [0.0, HEIGHT, 0.0],
            forward: [0.0, -1.0, 0.0],
            right: [tan, 0.0, 0.0],
            up: [0.0, 0.0, -tan],
        },
        projection: Projection::Perspective,
        lens: Lens::default(),
    }
    .build()
    .unwrap();
    let mut trace = staged.trace(gpu, SIZE, SIZE).unwrap();
    let mut pacer = Pacer::default();
    let mut turns = Turns::default();
    trace
        .sample_paced(gpu, SAMPLES, 7, &mut pacer, |ms| turns.add(ms))
        .unwrap();
    let output = trace.readback(gpu).unwrap();
    output
        .color
        .chunks_exact(16)
        .map(|pixel| {
            std::array::from_fn(|channel| {
                f32::from_le_bytes(pixel[channel * 4..channel * 4 + 4].try_into().unwrap())
            })
        })
        .collect()
}

fn live(gpu: &Gpu, material: Material, sun: [f32; 3]) -> Vec<[f32; 3]> {
    let floor = Floor::new();
    let mut frame = Frame::new(gpu.clone(), SIZE, SIZE).unwrap();
    frame.set_sky(SkySource::Hdr(black_sky()), true);
    let mesh = frame.upload_mesh(floor.data()).unwrap();
    let mut instance = LiveInstance::new(mesh, identity(), 0, 1);
    instance.two_sided = true;
    let view: Matrix = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, -1.0, 0.0, 0.0],
        [0.0, 0.0, -HEIGHT, 1.0],
    ];
    let near = 0.1;
    let far = 12.0;
    let f = 1.0 / (FOV.to_radians() * 0.5).tan();
    let projection: Matrix = [
        [f, 0.0, 0.0, 0.0],
        [0.0, f, 0.0, 0.0],
        [0.0, 0.0, far / (near - far), -1.0],
        [0.0, 0.0, far * near / (near - far), 0.0],
    ];
    let materials = [material];
    let scene = LiveScene {
        camera: LiveCamera {
            view,
            projection,
            previous_view_projection: multiply(projection, view),
            position: [0.0, HEIGHT, 0.0],
        },
        time: 0.0,
        seed: 5,
        sun: LiveSun {
            direction: sun,
            colour: [1.0; 3],
            intensity: 3.0,
        },
        instances: &[instance],
        materials: &materials,
        deformers: &[],
        wind: SceneWind::default(),
    };
    frame.render(&scene).unwrap();
    frame
        .gpu
        .readback_rgba16(&frame.targets.hdr)
        .unwrap()
        .chunks_exact(4)
        .map(|pixel| std::array::from_fn(|channel| half::f16::from_bits(pixel[channel]).to_f32()))
        .collect()
}

type Render = fn(&Gpu, Material, [f32; 3]) -> Vec<[f32; 3]>;

const RENDERERS: [(&str, Render); 2] = [("live", live), ("traced", traced)];

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn an_idle_wet_table_renders_byte_identical_in_both_renderers() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let side = unit([1.0, 0.7, 0.0]);
    let idle = Wet {
        amount: 0.0,
        puddles: 0.0,
        puddle_scale: 7.0,
        seed: 3,
    };
    for (name, render) in RENDERERS {
        let dry = render(&gpu, asphalt(Wet::default()), side);
        let off = render(&gpu, asphalt(idle), side);
        assert!(mean(&dry) > 0.0, "{name}");
        let same = dry
            .iter()
            .zip(&off)
            .all(|(a, b)| a.map(f32::to_bits) == b.map(f32::to_bits));
        assert!(same, "{name}: an idle [wet] changed the image");
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_wet_floor_is_darker_and_glossier_and_its_puddles_follow_the_mask() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let side = unit([1.0, 0.7, 0.0]);
    let over = [0.0, 1.0, 0.0];
    let damp = Wet {
        amount: 0.8,
        ..Wet::default()
    };
    let flooded = Wet {
        puddles: 1.0,
        ..Wet::default()
    };
    let pools = Wet {
        puddles: 0.5,
        puddle_scale: 1.5,
        seed: 11,
        ..Wet::default()
    };
    for (name, render) in RENDERERS {
        let dry_side = render(&gpu, asphalt(Wet::default()), side);
        let damp_side = render(&gpu, asphalt(damp), side);
        let dry_over = render(&gpu, asphalt(Wet::default()), over);
        let damp_over = render(&gpu, asphalt(damp), over);
        let flooded_over = render(&gpu, asphalt(flooded), over);
        println!(
            "{name}: side mean dry {:.4} damp {:.4}; overhead peak dry {:.4} damp {:.4} flooded {:.4}",
            mean(&dry_side),
            mean(&damp_side),
            peak(&dry_over),
            peak(&damp_over),
            peak(&flooded_over)
        );
        assert!(
            mean(&damp_side) < mean(&dry_side) * 0.7,
            "{name}: not darker"
        );
        assert!(
            peak(&damp_over) / mean(&damp_over) > peak(&dry_over) / mean(&dry_over),
            "{name}: not glossier"
        );
        assert!(
            peak(&flooded_over) > peak(&dry_over) * 4.0,
            "{name}: a flooded floor shows no sun glint"
        );

        let pooled = render(&gpu, asphalt(pools), side);
        let (mut agree, mut decided, mut inside, mut outside) = (0, 0, 0, 0);
        for y in 0..SIZE {
            for x in 0..SIZE {
                let mask = puddle(pools, floor_xz(x, y), 1.0, 0.0);
                if mask > 0.0 && mask < 1.0 {
                    continue;
                }
                let index = (y * SIZE + x) as usize;
                let ratio = grey(pooled[index]) / grey(dry_side[index]).max(1e-6);
                decided += 1;
                if mask == 1.0 {
                    inside += 1;
                    agree += usize::from(ratio < 0.6);
                } else {
                    outside += 1;
                    agree += usize::from(ratio > 0.9);
                }
            }
        }
        println!("{name}: puddles agree {agree} of {decided} ({inside} in, {outside} out)");
        assert!(
            inside > 200 && outside > 200,
            "{name}: {inside} in, {outside} out"
        );
        assert!(
            agree as f32 > decided as f32 * 0.97,
            "{name}: {agree} of {decided}"
        );
    }
}
