use pfx_gpu::Gpu;
use pfx_gpu::pace::{Pacer, Turns};
use pfx_live::frame::{
    Camera as LiveCamera, Frame, Instance as LiveInstance, Matrix, MeshData, Scene as LiveScene,
    SceneWind, Sun as LiveSun, multiply,
};
use pfx_live::shadow::{Quality, ReceiverBox, Shadows, View};
use pfx_live::sky::SkySource;
use pfx_load::Sky;
use pfx_materials::Material;
use pfx_trace::detail::Lens;
use pfx_trace::stage::{Mesh, Placement, Stage};
use pfx_trace::{Camera, Projection, Sun};

const VIEW: u32 = 64;
const FOV_Y: f32 = 36.0;
const SAMPLES: u32 = 8;

const FLOOR: u32 = 96;
const FLOOR_HEIGHT: f32 = 4.0;
const FLOOR_FOV: f32 = 50.0;

struct Arrays {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    tangents: Vec<[f32; 4]>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
}

impl Arrays {
    fn quad(origin: [f32; 3], edge_u: [f32; 3], edge_v: [f32; 3]) -> Self {
        let normal = unit(cross(edge_u, edge_v));
        Self {
            positions: vec![
                origin,
                add(origin, edge_u),
                add(origin, add(edge_u, edge_v)),
                add(origin, edge_v),
            ],
            normals: vec![normal; 4],
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

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn unit(v: [f32; 3]) -> [f32; 3] {
    let length = dot(v, v).sqrt();
    v.map(|value| value / length)
}

fn identity_matrix() -> Matrix {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

fn black_sky() -> Sky {
    Sky {
        width: 1,
        height: 1,
        texels: vec![[0.0, 0.0, 0.0, 1.0]],
    }
}

fn lambert(base: [f32; 3]) -> Material {
    Material {
        base,
        roughness: 1.0,
        specular: 0.0,
        metalness: 0.0,
        clearcoat: 0.0,
        emission: [0.0; 3],
        ..Material::default()
    }
}

fn emitter(emission: [f32; 3]) -> Material {
    Material {
        base: [0.0; 3],
        roughness: 1.0,
        specular: 0.0,
        metalness: 0.0,
        clearcoat: 0.0,
        emission,
        ..Material::default()
    }
}

fn axes(forward: [f32; 3]) -> ([f32; 3], [f32; 3], [f32; 3]) {
    let forward = unit(forward);
    let right = unit(cross(forward, [0.0, 1.0, 0.0]));
    let up = cross(right, forward);
    (forward, right, up)
}

fn live_look(eye: [f32; 3], forward: [f32; 3], fov_y: f32) -> LiveCamera {
    let (forward, right, up) = axes(forward);
    let view: Matrix = [
        [right[0], up[0], -forward[0], 0.0],
        [right[1], up[1], -forward[1], 0.0],
        [right[2], up[2], -forward[2], 0.0],
        [-dot(right, eye), -dot(up, eye), dot(forward, eye), 1.0],
    ];
    let near = 0.05;
    let far = 40.0;
    let tan = (fov_y.to_radians() * 0.5).tan();
    let aspect = 1.0;
    let projection: Matrix = [
        [1.0 / (tan * aspect), 0.0, 0.0, 0.0],
        [0.0, 1.0 / tan, 0.0, 0.0],
        [0.0, 0.0, far / (near - far), -1.0],
        [0.0, 0.0, far * near / (near - far), 0.0],
    ];
    LiveCamera {
        view,
        projection,
        previous_view_projection: multiply(projection, view),
        position: eye,
    }
}

fn trace_look(eye: [f32; 3], forward: [f32; 3], fov_y: f32) -> Camera {
    let (forward, right, up) = axes(forward);
    let tan = (fov_y.to_radians() * 0.5).tan();
    Camera {
        origin: eye,
        forward,
        right: right.map(|value| value * tan),
        up: up.map(|value| value * tan),
    }
}

struct ViewTrace<'a> {
    meshes: &'a [Mesh<'a>],
    placements: &'a [Placement<'a>],
    materials: &'a [Material],
    camera: Camera,
    projection: Projection,
    sun: Sun,
    size: u32,
    samples: u32,
}

fn trace_color(gpu: &Gpu, view: &ViewTrace<'_>, turns: &mut Turns) -> (Vec<[f32; 3]>, f64) {
    let staged = Stage {
        meshes: view.meshes,
        instances: view.placements,
        materials: view.materials,
        sky: black_sky(),
        sun: view.sun,
        camera: view.camera,
        projection: view.projection,
        lens: Lens::default(),
    }
    .build()
    .unwrap();
    let mut trace = staged.trace(gpu, view.size, view.size).unwrap();
    let mut pacer = Pacer::default();
    let stats = trace
        .sample_paced(gpu, view.samples, 7, &mut pacer, |ms| turns.add(ms))
        .unwrap();
    turns.turn();
    let output = trace.readback(gpu).unwrap();
    let pixels = output
        .color
        .chunks_exact(16)
        .map(|pixel| {
            std::array::from_fn(|channel| {
                f32::from_le_bytes(pixel[channel * 4..channel * 4 + 4].try_into().unwrap())
            })
        })
        .collect();
    (pixels, stats.longest_ms())
}

fn live_color(frame: &Frame) -> Vec<[f32; 3]> {
    frame
        .gpu
        .readback_rgba16(&frame.targets.hdr)
        .unwrap()
        .chunks_exact(4)
        .map(|pixel| std::array::from_fn(|channel| half::f16::from_bits(pixel[channel]).to_f32()))
        .collect()
}

fn mean_center(pixels: &[[f32; 3]], size: u32) -> [f32; 3] {
    let lo = size / 4;
    let hi = size - lo;
    let mut sum = [0.0; 3];
    let mut count = 0.0;
    for y in lo..hi {
        for x in lo..hi {
            let pixel = pixels[(y * size + x) as usize];
            for channel in 0..3 {
                sum[channel] += pixel[channel];
            }
            count += 1.0;
        }
    }
    sum.map(|value| value / count)
}

fn channel_is(color: [f32; 3], channel: usize) {
    let other = (0..3)
        .filter(|&index| index != channel)
        .map(|index| color[index])
        .fold(0.0, f32::max);
    assert!(
        color[channel] > 0.2 && color[channel] > 5.0 * other,
        "channel {channel} of {color:?} does not lead"
    );
}

fn agree(live: f32, traced: f32, what: &str) {
    let ratio = live / traced;
    assert!(
        (ratio - 1.0).abs() <= 0.1,
        "{what}: live {live} traced {traced} (ratio {ratio:.3})"
    );
}

fn grey(color: [f32; 3]) -> f32 {
    (color[0] + color[1] + color[2]) / 3.0
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_one_sided_wall_is_open_from_behind_in_both_renderers() {
    let wall = Arrays::quad([-2.5, -2.5, 0.0], [5.0, 0.0, 0.0], [0.0, 5.0, 0.0]);
    let panel = Arrays::quad([-3.5, 3.5, 4.0], [7.0, 0.0, 0.0], [0.0, -7.0, 0.0]);
    let meshes = [wall.mesh(), panel.mesh()];
    let materials = [emitter([1.2, 0.02, 0.02]), emitter([0.02, 1.2, 0.02])];
    let sun = Sun {
        direction: [0.0, 1.0, 0.0],
        color: [0.0; 3],
        intensity: 0.0,
    };
    let front = trace_look([0.0, 0.0, 3.0], [0.0, 0.0, -1.0], FOV_Y);
    let back = trace_look([0.0, 0.0, -3.0], [0.0, 0.0, 1.0], FOV_Y);
    let one_sided = [
        Placement {
            two_sided: false,
            ..Placement::new(0, identity_matrix(), 0)
        },
        Placement {
            two_sided: false,
            ..Placement::new(1, identity_matrix(), 1)
        },
    ];
    let both = [
        Placement {
            two_sided: true,
            ..Placement::new(0, identity_matrix(), 0)
        },
        one_sided[1],
    ];
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut frame = Frame::new(gpu, VIEW, VIEW).unwrap();
    frame.set_sky(SkySource::Hdr(black_sky()), true);
    let mut turns = Turns::default();
    let (front_traced, front_ms, back_traced, back_ms, open_traced, open_ms) = {
        let shoot = |placements: &[Placement<'_>], camera: Camera, turns: &mut Turns| {
            trace_color(
                &frame.gpu,
                &ViewTrace {
                    meshes: &meshes,
                    placements,
                    materials: &materials,
                    camera,
                    projection: Projection::Perspective,
                    sun,
                    size: VIEW,
                    samples: SAMPLES,
                },
                turns,
            )
        };
        let (front_traced, front_ms) = shoot(&one_sided, front, &mut turns);
        let (back_traced, back_ms) = shoot(&one_sided, back, &mut turns);
        let (open_traced, open_ms) = shoot(&both, back, &mut turns);
        (
            front_traced,
            front_ms,
            back_traced,
            back_ms,
            open_traced,
            open_ms,
        )
    };
    assert!(front_ms < 300.0 && back_ms < 300.0 && open_ms < 300.0);

    let wall_mesh = frame.upload_mesh(wall.data()).unwrap();
    let panel_mesh = frame.upload_mesh(panel.data()).unwrap();
    let mut wall_instance = LiveInstance::new(wall_mesh, identity_matrix(), 0, 1);
    wall_instance.two_sided = false;
    let mut panel_instance = LiveInstance::new(panel_mesh, identity_matrix(), 1, 2);
    panel_instance.two_sided = false;
    let live_sun = LiveSun {
        direction: [0.0, 1.0, 0.0],
        colour: [0.0; 3],
        intensity: 0.0,
    };
    let draw = |frame: &mut Frame, camera: LiveCamera, instances: &[LiveInstance]| {
        let scene = LiveScene {
            camera,
            time: 0.0,
            seed: 7,
            sun: live_sun,
            instances,
            materials: &materials,
            deformers: &[],
            wind: SceneWind::default(),
        };
        frame.render(&scene).unwrap();
        live_color(frame)
    };
    let front_live = draw(
        &mut frame,
        live_look([0.0, 0.0, 3.0], [0.0, 0.0, -1.0], FOV_Y),
        &[wall_instance, panel_instance],
    );
    let back_live = draw(
        &mut frame,
        live_look([0.0, 0.0, -3.0], [0.0, 0.0, 1.0], FOV_Y),
        &[wall_instance, panel_instance],
    );
    wall_instance.two_sided = true;
    let open_live = draw(
        &mut frame,
        live_look([0.0, 0.0, -3.0], [0.0, 0.0, 1.0], FOV_Y),
        &[wall_instance, panel_instance],
    );

    let front_t = mean_center(&front_traced, VIEW);
    let front_l = mean_center(&front_live, VIEW);
    let back_t = mean_center(&back_traced, VIEW);
    let back_l = mean_center(&back_live, VIEW);
    let open_t = mean_center(&open_traced, VIEW);
    let open_l = mean_center(&open_live, VIEW);
    println!("front live {front_l:?} traced {front_t:?}");
    println!("back live {back_l:?} traced {back_t:?}");
    println!("two-sided back live {open_l:?} traced {open_t:?}");
    channel_is(front_l, 0);
    channel_is(front_t, 0);
    channel_is(back_l, 1);
    channel_is(back_t, 1);
    channel_is(open_l, 0);
    channel_is(open_t, 0);
    agree(front_l[0], front_t[0], "front red");
    agree(back_l[1], back_t[1], "back green");
    agree(open_l[0], open_t[0], "two-sided back red");
}

fn floor_xz(px: u32, py: u32) -> (f32, f32) {
    let ndc_x = (px as f32 + 0.5) / FLOOR as f32 * 2.0 - 1.0;
    let ndc_y = (py as f32 + 0.5) / FLOOR as f32 * 2.0 - 1.0;
    let tan = (FLOOR_FOV.to_radians() * 0.5).tan();
    (FLOOR_HEIGHT * tan * ndc_x, FLOOR_HEIGHT * tan * ndc_y)
}

fn region(pixels: &[[f32; 3]], shadow: bool) -> f32 {
    let mut sum = 0.0;
    let mut count = 0.0;
    for y in 0..FLOOR {
        for x in 0..FLOOR {
            let (world_x, world_z) = floor_xz(x, y);
            if world_z.abs() > 0.3 {
                continue;
            }
            let inside = if shadow {
                (-1.2..-0.4).contains(&world_x)
            } else {
                (0.5..1.3).contains(&world_x)
            };
            if !inside {
                continue;
            }
            sum += grey(pixels[(y * FLOOR + x) as usize]);
            count += 1.0;
        }
    }
    assert!(count > 20.0, "region covers {count} pixels");
    sum / count
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_one_sided_wall_still_casts_across_its_back() {
    let floor = Arrays::quad([-2.0, 0.0, 2.0], [4.0, 0.0, 0.0], [0.0, 0.0, -4.0]);
    let wall = Arrays::quad([0.0, 0.0, -0.6], [0.0, 1.4, 0.0], [0.0, 0.0, 1.2]);
    let meshes = [floor.mesh(), wall.mesh()];
    let materials = [lambert([0.62, 0.62, 0.62]), lambert([0.62, 0.62, 0.62])];
    let direction = unit([1.0, 0.7, 0.0]);
    let sun = Sun {
        direction,
        color: [1.0; 3],
        intensity: 3.0,
    };
    let camera = Camera {
        origin: [0.0, FLOOR_HEIGHT, 0.0],
        forward: [0.0, -1.0, 0.0],
        right: {
            let tan = (FLOOR_FOV.to_radians() * 0.5).tan();
            [tan, 0.0, 0.0]
        },
        up: {
            let tan = (FLOOR_FOV.to_radians() * 0.5).tan();
            [0.0, 0.0, -tan]
        },
    };
    let placements = [
        Placement {
            two_sided: true,
            ..Placement::new(0, identity_matrix(), 0)
        },
        Placement {
            two_sided: false,
            ..Placement::new(1, identity_matrix(), 1)
        },
    ];
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut frame = Frame::new(gpu, FLOOR, FLOOR).unwrap();
    frame.set_sky(SkySource::Hdr(black_sky()), true);
    let mut turns = Turns::default();
    let (traced, longest) = trace_color(
        &frame.gpu,
        &ViewTrace {
            meshes: &meshes,
            placements: &placements,
            materials: &materials,
            camera,
            projection: Projection::Perspective,
            sun,
            size: FLOOR,
            samples: 16,
        },
        &mut turns,
    );
    assert!(longest < 300.0, "longest submission {longest:.2} ms");

    let floor_mesh = frame.upload_mesh(floor.data()).unwrap();
    let wall_mesh = frame.upload_mesh(wall.data()).unwrap();
    let mut floor_instance = LiveInstance::new(floor_mesh, identity_matrix(), 0, 1);
    floor_instance.two_sided = true;
    let wall_instance = LiveInstance::new(wall_mesh, identity_matrix(), 1, 2);
    let look_down: Matrix = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, -1.0, 0.0, 0.0],
        [0.0, 0.0, -FLOOR_HEIGHT, 1.0],
    ];
    let near = 0.1;
    let far = 12.0;
    let f = 1.0 / (FLOOR_FOV.to_radians() * 0.5).tan();
    let projection: Matrix = [
        [f, 0.0, 0.0, 0.0],
        [0.0, f, 0.0, 0.0],
        [0.0, 0.0, far / (near - far), -1.0],
        [0.0, 0.0, far * near / (near - far), 0.0],
    ];
    let scene = LiveScene {
        camera: LiveCamera {
            view: look_down,
            projection,
            previous_view_projection: multiply(projection, look_down),
            position: [0.0, FLOOR_HEIGHT, 0.0],
        },
        time: 0.0,
        seed: 5,
        sun: LiveSun {
            direction,
            colour: [1.0; 3],
            intensity: 3.0,
        },
        instances: &[floor_instance, wall_instance],
        materials: &materials,
        deformers: &[],
        wind: SceneWind::default(),
    };
    let mut shadows = Shadows::new(
        &frame.gpu.device,
        Quality {
            resolution: 1024,
            receiver: Some(ReceiverBox {
                min: [-2.2, -0.1, -2.2],
                max: [2.2, 1.6, 2.2],
            }),
            sun_radius_deg: 0.0,
            caster_margin: 2.0,
            ..Quality::default()
        },
    );
    let view = View {
        eye: [0.0, FLOOR_HEIGHT, 0.0],
        forward: [0.0, -1.0, 0.0],
        up: [0.0, 0.0, -1.0],
        fov_y: FLOOR_FOV.to_radians(),
        aspect: 1.0,
        near: 0.1,
        far: 12.0,
    };
    let fit = shadows.fit(&view, direction);
    let mut encoder = frame.gpu.device.create_command_encoder(&Default::default());
    frame
        .encode(&scene, &mut encoder, Some((&mut shadows, &fit)), None)
        .unwrap();
    frame.gpu.queue.submit(Some(encoder.finish()));
    let live = live_color(&frame);

    let traced_shadow = region(&traced, true);
    let traced_lit = region(&traced, false);
    let live_shadow = region(&live, true);
    let live_lit = region(&live, false);
    println!(
        "shadow live {live_shadow:.4} of lit {live_lit:.4}, traced {traced_shadow:.4} of lit {traced_lit:.4}"
    );
    assert!(traced_lit > 0.05 && live_lit > 0.05);
    assert!(
        traced_shadow < 0.45 * traced_lit,
        "traced shadow {traced_shadow} against lit {traced_lit}"
    );
    assert!(
        live_shadow < 0.7 * live_lit,
        "live shadow {live_shadow} against lit {live_lit}"
    );
}

fn carrier_pixels(
    gpu: &Gpu,
    floor: &Arrays,
    card: &Arrays,
    shadow_only: bool,
    casts_shadow: bool,
    card_material: u32,
    turns: &mut Turns,
) -> (Vec<[f32; 3]>, f64) {
    let meshes = [floor.mesh(), card.mesh()];
    let materials = [lambert([0.6, 0.6, 0.6]), lambert([0.6, 0.6, 0.6])];
    let placements = [
        Placement::new(0, identity_matrix(), 0),
        Placement {
            shadow_only,
            casts_shadow,
            ..Placement::new(1, identity_matrix(), card_material)
        },
    ];
    trace_color(
        gpu,
        &ViewTrace {
            meshes: &meshes,
            placements: &placements,
            materials: &materials,
            camera: Camera {
                origin: [0.0, 2.0, 0.0],
                forward: [0.0, -1.0, 0.0],
                right: [1.0, 0.0, 0.0],
                up: [0.0, 0.0, -1.0],
            },
            projection: Projection::Orthographic {
                width: 2.0,
                height: 2.0,
            },
            sun: Sun {
                direction: [0.0, 1.0, 0.0],
                color: [1.0; 3],
                intensity: 3.0,
            },
            size: VIEW,
            samples: 16,
        },
        turns,
    )
}

fn ortho_mean(pixels: &[[f32; 3]], inside: bool) -> f32 {
    let mut sum = 0.0;
    let mut count = 0.0;
    for y in 0..VIEW {
        for x in 0..VIEW {
            let ndc_x = (x as f32 + 0.5) / VIEW as f32 * 2.0 - 1.0;
            let ndc_y = (y as f32 + 0.5) / VIEW as f32 * 2.0 - 1.0;
            let reach = ndc_x.abs().max(ndc_y.abs());
            let take = if inside {
                reach < 0.12
            } else {
                (0.55..0.9).contains(&reach)
            };
            if !take {
                continue;
            }
            sum += grey(pixels[(y * VIEW + x) as usize]);
            count += 1.0;
        }
    }
    assert!(count > 20.0, "ortho region covers {count} pixels");
    sum / count
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_half_millimetre_carrier_casts_without_shadowing_itself() {
    let floor = Arrays::quad([-1.0, 0.0, 1.0], [2.0, 0.0, 0.0], [0.0, 0.0, -2.0]);
    let low = Arrays::quad([-0.25, 0.0005, 0.25], [0.5, 0.0, 0.0], [0.0, 0.0, -0.5]);
    let high = Arrays::quad([-0.25, 0.05, 0.25], [0.5, 0.0, 0.0], [0.0, 0.0, -0.5]);
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut turns = Turns::default();
    let (casting, casting_ms) = carrier_pixels(&gpu, &floor, &low, true, true, 1, &mut turns);
    let (clear, clear_ms) = carrier_pixels(&gpu, &floor, &low, true, false, 1, &mut turns);
    let (near, near_ms) = carrier_pixels(&gpu, &floor, &low, false, true, 0, &mut turns);
    let (far, far_ms) = carrier_pixels(&gpu, &floor, &high, false, true, 0, &mut turns);
    assert!(
        casting_ms < 300.0 && clear_ms < 300.0 && near_ms < 300.0 && far_ms < 300.0,
        "longest {casting_ms:.2} {clear_ms:.2} {near_ms:.2} {far_ms:.2} ms"
    );
    let under = ortho_mean(&casting, true);
    let open = ortho_mean(&casting, false);
    let clear_under = ortho_mean(&clear, true);
    let clear_open = ortho_mean(&clear, false);
    let near_top = ortho_mean(&near, true);
    let far_top = ortho_mean(&far, true);
    println!(
        "carrier under {under:.4} open {open:.4}, clear {clear_under:.4} of {clear_open:.4}, tops {near_top:.4} and {far_top:.4}"
    );
    assert!(open > 0.15, "open floor {open}");
    assert!(
        under < 0.45 * open,
        "casting quad leaks: {under} against {open}"
    );
    let clear_ratio = clear_under / clear_open;
    assert!(
        (clear_ratio - 1.0).abs() < 0.08,
        "a quad that casts nothing still shades the floor: {clear_under} against {clear_open}"
    );
    let top_ratio = near_top / far_top;
    assert!(
        near_top > 0.15 && (top_ratio - 1.0).abs() < 0.08,
        "the 0.5 mm quad shades itself: {near_top} against the quad at 5 cm {far_top}"
    );
}
