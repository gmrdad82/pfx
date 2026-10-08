use std::path::Path;

use pfx_bake::reflection::{
    ReflectionSpec, ReflectionWrite, bake_reflection, read_reflection_artifact,
    write_reflection_artifact,
};
use pfx_bake::{Anchor, BakeScene, Grid, GridSpec, bake, scene_hash, write_artifact};
use pfx_live::deform::Deformer;
use pfx_live::frame::{Instance, Scene, SceneWind};
use pfx_live::probes::ProbeLighting;
use pfx_live::renderer::{Effects, Finish, Renderer, Text};
use pfx_live::sky::SkySource;
use pfx_load::Sky;
use pfx_materials::Material;
use pfx_trace::bvh::Triangle;
use pfx_trace::detail::{Detail, Lens as TraceLens};
use pfx_trace::{Camera as TraceCamera, Scene as TraceScene, Trace};

use super::{HOUR, Lens, Lighting, PLATE, SAMPLES, SEED, cross, roller, sub, target, unit};

const AXES: [[f32; 3]; 6] = [
    [1.0, 0.0, 0.0],
    [-1.0, 0.0, 0.0],
    [0.0, 1.0, 0.0],
    [0.0, -1.0, 0.0],
    [0.0, 0.0, 1.0],
    [0.0, 0.0, -1.0],
];
const TRACE_SAMPLES: u32 = 256;
const FACE_TEXELS: u32 = 64;
const FACE_SAMPLES: u32 = 64;

pub struct Terms<'a> {
    pub renderer: &'a mut Renderer,
    pub instances: &'a [Instance],
    pub materials: &'a [Material],
    pub triangles: &'a [Triangle],
    pub lens: &'a Lens,
    pub lighting: &'a Lighting,
    pub spec: GridSpec,
    pub reflection: ReflectionSpec,
}

#[derive(Clone, Copy, PartialEq)]
enum Light {
    Sun,
    Sky,
    Both,
}

#[derive(Clone, Copy, PartialEq)]
enum Surface {
    Diffuse,
    Full,
}

struct Captures {
    trace: Vec<[f32; 3]>,
    covered: Vec<bool>,
    full: Vec<[f32; 3]>,
    no_ssr: Vec<[f32; 3]>,
    no_local: Vec<[f32; 3]>,
    direct: Vec<[f32; 3]>,
    sh: Vec<[f32; 3]>,
}

fn luma(c: [f32; 3]) -> f64 {
    0.2126 * f64::from(c[0]) + 0.7152 * f64::from(c[1]) + 0.0722 * f64::from(c[2])
}

fn black() -> Sky {
    Sky {
        width: 1,
        height: 1,
        texels: vec![[0.0, 0.0, 0.0, 1.0]],
    }
}

fn diffuse_only(material: &Material) -> Material {
    Material {
        specular: 0.0,
        metalness: 0.0,
        clearcoat: 0.0,
        sheen: 0.0,
        ..*material
    }
}

impl Light {
    fn name(self) -> &'static str {
        match self {
            Light::Sun => "sun",
            Light::Sky => "sky",
            Light::Both => "both",
        }
    }
}

impl Surface {
    fn name(self) -> &'static str {
        match self {
            Surface::Diffuse => "diffuse",
            Surface::Full => "full",
        }
    }
}

fn save_pair(path: &Path, left: &[[f32; 3]], right: &[[f32; 3]]) {
    let (width, height) = PLATE;
    let encode = |v: f32| {
        let v = (v * 2.0 / (1.0 + v * 2.0)).clamp(0.0, 1.0);
        let s = pfx_materials::encode_channel(v);
        (s * 255.0).round() as u8
    };
    let mut rgba = Vec::with_capacity((width * height * 8) as usize);
    for y in 0..height as usize {
        for image in [left, right] {
            for x in 0..width as usize {
                let c = image[y * width as usize + x];
                rgba.extend([encode(c[0]), encode(c[1]), encode(c[2]), 255]);
            }
        }
    }
    super::save_png(path, width * 2, height, &rgba);
}

impl Terms<'_> {
    fn anchor(&self, light: Light) -> Anchor {
        let mut anchor = self.lighting.anchor(HOUR);
        if light == Light::Sun {
            anchor.sky = black();
        }
        if light == Light::Sky {
            anchor.sun.intensity = 0.0;
        }
        anchor
    }

    fn live(&mut self, materials: &[Material], light: Light, frames: u32) -> Vec<[f32; 3]> {
        let (width, height) = PLATE;
        let output = target(self.renderer.gpu(), width, height);
        let mut sun = self.lighting.sun(HOUR);
        if light == Light::Sky {
            sun.intensity = 0.0;
        }
        let deformers = [Deformer::Roller {
            current: roller(0.0),
            previous: roller(0.0),
        }];
        for number in 0..frames {
            let scene = Scene {
                camera: self.lens.camera(number, true, width, height),
                time: 0.0,
                seed: 23,
                sun,
                instances: self.instances,
                materials,
                deformers: &deformers,
                wind: SceneWind::default(),
            };
            self.renderer
                .render(
                    &scene,
                    &Text::default(),
                    &Effects::default(),
                    Finish::Standard,
                    &output.view,
                )
                .unwrap();
        }
        self.renderer
            .readback_geometry()
            .unwrap()
            .colour
            .into_iter()
            .map(|c| [c[0], c[1], c[2]])
            .collect()
    }

    fn reset(&mut self, anchor: &Anchor) {
        self.renderer
            .set_sky(SkySource::Hdr(anchor.sky.clone()), HOUR)
            .unwrap();
    }

    fn trace(&self, materials: &[Material], anchor: &Anchor) -> (Vec<[f32; 3]>, Vec<bool>) {
        let (width, height) = PLATE;
        let forward = unit(sub(self.lens.target, self.lens.position));
        let right = unit(cross(forward, [0.0, 1.0, 0.0]));
        let up = cross(right, forward);
        let tan_y = (self.lens.fov_y.to_radians() * 0.5).tan();
        let tan_x = tan_y * width as f32 / height as f32;
        let scene = TraceScene {
            triangles: self.triangles.to_vec(),
            shapes: Vec::new(),
            materials: materials.to_vec(),
            sky: anchor.sky.clone(),
            camera: TraceCamera {
                origin: self.lens.position,
                forward,
                right: right.map(|v| v * tan_x),
                up: up.map(|v| v * tan_y),
            },
            sun: anchor.sun,
        };
        let detail = Detail {
            lens: TraceLens {
                shift: self.lens.shift,
                ..TraceLens::default()
            },
            ..Detail::default()
        };
        let gpu = self.renderer.gpu();
        let mut trace = Trace::new_detailed(gpu, &scene, &detail, width, height).unwrap();
        for _ in 0..TRACE_SAMPLES / 4 {
            trace.sample(gpu, 4, SEED).unwrap();
        }
        let output = trace.readback(gpu).unwrap();
        let floats = |bytes: &[u8]| -> Vec<[f32; 4]> {
            bytes
                .chunks_exact(16)
                .map(|p| {
                    std::array::from_fn(|i| {
                        f32::from_le_bytes(p[i * 4..i * 4 + 4].try_into().unwrap())
                    })
                })
                .collect()
        };
        let colour = floats(&output.color)
            .into_iter()
            .map(|c| [c[0], c[1], c[2]])
            .collect();
        let covered = floats(&output.normal)
            .into_iter()
            .map(|n| n[3] > 0.99)
            .collect();
        (colour, covered)
    }

    fn bake(&self, name: &str, materials: &[Material], anchor: &Anchor) -> Grid {
        let scene = BakeScene {
            triangles: self.triangles.to_vec(),
            shapes: Vec::new(),
            materials: materials.to_vec(),
            anchors: vec![anchor.clone()],
        };
        let gpu = self.renderer.gpu();
        let grids = bake(gpu, &scene, self.spec, SAMPLES, SEED).unwrap();
        let relative = format!("bench/terms/{name}");
        let root = Path::new("tmp").join(&relative);
        if root.exists() {
            std::fs::remove_dir_all(&root).unwrap();
        }
        write_artifact(
            Path::new("tmp"),
            &Path::new(&relative).join("probes"),
            &scene,
            &grids,
            SAMPLES,
            SEED,
        )
        .unwrap();
        let cube = bake_reflection(gpu, &scene, &self.reflection, SAMPLES, SEED, 0).unwrap();
        write_reflection_artifact(ReflectionWrite {
            tmp_root: Path::new("tmp"),
            relative: &Path::new(&relative).join("reflection"),
            scene_hash: scene_hash(&scene, self.spec, SAMPLES, SEED).unwrap(),
            spec: &self.reflection,
            anchors: &scene.anchors,
            cubes: &[cube],
            samples: SAMPLES,
            seed: SEED,
        })
        .unwrap();
        grids.into_iter().next().unwrap()
    }

    fn capture(&mut self, light: Light, surface: Surface) -> (Captures, Grid) {
        let name = format!(
            "{}-{}",
            match light {
                Light::Sun => "sun",
                Light::Sky => "sky",
                Light::Both => "both",
            },
            match surface {
                Surface::Diffuse => "diffuse",
                Surface::Full => "full",
            }
        );
        let materials: Vec<Material> = match surface {
            Surface::Diffuse => self.materials.iter().map(diffuse_only).collect(),
            Surface::Full => self.materials.to_vec(),
        };
        let anchor = self.anchor(light);
        let grid = self.bake(&name, &materials, &anchor);
        let root = Path::new("tmp/bench/terms").join(&name);
        let gpu = self.renderer.gpu();
        let probes = ProbeLighting::load(
            &gpu.device,
            &gpu.queue,
            &root.join("probes"),
            HOUR,
            [-1.0; 3],
        )
        .unwrap();
        self.renderer.set_probes(probes).unwrap();
        self.renderer
            .frame()
            .set_local_reflections(
                Some(vec![
                    read_reflection_artifact(&root.join("reflection")).unwrap(),
                ]),
                HOUR,
            )
            .unwrap();
        self.reset(&anchor);
        let full = self.live(&materials, light, 32);
        self.reset(&anchor);
        let no_ssr = self.live(&materials, light, 1);
        self.renderer
            .frame()
            .set_local_reflections(None, HOUR)
            .unwrap();
        self.reset(&anchor);
        let no_local = self.live(&materials, light, 1);
        let gpu = self.renderer.gpu();
        let zero = ProbeLighting::fallback(&gpu.device, &gpu.queue, [0.0; 3]);
        self.renderer.set_probes(zero).unwrap();
        self.reset(&anchor);
        let direct = self.live(&materials, light, 1);
        let gpu = self.renderer.gpu();
        let sky = ProbeLighting::fallback(&gpu.device, &gpu.queue, [-1.0; 3]);
        self.renderer.set_probes(sky).unwrap();
        self.reset(&anchor);
        let sh = self.live(&materials, light, 1);
        let (trace, covered) = self.trace(&materials, &anchor);
        (
            Captures {
                trace,
                covered,
                full,
                no_ssr,
                no_local,
                direct,
                sh,
            },
            grid,
        )
    }

    fn probe_reference(
        &self,
        materials: &[Material],
        anchor: &Anchor,
        position: [f32; 3],
    ) -> [[f32; 3]; 6] {
        const FACES: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
            ([1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]),
            ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
            ([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]),
            ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
            ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
            ([0.0, 0.0, -1.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ];
        let gpu = self.renderer.gpu();
        let scene = TraceScene {
            triangles: self.triangles.to_vec(),
            shapes: Vec::new(),
            materials: materials.to_vec(),
            sky: anchor.sky.clone(),
            camera: TraceCamera {
                origin: position,
                forward: FACES[0].0,
                right: FACES[0].1,
                up: FACES[0].2,
            },
            sun: anchor.sun,
        };
        let mut trace = Trace::new(gpu, &scene, FACE_TEXELS, FACE_TEXELS).unwrap();
        let mut lobes = [[0.0f64; 3]; 6];
        for (forward, right, up) in FACES {
            trace
                .set_camera(TraceCamera {
                    origin: position,
                    forward,
                    right,
                    up,
                })
                .unwrap();
            for _ in 0..FACE_SAMPLES / 4 {
                trace.sample(gpu, 4, SEED).unwrap();
            }
            let bytes = trace.readback(gpu).unwrap().color;
            for y in 0..FACE_TEXELS {
                for x in 0..FACE_TEXELS {
                    let at = ((y * FACE_TEXELS + x) * 16) as usize;
                    let rgb: [f32; 3] = std::array::from_fn(|c| {
                        f32::from_le_bytes(bytes[at + c * 4..at + c * 4 + 4].try_into().unwrap())
                    });
                    let u = (x as f32 + 0.5) / FACE_TEXELS as f32 * 2.0 - 1.0;
                    let v = 1.0 - (y as f32 + 0.5) / FACE_TEXELS as f32 * 2.0;
                    let squared = 1.0 + u * u + v * v;
                    let dir: [f32; 3] = std::array::from_fn(|k| {
                        (forward[k] + right[k] * u + up[k] * v) / squared.sqrt()
                    });
                    let solid = (2.0 / FACE_TEXELS as f32).powi(2) / squared.powf(1.5);
                    for (lobe, axis) in AXES.iter().enumerate() {
                        let cosine = (0..3).map(|k| dir[k] * axis[k]).sum::<f32>().max(0.0);
                        for c in 0..3 {
                            lobes[lobe][c] += f64::from(rgb[c] * cosine * solid);
                        }
                    }
                }
            }
        }
        lobes.map(|lobe| lobe.map(|v| v as f32))
    }

    pub fn run(mut self) {
        let (width, height) = PLATE;
        self.renderer.resize(width, height).unwrap();
        let mut lit: Option<Vec<bool>> = None;
        for (light, surface) in [
            (Light::Sun, Surface::Diffuse),
            (Light::Sky, Surface::Diffuse),
            (Light::Sun, Surface::Full),
            (Light::Sky, Surface::Full),
            (Light::Both, Surface::Full),
        ] {
            let (captures, grid) = self.capture(light, surface);
            let lit = lit
                .get_or_insert_with(|| {
                    let peak = captures.direct.iter().map(|&c| luma(c)).fold(0.0, f64::max);
                    captures
                        .direct
                        .iter()
                        .map(|&c| luma(c) > peak * 0.05)
                        .collect()
                })
                .clone();
            report(light, surface, &captures, &lit);
            let name = format!("tmp/bench/terms/{}-{}.png", light.name(), surface.name());
            save_pair(Path::new(&name), &captures.trace, &captures.full);
            if light == Light::Both {
                let materials = self.materials.to_vec();
                let anchor = self.anchor(light);
                let dims = grid.dims;
                for xyz in [
                    [dims[0] / 2, dims[1] / 2, dims[2] - 2],
                    [dims[0] / 4, dims[1] / 2, dims[2] - 3],
                    [dims[0] * 3 / 4, dims[1] / 3, dims[2] - 1],
                    [dims[0] / 2, dims[1] - 2, dims[2] - 2],
                ] {
                    let index = GridSpec::index(dims, xyz);
                    let position = grid.spec.position(dims, index);
                    let reference = self.probe_reference(&materials, &anchor, position);
                    let baked = grid.probes[index].lobes;
                    let ratios: Vec<String> = (0..6)
                        .map(|lobe| {
                            format!("{:.3}", luma(baked[lobe]) / luma(reference[lobe]).max(1e-9))
                        })
                        .collect();
                    println!(
                        "probe {xyz:?} at {position:?}: baked/traced irradiance per lobe +x -x +y -y +z -z {}; traced +z luma {:.4}",
                        ratios.join(" "),
                        luma(reference[4])
                    );
                }
            }
        }
    }
}

struct Region {
    name: &'static str,
    keep: Box<dyn Fn(usize, usize) -> bool>,
}

fn report(light: Light, surface: Surface, c: &Captures, lit: &[bool]) {
    let (width, height) = PLATE;
    let w = width as usize;
    let h = height as usize;
    let face = move |x: usize, y: usize| {
        let fx = x as f32 / w as f32;
        let fy = y as f32 / h as f32;
        (0.15..0.9).contains(&fx) && (0.55..0.95).contains(&fy)
    };
    let lit_owned = lit.to_vec();
    let shade_owned = lit.to_vec();
    let regions = [
        Region {
            name: "frame",
            keep: Box::new(|_, _| true),
        },
        Region {
            name: "table",
            keep: Box::new(face),
        },
        Region {
            name: "sunlit",
            keep: Box::new(move |x, y| lit_owned[y * w + x]),
        },
        Region {
            name: "shaded",
            keep: Box::new(move |x, y| !shade_owned[y * w + x]),
        },
    ];
    println!(
        "== {} light, {} surfaces: mean linear luma (trace | live full, no SSR, no local cube, no probes, SH instead of probes) and live terms",
        match light {
            Light::Sun => "sun",
            Light::Sky => "sky",
            Light::Both => "sun and sky",
        },
        match surface {
            Surface::Diffuse => "diffuse",
            Surface::Full => "full",
        }
    );
    for region in &regions {
        let mut sums = [0.0f64; 6];
        let mut count = 0.0f64;
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                if !c.covered[i] || !(region.keep)(x, y) {
                    continue;
                }
                count += 1.0;
                for (sum, image) in sums.iter_mut().zip([
                    &c.trace,
                    &c.full,
                    &c.no_ssr,
                    &c.no_local,
                    &c.direct,
                    &c.sh,
                ]) {
                    *sum += luma(image[i]);
                }
            }
        }
        let m = sums.map(|s| s / count.max(1.0));
        println!(
            "  {:7} {:8.0} px: trace {:.4} | live {:.4} {:.4} {:.4} {:.4} {:.4} | live/trace {:.3}; ssr {:+.4} local {:+.4} probes {:.4} direct+sky cube {:.4}; trace minus live direct {:.4}, probes/that {:.3}",
            region.name,
            count,
            m[0],
            m[1],
            m[2],
            m[3],
            m[4],
            m[5],
            m[1] / m[0].max(1e-9),
            m[1] - m[2],
            m[2] - m[3],
            m[3] - m[4],
            m[4],
            m[0] - m[4],
            (m[3] - m[4]) / (m[0] - m[4]).max(1e-9),
        );
    }
    for (label, fx, fy) in [
        ("centre", 0.5, 0.5),
        ("window", 0.08, 0.4),
        ("shelves", 0.6, 0.3),
        ("table", 0.4, 0.75),
        ("floor", 0.6, 0.97),
        ("cavity", 0.9, 0.85),
    ] {
        let cx = (fx * w as f32) as usize;
        let cy = (fy * h as f32) as usize;
        let mut sums = [0.0f64; 5];
        let mut count = 0.0f64;
        for y in cy.saturating_sub(4)..(cy + 5).min(h) {
            for x in cx.saturating_sub(4)..(cx + 5).min(w) {
                let i = y * w + x;
                count += 1.0;
                for (sum, image) in
                    sums.iter_mut()
                        .zip([&c.trace, &c.full, &c.no_ssr, &c.no_local, &c.direct])
                {
                    *sum += luma(image[i]);
                }
            }
        }
        let m = sums.map(|s| s / count);
        println!(
            "  pixel {label:10} trace {:.4} live {:.4} ({:.3}); probes {:.4} vs trace-direct {:.4}",
            m[0],
            m[1],
            m[1] / m[0].max(1e-9),
            m[3] - m[4],
            m[0] - m[4]
        );
    }
}
