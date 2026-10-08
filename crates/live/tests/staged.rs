use pfx_gpu::Gpu;
use pfx_gpu::wgpu;
use pfx_live::frame::{Camera, Frame, Instance, Matrix, MeshData, Scene, SceneWind, Sun, multiply};
use pfx_live::maps::ContentFormat;
use pfx_live::sky::SkySource;
use pfx_load::Sky;
use pfx_materials::{Content, ContentLayer, Material};
use pfx_trace::detail::Lens;
use pfx_trace::stage::{Image, Mesh, Placement, PlacementContent, Stage};
use pfx_trace::{Camera as TraceCamera, Projection, Sun as TraceSun};

const WIDTH: u32 = 160;
const HEIGHT: u32 = 96;
const FOV_Y: f32 = 36.0;
const NEAR: f32 = 0.05;
const FAR: f32 = 30.0;
const EYE: [f32; 3] = [0.0, 0.6, 2.4];
const TARGET: [f32; 3] = [0.0, 0.0, -0.2];
const SCREEN: [u32; 2] = [48, 32];

struct Arrays {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    tangents: Vec<[f32; 4]>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
}

impl Arrays {
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

    fn sphere(rings: u32, segments: u32) -> Self {
        let mut positions = Vec::new();
        let mut uvs = Vec::new();
        for ring in 0..=rings {
            let theta = std::f32::consts::PI * ring as f32 / rings as f32;
            for segment in 0..=segments {
                let phi = std::f32::consts::TAU * segment as f32 / segments as f32;
                positions.push([
                    theta.sin() * phi.cos(),
                    theta.cos(),
                    theta.sin() * phi.sin(),
                ]);
                uvs.push([segment as f32 / segments as f32, ring as f32 / rings as f32]);
            }
        }
        let mut indices = Vec::new();
        let row = segments + 1;
        for ring in 0..rings {
            for segment in 0..segments {
                let a = ring * row + segment;
                let b = a + row;
                indices.extend([a, a + 1, b, a + 1, b + 1, b]);
            }
        }
        Self {
            normals: positions.clone(),
            tangents: vec![[1.0, 0.0, 0.0, 1.0]; positions.len()],
            positions,
            uvs,
            indices,
        }
    }

    fn cube() -> Self {
        let mut arrays = Self {
            positions: Vec::new(),
            normals: Vec::new(),
            tangents: Vec::new(),
            uvs: Vec::new(),
            indices: Vec::new(),
        };
        for axis in 0..3 {
            for sign in [-1.0_f32, 1.0] {
                let mut normal = [0.0; 3];
                normal[axis] = sign;
                let u_axis = (axis + 1) % 3;
                let v_axis = (axis + 2) % 3;
                let start = arrays.positions.len() as u32;
                for (a, b) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                    let mut p = normal;
                    p[u_axis] = a;
                    p[v_axis] = b;
                    arrays.positions.push(p.map(|v| v * 0.5));
                    arrays.normals.push(normal);
                    arrays.tangents.push([1.0, 0.0, 0.0, 1.0]);
                    arrays.uvs.push([(a + 1.0) * 0.5, (b + 1.0) * 0.5]);
                }
                let quad = if sign > 0.0 {
                    [0, 1, 2, 0, 2, 3]
                } else {
                    [0, 2, 1, 0, 3, 2]
                };
                arrays.indices.extend(quad.map(|k| start + k));
            }
        }
        arrays
    }

    fn panel() -> Self {
        Self {
            positions: vec![
                [-0.5, -0.5, 0.0],
                [0.5, -0.5, 0.0],
                [0.5, 0.5, 0.0],
                [-0.5, 0.5, 0.0],
            ],
            normals: vec![[0.0, 0.0, 1.0]; 4],
            tangents: vec![[1.0, 0.0, 0.0, 1.0]; 4],
            uvs: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            indices: vec![0, 1, 2, 0, 2, 3],
        }
    }
}

fn staged_mesh<'a>(data: &MeshData<'a>) -> Mesh<'a> {
    Mesh {
        positions: data.positions,
        normals: data.normals,
        tangents: data.tangents,
        uvs: data.uvs,
        alpha: data.alpha,
        indices: data.indices,
    }
}

fn unit(v: [f32; 3]) -> [f32; 3] {
    let length = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    v.map(|x| x / length)
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a.iter().zip(b).map(|(a, b)| a * b).sum()
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn axes() -> ([f32; 3], [f32; 3], [f32; 3]) {
    let forward = unit(std::array::from_fn(|k| TARGET[k] - EYE[k]));
    let right = unit(cross(forward, [0.0, 1.0, 0.0]));
    let up = cross(right, forward);
    (forward, right, up)
}

fn live_camera() -> Camera {
    let (forward, right, up) = axes();
    let view: Matrix = [
        [right[0], up[0], -forward[0], 0.0],
        [right[1], up[1], -forward[1], 0.0],
        [right[2], up[2], -forward[2], 0.0],
        [-dot(right, EYE), -dot(up, EYE), dot(forward, EYE), 1.0],
    ];
    let tan = (FOV_Y.to_radians() * 0.5).tan();
    let aspect = WIDTH as f32 / HEIGHT as f32;
    let projection: Matrix = [
        [1.0 / (tan * aspect), 0.0, 0.0, 0.0],
        [0.0, 1.0 / tan, 0.0, 0.0],
        [0.0, 0.0, FAR / (NEAR - FAR), -1.0],
        [0.0, 0.0, FAR * NEAR / (NEAR - FAR), 0.0],
    ];
    Camera {
        view,
        projection,
        previous_view_projection: multiply(projection, view),
        position: EYE,
    }
}

fn trace_camera() -> TraceCamera {
    let (forward, right, up) = axes();
    let tan = (FOV_Y.to_radians() * 0.5).tan();
    let aspect = WIDTH as f32 / HEIGHT as f32;
    TraceCamera {
        origin: EYE,
        forward,
        right: right.map(|v| v * tan * aspect),
        up: up.map(|v| v * tan),
    }
}

fn place(scale: f32, turn: f32, at: [f32; 3]) -> Matrix {
    let (s, c) = turn.sin_cos();
    [
        [c * scale, 0.0, -s * scale, 0.0],
        [0.0, scale, 0.0, 0.0],
        [s * scale, 0.0, c * scale, 0.0],
        [at[0], at[1], at[2], 1.0],
    ]
}

fn slab(size: [f32; 3], at: [f32; 3]) -> Matrix {
    [
        [size[0], 0.0, 0.0, 0.0],
        [0.0, size[1], 0.0, 0.0],
        [0.0, 0.0, size[2], 0.0],
        [at[0], at[1], at[2], 1.0],
    ]
}

fn screen_texels() -> Vec<[u8; 4]> {
    let [width, height] = SCREEN;
    (0..width * height)
        .map(|index| {
            let x = index % width;
            let y = index / width;
            let check = if (x / 6 + y / 6) % 2 == 0 { 220 } else { 60 };
            [
                (40 + x * 200 / width) as u8,
                check,
                (230 - y * 180 / height) as u8,
                255,
            ]
        })
        .collect()
}

fn lambert(base: [f32; 3]) -> Material {
    Material {
        base,
        roughness: 1.0,
        specular: 0.0,
        ..Material::default()
    }
}

fn read_ids(frame: &Frame) -> Vec<u32> {
    let texture = &frame.targets.ids;
    let size = texture.size();
    let row = (size.width * 4).div_ceil(256) * 256;
    let buffer = frame.gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("staged ids readback"),
        size: u64::from(row * size.height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = frame.gpu.device.create_command_encoder(&Default::default());
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
    frame.gpu.queue.submit(Some(encoder.finish()));
    buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    frame
        .gpu
        .device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    let mapped = buffer.slice(..).get_mapped_range();
    let mut ids = Vec::new();
    for y in 0..size.height {
        let start = (y * row) as usize;
        ids.extend(
            mapped[start..start + (size.width * 4) as usize]
                .chunks_exact(4)
                .map(|b| u32::from_le_bytes(b.try_into().unwrap())),
        );
    }
    drop(mapped);
    buffer.unmap();
    ids
}

fn mean(image: &[[f32; 3]], ids: &[u32], id: u32) -> [f64; 3] {
    let width = WIDTH as i32;
    let height = HEIGHT as i32;
    let mut sum = [0.0_f64; 3];
    let mut count = 0.0;
    for y in 1..height - 1 {
        for x in 1..width - 1 {
            let inside = (-1..=1)
                .all(|dy| (-1..=1).all(|dx| ids[((y + dy) * width + x + dx) as usize] == id));
            if inside {
                let pixel = image[(y * width + x) as usize];
                for c in 0..3 {
                    sum[c] += f64::from(pixel[c]);
                }
                count += 1.0;
            }
        }
    }
    assert!(count > 100.0, "instance {id} covers {count} pixels");
    sum.map(|s| s / count)
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn one_staged_scene_draws_alike_in_the_live_renderer_and_the_tracer() {
    let sphere = Arrays::sphere(48, 96);
    let cube = Arrays::cube();
    let panel = Arrays::panel();
    let arrays = [&sphere, &cube, &panel];
    let materials = [
        lambert([0.8, 0.25, 0.2]),
        lambert([0.25, 0.7, 0.3]),
        Material {
            base: [0.0; 3],
            specular: 0.0,
            emission: [0.3, 0.5, 1.4],
            ..Material::default()
        },
        Material {
            content_layer: ContentLayer::from_kind(
                Content::Screen,
                0.0,
                0,
                &pfx_materials::ContentLook {
                    screen_gain: 1.5,
                    ..Default::default()
                },
            ),
            ..lambert([0.02; 3])
        },
    ];
    let placements = [
        (0, place(0.42, 0.0, [-0.85, 0.0, -0.3]), 0, 1),
        (1, place(0.6, 0.6, [0.85, -0.05, -0.3]), 1, 2),
        (2, place(0.7, 0.0, [0.0, 0.25, -2.6]), 2, 3),
        (1, slab([0.8, 0.5, 0.04], [0.0, -0.35, -1.6]), 3, 4),
    ];
    let screen = screen_texels();
    let screen_bytes: Vec<u8> = screen.iter().flatten().copied().collect();
    let sun_direction = unit([0.35, 0.8, 0.6]);
    let black = Sky {
        width: 1,
        height: 1,
        texels: vec![[0.0, 0.0, 0.0, 1.0]],
    };

    let meshes: Vec<MeshData<'_>> = arrays.iter().map(|a| a.data()).collect();
    let staged_meshes: Vec<Mesh<'_>> = meshes.iter().map(staged_mesh).collect();
    let instances: Vec<Placement> = placements
        .iter()
        .map(|&(mesh, model, material, _)| Placement {
            content: (material == 3).then(|| {
                PlacementContent::new(Image {
                    width: SCREEN[0],
                    height: SCREEN[1],
                    texels: &screen,
                    srgb: true,
                })
            }),
            ..Placement::new(mesh, model, material)
        })
        .collect();
    let staged = Stage {
        meshes: &staged_meshes,
        instances: &instances,
        materials: &materials,
        sky: black.clone(),
        sun: TraceSun {
            direction: sun_direction,
            color: [1.0; 3],
            intensity: 3.0,
        },
        camera: trace_camera(),
        projection: Projection::Perspective,
        lens: Lens::default(),
    }
    .build()
    .unwrap();

    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut trace = staged.trace(&gpu, WIDTH, HEIGHT).unwrap();
    for pass in 0..64 {
        trace.sample(&gpu, 4, 11 + pass).unwrap();
    }
    let traced: Vec<[f32; 3]> = trace
        .readback(&gpu)
        .unwrap()
        .color
        .chunks_exact(16)
        .map(|p| {
            std::array::from_fn(|c| f32::from_le_bytes(p[c * 4..c * 4 + 4].try_into().unwrap()))
        })
        .collect();

    let mut frame = Frame::new(gpu, WIDTH, HEIGHT).unwrap();
    frame.set_sky(SkySource::Hdr(black), true);
    frame
        .set_content(0, SCREEN[0], SCREEN[1], ContentFormat::Srgb8)
        .unwrap();
    frame
        .write_content(0, [0, 0, SCREEN[0], SCREEN[1]], &screen_bytes)
        .unwrap();
    let handles: Vec<_> = arrays
        .iter()
        .map(|a| frame.upload_mesh(a.data()).unwrap())
        .collect();
    let live_instances: Vec<Instance> = placements
        .iter()
        .map(|&(mesh, model, material, id)| {
            Instance::new(handles[mesh as usize], model, material, id)
        })
        .collect();
    let scene = Scene {
        camera: live_camera(),
        time: 0.0,
        seed: 11,
        sun: Sun {
            direction: sun_direction,
            colour: [1.0; 3],
            intensity: 3.0,
        },
        instances: &live_instances,
        materials: &materials,
        deformers: &[],
        wind: SceneWind::default(),
    };
    frame.render(&scene).unwrap();
    let live: Vec<[f32; 3]> = frame
        .gpu
        .readback_rgba16(&frame.targets.hdr)
        .unwrap()
        .chunks_exact(4)
        .map(|p| std::array::from_fn(|c| half::f16::from_bits(p[c]).to_f32()))
        .collect();
    let ids = read_ids(&frame);

    let means: Vec<(u32, [f64; 3], [f64; 3])> = placements
        .iter()
        .map(|&(_, _, _, id)| (id, mean(&live, &ids, id), mean(&traced, &ids, id)))
        .collect();
    for (id, ours, reference) in &means {
        println!("instance {id}: live {ours:?}, traced {reference:?}");
    }
    for (id, ours, reference) in means {
        for c in 0..3 {
            let ratio = ours[c] / reference[c];
            assert!(
                (ratio - 1.0).abs() <= 0.05,
                "instance {id} channel {c}: live {} against traced {} (ratio {ratio:.4})",
                ours[c],
                reference[c]
            );
        }
    }
}
