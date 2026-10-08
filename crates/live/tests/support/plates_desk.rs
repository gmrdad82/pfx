#![allow(dead_code)]

use std::path::{Path, PathBuf};

use pfx_bake::plate::codec::from_rgb9e5;
use pfx_bake::plate::{Plate, PlateRun, bake_plates, parse_plates, read_plate};
use pfx_gpu::pace::Turns;
use pfx_gpu::{Gpu, OffscreenTarget, wgpu};
use pfx_live::renderer::{Exposure, Renderer};
use pfx_live::scene::{Staged, camera as live_camera};
use pfx_load::scene::{Camera as SceneCamera, Scene};
use pfx_post::Chain;
use pfx_post::color::srgb_channel;

pub struct Desk {
    pub root: PathBuf,
    pub size: [u32; 2],
}

fn copy(from: &Path, to: &Path) {
    std::fs::copy(from, to).unwrap_or_else(|e| panic!("{}: {e}", from.display()));
}

impl Desk {
    pub fn new(name: &str, extra: &str, size: [u32; 2]) -> Self {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("plates-live")
            .join(name);
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        for file in [
            "block.gltf",
            "panel.gltf",
            "pillar.gltf",
            "ball.gltf",
            "screen.png",
        ] {
            copy(
                &manifest.join("../load/tests/scenes").join(file),
                &root.join(file),
            );
        }
        copy(
            &manifest.join("../bake/tests/plates/desk.materials.toml"),
            &root.join("desk.materials.toml"),
        );
        copy(
            &manifest.join("../text/fonts/EBGaramond[wght].ttf"),
            &root.join("serif.ttf"),
        );
        let base =
            std::fs::read_to_string(manifest.join("../bake/tests/plates/desk.scene.toml")).unwrap();
        std::fs::write(
            root.join("desk.scene.toml"),
            format!("{base}\n{extra}\n[plates]\ndir = \"plates/main\"\n"),
        )
        .unwrap();
        std::fs::write(root.join("bare.scene.toml"), format!("{base}\n{extra}\n")).unwrap();
        Self { root, size }
    }

    pub fn path(&self) -> PathBuf {
        self.root.join("desk.scene.toml")
    }

    pub fn bake_with(&self, anchors: &[f32], threshold: f32, max_samples: u32) {
        let recipe = format!(
            "[plates]\nscene = \"desk.scene.toml\"\nsize = [{}, {}]\noverscan = 0.1\nthreshold = {threshold}\nmin_samples = 16\nmax_samples = {max_samples}\n",
            self.size[0], self.size[1]
        );
        let recipe = parse_plates(recipe.as_bytes()).unwrap().unwrap();
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut turns = Turns::default();
        let mut between = |ms: f64| turns.add(ms);
        let mut log = |line: &str| println!("{line}");
        bake_plates(PlateRun {
            gpu: &gpu,
            recipe: &recipe,
            scene: &self.path(),
            anchors,
            seed: 7,
            max_samples: None,
            out: &self.root.join("plates"),
            force: false,
            between: &mut between,
            log: &mut log,
        })
        .unwrap();
        turns.turn();
    }

    pub fn bake(&self, anchors: &[f32], max_samples: u32) {
        self.bake_with(anchors, 0.02, max_samples);
    }

    pub fn scene(&self) -> Scene {
        Scene::open(self.path()).unwrap()
    }

    pub fn bare(&self) -> Scene {
        Scene::open(self.root.join("bare.scene.toml")).unwrap()
    }

    pub fn plate(&self) -> Plate {
        read_plate(&self.root.join("plates/main")).unwrap()
    }
}

pub fn output(gpu: &Gpu, width: u32, height: u32) -> OffscreenTarget {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("plates output"),
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

pub struct Viewer {
    pub renderer: Renderer,
    pub staged: Staged,
    pub output: OffscreenTarget,
    pub turns: Turns,
    pub width: u32,
    pub height: u32,
}

impl Viewer {
    pub fn new(scene: &Scene, width: u32, height: u32) -> Self {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let output = output(&gpu, width, height);
        let mut renderer = Renderer::new(gpu, width, height).unwrap();
        renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
        let staged = renderer.stage(scene).unwrap();
        renderer.set_finish(Chain::new());
        Self {
            renderer,
            staged,
            output,
            turns: Turns::default(),
            width,
            height,
        }
    }

    pub fn aspect(&self) -> f32 {
        self.width as f32 / self.height as f32
    }

    fn render(&mut self, camera: Option<&SceneCamera>, time: f32, count: u32) {
        let aspect = self.aspect();
        for _ in 0..count {
            let finish = self.staged.finish();
            let drawn = self.staged.frame(aspect, time, 7);
            let mut scene = drawn.scene;
            if let Some(camera) = camera {
                scene.camera = live_camera(camera, aspect);
            }
            let renderer = &mut self.renderer;
            let output = &self.output;
            self.turns.time(|| {
                renderer
                    .render(&scene, &drawn.text, &drawn.effects, finish, &output.view)
                    .unwrap()
            });
        }
        self.turns.turn();
    }

    pub fn read(&self) -> Vec<[f32; 3]> {
        self.renderer
            .gpu()
            .readback_rgba16(&self.output)
            .unwrap()
            .chunks_exact(4)
            .map(|p| std::array::from_fn(|c| srgb_channel(half::f16::from_bits(p[c]).to_f32())))
            .collect()
    }

    pub fn frames(&mut self, time: f32, count: u32) -> Vec<[f32; 3]> {
        self.render(None, time, count);
        self.read()
    }

    pub fn frames_from(&mut self, camera: &SceneCamera, time: f32, count: u32) -> Vec<[f32; 3]> {
        self.render(Some(camera), time, count);
        self.read()
    }

    pub fn pixel_from(&self, camera: &SceneCamera, point: [f32; 3]) -> [u32; 2] {
        let view = camera.view();
        let projection = camera.projection(self.aspect());
        let eye: [f32; 4] = std::array::from_fn(|r| {
            (0..3).map(|c| view[c][r] * point[c]).sum::<f32>() + view[3][r]
        });
        let clip: [f32; 4] =
            std::array::from_fn(|r| (0..4).map(|c| projection[c][r] * eye[c]).sum::<f32>());
        let x = (clip[0] / clip[3] + 1.0) * 0.5 * self.width as f32;
        let y = (1.0 - clip[1] / clip[3]) * 0.5 * self.height as f32;
        [
            (x as u32).min(self.width - 1),
            (y as u32).min(self.height - 1),
        ]
    }

    pub fn through(&self, camera: &SceneCamera, pixel: [u32; 2], plane: [f32; 4]) -> [f32; 3] {
        let (forward, right, up) = camera.axes();
        let tan = camera.tan_half_height().unwrap();
        let x = (pixel[0] as f32 + 0.5) / self.width as f32 * 2.0 - 1.0;
        let y = 1.0 - (pixel[1] as f32 + 0.5) / self.height as f32 * 2.0;
        let direction: [f32; 3] = std::array::from_fn(|k| {
            forward[k]
                + right[k] * tan * self.aspect() * (x + camera.shift[0])
                + up[k] * tan * (y + camera.shift[1])
        });
        let normal = [plane[0], plane[1], plane[2]];
        let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        let t = (plane[3] - dot(normal, camera.at)) / dot(normal, direction);
        std::array::from_fn(|k| camera.at[k] + direction[k] * t)
    }

    pub fn pixel(&self, scene: &Scene, point: [f32; 3]) -> [u32; 2] {
        self.pixel_from(&scene.camera_or_default(), point)
    }

    pub fn pick(&mut self, pixel: [u32; 2]) -> u32 {
        self.renderer.pick(pixel[0], pixel[1]).unwrap();
        self.render(None, 0.0, 1);
        for _ in 0..1000 {
            if let Some(picked) = self.renderer.picked() {
                return picked.id;
            }
        }
        panic!("no pick came back");
    }
}

pub fn bilinear(plate: &Plate, anchor: usize, at: [f32; 2]) -> [f32; 3] {
    let width = plate.manifest.width as i32;
    let height = plate.manifest.height as i32;
    let q = [at[0] - 0.5, at[1] - 0.5];
    let base = [q[0].floor() as i32, q[1].floor() as i32];
    let f = [q[0] - q[0].floor(), q[1] - q[1].floor()];
    let texel = |x: i32, y: i32| {
        let x = x.clamp(0, width - 1);
        let y = y.clamp(0, height - 1);
        from_rgb9e5(plate.color[anchor][(y * width + x) as usize])
    };
    let a = texel(base[0], base[1]);
    let b = texel(base[0] + 1, base[1]);
    let c = texel(base[0], base[1] + 1);
    let d = texel(base[0] + 1, base[1] + 1);
    std::array::from_fn(|k| {
        let top = a[k] + (b[k] - a[k]) * f[0];
        let bottom = c[k] + (d[k] - c[k]) * f[0];
        top + (bottom - top) * f[1]
    })
}

pub const SHOWN: f32 = 0.95;

pub fn off(got: [f32; 3], expected: [f32; 3]) -> f32 {
    (0..3)
        .filter(|&k| expected[k] < SHOWN)
        .map(|k| (got[k] - expected[k]).abs() / expected[k].max(0.02))
        .fold(0.0, f32::max)
}

pub fn close(a: [f32; 3], b: [f32; 3], tolerance: f32) -> bool {
    off(a, b) <= tolerance
}

pub fn luminance(pixel: [f32; 3]) -> f32 {
    0.2126 * pixel[0] + 0.7152 * pixel[1] + 0.0722 * pixel[2]
}

pub fn save_png(path: &Path, width: u32, height: u32, pixels: &[[f32; 3]]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let file = std::fs::File::create(path).unwrap();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    let bytes: Vec<u8> = pixels
        .iter()
        .flat_map(|p| {
            p.map(|v| (pfx_post::color::encode_channel(v.clamp(0.0, 1.0)) * 255.0 + 0.5) as u8)
        })
        .collect();
    writer.write_image_data(&bytes).unwrap();
}

pub fn debug_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/plates")
}
