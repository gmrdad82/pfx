#[path = "support/output.rs"]
mod output;

use output::output;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use pfx_core::anim::{Animator, Pose};
use pfx_gpu::pace::{Pacer, Turns};
use pfx_gpu::{Gpu, wgpu};
use pfx_live::frame::Matrix;
use pfx_live::renderer::{Exposure, Renderer};
use pfx_live::skin::InstancePose;
use pfx_load::scene::{Posed, Scene};

const WIDTH: u32 = 192;
const HEIGHT: u32 = 160;

const SCENE: &str = r#"format = 1

[mesh.column]
file = "column.glb"
id = "ptrcm00001"

[[object]]
name = "post"
mesh = "column"
id = "ptrps00001"

[sun]
model = "authored"
toward = [-0.4, 0.8, 0.45]
color = [1.0, 1.0, 1.0]
irradiance = 2.0

[camera]
at = [0.0, 1.5, 5.0]
look_at = [0.0, 1.5, 0.0]
fov = 45.0
"#;

fn folder() -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("posed-trace");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("column.glb"),
        pfx_load::fixture::skinned_column(3),
    )
    .unwrap();
    std::fs::write(root.join("post.scene.toml"), SCENE).unwrap();
    root
}

fn read_ids(gpu: &Gpu, texture: &wgpu::Texture) -> Vec<u32> {
    let size = texture.size();
    let raw = size.width * 4;
    let row = raw.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("posed trace ids"),
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
    mapped
        .chunks_exact(row as usize)
        .flat_map(|padded| {
            padded[..raw as usize]
                .chunks_exact(4)
                .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .collect::<Vec<_>>()
        })
        .collect()
}

fn live_mask(gpu: Gpu, scene: &Scene, palette: Option<&[Matrix]>) -> Vec<bool> {
    let target = output(&gpu, WIDTH, HEIGHT);
    let mut turns = Turns::default();
    let mut renderer = Renderer::new(gpu, WIDTH, HEIGHT).unwrap();
    renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
    let mut staged = renderer.stage(scene).unwrap();
    renderer.wait_sky().unwrap();
    if let Some(palette) = palette {
        renderer
            .set_poses(&[InstancePose {
                instance: 0,
                joints: palette,
                previous: palette,
            }])
            .unwrap();
    }
    let aspect = WIDTH as f32 / HEIGHT as f32;
    let finish = staged.finish();
    let drawn = staged.frame(aspect, 0.0, 7);
    let timings = renderer
        .render(
            &drawn.scene,
            &drawn.text,
            &drawn.effects,
            finish,
            &target.view,
        )
        .unwrap();
    turns.add(pfx_gpu::pace::gpu_ms(&timings).unwrap_or(f64::NAN));
    turns.turn();
    let id = staged.instances()[0].id;
    read_ids(renderer.gpu(), &renderer.frame().targets.ids)
        .into_iter()
        .map(|found| found == id)
        .collect()
}

fn traced_mask(gpu: &Gpu, scene: &Scene, posed: &Posed) -> Vec<bool> {
    let staged = pfx_trace::stage::scene_posed(scene, WIDTH as f32 / HEIGHT as f32, posed).unwrap();
    let mut trace = staged.trace(gpu, WIDTH, HEIGHT).unwrap();
    let mut pacer = Pacer::default();
    let mut turns = Turns::default();
    for pass in 0..4 {
        trace
            .sample_paced(gpu, 1, 11 + pass, &mut pacer, |ms| turns.add(ms))
            .unwrap();
    }
    turns.turn();
    trace
        .readback(gpu)
        .unwrap()
        .albedo
        .chunks_exact(16)
        .map(|p| (0..3).any(|c| f32::from_le_bytes(p[c * 4..c * 4 + 4].try_into().unwrap()) > 0.0))
        .collect()
}

fn iou(a: &[bool], b: &[bool]) -> f64 {
    let both = a.iter().zip(b).filter(|(a, b)| **a && **b).count();
    let either = a.iter().zip(b).filter(|(a, b)| **a || **b).count();
    both as f64 / either.max(1) as f64
}

fn within_a_pixel(from: &[bool], to: &[bool]) -> f64 {
    let (width, height) = (WIDTH as i64, HEIGHT as i64);
    let near = |x: i64, y: i64| {
        (-1..=1).any(|dy| {
            (-1..=1).any(|dx| {
                let (nx, ny) = (x + dx, y + dy);
                nx >= 0 && ny >= 0 && nx < width && ny < height && to[(ny * width + nx) as usize]
            })
        })
    };
    let mut kept = 0usize;
    let mut total = 0usize;
    for y in 0..height {
        for x in 0..width {
            if from[(y * width + x) as usize] {
                total += 1;
                kept += usize::from(near(x, y));
            }
        }
    }
    kept as f64 / total.max(1) as f64
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_traced_snapshot_shows_the_live_pose() {
    let root = folder();
    let scene = Scene::open(root.join("post.scene.toml")).unwrap();
    let rig = scene.meshes["column"].rig.clone().unwrap();
    let mut animator = Animator::new(Arc::new(rig.as_ref().clone()), 60);
    animator.play("bend", 0).unwrap();
    animator.blend("twist", 0.5, 0).unwrap();
    let palette = animator.palette(24);
    let rest = Pose::rest(rig.skeleton()).palette(rig.skeleton());
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let live = live_mask(gpu.clone(), &scene, Some(&palette));
    let posed = Posed {
        palettes: [(0, palette.clone())].into_iter().collect(),
        ..Posed::at(0.0)
    };
    let traced = traced_mask(&gpu, &scene, &posed);
    let still = traced_mask(&gpu, &scene, &Posed::at(0.0));
    let unposed_palette = Posed {
        palettes: [(0, rest)].into_iter().collect(),
        ..Posed::at(0.0)
    };
    let rest_traced = traced_mask(&gpu, &scene, &unposed_palette);
    let covered = live.iter().filter(|&&on| on).count();
    let same = iou(&live, &traced);
    let live_in_trace = within_a_pixel(&live, &traced);
    let trace_in_live = within_a_pixel(&traced, &live);
    let rest_in_live = within_a_pixel(&still, &live);
    println!(
        "live covers {covered} pixels; posed trace IoU {same:.4}, within a pixel {live_in_trace:.4} and {trace_in_live:.4}; unposed trace IoU {:.4}, within a pixel {rest_in_live:.4}",
        iou(&live, &still)
    );
    assert!(covered > 800, "the column covers {covered} pixels");
    assert!(
        live_in_trace > 0.995 && trace_in_live > 0.995,
        "the traced pose differs from live: {live_in_trace} and {trace_in_live} within a pixel"
    );
    assert!(
        rest_in_live < 0.8,
        "the pose barely moved the column: {rest_in_live}"
    );
    assert_eq!(
        still, rest_traced,
        "the bind pose traces as the rest palette does"
    );
}
