use std::path::{Path, PathBuf};
use std::time::Instant;

use pfx_gpu::pace::{Pacer, Turns};
use pfx_gpu::{Gpu, OffscreenTarget, wgpu};
use pfx_live::renderer::{Exposure, Renderer};
use pfx_load::scene::Scene;
use pfx_post::Chain;
use pfx_post::color::encode_channel;
use pfx_trace::sprites::Spray;
use pfx_trace::stage;

const MATERIALS: &str = "[materials.matte]
base = [0.6, 0.6, 0.6]
roughness = 1.0
specular = 0.0

[materials.black]
base = [0.0, 0.0, 0.0]
roughness = 1.0
specular = 0.0

[materials.crate]
base = [0.55, 0.3, 0.15]
roughness = 0.8
specular = 0.04
";

const PLUME: &str = "
[[particles]]
name = \"exhaust\"
at = [0.45, 0.25, -0.9]
rate = 140.0
max = 4000
lifetime = [1.6, 2.4]
speed = [1.4, 2.2]
direction = [0.25, 0.35, -1.0]
cone = 14.0
drag = 0.9
gravity = [0.0, 0.5, 0.0]
size = [0.12, 1.1]
alpha = [[0.0, 0.0], [0.08, 0.5], [0.5, 0.3], [1.0, 0.0]]
color = [0.82, 0.82, 0.86]
spin = [10.0, 40.0]
seed = 3
";

const DUST: &str = "
[[particles]]
name = \"dust\"
at = [1.3, 0.02, -1.0]
rate = 40.0
max = 400
lifetime = [2.0, 3.0]
speed = [0.3, 0.9]
direction = [1.0, 0.1, -0.3]
cone = 50.0
drag = 1.5
size = [0.5, 1.2]
alpha = [[0.0, 0.0], [0.2, 0.35], [1.0, 0.0]]
color = [0.62, 0.56, 0.46]
seed = 5
";

fn root_of(name: &str) -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("sprays")
        .join(name)
}

fn root(name: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("sprays")
        .join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn scene(name: &str, particles: &str) -> Scene {
    grounded(name, "matte", particles)
}

fn grounded(name: &str, ground: &str, particles: &str) -> Scene {
    let root = root(name);
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../load/tests/scenes");
    std::fs::copy(fixtures.join("block.gltf"), root.join("block.gltf")).unwrap();
    std::fs::write(root.join("sprays.materials.toml"), MATERIALS).unwrap();
    let path = root.join("sprays.scene.toml");
    std::fs::write(
        &path,
        format!(
            "materials = [\"sprays.materials.toml\"]\nfallback = \"matte\"\n\n[mesh.block]\nfile = \"block.gltf\"\n\n[[object]]\nname = \"ground\"\nmesh = \"block\"\nat = [0.0, -0.1, -2.0]\nscale = [24.0, 0.2, 24.0]\nmaterial = \"{ground}\"\n\n[[object]]\nname = \"box\"\nmesh = \"block\"\nat = [0.0, 0.6, 0.0]\nscale = [1.4, 1.2, 1.4]\nmaterial = \"crate\"\n\n[sun]\nmodel = \"authored\"\ntoward = [-0.55, 0.22, -0.8]\nirradiance = 3.0\n\n[sky]\nkind = \"room\"\n\n[sky.room]\nwidth = 64\nfloor = [0.0, 0.0, 0.0]\nwall = [0.0, 0.0, 0.0]\nceiling = [0.0, 0.0, 0.0]\n\n[camera]\nat = [3.2, 1.5, 4.2]\nlook_at = [-0.2, 0.9, -1.6]\nfov = 45.0\n{particles}"
        ),
    )
    .unwrap();
    Scene::open(&path).unwrap()
}

fn output(gpu: &Gpu, width: u32, height: u32) -> OffscreenTarget {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("sprays output"),
        size: wgpu::Extent3d {
            width,
            height,
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
        width,
        height,
    }
}

fn live(scene: &Scene, size: [u32; 2], time: f32) -> Vec<[f32; 3]> {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let target = output(&gpu, size[0], size[1]);
    let mut renderer = Renderer::new(gpu, size[0], size[1]).unwrap();
    renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
    let mut staged = renderer.stage(scene).unwrap();
    renderer.set_finish(Chain::new());
    for _ in 0..8 {
        let finish = staged.finish();
        let drawn = staged.frame(size[0] as f32 / size[1] as f32, time, 7);
        renderer
            .render(
                &drawn.scene,
                &drawn.text,
                &drawn.effects,
                finish,
                &target.view,
            )
            .unwrap();
    }
    renderer
        .gpu()
        .readback_rgba16(&target)
        .unwrap()
        .chunks_exact(4)
        .map(|p| std::array::from_fn(|c| half::f16::from_bits(p[c]).to_f32()))
        .collect()
}

fn staged(
    scene: &Scene,
    size: [u32; 2],
    time: f32,
    soft: bool,
) -> stage::Staged<pfx_trace::sky::Environment> {
    let aspect = size[0] as f32 / size[1] as f32;
    if soft {
        return stage::scene_at(scene, aspect, time).unwrap();
    }
    let bare = Scene {
        particles: Default::default(),
        ..scene.clone()
    };
    let mut staged = stage::scene_at(&bare, aspect, time).unwrap();
    let puffs = stage::scene_puffs(scene, &scene.camera_or_default(), time);
    let camera = stage::scene_camera(&scene.camera_or_default(), aspect).0;
    staged
        .spray(&Spray {
            soft: false,
            ..Spray::new(&puffs, camera)
        })
        .unwrap();
    staged
}

struct Traced {
    colour: Vec<[f32; 3]>,
    albedo: Vec<[f32; 3]>,
    gpu_ms: f64,
}

fn floats(bytes: &[u8]) -> Vec<[f32; 3]> {
    bytes
        .chunks_exact(16)
        .map(|p| {
            std::array::from_fn(|c| f32::from_le_bytes(p[c * 4..c * 4 + 4].try_into().unwrap()))
        })
        .collect()
}

fn traced(
    staged: &stage::Staged<pfx_trace::sky::Environment>,
    size: [u32; 2],
    samples: u32,
) -> Traced {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut trace = staged.trace(&gpu, size[0], size[1]).unwrap();
    let mut pacer = Pacer::default();
    let mut turns = Turns::default();
    let stats = trace
        .sample_paced(&gpu, samples, 11, &mut pacer, |ms| turns.add(ms))
        .unwrap();
    turns.turn();
    let output = trace.readback(&gpu).unwrap();
    Traced {
        colour: floats(&output.color),
        albedo: floats(&output.albedo),
        gpu_ms: stats.milliseconds.iter().sum(),
    }
}

fn level(linear: f32) -> f64 {
    f64::from(encode_channel(linear.clamp(0.0, 1.0))) * 255.0
}

fn save(name: &str, size: [u32; 2], pixels: &[[f32; 3]]) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/sprays");
    std::fs::create_dir_all(&dir).unwrap();
    let file = std::fs::File::create(dir.join(name)).unwrap();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), size[0], size[1]);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    let bytes: Vec<u8> = pixels
        .iter()
        .flat_map(|p| p.map(|v| (level(v) + 0.5) as u8))
        .collect();
    writer.write_image_data(&bytes).unwrap();
}

const BLOCK: u32 = 8;

fn blocks(size: [u32; 2], pixels: &[[f32; 3]]) -> Vec<[f64; 3]> {
    let (bw, bh) = (size[0] / BLOCK, size[1] / BLOCK);
    let mut out = vec![[0.0; 3]; (bw * bh) as usize];
    for y in 0..bh * BLOCK {
        for x in 0..bw * BLOCK {
            let p = pixels[(y * size[0] + x) as usize];
            let cell = &mut out[((y / BLOCK) * bw + x / BLOCK) as usize];
            for c in 0..3 {
                cell[c] += level(p[c]) / f64::from(BLOCK * BLOCK);
            }
        }
    }
    out
}

fn mean(cells: &[[f64; 3]], mask: &[bool]) -> [f64; 3] {
    let count = mask.iter().filter(|&&m| m).count().max(1) as f64;
    let mut sum = [0.0; 3];
    for (cell, _) in cells.iter().zip(mask).filter(|(_, m)| **m) {
        for c in 0..3 {
            sum[c] += cell[c] / count;
        }
    }
    sum
}

fn extent(mask: &[bool], columns: u32) -> [u32; 4] {
    let mut bounds = [u32::MAX, u32::MAX, 0, 0];
    for (index, _) in mask.iter().enumerate().filter(|(_, m)| **m) {
        let (x, y) = (index as u32 % columns, index as u32 / columns);
        bounds = [
            bounds[0].min(x),
            bounds[1].min(y),
            bounds[2].max(x),
            bounds[3].max(y),
        ];
    }
    bounds
}

fn changed(with: &[[f64; 3]], without: &[[f64; 3]], over: f64) -> Vec<bool> {
    with.iter()
        .zip(without)
        .map(|(a, b)| (0..3).map(|c| (a[c] - b[c]).abs()).sum::<f64>() > over)
        .collect()
}

fn count(mask: &[bool]) -> usize {
    mask.iter().filter(|&&m| m).count()
}

fn hardware(on: bool) {
    unsafe { std::env::set_var("PFX_TRACE_RT", if on { "1" } else { "0" }) };
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_traced_plume_lies_where_the_live_plume_does() {
    const SIZE: [u32; 2] = [1920, 1080];
    const TIME: f32 = 4.0;
    const SAMPLES: u32 = 128;
    const SMALL: [u32; 2] = [480, 270];
    let with = grounded("plume", "black", &format!("{PLUME}{DUST}"));
    let without = grounded("bare", "black", "");
    let live_with = live(&with, SIZE, TIME);
    let live_without = live(&without, SIZE, TIME);
    hardware(true);
    let soft = traced(&staged(&with, SIZE, TIME, true), SIZE, SAMPLES);
    let bare = traced(&staged(&without, SIZE, TIME, true), SIZE, SAMPLES);
    let hard = traced(&staged(&with, SIZE, TIME, false), SIZE, SAMPLES);
    let queried = traced(&staged(&with, SMALL, TIME, true), SMALL, 64);
    hardware(false);
    let software = traced(&staged(&with, SMALL, TIME, true), SMALL, 64);
    save("live.png", SIZE, &live_with);
    save("live-bare.png", SIZE, &live_without);
    save("traced.png", SIZE, &soft.colour);
    save("traced-bare.png", SIZE, &bare.colour);
    save("traced-hard.png", SIZE, &hard.colour);
    save("traced-software.png", SMALL, &software.colour);
    let columns = SIZE[0] / BLOCK;
    let [lw, lb, tw, tb, th] = [
        &live_with,
        &live_without,
        &soft.colour,
        &bare.colour,
        &hard.colour,
    ]
    .map(|pixels| blocks(SIZE, pixels));
    let [sq, ss] = [&queried.colour, &software.colour].map(|pixels| blocks(SMALL, pixels));
    let small_mask = vec![true; sq.len()];
    let [aw, ab] = [&soft.albedo, &bare.albedo].map(|pixels| blocks(SIZE, pixels));
    let live_mask = changed(&lw, &lb, 6.0);
    let traced_mask = changed(&aw, &ab, 6.0);
    let both = live_mask
        .iter()
        .zip(&traced_mask)
        .filter(|(a, b)| **a && **b)
        .count();
    let live_mean = mean(&lw, &live_mask);
    let traced_mean = mean(&tw, &live_mask);
    let queried_mean = mean(&sq, &small_mask);
    let software_mean = mean(&ss, &small_mask);
    let live_bare_mean = mean(&lb, &live_mask);
    let traced_bare_mean = mean(&tb, &live_mask);
    let faded = changed(&th, &tw, 3.0);
    let fade_mean = th
        .iter()
        .zip(&tw)
        .zip(&faded)
        .filter(|(_, m)| **m)
        .map(|((a, b), _)| (0..3).map(|c| a[c] - b[c]).sum::<f64>() / 3.0)
        .sum::<f64>()
        / count(&faded).max(1) as f64;
    println!(
        "plume blocks: live {}, traced {}, both {both}; extent live {:?} traced {:?}",
        count(&live_mask),
        count(&traced_mask),
        extent(&live_mask, columns),
        extent(&traced_mask, columns)
    );
    println!(
        "plume mean levels: live {live_mean:.1?} traced {traced_mean:.1?} whole small frame: ray queries {queried_mean:.2?} software BVH {software_mean:.2?}; behind it, bare: live {live_bare_mean:.1?} traced {traced_bare_mean:.1?}"
    );
    println!(
        "soft fade: {} blocks move by over 3 levels, the unfaded ones by {fade_mean:.1} levels on average",
        count(&faded)
    );
    println!(
        "GPU at 1920x1080: {:.0} ms for {SAMPLES} samples; at 480x270, 64 samples: ray queries {:.0} ms, software BVH {:.0} ms",
        soft.gpu_ms, queried.gpu_ms, software.gpu_ms
    );
    assert!(
        count(&live_mask) > 500,
        "the live plume covers {} blocks",
        count(&live_mask)
    );
    let [a, b] = [extent(&live_mask, columns), extent(&traced_mask, columns)];
    assert!(
        (0..4).all(|k| a[k].abs_diff(b[k]) <= 2),
        "extent live {a:?} traced {b:?}"
    );
    for c in 0..3 {
        assert!(
            (software_mean[c] - queried_mean[c]).abs() < 1.0,
            "channel {c}: software {:.2} ray queries {:.2}",
            software_mean[c],
            queried_mean[c]
        );
    }
    let overlap = both as f64 / count(&live_mask).max(count(&traced_mask)) as f64;
    assert!(overlap > 0.85, "the plumes overlap by {overlap:.2}");
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_scene_whose_emitters_show_nothing_traces_the_bytes_of_one_without_them() {
    const SIZE: [u32; 2] = [160, 96];
    let idle = scene(
        "idle",
        "\n[[particles]]\nname = \"idle\"\nat = [0.0, 1.0, 0.0]\nrate = 0.0\n",
    );
    let bare = scene("idle-bare", "");
    let a = traced(&stage::scene_at(&idle, 160.0 / 96.0, 1.0).unwrap(), SIZE, 4);
    let b = traced(&stage::scene_at(&bare, 160.0 / 96.0, 1.0).unwrap(), SIZE, 4);
    assert!(
        a.colour == b.colour && a.albedo == b.albedo,
        "an emitter that shows nothing changes the bytes"
    );
    let plume = scene("idle-plume", PLUME);
    let staged = stage::scene_at(&plume, 160.0 / 96.0, 1.0).unwrap();
    assert!(
        staged
            .detail
            .instances
            .iter()
            .any(|i| i.surface.sprite.is_some())
    );
}

fn columns_covered(size: [u32; 2], with: &[[f32; 3]], without: &[[f32; 3]]) -> usize {
    (0..size[0])
        .filter(|&x| {
            (0..size[1]).any(|y| {
                let at = (y * size[0] + x) as usize;
                (0..3).any(|c| (with[at][c] - without[at][c]).abs() > 0.05)
            })
        })
        .count()
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_moving_particle_streaks_over_the_shutter() {
    const SIZE: [u32; 2] = [320, 180];
    let spark = "\n[[particles]]\nname = \"spark\"\nat = [-1.0, 1.6, -1.0]\nburst = 1\nmax = 1\nlifetime = [60.0, 60.0]\nspeed = [4.0, 4.0]\ndirection = [1.0, 0.0, 0.0]\nsize = [0.3, 0.3]\nalpha = [[0.0, 1.0], [1.0, 1.0]]\ncolor = [1.0, 1.0, 1.0]\nlit = true\nseed = 1\n";
    let with = scene("spark", spark);
    let bare = scene("spark-bare", "");
    let aspect = SIZE[0] as f32 / SIZE[1] as f32;
    let time = 0.5;
    let still = staged(&with, SIZE, time, true);
    let mut moving = stage::scene_at(&bare, aspect, time).unwrap();
    moving.detail.motion = Some(pfx_trace::motion::Motion {
        steps: 3,
        cameras: Vec::new(),
        curve: Vec::new(),
    });
    let puffs = stage::scene_puffs(&with, &with.camera_or_default(), time);
    assert_eq!(puffs.len(), 1);
    let camera = stage::scene_camera(&with.camera_or_default(), aspect).0;
    moving
        .spray(&Spray {
            steps: &[-0.05, 0.0, 0.05],
            ..Spray::new(&puffs, camera)
        })
        .unwrap();
    let empty = traced(&stage::scene_at(&bare, aspect, time).unwrap(), SIZE, 32);
    let sharp = traced(&still, SIZE, 32);
    let blurred = traced(&moving, SIZE, 32);
    let sharp_columns = columns_covered(SIZE, &sharp.albedo, &empty.albedo);
    let blurred_columns = columns_covered(SIZE, &blurred.albedo, &empty.albedo);
    println!(
        "the spark covers {sharp_columns} columns still and {blurred_columns} over the shutter"
    );
    assert!(sharp_columns > 4, "{sharp_columns}");
    assert!(
        blurred_columns as f64 > 1.8 * sharp_columns as f64,
        "{blurred_columns} against {sharp_columns}"
    );
}

fn cloud(count: u32) -> String {
    format!(
        "\n[[particles]]\nname = \"cloud\"\nat = [-0.6, 1.4, -1.6]\nburst = {count}\nmax = {count}\nlifetime = [60.0, 60.0]\nspeed = [0.0, 3.0]\ncone = 180.0\ndrag = 2.0\nsize = [0.18, 0.18]\nalpha = [[0.0, 0.4], [1.0, 0.4]]\ncolor = [0.8, 0.8, 0.82]\nseed = 9\n"
    )
}

#[test]
#[ignore = "a benchmark; run under pgpu --class bench"]
fn traced_sprays_cost_at_3840x2160() {
    use std::io::Write;
    const SIZE: [u32; 2] = [3840, 2160];
    const SAMPLES: u32 = 16;
    let log = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/sprays/cost.log");
    std::fs::create_dir_all(log.parent().unwrap()).unwrap();
    for (count, rounds) in [(0, 3), (1_000, 3), (10_000, 3), (50_000, 1)] {
        let particles = if count == 0 {
            String::new()
        } else {
            cloud(count)
        };
        let scene = scene(&format!("cost-{count}"), &particles);
        for _ in 0..rounds {
            let start = Instant::now();
            let staged = stage::scene_at(&scene, SIZE[0] as f32 / SIZE[1] as f32, 1.0).unwrap();
            let staging = start.elapsed().as_secs_f64() * 1e3;
            let sprites = staged
                .detail
                .instances
                .iter()
                .filter(|i| i.surface.sprite.is_some())
                .count();
            assert!(sprites as u32 <= count);
            let ms = traced(&staged, SIZE, SAMPLES).gpu_ms;
            let line = format!(
                "{count} particles ({sprites} drawn): GPU {ms:.0} ms for {SAMPLES} samples at {}x{}, staging {staging:.0} ms",
                SIZE[0], SIZE[1]
            );
            println!("{line}");
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log)
                .unwrap();
            writeln!(file, "{line}").unwrap();
        }
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_thin_puff_scatters_as_the_live_pass_formula_says() {
    const SIZE: [u32; 2] = [192, 128];
    for toward in [[-0.55f32, 0.22, -0.8], [0.0, 0.25, 1.0], [1.0, 0.3, -0.3]] {
        let puff = "\n[[particles]]\nname = \"one\"\nat = [0.0, 1.6, -1.6]\nburst = 1\nmax = 1\nlifetime = [60.0, 60.0]\nspeed = [0.0, 0.0]\nsize = [1.0, 1.0]\nalpha = [[0.0, 0.2], [1.0, 0.2]]\ncolor = [1.0, 1.0, 1.0]\nseed = 1\n";
        let name = format!("one-{}", toward[0]);
        grounded(&name, "black", puff);
        let text = std::fs::read_to_string(root_of(&name).join("sprays.scene.toml"))
            .unwrap()
            .replace(
                "toward = [-0.55, 0.22, -0.8]",
                &format!("toward = [{}, {}, {}]", toward[0], toward[1], toward[2]),
            );
        std::fs::write(root_of(&name).join("sprays.scene.toml"), text).unwrap();
        let with = Scene::open(root_of(&name).join("sprays.scene.toml")).unwrap();
        let aspect = SIZE[0] as f32 / SIZE[1] as f32;
        let lv = live(&with, SIZE, 1.0);
        let tr = traced(&stage::scene_at(&with, aspect, 1.0).unwrap(), SIZE, 1024);
        let camera = with.camera_or_default();
        let puffs = stage::scene_puffs(&with, &camera, 1.0);
        let p = puffs[0].sprite.position;
        let (cam, _) = stage::scene_camera(&camera, aspect);
        let v: Vec<f32> = (0..3).map(|k| p[k] - cam.origin[k]).collect();
        let dot = |a: &[f32], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        let z = dot(&v, cam.forward);
        let rr = dot(&cam.right, cam.right);
        let uu = dot(&cam.up, cam.up);
        let x = dot(&v, cam.right) / (rr * z);
        let y = -dot(&v, cam.up) / (uu * z);
        let (px, py) = (
            ((x + 1.0) * 0.5 * SIZE[0] as f32) as u32,
            ((y + 1.0) * 0.5 * SIZE[1] as f32) as u32,
        );
        let len = dot(&v, [v[0], v[1], v[2]]).sqrt();
        let tl = dot(&toward, toward).sqrt();
        let cosine = (0..3).map(|k| v[k] / len * toward[k] / tl).sum::<f32>();
        let g = 0.5f32;
        let phase = (1.0 - g * g)
            / (4.0 * std::f32::consts::PI * (1.0 + g * g - 2.0 * g * cosine).powf(1.5));
        let n: Vec<f32> = v.iter().map(|c| -c / len).collect();
        let wrap = (dot(&n, [toward[0] / tl, toward[1] / tl, toward[2] / tl]) + 0.5).max(0.0)
            / 1.5
            / std::f32::consts::PI;
        let centre = |img: &[[f32; 3]]| {
            let mut sum = 0.0;
            for dy in 0..5 {
                for dx in 0..5 {
                    sum += img[((py + dy - 2) * SIZE[0] + px + dx - 2) as usize][0];
                }
            }
            sum / 25.0
        };
        let formula = 0.6 * (0.8 * phase + 0.2 * wrap);
        let normalised = 0.6 * (0.8 * phase + 0.2 * wrap / 1.5);
        let traced = centre(&tr.colour);
        println!(
            "toward {toward:?}: cos {cosine:.3}; the live formula gives {formula:.4}, normalised {normalised:.4}; traced {traced:.4}, live {:.4}",
            centre(&lv)
        );
        assert!(
            (traced - normalised).abs() < 0.12 * normalised + 0.002,
            "toward {toward:?}: traced {traced} against {normalised}"
        );
    }
}
