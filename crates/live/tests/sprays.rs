use std::path::{Path, PathBuf};

use pfx_gpu::{Gpu, OffscreenTarget, wgpu};
use pfx_live::renderer::{Exposure, Renderer};
use pfx_live::scene::Staged;
use pfx_load::scene::Scene;
use pfx_post::Chain;
use pfx_post::color::srgb_channel;

const WIDTH: u32 = 192;
const HEIGHT: u32 = 128;
const ASPECT: f32 = WIDTH as f32 / HEIGHT as f32;

const MATERIALS: &str = "[materials.glow]
base = [0.0, 0.0, 0.0]
roughness = 1.0
specular = 0.0
emission = [1.0, 1.0, 1.0]

[materials.matte]
base = [0.7, 0.7, 0.7]
roughness = 1.0
specular = 0.0
";

fn scene(name: &str, irradiance: f32, particles: &str) -> Scene {
    let root: PathBuf = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("sprays")
        .join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../load/tests/scenes");
    std::fs::copy(fixtures.join("block.gltf"), root.join("block.gltf")).unwrap();
    std::fs::write(root.join("sprays.materials.toml"), MATERIALS).unwrap();
    let path = root.join("sprays.scene.toml");
    std::fs::write(
        &path,
        format!(
            "materials = [\"sprays.materials.toml\"]\nfallback = \"matte\"\n\n[mesh.block]\nfile = \"block.gltf\"\n\n[[object]]\nname = \"wall\"\nmesh = \"block\"\nat = [0.0, 0.5, -1.5]\nscale = [6.0, 4.0, 0.2]\nmaterial = \"glow\"\n\n[sun]\nmodel = \"authored\"\ntoward = [0.3, 0.8, 0.5]\nirradiance = {irradiance}\n\n[sky]\nkind = \"room\"\n\n[sky.room]\nwidth = 64\nfloor = [0.05, 0.05, 0.05]\nwall = [0.05, 0.05, 0.05]\nceiling = [0.05, 0.05, 0.05]\n\n[camera]\nat = [0.0, 0.5, 4.0]\nlook_at = [0.0, 0.5, 0.0]\nfov = 40.0\n{particles}"
        ),
    )
    .unwrap();
    Scene::open(&path).unwrap()
}

fn puff(blend: &str, color: &str, lit: bool) -> String {
    format!(
        "\n[[particles]]\nname = \"puff\"\nat = [0.0, 0.5, 0.0]\nburst = 400\nmax = 400\nlifetime = [30.0, 30.0]\nspeed = [0.0, 0.6]\ncone = 180.0\ndrag = 4.0\nsize = [0.35, 0.35]\nalpha = [[0.0, 0.8], [1.0, 0.8]]\ncolor = \"{color}\"\nblend = \"{blend}\"\nlit = {lit}\nseed = 5\n"
    )
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

fn frames(scene: &Scene, time: f32, count: u32) -> Vec<[f32; 3]> {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let target = output(&gpu, WIDTH, HEIGHT);
    let mut renderer = Renderer::new(gpu, WIDTH, HEIGHT).unwrap();
    renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
    let mut staged: Staged = renderer.stage(scene).unwrap();
    renderer.set_finish(Chain::new());
    for _ in 0..count {
        let finish = staged.finish();
        let drawn = staged.frame(ASPECT, time, 7);
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
        .map(|p| std::array::from_fn(|c| srgb_channel(half::f16::from_bits(p[c]).to_f32())))
        .collect()
}

fn luminance(pixel: &[f32; 3]) -> f32 {
    0.2126 * pixel[0] + 0.7152 * pixel[1] + 0.0722 * pixel[2]
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn particles_draw_lit_and_blended() {
    let bare = frames(&scene("bare", 4.0, ""), 1.0, 4);
    let bright = frames(
        &scene("bright", 6.0, &puff("alpha", "#808080", true)),
        1.0,
        4,
    );
    let dim = frames(&scene("dim", 0.5, &puff("alpha", "#808080", true)), 1.0, 4);
    let dark = frames(
        &scene("dark", 4.0, &puff("alpha", "#000000", false)),
        1.0,
        4,
    );
    let added = frames(&scene("add", 4.0, &puff("add", "#204060", false)), 1.0, 4);
    let changed: Vec<usize> = (0..bare.len())
        .filter(|&i| (luminance(&dark[i]) - luminance(&bare[i])).abs() > 0.02)
        .collect();
    assert!(
        changed.len() > 200,
        "{} pixels show the spray",
        changed.len()
    );
    let darker = changed
        .iter()
        .filter(|&&i| luminance(&dark[i]) < luminance(&bare[i]))
        .count();
    assert!(
        darker * 10 >= changed.len() * 9,
        "black alpha particles cover the glowing wall: {darker} of {}",
        changed.len()
    );
    assert!(
        (0..bare.len()).all(|i| (0..3).all(|k| added[i][k] >= bare[i][k] - 2e-3)),
        "additive particles only add"
    );
    assert!(
        changed
            .iter()
            .any(|&i| luminance(&added[i]) > luminance(&bare[i]) + 0.01)
    );
    let sum = |pixels: &[[f32; 3]]| -> f64 {
        changed
            .iter()
            .map(|&i| f64::from(luminance(&pixels[i])))
            .sum()
    };
    println!("sun 6: {}, sun 0.5: {}", sum(&bright), sum(&dim));
    assert!(
        sum(&bright) > sum(&dim) * 1.2,
        "lit particles follow the sun"
    );
}

fn spray_of(count: u32) -> String {
    format!(
        "\n[[particles]]\nname = \"spray\"\nat = [0.0, 0.1, 0.0]\nburst = {count}\nmax = {count}\nlifetime = [30.0, 30.0]\nspeed = [1.0, 4.0]\ndirection = [0.0, 1.0, 0.6]\ncone = 50.0\ngravity = [0.0, -2.0, 0.0]\ndrag = 1.5\nsize = [0.08, 0.2]\nalpha = [[0.0, 0.5], [1.0, 0.5]]\ncolor = \"#c8c8d0\"\nstretch = 0.02\nseed = 9\n"
    )
}

#[test]
#[ignore = "needs a GPU; run under pgpu --class bench"]
fn spray_cost_at_2160x3840() {
    let (width, height) = (2160, 3840);
    for count in [2_000, 20_000] {
        let scene = scene(&format!("cost-{count}"), 4.0, &spray_of(count));
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let target = output(&gpu, width, height);
        let mut renderer = Renderer::new(gpu, width, height).unwrap();
        renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
        let mut staged: Staged = renderer.stage(&scene).unwrap();
        renderer.set_finish(Chain::new());
        let mut costs = Vec::new();
        let mut cpu = Vec::new();
        for frame in 0..120 {
            let finish = staged.finish();
            let started = std::time::Instant::now();
            let drawn = staged.frame(width as f32 / height as f32, 1.0 + frame as f32 / 60.0, 7);
            if frame >= 20 {
                cpu.push(started.elapsed().as_secs_f64() * 1e3);
            }
            let timings = renderer
                .render(
                    &drawn.scene,
                    &drawn.text,
                    &drawn.effects,
                    finish,
                    &target.view,
                )
                .unwrap();
            if frame >= 20
                && let Some(pass) = timings.iter().find(|pass| pass.label == "sprays")
            {
                costs.push(pass.milliseconds);
            }
        }
        let live = staged.sprays().live();
        costs.sort_by(f64::total_cmp);
        cpu.sort_by(f64::total_cmp);
        assert!(!costs.is_empty(), "no sprays pass was timed");
        let at = |q: f64| costs[((costs.len() - 1) as f64 * q) as usize];
        println!(
            "sprays {count} ({live} live) at {width}x{height}: median {:.3} ms, p10 {:.3}, p90 {:.3}",
            at(0.5),
            at(0.1),
            at(0.9)
        );
        println!(
            "sprays {count}: frame step, pack and upload list on the CPU, median {:.3} ms",
            cpu[cpu.len() / 2]
        );
    }
}
