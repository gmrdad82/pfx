use pfx_gpu::{Gpu, wgpu};
use pfx_live::frame::{
    Camera, Frame, Instance, Matrix, MeshData, MeshHandle, Scene, SceneWind, Sun, multiply,
};
use pfx_live::shadow::{Quality, Shadows, View};
use pfx_materials::Material;

const SIZE: u32 = 160;

fn identity() -> Matrix {
    let mut m = [[0.0; 4]; 4];
    for (i, column) in m.iter_mut().enumerate() {
        column[i] = 1.0;
    }
    m
}

fn sided(mut instance: Instance) -> Instance {
    instance.two_sided = true;
    instance
}

fn at(x: f32, y: f32, z: f32) -> Matrix {
    let mut m = identity();
    m[3] = [x, y, z, 1.0];
    m
}

fn perspective(near: f32, far: f32) -> Matrix {
    let f = 1.0 / (0.45f32).tan();
    let range = far / (near - far);
    [
        [f, 0.0, 0.0, 0.0],
        [0.0, f, 0.0, 0.0],
        [0.0, 0.0, range, -1.0],
        [0.0, 0.0, range * near, 0.0],
    ]
}

fn look_along_minus_z(eye: [f32; 3]) -> Matrix {
    at(-eye[0], -eye[1], -eye[2])
}

fn ball(frame: &mut Frame) -> MeshHandle {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut tangents = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();
    let (rings, sides) = (10u32, 16u32);
    for ring in 0..=rings {
        let phi = std::f32::consts::PI * ring as f32 / rings as f32;
        for side in 0..=sides {
            let theta = std::f32::consts::TAU * side as f32 / sides as f32;
            let n = [phi.sin() * theta.cos(), phi.cos(), phi.sin() * theta.sin()];
            positions.push(n.map(|v| v * 0.5));
            normals.push(n);
            tangents.push([-theta.sin(), 0.0, theta.cos(), 1.0]);
            uvs.push([side as f32 / sides as f32, ring as f32 / rings as f32]);
        }
    }
    for ring in 0..rings {
        for side in 0..sides {
            let a = ring * (sides + 1) + side;
            let b = a + sides + 1;
            indices.extend_from_slice(&[a, a + 1, b, a + 1, b + 1, b]);
        }
    }
    frame
        .upload_mesh(MeshData {
            positions: &positions,
            normals: &normals,
            tangents: &tangents,
            uvs: &uvs,
            uvs1: None,
            alpha: None,
            indices: &indices,
        })
        .unwrap()
}

fn quad(frame: &mut Frame, corners: [[f32; 3]; 4], normal: [f32; 3]) -> MeshHandle {
    frame
        .upload_mesh(MeshData {
            positions: &corners,
            normals: &[normal; 4],
            tangents: &[[1.0, 0.0, 0.0, 1.0]; 4],
            uvs: &[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]],
            uvs1: None,
            alpha: None,
            indices: &[0, 2, 1, 1, 2, 3],
        })
        .unwrap()
}

fn read(
    frame: &Frame,
    texture: &wgpu::Texture,
    texel: u32,
    aspect: wgpu::TextureAspect,
) -> Vec<u8> {
    let size = texture.size();
    let row = (size.width * texel).div_ceil(256) * 256;
    let buffer = frame.gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("culling readback"),
        size: u64::from(row * size.height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = frame.gpu.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(size.height),
            },
        },
        wgpu::Extent3d {
            width: size.width,
            height: size.height,
            depth_or_array_layers: 1,
        },
    );
    frame.gpu.queue.submit(Some(encoder.finish()));
    buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    frame
        .gpu
        .device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    let mapped = buffer.slice(..).get_mapped_range().expect("a mapped range");
    let mut out = Vec::new();
    for y in 0..size.height {
        let start = (y * row) as usize;
        out.extend_from_slice(&mapped[start..start + (size.width * texel) as usize]);
    }
    drop(mapped);
    buffer.unmap();
    out
}

#[derive(PartialEq)]
struct Shot {
    hdr: Vec<u16>,
    ids: Vec<u8>,
    depth: Vec<u8>,
}

struct Stage {
    frame: Frame,
    shadows: Shadows,
    ball: MeshHandle,
    floor: MeshHandle,
    wall: MeshHandle,
}

const EYE: [f32; 3] = [0.0, 1.5, 6.0];
const SUN: [f32; 3] = [0.35, 0.85, 0.4];

fn stage(culling: bool) -> Stage {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut frame = Frame::new(gpu, SIZE, SIZE).unwrap();
    frame.set_culling(culling);
    let ball = ball(&mut frame);
    let floor = quad(
        &mut frame,
        [
            [-40.0, 0.0, -40.0],
            [40.0, 0.0, -40.0],
            [-40.0, 0.0, 40.0],
            [40.0, 0.0, 40.0],
        ],
        [0.0, 1.0, 0.0],
    );
    let wall = quad(
        &mut frame,
        [
            [-1.5, 0.0, 0.0],
            [1.5, 0.0, 0.0],
            [-1.5, 3.0, 0.0],
            [1.5, 3.0, 0.0],
        ],
        [0.0, 0.0, 1.0],
    );
    let shadows = Shadows::new(
        &frame.gpu.device,
        Quality {
            resolution: 512,
            ..Default::default()
        },
    );
    Stage {
        frame,
        shadows,
        ball,
        floor,
        wall,
    }
}

fn shoot(
    stage: &mut Stage,
    instances: &[Instance],
    eye: [f32; 3],
    previous: Matrix,
) -> (Shot, Matrix) {
    let (near, far) = (0.1, 30.0);
    let view = look_along_minus_z(eye);
    let projection = perspective(near, far);
    let camera = Camera {
        view,
        projection,
        previous_view_projection: previous,
        position: eye,
    };
    let fit = stage.shadows.fit(
        &View {
            eye,
            forward: [0.0, 0.0, -1.0],
            up: [0.0, 1.0, 0.0],
            fov_y: 0.9,
            aspect: 1.0,
            near,
            far,
        },
        SUN,
    );
    let materials = [Material::default()];
    let scene = Scene {
        camera,
        time: 0.0,
        seed: 3,
        sun: Sun {
            direction: SUN,
            colour: [1.0; 3],
            intensity: 3.0,
        },
        instances,
        materials: &materials,
        deformers: &[],
        wind: SceneWind::default(),
    };
    let frame = &mut stage.frame;
    let mut encoder = frame.gpu.device.create_command_encoder(&Default::default());
    frame
        .encode(&scene, &mut encoder, Some((&mut stage.shadows, &fit)), None)
        .unwrap();
    frame.gpu.queue.submit(Some(encoder.finish()));
    let shot = Shot {
        hdr: frame.gpu.readback_rgba16(&frame.targets.hdr).unwrap(),
        ids: read(frame, &frame.targets.ids, 4, wgpu::TextureAspect::All),
        depth: read(
            frame,
            &frame.targets.depth,
            4,
            wgpu::TextureAspect::DepthOnly,
        ),
    };
    (shot, multiply(projection, view))
}

fn scattered(stage: &Stage) -> Vec<Instance> {
    let mut instances = vec![sided(Instance::new(stage.floor, identity(), 0, 1))];
    let mut id = 2;
    for x in -6..=6 {
        for z in -6..=6 {
            let place = at(x as f32 * 2.5, 0.5, z as f32 * 2.5 + 2.0);
            instances.push(Instance::new(stage.ball, place, 0, id));
            id += 1;
        }
    }
    instances.push(Instance::new(stage.ball, at(0.0, 0.5, -200.0), 0, id));
    instances
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn culling_leaves_every_pixel_id_and_depth_as_it_was() {
    let mut off = stage(false);
    let mut on = stage(true);
    let instances = scattered(&on);
    let mut previous = [identity(); 2];
    for step in 0..3 {
        let eye = [step as f32 * 0.7, 1.5, 6.0 - step as f32];
        let (plain, seen_plain) = shoot(&mut off, &instances, eye, previous[0]);
        let (culled, seen_culled) = shoot(&mut on, &instances, eye, previous[1]);
        previous = [seen_plain, seen_culled];
        assert!(plain.ids == culled.ids, "ids differ at step {step}");
        assert!(plain.depth == culled.depth, "depth differs at step {step}");
        assert!(plain.hdr == culled.hdr, "colour differs at step {step}");
    }
    let stats = on.frame.cull_stats();
    assert_eq!(stats.instances as usize, instances.len());
    assert!(
        stats.camera.instances < stats.instances / 2,
        "the camera kept {} of {}",
        stats.camera.instances,
        stats.instances
    );
    assert!(
        stats
            .cascades
            .iter()
            .all(|cascade| cascade.instances < stats.instances)
    );
    assert_eq!(stats.occlusion_tested, on.frame.occlusion_supported());
    assert!(stats.culled() > 0);
    assert_eq!(stats.drawable, stats.instances);
    let plain = off.frame.cull_stats();
    assert_eq!(plain.camera.instances, plain.instances);
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn an_object_that_moves_into_view_or_out_from_behind_a_wall_is_drawn_on_its_first_visible_frame() {
    emerging(false, true);
    emerging(true, true);
    emerging(true, false);
}

fn emerging(occluded: bool, counted: bool) {
    let mut off = stage(false);
    let mut on = stage(true);
    on.frame.set_occlusion(occluded);
    on.frame.set_occlusion_counts(counted);
    let eye = EYE;
    let still = multiply(perspective(0.1, 30.0), look_along_minus_z(eye));
    let mut drawn_on_some_frame = false;
    let mut hidden_behind_the_wall = false;
    for step in 0..24 {
        let x = -9.0 + step as f32 * 0.75;
        let instances = [
            sided(Instance::new(off.floor, identity(), 0, 1)),
            sided(Instance::new(off.wall, at(0.0, 0.0, 2.0), 0, 2)),
            Instance::new(off.ball, at(x, 1.0, -1.0), 0, 3),
        ];
        let (plain, _) = shoot(&mut off, &instances, eye, still);
        let (culled, _) = shoot(&mut on, &instances, eye, still);
        let shown = |ids: &[u8]| {
            ids.chunks_exact(4)
                .filter(|pixel| u32::from_le_bytes((*pixel).try_into().unwrap()) == 3)
                .count()
        };
        assert_eq!(
            shown(&plain.ids),
            shown(&culled.ids),
            "ball pixels at step {step}"
        );
        assert!(plain.ids == culled.ids, "ids differ at step {step}");
        assert!(plain.hdr == culled.hdr, "colour differs at step {step}");
        drawn_on_some_frame |= shown(&culled.ids) > 0;
        if let Some(visible) = on.frame.occlusion_visible() {
            assert!(on.frame.cull_stats().occlusion_tested);
            if shown(&culled.ids) > 0 {
                assert_eq!(
                    visible[2], 1,
                    "the ball shows but was not kept at step {step}"
                );
            }
            if step == 12 {
                assert_eq!(visible[2], 0, "the ball behind the wall was not occluded");
                hidden_behind_the_wall = true;
            }
        }
    }
    assert!(drawn_on_some_frame);
    assert!(hidden_behind_the_wall || !occluded || !on.frame.occlusion_supported());
}

fn shown(ids: &[u8], id: u32) -> usize {
    ids.chunks_exact(4)
        .filter(|pixel| u32::from_le_bytes((*pixel).try_into().unwrap()) == id)
        .count()
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn occlusion_is_on_by_default_and_what_moves_is_never_stale_against_a_fresh_frame() {
    let mut on = stage(true);
    assert!(on.frame.occlusion());
    let eye = EYE;
    let still = multiply(perspective(0.1, 30.0), look_along_minus_z(eye));
    let mut hidden_once = false;
    let mut kept_static = false;
    for step in 0..10 {
        let wall_x = if step < 4 {
            0.0
        } else {
            (step - 3) as f32 * 1.2
        };
        let jumper = if step < 6 {
            at(0.0, 1.0, -1.5)
        } else {
            at(-2.5 + (step - 6) as f32 * 0.4, 1.0, -1.5)
        };
        let instances = [
            sided(Instance::new(on.floor, identity(), 0, 1)),
            sided(Instance::new(on.wall, at(wall_x, 0.0, 2.0), 0, 2)),
            Instance::new(on.ball, jumper, 0, 3),
            Instance::new(on.ball, at(0.6, 0.6, -2.5), 0, 4),
            Instance::new(on.ball, at(-0.6, 0.6, -2.5), 0, 5),
            Instance::new(on.ball, at(3.0, 0.6, -1.0), 0, 6),
        ];
        let mut fresh = stage(false);
        let (reference, _) = shoot(&mut fresh, &instances, eye, still);
        let (culled, _) = shoot(&mut on, &instances, eye, still);
        for id in 1..=6 {
            assert_eq!(
                shown(&reference.ids, id),
                shown(&culled.ids, id),
                "instance {id} pixels at step {step}"
            );
        }
        assert!(reference.ids == culled.ids, "ids differ at step {step}");
        assert!(
            reference.depth == culled.depth,
            "depth differs at step {step}"
        );
        assert!(reference.hdr == culled.hdr, "colour differs at step {step}");
        if let Some(visible) = on.frame.occlusion_visible() {
            assert!(on.frame.cull_stats().occlusion_tested);
            for (index, id) in (2..6).zip(3..=6) {
                if shown(&culled.ids, id) > 0 {
                    assert_eq!(
                        visible[index], 1,
                        "instance {id} shows but was dropped at {step}"
                    );
                }
            }
            if (1..4).contains(&step) {
                hidden_once |= visible[2] == 0;
                kept_static |= visible[3] == 0 && visible[4] == 0;
            }
        }
    }
    assert!(!on.frame.occlusion_supported() || (hidden_once && kept_static));
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_draw_distance_draws_inside_it_fades_across_its_last_tenth_and_culls_past_it() {
    let mut plain = stage(true);
    let mut far = stage(true);
    let eye = EYE;
    let still = multiply(perspective(0.1, 30.0), look_along_minus_z(eye));
    let place = at(1.0, 0.5, -4.0);
    let distance = {
        let dx = 1.0 - eye[0];
        let dy = 0.5 - eye[1];
        let dz = -4.0 - eye[2];
        (dx * dx + dy * dy + dz * dz).sqrt()
    };
    let floor = sided(Instance::new(plain.floor, identity(), 0, 1));
    let ball = Instance::new(plain.ball, place, 0, 2);
    let (whole, _) = shoot(&mut plain, &[floor, ball], eye, still);
    let full = shown(&whole.ids, 2);
    assert!(full > 0);
    let mut seen = Vec::new();
    for reach in [distance * 2.0, distance / 0.95, distance * 0.98] {
        let limited = Instance {
            max_distance: reach,
            ..ball
        };
        let (shot, _) = shoot(&mut far, &[floor, limited], eye, still);
        seen.push((shot, far.frame.cull_stats().culled()));
    }
    assert!(
        seen[0].0 == whole,
        "a ball well inside its distance changed"
    );
    assert_eq!(seen[0].1, 0);
    let banded = shown(&seen[1].0.ids, 2);
    assert!(
        banded > 0 && banded < full,
        "{banded} of {full} in the fade band"
    );
    assert_eq!(seen[1].1, 0);
    assert_eq!(shown(&seen[2].0.ids, 2), 0, "a ball past its distance drew");
    assert_eq!(seen[2].1, 1);
    assert!(seen[2].0.hdr != whole.hdr);
}
