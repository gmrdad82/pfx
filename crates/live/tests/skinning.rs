use std::sync::Arc;

use pfx_core::anim::{Animator, Pose, fixture, skin};
use pfx_gpu::Gpu;
use pfx_live::frame::{
    Camera, Frame, Instance, Matrix, MeshData, MeshHandle, Scene, SceneWind, Sun, multiply,
    transform, velocity_uv,
};
use pfx_live::shadow::{Quality, ReceiverBox, Shadows, View};
use pfx_live::skin::{InstancePose, SkinData};
use pfx_materials::Material;

const SIZE: u32 = 320;
const EYE: [f32; 3] = [0.0, 1.5, 4.5];
const FOV: f32 = 50.0;
const BONES: usize = 3;
const COLUMN_ID: u32 = 7;
const FLOOR_ID: u32 = 9;

fn identity() -> Matrix {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
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

fn shadow_view() -> View {
    View {
        eye: EYE,
        forward: [0.0, 0.0, -1.0],
        up: [0.0, 1.0, 0.0],
        fov_y: FOV.to_radians(),
        aspect: 1.0,
        near: 0.1,
        far: 12.0,
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
    Instance::new(handle, identity(), 1, FLOOR_ID)
}

fn column_data(column: &fixture::Column) -> MeshData<'_> {
    MeshData {
        positions: &column.positions,
        normals: &column.normals,
        tangents: &column.tangents,
        uvs: &column.uvs,
        uvs1: None,
        alpha: None,
        indices: &column.indices,
    }
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

struct Shot {
    hdr: Vec<[f32; 3]>,
    ids: Vec<u32>,
    velocity: Vec<[f32; 2]>,
}

fn read(gpu: &Gpu, texture: &wgpu::Texture, bytes_per_pixel: u32) -> Vec<u8> {
    let size = texture.size();
    let raw = size.width * bytes_per_pixel;
    let row = raw.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("skinning test readback"),
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
    let mapped = slice.get_mapped_range();
    let mut out = Vec::new();
    for padded in mapped.chunks_exact(row as usize) {
        out.extend_from_slice(&padded[..raw as usize]);
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
    let fit = shadows.fit(&shadow_view(), scene.sun.direction);
    let mut encoder = frame.gpu.device.create_command_encoder(&Default::default());
    frame
        .encode(&scene, &mut encoder, Some((shadows, &fit)), None)
        .unwrap();
    frame.gpu.queue.submit(Some(encoder.finish()));
    let hdr = frame
        .gpu
        .readback_rgba16(&frame.targets.hdr)
        .unwrap()
        .chunks_exact(4)
        .map(|p| std::array::from_fn(|c| half::f16::from_bits(p[c]).to_f32()))
        .collect();
    let ids = read(&frame.gpu, &frame.targets.ids, 4)
        .chunks_exact(4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect();
    let velocity = frame
        .readback_velocity()
        .unwrap()
        .into_iter()
        .map(|v| v.map(|bits| half::f16::from_bits(bits).to_f32()))
        .collect();
    Shot { hdr, ids, velocity }
}

fn posed(rig: &Arc<pfx_core::anim::Rig>, tick: u64) -> Vec<Matrix> {
    let mut animator = Animator::new(rig.clone(), 60);
    animator.play("bend", 0).unwrap();
    animator.blend("twist", 0.5, 0).unwrap();
    animator.palette(tick)
}

fn skinned(frame: &mut Frame, column: &fixture::Column) -> MeshHandle {
    frame
        .upload_skinned_mesh(
            column_data(column),
            SkinData {
                joints: &column.joints,
                weights: &column.weights,
            },
        )
        .unwrap()
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_skinned_column_draws_as_its_cpu_skinned_twin_with_its_shadow_and_ids() {
    let rig = Arc::new(fixture::rig(BONES));
    let column = fixture::column(BONES);
    let palette = posed(&rig, 18);
    let baked = skin(
        &column.positions,
        &column.normals,
        &column.tangents,
        &column.joints,
        &column.weights,
        &palette,
    )
    .unwrap();
    let (mut frame, mut shadows) = setup();
    let floor = floor(&mut frame);
    let gpu_mesh = skinned(&mut frame, &column);
    assert!(frame.skinned(gpu_mesh));
    let cpu_mesh = frame
        .upload_mesh(MeshData {
            positions: &baked.positions,
            normals: &baked.normals,
            tangents: &baked.tangents,
            ..column_data(&column)
        })
        .unwrap();
    frame
        .set_poses(&[InstancePose {
            instance: 0,
            joints: &palette,
            previous: &palette,
        }])
        .unwrap();
    let live = shoot(
        &mut frame,
        &mut shadows,
        &[Instance::new(gpu_mesh, identity(), 0, COLUMN_ID), floor],
    );
    frame.set_poses(&[]).unwrap();
    let reference = shoot(
        &mut frame,
        &mut shadows,
        &[Instance::new(cpu_mesh, identity(), 0, COLUMN_ID), floor],
    );
    let covered = live.ids.iter().filter(|&&id| id == COLUMN_ID).count();
    assert!(covered > 1500, "the column covers {covered} pixels");
    let flips = live
        .ids
        .iter()
        .zip(&reference.ids)
        .filter(|(a, b)| a != b)
        .count();
    assert!(flips <= 4, "{flips} pixels changed id");
    let mut worst = 0.0f32;
    for index in 0..live.hdr.len() {
        if live.ids[index] != reference.ids[index] {
            continue;
        }
        for channel in 0..3 {
            let a = live.hdr[index][channel].clamp(0.0, 1.0);
            let b = reference.hdr[index][channel].clamp(0.0, 1.0);
            worst = worst.max((a - b).abs());
        }
    }
    println!("column pixels {covered}, id flips {flips}, worst channel difference {worst}");
    assert!(worst <= 1.0 / 255.0, "worst channel difference {worst}");
    let shaded_floor = live
        .hdr
        .iter()
        .zip(&live.ids)
        .filter(|(_, id)| **id == FLOOR_ID)
        .map(|(rgb, _)| rgb[0])
        .collect::<Vec<_>>();
    let darkest = shaded_floor.iter().copied().fold(f32::MAX, f32::min);
    let brightest = shaded_floor.iter().copied().fold(0.0f32, f32::max);
    assert!(
        darkest < brightest * 0.6,
        "the column casts a shadow on the floor: {darkest} against {brightest}"
    );
    let rest = shoot(
        &mut frame,
        &mut shadows,
        &[Instance::new(gpu_mesh, identity(), 0, COLUMN_ID), floor],
    );
    let moved = rest
        .ids
        .iter()
        .zip(&live.ids)
        .filter(|(a, b)| a != b)
        .count();
    assert!(
        moved > 200,
        "an unposed skinned mesh draws its bind pose ({moved} pixels differ)"
    );
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_moving_joint_writes_its_motion_and_a_still_one_writes_none() {
    let rig = Arc::new(fixture::rig(BONES));
    let column = fixture::column(BONES);
    let skeleton = rig.skeleton();
    let previous = Pose::rest(skeleton).palette(skeleton);
    let mut moved = Pose::rest(skeleton);
    let shift = 0.12;
    moved.locals_mut()[BONES - 1].translation[0] += shift;
    let current = moved.palette(skeleton);
    let (mut frame, mut shadows) = setup();
    let floor = floor(&mut frame);
    let mesh = skinned(&mut frame, &column);
    frame
        .set_poses(&[InstancePose {
            instance: 0,
            joints: &current,
            previous: &previous,
        }])
        .unwrap();
    let shot = shoot(
        &mut frame,
        &mut shadows,
        &[Instance::new(mesh, identity(), 0, COLUMN_ID), floor],
    );
    let vp = view_projection();
    let pixel = |point: [f32; 3]| {
        let clip = transform(vp, [point[0], point[1], point[2], 1.0]);
        let x = (clip[0] / clip[3] * 0.5 + 0.5) * SIZE as f32;
        let y = (0.5 - clip[1] / clip[3] * 0.5) * SIZE as f32;
        (x as usize, y as usize)
    };
    let high = [shift, 2.8, 0.15];
    let (x, y) = pixel(high);
    let index = y * SIZE as usize + x;
    assert_eq!(shot.ids[index], COLUMN_ID);
    let expected = velocity_uv(
        transform(vp, [high[0], high[1], high[2], 1.0]),
        transform(vp, [high[0] - shift, high[1], high[2], 1.0]),
    );
    let got = shot.velocity[index];
    assert!(
        (got[0] - expected[0]).abs() < 2e-4 && (got[1] - expected[1]).abs() < 2e-4,
        "the moving joint's motion {got:?} against {expected:?}"
    );
    assert!(expected[0].abs() > 0.005);
    let low = [0.0, 0.6, 0.15];
    let (x, y) = pixel(low);
    let index = y * SIZE as usize + x;
    assert_eq!(shot.ids[index], COLUMN_ID);
    assert_eq!(shot.velocity[index], [0.0, 0.0], "the still joint's motion");
}
