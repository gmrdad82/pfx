use std::path::Path;

use pfx_gpu::{Gpu, OffscreenTarget, wgpu};
use pfx_live::height_fog::HeightFog;
use pfx_live::renderer::{Exposure, Renderer};
use pfx_live::scene::Staged;
use pfx_load::scene::Scene;
use pfx_post::Chain;

const WIDTH: u32 = 192;
const HEIGHT: u32 = 128;
const ASPECT: f32 = WIDTH as f32 / HEIGHT as f32;
const EYE: [f32; 3] = [0.0, 1.5, 10.0];
const LOOK: [f32; 3] = [0.0, 3.0, -20.0];
const FOV: f32 = 50.0;

fn scene(name: &str, haze: &str) -> Scene {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("height-fog")
        .join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../load/tests/scenes");
    std::fs::copy(fixtures.join("block.gltf"), root.join("block.gltf")).unwrap();
    std::fs::write(
        root.join("fog.materials.toml"),
        "[materials.matte]\nbase = [0.5, 0.5, 0.5]\nroughness = 1.0\nspecular = 0.0\n",
    )
    .unwrap();
    let path = root.join("fog.scene.toml");
    std::fs::write(
        &path,
        format!(
            "materials = [\"fog.materials.toml\"]\nfallback = \"matte\"\n\n[mesh.block]\nfile = \"block.gltf\"\n\n[[object]]\nname = \"ground\"\nmesh = \"block\"\nat = [0.0, -0.05, -20.0]\nscale = [80.0, 0.1, 80.0]\n\n[[object]]\nname = \"wall\"\nmesh = \"block\"\nat = [0.0, 4.0, -20.0]\nscale = [6.0, 8.0, 1.0]\n\n[sun]\nmodel = \"authored\"\ntoward = [0.0, 0.12, -1.0]\nirradiance = 4.0\n\n[sky]\nkind = \"room\"\n\n[sky.room]\nwidth = 64\nfloor = [0.05, 0.05, 0.05]\nwall = [0.05, 0.05, 0.05]\nceiling = [0.05, 0.05, 0.05]\n\n[camera]\nat = {EYE:?}\nlook_at = {LOOK:?}\nfov = {FOV:?}\n{haze}"
        ),
    )
    .unwrap();
    Scene::open(&path).unwrap()
}

struct Viewer {
    renderer: Renderer,
    staged: Staged,
    output: OffscreenTarget,
}

impl Viewer {
    fn new(scene: &Scene) -> Self {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("height fog output"),
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
        let output = OffscreenTarget {
            texture,
            view,
            format: wgpu::TextureFormat::Rgba16Float,
            width: WIDTH,
            height: HEIGHT,
        };
        let mut renderer = Renderer::new(gpu, WIDTH, HEIGHT).unwrap();
        renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
        let staged = renderer.stage(scene).unwrap();
        renderer.set_finish(Chain::new());
        Self {
            renderer,
            staged,
            output,
        }
    }

    fn frames(&mut self, count: u32) -> Vec<u16> {
        for _ in 0..count {
            let finish = self.staged.finish();
            let drawn = self.staged.frame(ASPECT, 0.0, 7);
            self.renderer
                .render(
                    &drawn.scene,
                    &drawn.text,
                    &drawn.effects,
                    finish,
                    &self.output.view,
                )
                .unwrap();
        }
        self.renderer.gpu().readback_rgba16(&self.output).unwrap()
    }
}

fn pixel(point: [f32; 3]) -> [usize; 2] {
    let sub = |a: [f32; 3], b: [f32; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let unit = |a: [f32; 3]| {
        let length = dot(a, a).sqrt();
        a.map(|value| value / length)
    };
    let cross = |a: [f32; 3], b: [f32; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let forward = unit(sub(LOOK, EYE));
    let right = unit(cross(forward, [0.0, 1.0, 0.0]));
    let up = cross(right, forward);
    let d = sub(point, EYE);
    let depth = dot(d, forward);
    let tan_half = (FOV.to_radians() * 0.5).tan();
    let x = dot(d, right) / depth / (tan_half * ASPECT);
    let y = dot(d, up) / depth / tan_half;
    [
        ((x + 1.0) * 0.5 * WIDTH as f32) as usize,
        ((1.0 - y) * 0.5 * HEIGHT as f32) as usize,
    ]
}

fn luminance(pixels: &[u16], at: [usize; 2]) -> f64 {
    let mut sum = 0.0;
    for y in at[1] - 2..=at[1] + 2 {
        for x in at[0] - 2..=at[0] + 2 {
            let c: Vec<f64> = pixels[(y * WIDTH as usize + x) * 4..][..3]
                .iter()
                .map(|&bits| f64::from(half::f16::from_bits(bits).to_f32()))
                .collect();
            sum += 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        }
    }
    sum / 25.0
}

const OUTDOOR: &str =
    "\n[haze]\nheight = 0.0\nfalloff = 0.15\nfog = 0.08\nreach = 120.0\nsteps = 48\n";

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn height_fog_off_leaves_the_frame_byte_identical() {
    let clear = scene("off", "");
    let plain = Viewer::new(&clear).frames(4);
    let mut toggled = Viewer::new(&clear);
    toggled.renderer.set_height_fog(Some(HeightFog::default()));
    toggled.renderer.set_height_fog(None);
    assert!(plain == toggled.frames(4));
    let mut fogged = Viewer::new(&scene("on", OUTDOOR));
    assert!(plain != fogged.frames(4));
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn shafts_darken_the_fog_in_an_occluders_shadow_toward_a_low_sun() {
    let clear = Viewer::new(&scene("clear", "")).frames(8);
    let flat = Viewer::new(&scene("flat", &format!("{OUTDOOR}shafts = 0.0\n"))).frames(8);
    let shafts = Viewer::new(&scene("shafts", &format!("{OUTDOOR}shafts = 1.0\n"))).frames(8);
    let wall = pixel([0.0, 4.0, -20.0]);
    let open = pixel([0.0, 11.0, -20.0]);
    let added = |frame: &[u16], at| luminance(frame, at) - luminance(&clear, at);
    let wall_flat = added(&flat, wall);
    let wall_shafts = added(&shafts, wall);
    let open_flat = added(&flat, open);
    let open_shafts = added(&shafts, open);
    println!(
        "wall {wall:?}: {wall_flat:.4} flat, {wall_shafts:.4} shafts; open {open:?}: {open_flat:.4} flat, {open_shafts:.4} shafts"
    );
    assert!(wall_flat > 0.02, "the fog adds light in front of the wall");
    assert!(
        open_shafts > 0.3 * open_flat,
        "rays over the wall keep most of their sunlight"
    );
    assert!(
        wall_shafts / wall_flat < 0.5 * open_shafts / open_flat,
        "the wall's shadow carves a darker shaft than the open sky beside it"
    );
}
