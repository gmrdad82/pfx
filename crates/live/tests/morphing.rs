use pfx_gpu::Gpu;
use pfx_live::frame::{
    Camera, Frame, Instance, Matrix, MeshData, MeshHandle, Scene, SceneWind, Sun, multiply,
    transform, velocity_uv,
};
use pfx_live::morph::{InstanceWeights, MorphData, MorphTargetData};
use pfx_live::shadow::{Quality, ReceiverBox, Shadows, View};
use pfx_materials::Material;

const SIZE: u32 = 320;
const EYE: [f32; 3] = [0.0, 1.5, 4.5];
const FOV: f32 = 50.0;
const HEAD_ID: u32 = 7;
const OTHER_ID: u32 = 8;
const FLOOR_ID: u32 = 9;
const CENTRE: [f32; 3] = [0.0, 1.1, 0.0];
const RADIUS: f32 = 0.6;
const SHIFT: f32 = 0.3;

fn translation(x: f32) -> Matrix {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [x, 0.0, 0.0, 1.0],
    ]
}

fn perspective() -> Matrix {
    let near = 0.1;
    let far = 100.0;
    let f = 1.0 / (FOV.to_radians() * 0.5).tan();
    [
        [f, 0.0, 0.0, 0.0],
        [0.0, f, 0.0, 0.0],
        [0.0, 0.0, far / (near - far), -1.0],
        [0.0, 0.0, far * near / (near - far), 0.0],
    ]
}

fn look() -> Matrix {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [-EYE[0], -EYE[1], -EYE[2], 1.0],
    ]
}

fn view_projection() -> Matrix {
    multiply(perspective(), look())
}

fn camera() -> Camera {
    Camera {
        view: look(),
        projection: perspective(),
        previous_view_projection: view_projection(),
        position: EYE,
    }
}

fn sun() -> Sun {
    let direction = [-0.45f32, 0.8, 0.4];
    let length = (direction.iter().map(|v| v * v).sum::<f32>()).sqrt();
    Sun {
        direction: direction.map(|v| v / length),
        colour: [1.0; 3],
        intensity: 1.0,
    }
}

fn materials() -> [Material; 2] {
    let lambert = |base| Material {
        base,
        roughness: 1.0,
        specular: 0.0,
        metalness: 0.0,
        clearcoat: 0.0,
        ..Material::default()
    };
    [lambert([0.55, 0.5, 0.45]), lambert([0.35, 0.4, 0.45])]
}

struct Head {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    tangents: Vec<[f32; 4]>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
    bulge: Vec<[f32; 3]>,
    bulge_normals: Vec<[f32; 3]>,
    slide: Vec<[f32; 3]>,
}

fn head() -> Head {
    let rings = 24u32;
    let segments = 32u32;
    let mut head = Head {
        positions: Vec::new(),
        normals: Vec::new(),
        tangents: Vec::new(),
        uvs: Vec::new(),
        indices: Vec::new(),
        bulge: Vec::new(),
        bulge_normals: Vec::new(),
        slide: Vec::new(),
    };
    for ring in 0..=rings {
        let v = ring as f32 / rings as f32;
        let theta = v * std::f32::consts::PI;
        for segment in 0..=segments {
            let u = segment as f32 / segments as f32;
            let phi = u * std::f32::consts::TAU;
            let n = [
                theta.sin() * phi.sin(),
                theta.cos(),
                theta.sin() * phi.cos(),
            ];
            head.positions.push([
                CENTRE[0] + n[0] * RADIUS,
                CENTRE[1] + n[1] * RADIUS,
                CENTRE[2] + n[2] * RADIUS,
            ]);
            head.normals.push(n);
            head.tangents.push([phi.cos(), 0.0, -phi.sin(), 1.0]);
            head.uvs.push([u, v]);
            let lift = n[1].max(0.0) * 0.3;
            head.bulge.push(n.map(|c| c * lift));
            head.bulge_normals
                .push([-0.4 * n[0] * lift, 0.5 * lift, -0.4 * n[2] * lift]);
            head.slide.push([SHIFT, 0.0, 0.0]);
        }
    }
    let row = segments + 1;
    for ring in 0..rings {
        for segment in 0..segments {
            let a = ring * row + segment;
            let b = a + row;
            head.indices.extend([a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    head
}

fn data<'a>(head: &'a Head, positions: &'a [[f32; 3]], normals: &'a [[f32; 3]]) -> MeshData<'a> {
    MeshData {
        positions,
        normals,
        tangents: &head.tangents,
        uvs: &head.uvs,
        uvs1: None,
        alpha: None,
        indices: &head.indices,
    }
}

fn morphed(frame: &mut Frame, head: &Head) -> MeshHandle {
    let targets = [
        MorphTargetData {
            positions: &head.bulge,
            normals: Some(&head.bulge_normals),
        },
        MorphTargetData {
            positions: &head.slide,
            normals: None,
        },
    ];
    let handle = frame
        .upload_morphed_mesh(
            data(head, &head.positions, &head.normals),
            None,
            MorphData { targets: &targets },
        )
        .unwrap();
    assert!(frame.morphed(handle) && !frame.skinned(handle));
    handle
}

fn blended(frame: &mut Frame, head: &Head, weights: [f32; 2]) -> MeshHandle {
    let positions: Vec<[f32; 3]> = (0..head.positions.len())
        .map(|i| {
            std::array::from_fn(|c| {
                head.positions[i][c] + head.bulge[i][c] * weights[0] + head.slide[i][c] * weights[1]
            })
        })
        .collect();
    let normals: Vec<[f32; 3]> = (0..head.normals.len())
        .map(|i| {
            std::array::from_fn(|c| head.normals[i][c] + head.bulge_normals[i][c] * weights[0])
        })
        .collect();
    frame.upload_mesh(data(head, &positions, &normals)).unwrap()
}

fn plain(frame: &mut Frame, head: &Head) -> MeshHandle {
    frame
        .upload_mesh(data(head, &head.positions, &head.normals))
        .unwrap()
}

fn floor(frame: &mut Frame) -> Instance {
    let corners = [
        [-2.5, 0.0, -2.5],
        [2.5, 0.0, -2.5],
        [-2.5, 0.0, 2.5],
        [2.5, 0.0, 2.5],
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
    Instance::new(handle, translation(0.0), 1, FLOOR_ID)
}

fn setup() -> (Frame, Shadows) {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let frame = Frame::new(gpu, SIZE, SIZE).unwrap();
    let shadows = Shadows::new(
        &frame.gpu.device,
        Quality {
            resolution: 2048,
            receiver: Some(ReceiverBox {
                min: [-2.5, -0.1, -2.5],
                max: [2.5, 3.5, 2.5],
            }),
            caster_margin: 3.0,
            sun_radius_deg: 0.0,
            ..Quality::default()
        },
    );
    (frame, shadows)
}

#[derive(PartialEq)]
struct Shot {
    hdr: Vec<u16>,
    ids: Vec<u32>,
    velocity: Vec<[u16; 2]>,
}

impl Shot {
    fn rgb(&self, index: usize) -> [f32; 3] {
        std::array::from_fn(|c| half::f16::from_bits(self.hdr[index * 4 + c]).to_f32())
    }

    fn motion(&self, index: usize) -> [f32; 2] {
        self.velocity[index].map(|bits| half::f16::from_bits(bits).to_f32())
    }
}

fn read_ids(gpu: &Gpu, texture: &wgpu::Texture) -> Vec<u32> {
    let size = texture.size();
    let raw = size.width * 4;
    let row = raw.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("morphing test readback"),
        size: u64::from(row) * u64::from(size.height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(size.height),
            },
        },
        size,
    );
    gpu.queue.submit(Some(encoder.finish()));
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    let mapped = slice.get_mapped_range().expect("a mapped range");
    let mut out = Vec::new();
    for padded in mapped.chunks_exact(row as usize) {
        out.extend(
            padded[..raw as usize]
                .chunks_exact(4)
                .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])),
        );
    }
    out
}

fn shoot(frame: &mut Frame, shadows: &mut Shadows, instances: &[Instance]) -> Shot {
    let materials = materials();
    let scene = Scene {
        camera: camera(),
        time: 0.0,
        seed: 3,
        sun: sun(),
        instances,
        materials: &materials,
        deformers: &[],
        wind: SceneWind::default(),
    };
    let view = View {
        eye: EYE,
        forward: [0.0, 0.0, -1.0],
        up: [0.0, 1.0, 0.0],
        fov_y: FOV.to_radians(),
        aspect: 1.0,
        near: 0.1,
        far: 12.0,
    };
    let fit = shadows.fit(&view, scene.sun.direction);
    let mut encoder = frame.gpu.device.create_command_encoder(&Default::default());
    frame
        .encode(&scene, &mut encoder, Some((shadows, &fit)), None)
        .unwrap();
    frame.gpu.queue.submit(Some(encoder.finish()));
    Shot {
        hdr: frame.gpu.readback_rgba16(&frame.targets.hdr).unwrap(),
        ids: read_ids(&frame.gpu, &frame.targets.ids),
        velocity: frame.readback_velocity().unwrap(),
    }
}

fn weights(instance: usize, current: &[f32; 2]) -> InstanceWeights<'_> {
    InstanceWeights {
        instance,
        current,
        previous: current,
    }
}

fn assert_twins(live: &Shot, reference: &Shot, ids: &[u32]) {
    for id in ids {
        let covered = live.ids.iter().filter(|&&got| got == *id).count();
        assert!(covered > 1500, "instance {id} covers {covered} pixels");
    }
    let flips = live
        .ids
        .iter()
        .zip(&reference.ids)
        .filter(|(a, b)| a != b)
        .count();
    assert!(flips <= 4, "{flips} pixels changed id");
    let mut worst = 0.0f32;
    for index in 0..live.ids.len() {
        if live.ids[index] != reference.ids[index] {
            continue;
        }
        let (a, b) = (live.rgb(index), reference.rgb(index));
        for channel in 0..3 {
            worst = worst.max((a[channel].clamp(0.0, 1.0) - b[channel].clamp(0.0, 1.0)).abs());
        }
    }
    println!("id flips {flips}, worst channel difference {worst}");
    assert!(worst <= 1.0 / 255.0, "worst channel difference {worst}");
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_morphed_head_draws_as_its_cpu_blended_twin_with_its_shadow_ids_and_no_motion() {
    let head = head();
    let (mut frame, mut shadows) = setup();
    let floor = floor(&mut frame);
    let gpu_mesh = morphed(&mut frame, &head);
    let set = [0.7, 0.4];
    let cpu_mesh = blended(&mut frame, &head, set);
    frame.set_weights(&[weights(0, &set)]).unwrap();
    let live = shoot(
        &mut frame,
        &mut shadows,
        &[Instance::new(gpu_mesh, translation(0.0), 0, HEAD_ID), floor],
    );
    frame.set_weights(&[]).unwrap();
    let reference = shoot(
        &mut frame,
        &mut shadows,
        &[Instance::new(cpu_mesh, translation(0.0), 0, HEAD_ID), floor],
    );
    assert_twins(&live, &reference, &[HEAD_ID]);
    let shaded_floor: Vec<f32> = (0..live.ids.len())
        .filter(|&index| live.ids[index] == FLOOR_ID)
        .map(|index| live.rgb(index)[0])
        .collect();
    let darkest = shaded_floor.iter().copied().fold(f32::MAX, f32::min);
    let brightest = shaded_floor.iter().copied().fold(0.0f32, f32::max);
    assert!(
        darkest < brightest * 0.6,
        "the head casts a shadow on the floor: {darkest} against {brightest}"
    );
    let mut floor_worst = 0.0f32;
    for index in 0..live.ids.len() {
        if live.ids[index] == FLOOR_ID && reference.ids[index] == FLOOR_ID {
            floor_worst = floor_worst.max((live.rgb(index)[0] - reference.rgb(index)[0]).abs());
        }
    }
    assert!(
        floor_worst <= 1.0 / 255.0,
        "the floor's shadow differs by {floor_worst}"
    );
    assert!(
        live.ids
            .iter()
            .enumerate()
            .filter(|(_, id)| **id == HEAD_ID)
            .all(|(index, _)| live.motion(index) == [0.0, 0.0]),
        "unchanged weights write no motion"
    );
    let rest = shoot(
        &mut frame,
        &mut shadows,
        &[Instance::new(gpu_mesh, translation(0.0), 0, HEAD_ID), floor],
    );
    let moved = rest
        .ids
        .iter()
        .zip(&live.ids)
        .filter(|(a, b)| a != b)
        .count();
    assert!(
        moved > 200,
        "an unweighted head draws its rest shape ({moved} pixels differ)"
    );
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn two_instances_of_one_morphed_mesh_take_their_own_weights() {
    let head = head();
    let (mut frame, mut shadows) = setup();
    let floor = floor(&mut frame);
    let gpu_mesh = morphed(&mut frame, &head);
    let left = [1.0, 0.0];
    let right = [0.0, 1.0];
    let cpu_left = blended(&mut frame, &head, left);
    let cpu_right = blended(&mut frame, &head, right);
    let place = |mesh, x, id| Instance::new(mesh, translation(x), 0, id);
    frame
        .set_weights(&[weights(1, &right), weights(0, &left)])
        .unwrap();
    let live = shoot(
        &mut frame,
        &mut shadows,
        &[
            place(gpu_mesh, -1.0, HEAD_ID),
            place(gpu_mesh, 1.0, OTHER_ID),
            floor,
        ],
    );
    frame.set_weights(&[]).unwrap();
    let reference = shoot(
        &mut frame,
        &mut shadows,
        &[
            place(cpu_left, -1.0, HEAD_ID),
            place(cpu_right, 1.0, OTHER_ID),
            floor,
        ],
    );
    assert_twins(&live, &reference, &[HEAD_ID, OTHER_ID]);
    frame
        .set_weights(&[weights(0, &right), weights(1, &left)])
        .unwrap();
    let swapped = shoot(
        &mut frame,
        &mut shadows,
        &[
            place(gpu_mesh, -1.0, HEAD_ID),
            place(gpu_mesh, 1.0, OTHER_ID),
            floor,
        ],
    );
    let changed = swapped
        .ids
        .iter()
        .zip(&live.ids)
        .filter(|(a, b)| a != b)
        .count();
    assert!(changed > 200, "swapped weights move {changed} pixels");
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn zero_weights_draw_the_plain_mesh_byte_for_byte() {
    let head = head();
    let (mut frame, mut shadows) = setup();
    let floor = floor(&mut frame);
    let gpu_mesh = morphed(&mut frame, &head);
    let plain_mesh = plain(&mut frame, &head);
    let reference = shoot(
        &mut frame,
        &mut shadows,
        &[
            Instance::new(plain_mesh, translation(0.0), 0, HEAD_ID),
            floor,
        ],
    );
    let unset = shoot(
        &mut frame,
        &mut shadows,
        &[Instance::new(gpu_mesh, translation(0.0), 0, HEAD_ID), floor],
    );
    frame.set_weights(&[weights(0, &[0.0, 0.0])]).unwrap();
    let zero = shoot(
        &mut frame,
        &mut shadows,
        &[Instance::new(gpu_mesh, translation(0.0), 0, HEAD_ID), floor],
    );
    assert!(reference.ids.contains(&HEAD_ID));
    assert!(
        unset == reference,
        "an unweighted morphed mesh differs from the plain mesh"
    );
    assert!(zero == reference, "zero weights differ from the plain mesh");
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_changing_weight_writes_its_motion() {
    let head = head();
    let (mut frame, mut shadows) = setup();
    let floor = floor(&mut frame);
    let mesh = morphed(&mut frame, &head);
    frame
        .set_weights(&[InstanceWeights {
            instance: 0,
            current: &[0.0, 1.0],
            previous: &[0.0, 0.0],
        }])
        .unwrap();
    let shot = shoot(
        &mut frame,
        &mut shadows,
        &[Instance::new(mesh, translation(0.0), 0, HEAD_ID), floor],
    );
    let vp = view_projection();
    let front = [CENTRE[0] + SHIFT, CENTRE[1], CENTRE[2] + RADIUS];
    let clip = transform(vp, [front[0], front[1], front[2], 1.0]);
    let x = ((clip[0] / clip[3] * 0.5 + 0.5) * SIZE as f32) as usize;
    let y = ((0.5 - clip[1] / clip[3] * 0.5) * SIZE as f32) as usize;
    let index = y * SIZE as usize + x;
    assert_eq!(shot.ids[index], HEAD_ID);
    let expected = velocity_uv(
        clip,
        transform(vp, [front[0] - SHIFT, front[1], front[2], 1.0]),
    );
    let got = shot.motion(index);
    assert!(
        (got[0] - expected[0]).abs() < 2e-4 && (got[1] - expected[1]).abs() < 2e-4,
        "the sliding head's motion {got:?} against {expected:?}"
    );
    assert!(expected[0].abs() > 0.005);
}
