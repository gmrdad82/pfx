#![allow(dead_code)]

use std::fmt::Write;
use std::path::{Path, PathBuf};

use pfx_gpu::Gpu;
use pfx_gpu::pace::Turns;
use pfx_live::renderer::{Exposure, Renderer};
use pfx_load::scene::Scene;
use pfx_post::Chain;

pub const EYE: [f32; 3] = [0.0, 1.7, 10.0];
pub const LOOK: [f32; 3] = [0.0, 2.6, -10.0];
pub const FOV: f32 = 50.0;
pub const LATTICE_Z: f32 = -14.0;
pub const SUN: [f32; 3] = [0.1, 0.7, -1.0];
pub const SIDE_SUN: [f32; 3] = [0.45, 0.62, -1.0];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Haze {
    pub fog: f32,
    pub falloff: f32,
    pub reach: f32,
    pub phase: f32,
    pub color: [f32; 3],
}

impl Haze {
    pub const STREET: Haze = Haze {
        fog: 0.06,
        falloff: 0.15,
        reach: 80.0,
        phase: 0.6,
        color: [1.0, 1.0, 1.0],
    };
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Street {
    pub haze: Option<Haze>,
    pub lamp: bool,
    pub dark: bool,
    pub sun: [f32; 3],
}

impl Street {
    pub fn cleared(self) -> Street {
        Street { haze: None, ..self }
    }
}

impl Default for Street {
    fn default() -> Self {
        Self {
            haze: None,
            lamp: false,
            dark: false,
            sun: SUN,
        }
    }
}

const DARK: &str = "format = 1\n\n[materials.ground]\nbase = [0.0, 0.0, 0.0]\nspecular = 0.0\n\n[materials.wall]\nbase = [0.0, 0.0, 0.0]\nspecular = 0.0\n\n[materials.bark]\nbase = [0.0, 0.0, 0.0]\nspecular = 0.0\n";

const MATERIALS: &str = "format = 1\n\n[materials.ground]\nbase = [0.3, 0.29, 0.27]\nroughness = 0.9\n\n[materials.wall]\nbase = [0.55, 0.52, 0.48]\nroughness = 0.8\n\n[materials.bark]\nbase = [0.12, 0.1, 0.08]\nroughness = 0.9\n";

fn block(out: &mut String, name: &str, material: &str, at: [f32; 3], size: [f32; 3]) {
    let _ = write!(
        out,
        "\n[[object]]\nname = \"{name}\"\nmesh = \"block\"\nmaterial = \"{material}\"\nat = [{}, {}, {}]\nscale = [{}, {}, {}]\n",
        at[0], at[1], at[2], size[0], size[1], size[2]
    );
}

pub fn text(street: Street) -> String {
    let mut out = String::from(
        "format = 1\n\nmaterials = [\"street.materials.toml\"]\nfallback = \"wall\"\n\n[mesh.block]\nfile = \"block.gltf\"\n",
    );
    block(
        &mut out,
        "ground",
        "ground",
        [0.0, -0.05, -20.0],
        [40.0, 0.1, 80.0],
    );
    for (k, z) in [-2.0_f32, -9.0, -16.0, -23.0, -30.0]
        .into_iter()
        .enumerate()
    {
        let height = 5.0 + 1.3 * ((k * 3) % 4) as f32;
        for (side, x) in [("left", -5.5_f32), ("right", 5.5)] {
            block(
                &mut out,
                &format!("{side} {k}"),
                "wall",
                [x, height * 0.5, z],
                [3.0, height, 5.0],
            );
        }
    }
    block(
        &mut out,
        "trunk",
        "bark",
        [0.0, 1.5, LATTICE_Z],
        [0.4, 3.0, 0.4],
    );
    for k in 0..8 {
        let x = -2.45 + 0.7 * k as f32;
        block(
            &mut out,
            &format!("upright {k}"),
            "bark",
            [x, 5.0, LATTICE_Z],
            [0.18, 4.0, 0.18],
        );
    }
    for k in 0..6 {
        let y = 3.25 + 0.7 * k as f32;
        block(
            &mut out,
            &format!("bough {k}"),
            "bark",
            [0.0, y, LATTICE_Z],
            [5.2, 0.18, 0.18],
        );
    }
    let _ = write!(
        out,
        "\n[sun]\nmodel = \"authored\"\ntoward = [{}, {}, {}]\ncolor = [1.0, 0.9, 0.75]\nirradiance = 4.0\n{}\n[camera]\nat = [{}, {}, {}]\nlook_at = [{}, {}, {}]\nfov = {FOV}\nfar = 200.0\n",
        street.sun[0],
        street.sun[1],
        street.sun[2],
        if street.dark {
            ""
        } else {
            "\n[sky]\nkind = \"analytic\"\n"
        },
        EYE[0],
        EYE[1],
        EYE[2],
        LOOK[0],
        LOOK[1],
        LOOK[2]
    );
    if street.lamp {
        out.push_str("\n[[light]]\nname = \"lamp\"\nposition = [-2.5, 4.0, -6.0]\ncolor = [1.0, 0.8, 0.55]\nintensity = 60.0\nradius = 0.05\nrange = 25.0\nshadow = true\n");
    }
    if let Some(haze) = street.haze {
        let _ = write!(
            out,
            "\n[haze]\nheight = 0.0\nfalloff = {}\nfog = {}\nreach = {}\nphase = {}\ncolor = [{}, {}, {}]\n",
            haze.falloff,
            haze.fog,
            haze.reach,
            haze.phase,
            haze.color[0],
            haze.color[1],
            haze.color[2]
        );
        if street.dark {
            out.push_str("ambient = [0.0, 0.0, 0.0]\n");
        }
    }
    out
}

pub fn folder(base: &Path, name: &str, street: Street) -> PathBuf {
    let root = base.join("trace-street").join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../load/tests/scenes");
    std::fs::copy(fixtures.join("block.gltf"), root.join("block.gltf")).unwrap();
    std::fs::write(
        root.join("street.materials.toml"),
        if street.dark { DARK } else { MATERIALS },
    )
    .unwrap();
    std::fs::write(root.join("street.scene.toml"), text(street)).unwrap();
    root
}

pub fn scene(base: &Path, name: &str, street: Street) -> Scene {
    Scene::open(folder(base, name, street).join("street.scene.toml")).unwrap()
}

pub fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

pub fn pixels(bytes: &[u8]) -> Vec<[f32; 4]> {
    bytes
        .chunks_exact(16)
        .map(|texel| {
            std::array::from_fn(|k| f32::from_le_bytes(texel[4 * k..4 * k + 4].try_into().unwrap()))
        })
        .collect()
}

pub fn luminance(pixel: [f32; 4]) -> f32 {
    0.2126 * pixel[0] + 0.7152 * pixel[1] + 0.0722 * pixel[2]
}

pub fn live(gpu: &Gpu, scene: &Scene, size: [u32; 2]) -> Vec<[f32; 3]> {
    use pfx_gpu::wgpu;
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("fog live output"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
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
    let output = pfx_gpu::OffscreenTarget {
        view: texture.create_view(&Default::default()),
        texture,
        format: wgpu::TextureFormat::Rgba16Float,
        width: size[0],
        height: size[1],
    };
    let mut renderer = Renderer::new(gpu.clone(), size[0], size[1]).unwrap();
    let mut staged = renderer.stage(scene).unwrap();
    renderer.wait_sky().unwrap();
    renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
    renderer.set_finish(Chain::new());
    let mut turns = Turns::default();
    let aspect = size[0] as f32 / size[1] as f32;
    for _ in 0..32 {
        let finish = staged.finish();
        let drawn = staged.frame(aspect, 0.0, 7);
        turns.time(|| {
            renderer
                .render(
                    &drawn.scene,
                    &drawn.text,
                    &drawn.effects,
                    finish,
                    &output.view,
                )
                .unwrap()
        });
    }
    turns.turn();
    renderer
        .gpu()
        .readback_rgba16(&output)
        .unwrap()
        .chunks_exact(4)
        .map(|p| {
            std::array::from_fn(|c| {
                pfx_materials::linear_channel(half::f16::from_bits(p[c]).to_f32())
            })
        })
        .collect()
}

pub fn levels(pixels: &[[f32; 4]]) -> Vec<[f32; 3]> {
    pixels
        .iter()
        .map(|p| std::array::from_fn(|c| pfx_trace::film::srgb_byte(p[c]).round()))
        .collect()
}

pub fn mean(values: impl Iterator<Item = f32>) -> f32 {
    let (sum, count) = values.fold((0.0_f64, 0_usize), |(sum, count), value| {
        (sum + f64::from(value), count + 1)
    });
    (sum / count.max(1) as f64) as f32
}

pub fn apart(a: &[[f32; 3]], b: &[[f32; 3]]) -> f32 {
    mean(
        a.iter()
            .zip(b)
            .flat_map(|(a, b)| (0..3).map(move |c| (a[c] - b[c]).abs())),
    )
}

pub fn blocks(image: &[[f32; 3]], size: [u32; 2], side: u32) -> Vec<[f32; 3]> {
    let mut out = Vec::new();
    for by in 0..size[1] / side {
        for bx in 0..size[0] / side {
            let mut sum = [0.0_f32; 3];
            for y in by * side..(by + 1) * side {
                for x in bx * side..(bx + 1) * side {
                    let p = image[(y * size[0] + x) as usize];
                    for c in 0..3 {
                        sum[c] += p[c];
                    }
                }
            }
            out.push(sum.map(|v| v / (side * side) as f32));
        }
    }
    out
}
