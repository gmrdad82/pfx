#[path = "support/output.rs"]
mod output;

use output::output;

use pfx_gpu::{Gpu, OffscreenTarget, wgpu};
use pfx_live::frame::{
    Camera, Instance, Matrix, MeshData, MeshHandle, Scene, SceneWind, Sun, jitter_matrix, multiply,
    transform,
};
use pfx_live::morph::{InstanceWeights, MorphData, MorphTargetData};
use pfx_live::renderer::{Effects, Exposure, Finish, Renderer, Text};
use pfx_live::taa;
use pfx_materials::Material;

const WIDTH: u32 = 64;
const HEIGHT: u32 = 48;
const NEAR: f32 = 0.5;
const FAR: f32 = 20.0;
const SLOPE: f32 = 0.8;
const PLANE: f32 = 4.0;
const WEIGHTS: [f32; 2] = [0.7, 0.35];

const IDENTITY: Matrix = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

fn projection() -> Matrix {
    let f = 1.0 / 25f32.to_radians().tan();
    let aspect = WIDTH as f32 / HEIGHT as f32;
    [
        [f / aspect, 0.0, 0.0, 0.0],
        [0.0, f, 0.0, 0.0],
        [0.0, 0.0, FAR / (NEAR - FAR), -1.0],
        [0.0, 0.0, FAR * NEAR / (NEAR - FAR), 0.0],
    ]
}

fn translation(by: [f32; 3]) -> Matrix {
    let mut matrix = IDENTITY;
    matrix[3] = [by[0], by[1], by[2], 1.0];
    matrix
}

fn scene<'a>(instances: &'a [Instance], materials: &'a [Material], number: u32) -> Scene<'a> {
    Scene {
        camera: Camera {
            view: IDENTITY,
            projection: projection(),
            previous_view_projection: projection(),
            position: [0.0; 3],
        },
        time: number as f32 / 60.0,
        seed: 3,
        sun: Sun {
            direction: [0.3, 0.8, 0.5],
            colour: [1.0; 3],
            intensity: 1.0,
        },
        instances,
        materials,
        deformers: &[],
        wind: SceneWind::default(),
    }
}

struct Grid {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    tangents: Vec<[f32; 4]>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
    lift: Vec<[f32; 3]>,
    slide: Vec<[f32; 3]>,
}

fn grid(side: u32, extent: f32, height: impl Fn(f32) -> f32) -> Grid {
    let mut grid = Grid {
        positions: Vec::new(),
        normals: Vec::new(),
        tangents: Vec::new(),
        uvs: Vec::new(),
        indices: Vec::new(),
        lift: Vec::new(),
        slide: Vec::new(),
    };
    for row in 0..=side {
        for column in 0..=side {
            let u = column as f32 / side as f32;
            let v = row as f32 / side as f32;
            let x = (u * 2.0 - 1.0) * extent;
            let y = (v * 2.0 - 1.0) * extent;
            grid.positions.push([x, y, height(x)]);
            grid.normals.push([0.0, 0.0, 1.0]);
            grid.tangents.push([1.0, 0.0, 0.0, 1.0]);
            grid.uvs.push([u, v]);
            grid.lift.push([0.0, 0.0, 0.1 + 0.2 * u * v]);
            grid.slide.push([0.05 * v, -0.03 * u, 0.0]);
        }
    }
    let stride = side + 1;
    for row in 0..side {
        for column in 0..side {
            let a = row * stride + column;
            grid.indices
                .extend([a, a + 1, a + stride, a + 1, a + stride + 1, a + stride]);
        }
    }
    grid
}

fn data(grid: &Grid) -> MeshData<'_> {
    MeshData {
        positions: &grid.positions,
        normals: &grid.normals,
        tangents: &grid.tangents,
        uvs: &grid.uvs,
        uvs1: None,
        alpha: None,
        indices: &grid.indices,
    }
}

fn renderer() -> (Renderer, OffscreenTarget) {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let target = output(&gpu, WIDTH, HEIGHT);
    let mut renderer = Renderer::new(gpu, WIDTH, HEIGHT).unwrap();
    renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
    renderer.wait_sky().unwrap();
    (renderer, target)
}

fn draw(
    renderer: &mut Renderer,
    target: &OffscreenTarget,
    instances: &[Instance],
    materials: &[Material],
    number: u32,
) {
    renderer
        .render(
            &scene(instances, materials, number),
            &Text::default(),
            &Effects::default(),
            Finish::Standard,
            &target.view,
        )
        .unwrap();
}

fn read_buffer(gpu: &Gpu, source: &wgpu::Buffer, offset: u64, size: u64) -> Vec<f32> {
    let staging = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("overlay test readback"),
        size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(source, offset, &staging, 0, size);
    gpu.queue.submit(Some(encoder.finish()));
    staging.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    let values = bytemuck::cast_slice(&staging.slice(..).get_mapped_range().unwrap()).to_vec();
    staging.unmap();
    values
}

fn read_depth(gpu: &Gpu, view: &wgpu::TextureView) -> Vec<f32> {
    let device = &gpu.device;
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("overlay test depth taps"),
        source: wgpu::ShaderSource::Wgsl(
            format!(
                "@group(0) @binding(0) var depth: texture_depth_2d;
@group(0) @binding(1) var<storage, read_write> taps: array<f32>;
@compute @workgroup_size(8, 8) fn main(@builtin(global_invocation_id) id: vec3u) {{
    if (id.x >= {WIDTH}u || id.y >= {HEIGHT}u) {{ return; }}
    taps[id.y * {WIDTH}u + id.x] = textureLoad(depth, vec2i(id.xy), 0);
}}"
            )
            .into(),
        ),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("overlay test depth taps"),
        layout: None,
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let size = u64::from(WIDTH * HEIGHT) * 4;
    let taps = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("overlay test depth taps"),
        size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: taps.as_entire_binding(),
            },
        ],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(WIDTH.div_ceil(8), HEIGHT.div_ceil(8), 1);
    }
    gpu.queue.submit(Some(encoder.finish()));
    read_buffer(gpu, &taps, 0, size)
}

fn plane_depth(projection: Matrix, ndc: [f32; 2]) -> f32 {
    let a = ndc[0] / projection[0][0];
    let t = PLANE / (1.0 - SLOPE * a);
    let z = -t;
    (projection[2][2] * z + projection[3][2]) / -z
}

fn largest_error(depth: &[f32], projection: Matrix, shift: [f32; 2]) -> f32 {
    let mut largest = 0.0f32;
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let ndc = [
                (x as f32 + 0.5) / WIDTH as f32 * 2.0 - 1.0 - shift[0],
                1.0 - (y as f32 + 0.5) / HEIGHT as f32 * 2.0 - shift[1],
            ];
            let expected = plane_depth(projection, ndc);
            largest = largest.max((depth[(y * WIDTH + x) as usize] - expected).abs());
        }
    }
    largest
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn last_jitter_is_the_projections_shift_and_depth_view_holds_that_frames_depth() {
    let (mut renderer, target) = renderer();
    let plane = grid(1, 12.0, |x| -PLANE - SLOPE * x);
    let mesh = renderer.upload_mesh(data(&plane)).unwrap();
    let instances = [Instance::new(mesh, IDENTITY, 0, 1)];
    let materials = [Material::default()];
    let number = (0..16)
        .find(|&n| taa::jitter_pixels(n)[0].abs() > 0.2)
        .unwrap();
    draw(&mut renderer, &target, &instances, &materials, number);
    let jitter = renderer.last_jitter();
    let expected = taa::jitter_clip(number, WIDTH, HEIGHT);
    assert_eq!(jitter.map(f32::to_bits), expected.map(f32::to_bits));
    let point = [0.4, -0.3, -2.5, 1.0];
    let shifted = transform(multiply(jitter_matrix(jitter), projection()), point);
    let plain = transform(projection(), point);
    for axis in 0..2 {
        let moved = shifted[axis] / shifted[3] - plain[axis] / plain[3];
        assert!((moved - jitter[axis]).abs() < 1e-6, "ndc shift {moved}");
    }
    assert_eq!(shifted[2].to_bits(), plain[2].to_bits());
    let depth = read_depth(renderer.gpu(), renderer.depth_view());
    let jittered = largest_error(&depth, projection(), jitter);
    let unjittered = largest_error(&depth, projection(), [0.0; 2]);
    println!("depth error: jittered {jittered:e}, unjittered {unjittered:e}");
    assert!(jittered < 2e-5, "jittered error {jittered}");
    assert!(
        unjittered > 10.0 * jittered,
        "unjittered error {unjittered}"
    );
    let geometry = renderer.readback_geometry().unwrap();
    assert_eq!(
        geometry
            .depth
            .iter()
            .map(|d| d.to_bits())
            .collect::<Vec<_>>(),
        depth.iter().map(|d| d.to_bits()).collect::<Vec<_>>()
    );
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn frames_drawn_again_at_one_time_step_the_jitter_and_a_new_time_starts_from_it() {
    let (mut renderer, target) = renderer();
    let plane = grid(1, 12.0, |x| -PLANE - SLOPE * x);
    let mesh = renderer.upload_mesh(data(&plane)).unwrap();
    let instances = [Instance::new(mesh, IDENTITY, 0, 1)];
    let materials = [Material::default()];
    let mut seen = Vec::new();
    for number in [5, 5, 5, 9] {
        draw(&mut renderer, &target, &instances, &materials, number);
        seen.push(renderer.last_jitter().map(f32::to_bits));
    }
    let expected: Vec<[u32; 2]> = [5, 6, 7, 9]
        .map(|number| taa::jitter_clip(number, WIDTH, HEIGHT).map(f32::to_bits))
        .to_vec();
    assert_eq!(seen, expected);
}

fn morphed(renderer: &mut Renderer, head: &Grid) -> MeshHandle {
    let targets = [
        MorphTargetData {
            positions: &head.lift,
            normals: None,
        },
        MorphTargetData {
            positions: &head.slide,
            normals: None,
        },
    ];
    renderer
        .upload_morphed_mesh(data(head), None, MorphData { targets: &targets })
        .unwrap()
}

struct Shot {
    renderer: Renderer,
    target: OffscreenTarget,
    instances: Vec<Instance>,
    head: Grid,
    model: Matrix,
}

fn shot(expose: bool) -> Shot {
    let (mut renderer, target) = renderer();
    let floor = grid(1, 12.0, |_| -6.0);
    let head = grid(6, 0.6, |_| 0.0);
    let floor_mesh = renderer.upload_mesh(data(&floor)).unwrap();
    let head_mesh = morphed(&mut renderer, &head);
    let model = translation([0.2, -0.1, -3.0]);
    let instances = vec![
        Instance::new(floor_mesh, IDENTITY, 0, 1),
        Instance::new(head_mesh, model, 0, 2),
    ];
    renderer
        .set_weights(&[InstanceWeights {
            instance: 1,
            current: &WEIGHTS,
            previous: &WEIGHTS,
        }])
        .unwrap();
    if expose {
        renderer.set_deformed_positions(&[0, 1]);
    }
    let materials = [Material::default()];
    for number in 0..4 {
        draw(&mut renderer, &target, &instances, &materials, number);
    }
    Shot {
        renderer,
        target,
        instances,
        head,
        model,
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_morphed_instances_exposed_positions_are_its_cpu_blend_and_leave_the_frame_unchanged() {
    let plain = shot(false);
    assert!(plain.renderer.deformed_positions(1).is_none());
    let exposed = shot(true);
    assert_eq!(exposed.instances.len(), 2);
    assert!(exposed.renderer.deformed_positions(0).is_none());
    let deformed = exposed.renderer.deformed_positions(1).unwrap();
    let head = &exposed.head;
    let model = exposed.model;
    assert_eq!(deformed.vertices as usize, head.positions.len());
    let world = read_buffer(
        exposed.renderer.gpu(),
        deformed.buffer,
        deformed.offset,
        deformed.size(),
    );
    let mut largest = 0.0f32;
    for (vertex, rest) in head.positions.iter().enumerate() {
        let local: [f32; 3] = std::array::from_fn(|c| {
            rest[c] + head.lift[vertex][c] * WEIGHTS[0] + head.slide[vertex][c] * WEIGHTS[1]
        });
        let expected: [f32; 4] = std::array::from_fn(|row| {
            model[0][row] * local[0]
                + model[1][row] * local[1]
                + model[2][row] * local[2]
                + model[3][row]
        });
        for c in 0..4 {
            largest = largest.max((world[vertex * 4 + c] - expected[c]).abs());
        }
    }
    assert!(largest < 1e-5, "deformed positions differ by {largest}");
    let moved = (0..head.positions.len()).any(|vertex| {
        (world[vertex * 4 + 2] - (model[3][2] + head.positions[vertex][2])).abs() > 0.05
    });
    assert!(moved, "the blend moved the head off its rest pose");
    assert_eq!(
        plain.renderer.gpu().readback_rgba16(&plain.target).unwrap(),
        exposed
            .renderer
            .gpu()
            .readback_rgba16(&exposed.target)
            .unwrap(),
        "exposing positions changed the frame"
    );
}
