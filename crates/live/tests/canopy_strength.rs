use half::f16;
use pfx_geom::tree::{Tree, TreeSpec};
use pfx_gpu::Gpu;
use pfx_live::canopy::{CanopyMode, Gobo};
use pfx_live::frame::{
    Camera, Frame, Instance, Matrix, MeshData, OpaqueFeatures, Scene, SceneWind, Sun, multiply,
};
use pfx_materials::Material;

const SIZE: u32 = 256;
const HEIGHT: f32 = 6.0;

fn identity() -> Matrix {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

fn camera() -> Camera {
    let near = 0.1;
    let far = 100.0;
    let f = 1.0 / (50.0_f32.to_radians() * 0.5).tan();
    let projection = [
        [f, 0.0, 0.0, 0.0],
        [0.0, f, 0.0, 0.0],
        [0.0, 0.0, far / (near - far), -1.0],
        [0.0, 0.0, far * near / (near - far), 0.0],
    ];
    let view = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, -1.0, 0.0, 0.0],
        [0.0, 0.0, -HEIGHT, 1.0],
    ];
    Camera {
        view,
        projection,
        previous_view_projection: multiply(projection, view),
        position: [0.0, HEIGHT, 0.0],
    }
}

struct Scenario {
    frame: Frame,
}

impl Scenario {
    fn new() -> Self {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut frame = Frame::new(gpu, SIZE, SIZE).unwrap();
        let tree = Tree::new(9, TreeSpec::plum(), 0.02);
        frame.set_tree(&tree, [0.5, 0.0, 0.5], 0.0, 7).unwrap();
        frame.set_canopy_mode(CanopyMode::Procedural).unwrap();
        Self { frame }
    }

    fn render(&mut self, off: OpaqueFeatures) -> Vec<f32> {
        let frame = &mut self.frame;
        let corners = [
            [-3.0, 0.0, -3.0],
            [3.0, 0.0, -3.0],
            [-3.0, 0.0, 3.0],
            [3.0, 0.0, 3.0],
        ];
        let handle = frame
            .upload_mesh(MeshData {
                positions: &corners,
                normals: &[[0.0, 1.0, 0.0]; 4],
                tangents: &[[1.0, 0.0, 0.0, 1.0]; 4],
                uvs: &[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]],
                uvs1: None,
                alpha: None,
                indices: &[0, 2, 1, 1, 2, 3],
            })
            .unwrap();
        let instances = [Instance::new(handle, identity(), 0, 1)];
        let materials = [Material {
            base: [0.7, 0.6, 0.5],
            roughness: 0.4,
            ..Material::default()
        }];
        let scene = Scene {
            camera: camera(),
            time: 0.25,
            seed: 5,
            sun: Sun {
                direction: [0.5, 0.8, 0.3],
                colour: [1.0, 0.95, 0.9],
                intensity: 1.0,
            },
            instances: &instances,
            materials: &materials,
            deformers: &[],
            wind: SceneWind::default(),
        };
        frame.set_opaque_off(off);
        let mut encoder = frame.gpu.device.create_command_encoder(&Default::default());
        frame.encode(&scene, &mut encoder, None, None).unwrap();
        frame.gpu.queue.submit(Some(encoder.finish()));
        frame
            .gpu
            .readback_rgba16(&frame.targets.hdr)
            .unwrap()
            .into_iter()
            .map(|bits| f16::from_bits(bits).to_f32())
            .collect()
    }

    fn at(&mut self, strength: f32) -> Vec<f32> {
        self.frame.set_canopy_strength(strength).unwrap();
        self.render(OpaqueFeatures::NONE)
    }

    fn without_canopy(&mut self) -> Vec<f32> {
        let off = OpaqueFeatures {
            canopy: true,
            ..OpaqueFeatures::NONE
        };
        self.render(off)
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_default_strength_is_the_full_weight_byte_for_byte() {
    let mut scenario = Scenario::new();
    let default = scenario.render(OpaqueFeatures::NONE);
    let explicit = scenario.at(1.0);
    assert_eq!(default, explicit);
    let off = scenario.without_canopy();
    let shaded = default
        .chunks_exact(4)
        .zip(off.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count();
    assert!(shaded > 1000, "{shaded} pixels carry the gobo");
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn zero_strength_is_the_canopy_off_frame() {
    let mut scenario = Scenario::new();
    let none = scenario.at(0.0);
    let off = scenario.without_canopy();
    assert_eq!(none, off);
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn half_strength_is_half_the_shade_in_linear_light() {
    let mut scenario = Scenario::new();
    let none = scenario.at(0.0);
    let full = scenario.at(1.0);
    let half = scenario.at(0.5);
    let mut shaded = 0;
    for ((n, f), h) in none.iter().zip(&full).zip(&half) {
        let expected = (n + f) * 0.5;
        assert!(
            (h - expected).abs() <= 1.0 / 255.0,
            "half {h} against expected {expected} between {n} and {f}"
        );
        if (n - f).abs() > 4.0 / 255.0 {
            shaded += 1;
        }
    }
    assert!(shaded > 1000, "{shaded} pixels carry the gobo");
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_gobo_weight_is_the_strength_it_is_authored_at() {
    let mut scenario = Scenario::new();
    let reference = scenario.at(0.5);
    let gobo = Gobo {
        weight: 0.5,
        ..Gobo::default()
    };
    scenario.frame.set_canopy_gobo(gobo).unwrap();
    let authored = scenario.at(0.5);
    for (a, b) in reference.iter().zip(&authored) {
        assert!((a - b).abs() <= 1.0 / 255.0, "{a} against {b}");
    }
}
