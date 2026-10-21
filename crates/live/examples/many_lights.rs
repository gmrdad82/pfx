use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

use pfx_gpu::pace::Turns;
use pfx_gpu::{Gpu, OffscreenTarget, PassTiming, wgpu};
use pfx_live::clusters::{Assignment, Cone, Sphere};
use pfx_live::renderer::{Exposure, Renderer};
use pfx_load::scene::Scene;

const WIDTH: u32 = 2160;
const HEIGHT: u32 = 3840;
const FRAMES: u32 = 240;
const COUNTS: [usize; 3] = [0, 16, 128];

fn street(lights: usize) -> String {
    let mut text = String::from(
        "format = 1

materials = [\"street.materials.toml\"]
fallback = \"asphalt\"

[mesh.block]
file = \"block.gltf\"

[[object]]
name = \"road\"
mesh = \"block\"
at = [0.0, -0.05, -50.0]
scale = [14.0, 0.1, 120.0]

[[object]]
name = \"car\"
mesh = \"block\"
at = [0.0, 0.7, -6.0]
scale = [1.9, 1.3, 4.4]
material = \"paint\"

[[mover]]
name = \"drive\"
objects = [\"car\"]
kind = \"slide\"
axis = [0.0, 0.0, -1.0]
travel = [0.0, 30.0]
period = 8.0

[sun]
model = \"authored\"
toward = [0.3, 0.2, -0.9]
color = [1.0, 0.8, 0.6]
irradiance = 1.0

[camera]
at = [0.0, 1.0, 2.0]
look_at = [0.0, 0.8, -20.0]
fov = 55.0
",
    );
    for side in [-1.0_f32, 1.0] {
        for block in 0..8 {
            let _ = write!(
                text,
                "\n[[object]]\nname = \"building {side} {block}\"\nmesh = \"block\"\nat = [{x}, 6.0, {z}]\nscale = [6.0, 12.0, 11.0]\nmaterial = \"wall\"\n",
                x = side * 10.0,
                z = -6.0 - block as f32 * 12.0,
            );
        }
    }
    let mut seed = 0x2545_f491_u32;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed as f32 / u32::MAX as f32
    };
    for index in 0..lights {
        let side = if index % 2 == 0 { -1.0 } else { 1.0 };
        let z = -((index / 2) as f32) * 120.0 / (lights / 2).max(1) as f32;
        if index % 4 < 2 {
            let _ = write!(
                text,
                "\n[[light]]\nname = \"lamp {index}\"\nkind = \"spot\"\nposition = [{x}, 5.5, {z}]\ndirection = [{d}, -1.0, 0.0]\ncone = [25.0, 45.0]\ncolor = [1.0, 0.75, 0.45]\nintensity = {power}\nrange = 12.0\nshadow = false\n",
                x = side * 6.0,
                d = -side * 0.3,
                power = 20.0 + next() * 10.0,
            );
        } else {
            let _ = write!(
                text,
                "\n[[light]]\nname = \"window {index}\"\nposition = [{x}, {y}, {z}]\ncolor = [{r}, 0.8, 0.7]\nintensity = {power}\nrange = 6.0\nshadow = false\n",
                x = side * 6.9,
                y = 1.0 + next() * 8.0,
                r = 0.6 + next() * 0.4,
                power = 2.0 + next() * 3.0,
            );
        }
    }
    text
}

fn folder(lights: usize) -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tmp/many-lights")
        .join(lights.to_string());
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../load/tests/scenes/block.gltf"),
        root.join("block.gltf"),
    )
    .unwrap();
    std::fs::write(
        root.join("street.materials.toml"),
        "format = 1\n\n[materials.asphalt]\nbase = [0.08, 0.08, 0.09]\nroughness = 0.6\n\n[materials.wall]\nbase = [0.5, 0.45, 0.4]\nroughness = 0.9\n\n[materials.paint]\nbase = [0.6, 0.05, 0.04]\nroughness = 0.3\n",
    )
    .unwrap();
    std::fs::write(root.join("street.scene.toml"), street(lights)).unwrap();
    root
}

fn output(gpu: &Gpu) -> OffscreenTarget {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("many lights output"),
        size: wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::STORAGE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    OffscreenTarget {
        texture,
        view,
        format: wgpu::TextureFormat::Rgba16Float,
        width: WIDTH,
        height: HEIGHT,
    }
}

fn pass(timings: &[PassTiming], label: &str) -> Option<f64> {
    timings
        .iter()
        .find(|timing| timing.label == label)
        .map(|timing| timing.milliseconds)
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    values.get(values.len() / 2).copied().unwrap_or(f64::NAN)
}

fn main() {
    let aspect = WIDTH as f32 / HEIGHT as f32;
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let output = output(&gpu);
    let mut renderer = Renderer::new(gpu, WIDTH, HEIGHT).unwrap();
    renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
    let mut turns = Turns::default();
    for count in COUNTS {
        let scene = Scene::open(folder(count).join("street.scene.toml")).unwrap();
        let mut staged = renderer.stage(&scene).unwrap();
        let mut opaque = Vec::new();
        let mut total = Vec::new();
        for frame in 0..FRAMES + 30 {
            let finish = staged.finish();
            let drawn = staged.frame(aspect, frame as f32 / 60.0, 7);
            turns.poll();
            let timings = renderer
                .render(
                    &drawn.scene,
                    &drawn.text,
                    &drawn.effects,
                    finish,
                    &output.view,
                )
                .unwrap();
            turns.add(pfx_gpu::pace::gpu_ms(&timings).unwrap_or(f64::NAN));
            turns.turn();
            if frame >= 30 {
                opaque.extend(pass(&timings, "opaque PBR"));
                total.push(timings.iter().map(|timing| timing.milliseconds).sum());
            }
        }
        let camera = scene.camera_or_default();
        let view = camera.view();
        let projection = camera.projection(aspect);
        let spheres: Vec<Sphere> = scene
            .lights
            .values()
            .map(|light| Sphere {
                centre: light.position,
                radius: light.range,
                cone: light.spot.map(|spot| Cone {
                    direction: spot.direction,
                    outer: spot.outer.to_radians(),
                }),
            })
            .collect();
        let mut assignment = Assignment::default();
        let start = Instant::now();
        for _ in 0..FRAMES {
            assignment.build(&view, &projection, &spheres);
        }
        let build_us = start.elapsed().as_secs_f64() * 1e6 / f64::from(FRAMES);
        let pairs: u32 = assignment.table.iter().map(|entry| entry[1]).sum();
        println!(
            "{count} lights at {WIDTH}x{HEIGHT}: opaque p50 {:.3} ms, timed passes p50 {:.3} ms, cluster build {build_us:.1} us on the CPU, {pairs} light-cluster pairs over {} clusters",
            median(&mut opaque),
            median(&mut total),
            assignment.clusters(),
        );
    }
}
