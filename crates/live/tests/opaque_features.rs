use pfx_bake::detail::atlas::{Atlas, Piece, assemble};
use pfx_bake::detail::{Crop, Region};
use pfx_gpu::Gpu;
use pfx_live::frame::{
    Camera, Frame, Instance, Matrix, MeshData, OpaqueFeatures, Scene, SceneWind, Sun, multiply,
};
use pfx_live::maps::select_baked_detail;
use pfx_live::shadow::{Quality, ReceiverBox, Shadows, View};
use pfx_materials::{Grime, Material};

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

fn view() -> View {
    View {
        eye: [0.0, HEIGHT, 0.0],
        forward: [0.0, -1.0, 0.0],
        up: [0.0, 0.0, -1.0],
        fov_y: 50.0_f32.to_radians(),
        aspect: 1.0,
        near: 0.1,
        far: 12.0,
    }
}

fn plain() -> Material {
    Material {
        base: [0.7, 0.6, 0.5],
        roughness: 0.4,
        ..Material::default()
    }
}

fn grimy() -> Material {
    let mut material = plain();
    material.layers[0] = Grime::SAMPLE.layer(0.8, 3);
    material.ageing.scratch = 0.6;
    material.ageing.seed = 9;
    material
}

fn quad(frame: &mut Frame, corners: [[f32; 3]; 4], material: u32, id: u32) -> Instance {
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
    Instance::new(handle, identity(), material, id)
}

struct Setup {
    radius: f32,
    blend: f32,
    shadowed: bool,
    floor: u32,
    block: u32,
    age: f32,
}

fn render(frame: &mut Frame, shadows: &mut Shadows, setup: &Setup, on: OpaqueFeatures) -> Vec<u16> {
    let mut floor = quad(
        frame,
        [
            [-3.0, 0.0, -3.0],
            [3.0, 0.0, -3.0],
            [-3.0, 0.0, 3.0],
            [3.0, 0.0, 3.0],
        ],
        setup.floor,
        1,
    );
    floor.age = setup.age;
    let block = quad(
        frame,
        [
            [-1.0, 0.8, -1.0],
            [0.5, 0.8, -1.0],
            [-1.0, 0.8, 1.0],
            [0.5, 0.8, 1.0],
        ],
        setup.block,
        2,
    );
    let materials = [plain(), grimy()];
    let sun = Sun {
        direction: [0.5, 0.8, 0.3],
        colour: [1.0, 0.95, 0.9],
        intensity: 3.0,
    };
    let instances = [floor, block];
    let scene = Scene {
        camera: camera(),
        time: 0.25,
        seed: 5,
        sun,
        instances: &instances,
        materials: &materials,
        deformers: &[],
        wind: SceneWind::default(),
    };
    shadows
        .set_quality(Quality {
            sun_radius_deg: setup.radius,
            blend: setup.blend,
            ..shadows.quality()
        })
        .unwrap();
    let fit = shadows.fit(&view(), sun.direction);
    frame.set_opaque_on(on);
    let mut encoder = frame.gpu.device.create_command_encoder(&Default::default());
    frame
        .encode(
            &scene,
            &mut encoder,
            setup.shadowed.then_some((shadows, &fit)),
            None,
        )
        .unwrap();
    frame.gpu.queue.submit(Some(encoder.finish()));
    frame.gpu.readback_rgba16(&frame.targets.hdr).unwrap()
}

fn same_with_and_without_specialization(setup: Setup, unused: &[&str]) {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut frame = Frame::new(gpu, SIZE, SIZE).unwrap();
    let mut shadows = Shadows::new(
        &frame.gpu.device,
        Quality {
            resolution: 1024,
            receiver: Some(ReceiverBox {
                min: [-3.0, -0.2, -3.0],
                max: [3.0, 1.0, 3.0],
            }),
            caster_margin: 2.0,
            ..Quality::default()
        },
    );
    let all = general_features();
    let general = render(&mut frame, &mut shadows, &setup, all);
    assert!(frame.opaque_drawn().iter().all(|used| *used == all));
    let special = render(&mut frame, &mut shadows, &setup, OpaqueFeatures::NONE);
    for drawn in frame.opaque_drawn() {
        for name in unused {
            assert!(
                !drawn.names().contains(name),
                "{name} should be compiled out of {:?}",
                drawn.names()
            );
        }
    }
    let lit = general.chunks_exact(4).filter(|p| p[0] != 0).count();
    assert!(lit > (SIZE * SIZE / 2) as usize, "{lit} pixels drawn");
    let differing = general
        .chunks_exact(4)
        .zip(special.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count();
    assert_eq!(
        differing, 0,
        "{differing} pixels differ with {:?} compiled out",
        unused
    );
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_plain_scene_draws_the_same_bytes_with_its_unused_features_compiled_out() {
    same_with_and_without_specialization(
        Setup {
            radius: 0.0,
            blend: 0.1,
            shadowed: true,
            floor: 0,
            block: 0,
            age: 0.0,
        },
        &[
            "soft_search",
            "soft_filter",
            "probes",
            "local_reflections",
            "ssr",
            "canopy",
            "contact",
            "authored_ambient",
            "caustics",
            "content",
            "maps",
            "detail",
            "ageing",
            "uv1",
            "lacquer",
            "procedural_lacquer",
            "procedural_detail",
            "scratch",
            "grime",
            "wood",
            "wall",
            "fibre",
            "crinkle",
            "content_cutout",
            "film",
        ],
    );
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_soft_sun_scene_with_detail_on_one_part_draws_the_same_bytes() {
    same_with_and_without_specialization(
        Setup {
            radius: 1.5,
            blend: 0.0,
            shadowed: true,
            floor: 1,
            block: 0,
            age: 0.7,
        },
        &[
            "cascade_blend",
            "probes",
            "content",
            "maps",
            "canopy",
            "lacquer",
            "procedural_lacquer",
            "procedural_detail",
            "scratch",
            "wood",
            "wall",
            "fibre",
            "crinkle",
            "content_cutout",
            "film",
        ],
    );
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn an_unshadowed_scene_draws_the_same_bytes_without_the_sun_shadow() {
    same_with_and_without_specialization(
        Setup {
            radius: 1.5,
            blend: 0.1,
            shadowed: false,
            floor: 1,
            block: 1,
            age: 0.0,
        },
        &[
            "sun_shadow",
            "soft_search",
            "soft_filter",
            "cascade_blend",
            "ageing",
            "lacquer",
            "procedural_lacquer",
            "procedural_detail",
            "scratch",
            "wood",
            "wall",
            "fibre",
            "crinkle",
            "content_cutout",
            "film",
        ],
    );
}

const RED: [u8; 4] = [200, 40, 40, 255];
const BLUE: [u8; 4] = [40, 40, 200, 255];
const TILE: u32 = 64;
const FIRST: [[f32; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]];
const MIRRORED: [[f32; 2]; 4] = [[1.0, 0.0], [0.0, 0.0], [1.0, 1.0], [0.0, 1.0]];

fn halves(uv_set: u32) -> Atlas {
    let texels = |value: &dyn Fn(u32) -> [u8; 4]| -> Vec<u8> {
        (0..TILE * TILE)
            .flat_map(|index| value(index % TILE))
            .collect()
    };
    let region = Region {
        size: TILE,
        crop: Crop {
            x: 0,
            y: 0,
            width: TILE,
            height: TILE,
        },
        uv_set,
        base: texels(&|x| if x < TILE / 2 { RED } else { BLUE }),
        roughness: texels(&|_| [128, 128, 128, 255]),
        normal: texels(&|_| [128, 128, 255, 255]),
    };
    assemble(
        vec![Piece {
            index: 0,
            node: "planks".into(),
            input_hash: "a".repeat(64),
            region,
        }],
        TILE,
    )
    .unwrap()
}

fn detail_frame(
    frame: &mut Frame,
    atlas: &Atlas,
    uvs1: Option<&[[f32; 2]]>,
    on: OpaqueFeatures,
) -> Vec<u16> {
    frame.set_detail(atlas).unwrap();
    let mesh = frame
        .upload_mesh(MeshData {
            positions: &[
                [-3.0, 0.0, -3.0],
                [3.0, 0.0, -3.0],
                [-3.0, 0.0, 3.0],
                [3.0, 0.0, 3.0],
            ],
            normals: &[[0.0, 1.0, 0.0]; 4],
            tangents: &[[1.0, 0.0, 0.0, 1.0]; 4],
            uvs: &FIRST,
            uvs1,
            alpha: None,
            indices: &[0, 2, 1, 1, 2, 3],
        })
        .unwrap();
    let mut material = plain();
    select_baked_detail(&mut material, 0).unwrap();
    let instances = [Instance::new(mesh, identity(), 0, 1)];
    let materials = [material];
    let scene = Scene {
        camera: camera(),
        time: 0.25,
        seed: 5,
        sun: Sun {
            direction: [0.5, 0.8, 0.3],
            colour: [1.0, 0.95, 0.9],
            intensity: 3.0,
        },
        instances: &instances,
        materials: &materials,
        deformers: &[],
        wind: SceneWind::default(),
    };
    frame.set_opaque_on(on);
    let mut encoder = frame.gpu.device.create_command_encoder(&Default::default());
    frame.encode(&scene, &mut encoder, None, None).unwrap();
    frame.gpu.queue.submit(Some(encoder.finish()));
    let pixels = frame.gpu.readback_rgba16(&frame.targets.hdr).unwrap();
    frame.release_mesh(mesh).unwrap();
    pixels
}

fn redder(pixels: &[u16], x: u32) -> bool {
    let at = ((SIZE / 2 * SIZE + x) * 4) as usize;
    let (red, blue) = (pixels[at], pixels[at + 2]);
    assert!(red != blue, "column {x} shows neither half");
    red > blue
}

fn general_features() -> OpaqueFeatures {
    OpaqueFeatures::ALL
        .without(OpaqueFeatures::named("fine_noise").unwrap())
        .without(OpaqueFeatures::named("procedural_lacquer").unwrap())
        .without(OpaqueFeatures::named("procedural_detail").unwrap())
}

fn used_uv1(frame: &Frame) -> bool {
    frame.opaque_drawn().iter().any(|used| used.uv1)
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_quad_samples_detail_baked_through_its_second_uv_set_through_that_set() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut frame = Frame::new(gpu, SIZE, SIZE).unwrap();
    let (left, right) = (SIZE / 6, SIZE - SIZE / 6);

    let first = detail_frame(
        &mut frame,
        &halves(0),
        Some(&MIRRORED),
        OpaqueFeatures::NONE,
    );
    assert!(!used_uv1(&frame));
    assert!(redder(&first, left) && !redder(&first, right));

    let second = detail_frame(
        &mut frame,
        &halves(1),
        Some(&MIRRORED),
        OpaqueFeatures::NONE,
    );
    assert!(used_uv1(&frame));
    assert!(!redder(&second, left) && redder(&second, right));

    let general = detail_frame(&mut frame, &halves(1), Some(&MIRRORED), general_features());
    assert!(
        general == second,
        "the general pipeline samples the second set differently"
    );

    let without = detail_frame(&mut frame, &halves(0), None, OpaqueFeatures::NONE);
    assert!(!used_uv1(&frame));
    assert!(
        without == first,
        "a second UV set changed a part baked through the first"
    );
    let general = detail_frame(&mut frame, &halves(0), None, general_features());
    assert!(
        general == first,
        "the general pipeline changed a part baked through the first"
    );
}
