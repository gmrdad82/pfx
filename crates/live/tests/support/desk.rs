use pfx_core::daylight::{Daylight, REFERENCE_HOUR};
use pfx_geom::mesh::Mesh;
use pfx_geom::shapes::Shape;
use pfx_gpu::pace::Turns;
use pfx_gpu::{Gpu, OffscreenTarget, PassTiming, wgpu};
use pfx_live::demand::{Limits, Need, Next, Region};
use pfx_live::effects::{Plume, VolumeKind};
use pfx_live::frame::{Camera, Instance, Matrix, MeshData, Scene, SceneWind, Sun, multiply};
use pfx_live::renderer::{Effects, Exposure, Finish, Renderer, Text, TextItem, Volume};
use pfx_live::sky::{AnalyticSky, SkySource};
use pfx_live::text::{GpuAtlas, TextSpace};
use pfx_materials::Material;
use pfx_text::{Baseline, Paragraph, Representation, Style, TextEngine};

pub const FONT: &[u8] = pfx_text::fixture::EB_GARAMOND;
pub const TOP: f32 = 0.775;
pub const TARGET: [f32; 3] = [0.0, 0.8, -0.05];
pub const EYE: [f32; 3] = [0.0, 1.22, 1.05];
pub const FOV_Y: f32 = 40.0;
pub const NEAR: f32 = 0.05;
pub const FAR: f32 = 30.0;
pub const HOUR: f64 = 16.0;
pub const SEED: u32 = 11;
pub const STEAM_SOURCE: [f32; 3] = [0.35, TOP + 0.105, 0.05];

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pose {
    pub yaw: f32,
    pub pitch: f32,
    pub push: f32,
    pub zoom: f32,
}

impl Pose {
    pub fn drift(time: f32) -> Self {
        Self {
            yaw: 0.35 * (time * 0.41).sin() + 0.12 * (time * 1.13).sin(),
            pitch: 0.15 * (time * 0.57).sin(),
            push: 0.004 * (time * 0.29).sin(),
            zoom: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Show {
    pub steam: bool,
    pub overlay: bool,
    pub content: u64,
    pub overlay_revision: u64,
    pub card_shift: f32,
    pub card_tint: f32,
    pub text_shift: f32,
}

pub fn identity() -> Matrix {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

pub fn translate(at: [f32; 3]) -> Matrix {
    let mut model = identity();
    model[3] = [at[0], at[1], at[2], 1.0];
    model
}

fn turn_y(at: [f32; 3], angle: f32) -> Matrix {
    let (s, c) = angle.sin_cos();
    [
        [c, 0.0, -s, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [s, 0.0, c, 0.0],
        [at[0], at[1], at[2], 1.0],
    ]
}

pub fn flat(at: [f32; 3]) -> Matrix {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, -1.0, 0.0, 0.0],
        [at[0], at[1], at[2], 1.0],
    ]
}

pub fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub fn unit(v: [f32; 3]) -> [f32; 3] {
    let length = dot(v, v).sqrt();
    [v[0] / length, v[1] / length, v[2] / length]
}

pub fn look(eye: [f32; 3], target: [f32; 3]) -> Matrix {
    let forward = unit(sub(target, eye));
    let right = unit(cross(forward, [0.0, 1.0, 0.0]));
    let up = cross(right, forward);
    [
        [right[0], up[0], -forward[0], 0.0],
        [right[1], up[1], -forward[1], 0.0],
        [right[2], up[2], -forward[2], 0.0],
        [-dot(right, eye), -dot(up, eye), dot(forward, eye), 1.0],
    ]
}

pub fn projection(width: u32, height: u32, zoom: f32) -> Matrix {
    let f = (1.0 + zoom) / (FOV_Y.to_radians() * 0.5).tan();
    let aspect = width as f32 / height as f32;
    [
        [f / aspect, 0.0, 0.0, 0.0],
        [0.0, f, 0.0, 0.0],
        [0.0, 0.0, FAR / (NEAR - FAR), -1.0],
        [0.0, 0.0, FAR * NEAR / (NEAR - FAR), 0.0],
    ]
}

pub fn eye(pose: Pose) -> [f32; 3] {
    let offset = sub(EYE, TARGET);
    let (sy, cy) = pose.yaw.to_radians().sin_cos();
    let turned = [
        offset[0] * cy + offset[2] * sy,
        offset[1],
        -offset[0] * sy + offset[2] * cy,
    ];
    let flat_length = (turned[0] * turned[0] + turned[2] * turned[2]).sqrt();
    let (sp, cp) = pose.pitch.to_radians().sin_cos();
    let raised_flat = flat_length * cp - turned[1] * sp;
    let raised_y = flat_length * sp + turned[1] * cp;
    let scale = 1.0 - pose.push;
    [
        TARGET[0] + turned[0] / flat_length * raised_flat * scale,
        TARGET[1] + raised_y * scale,
        TARGET[2] + turned[2] / flat_length * raised_flat * scale,
    ]
}

fn mesh(shape: Shape) -> Mesh {
    shape.mesh(0.002)
}

pub fn material(base: [f32; 3], roughness: f32) -> Material {
    Material {
        base,
        roughness,
        ..Material::default()
    }
}

pub struct Desk {
    pub renderer: Renderer,
    pub output: OffscreenTarget,
    pub width: u32,
    pub height: u32,
    instances: Vec<Instance>,
    materials: Vec<Material>,
    card: usize,
    atlas: GpuAtlas,
    paragraph: Paragraph,
    overlay_atlas: GpuAtlas,
    overlay_paragraph: Paragraph,
    previous: Option<Matrix>,
    turns: Turns,
    pub steam_box: ([f32; 3], [f32; 3]),
}

impl Desk {
    pub fn new(width: u32, height: u32) -> Self {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        Self::with_gpu(gpu, width, height)
    }

    pub fn with_gpu(gpu: Gpu, width: u32, height: u32) -> Self {
        let output = gpu
            .offscreen(width, height, wgpu::TextureFormat::Rgba8UnormSrgb)
            .unwrap();
        let mut renderer = Renderer::new_with_output_format(
            gpu,
            width,
            height,
            wgpu::TextureFormat::Rgba8UnormSrgb,
        )
        .unwrap();
        renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
        let daylight = Daylight {
            hour: HOUR,
            day: Daylight::DAY,
            latitude: Daylight::LATITUDE,
            heading: Daylight::HEADING,
        };
        renderer
            .set_sky(
                SkySource::Analytic(AnalyticSky::new(
                    daylight,
                    REFERENCE_HOUR,
                    2.6,
                    [0.32, 0.3, 0.27],
                )),
                HOUR as f32,
            )
            .unwrap();
        renderer
            .set_frames_on_demand(Some(Limits::default()))
            .unwrap();
        let mut instances = Vec::new();
        let materials = vec![
            material([0.42, 0.3, 0.2], 0.55),
            material([0.78, 0.76, 0.72], 0.9),
            Material {
                base: [0.36, 0.06, 0.05],
                roughness: 0.4,
                clearcoat: 1.0,
                ..Material::default()
            },
            material([0.2, 0.32, 0.55], 0.5),
            Material {
                base: [0.92, 0.92, 0.92],
                roughness: 0.12,
                metalness: 1.0,
                ..Material::default()
            },
            material([0.9, 0.88, 0.82], 0.85),
            material([0.12, 0.4, 0.22], 0.6),
            material([0.08, 0.08, 0.08], 0.35),
        ];
        let mut add = |renderer: &mut Renderer, shape: Shape, model: Matrix, material: u32| {
            let mesh = mesh(shape);
            let handle = renderer
                .upload_mesh(MeshData {
                    positions: &mesh.positions,
                    normals: &mesh.normals,
                    tangents: &mesh.tangents,
                    uvs: &mesh.uvs,
                    uvs1: None,
                    alpha: None,
                    indices: &mesh.indices,
                })
                .unwrap();
            let id = instances.len() as u32 + 1;
            instances.push(Instance::new(handle, model, material, id));
            instances.len() - 1
        };
        let block = |half: [f32; 3], radius: f32| Shape::RoundBox { half, radius };
        add(
            &mut renderer,
            block([3.0, 0.02, 3.0], 0.005),
            translate([0.0, -0.02, 0.0]),
            0,
        );
        add(
            &mut renderer,
            block([3.0, 1.6, 0.03], 0.005),
            translate([0.0, 1.6, -1.2]),
            1,
        );
        add(
            &mut renderer,
            block([0.8, 0.025, 0.45], 0.01),
            translate([0.0, TOP - 0.025, 0.0]),
            2,
        );
        for (x, z) in [(-0.74, -0.39), (0.74, -0.39), (-0.74, 0.39), (0.74, 0.39)] {
            add(
                &mut renderer,
                block([0.03, 0.36, 0.03], 0.01),
                translate([x, 0.36, z]),
                7,
            );
        }
        add(
            &mut renderer,
            block([0.12, 0.02, 0.16], 0.004),
            turn_y([-0.45, TOP + 0.02, -0.12], 0.2),
            6,
        );
        add(
            &mut renderer,
            block([0.11, 0.018, 0.15], 0.004),
            turn_y([-0.44, TOP + 0.058, -0.11], -0.15),
            3,
        );
        add(
            &mut renderer,
            Shape::RoundCone {
                a: [0.0, 0.0, 0.0],
                b: [0.0, 0.1, 0.0],
                r1: 0.04,
                r2: 0.045,
            },
            translate([0.35, TOP, 0.05]),
            3,
        );
        add(
            &mut renderer,
            Shape::Ellipsoid {
                radius: 0.06,
                squash: 1.0,
            },
            translate([0.08, TOP + 0.06, -0.24]),
            4,
        );
        add(
            &mut renderer,
            Shape::Capsule {
                a: [0.0, 0.0, 0.0],
                b: [0.0, 0.42, 0.0],
                radius: 0.012,
            },
            translate([-0.62, TOP, -0.3]),
            7,
        );
        let card = add(
            &mut renderer,
            block([0.16, 0.002, 0.1], 0.001),
            translate([0.0, TOP + 0.002, 0.16]),
            5,
        );
        let mut engine = TextEngine::new(FONT).unwrap();
        let paragraph = engine
            .layout(
                "Frames on demand keep the desk\nstill when nothing moves",
                Style {
                    family: "EB Garamond",
                    size: 0.021,
                    line_height: 0.028,
                    wrap_width: None,
                    pixels_per_unit: 2400.0,
                    representation: Representation::Msdf,
                },
                &Baseline::Straight {
                    origin: [0.0, 0.0],
                    direction: [1.0, 0.0],
                },
            )
            .unwrap();
        let atlas = GpuAtlas::new(
            &renderer.gpu().device,
            &renderer.gpu().queue,
            &paragraph.atlas,
        )
        .unwrap();
        let overlay_paragraph = engine
            .layout(
                "DESK  ·  ON DEMAND",
                Style {
                    family: "EB Garamond",
                    size: height as f32 * 0.03,
                    line_height: height as f32 * 0.036,
                    wrap_width: None,
                    pixels_per_unit: 1.0,
                    representation: Representation::Coverage,
                },
                &Baseline::Straight {
                    origin: [height as f32 * 0.04, height as f32 * 0.07],
                    direction: [1.0, 0.0],
                },
            )
            .unwrap();
        let overlay_atlas = GpuAtlas::new(
            &renderer.gpu().device,
            &renderer.gpu().queue,
            &overlay_paragraph.atlas,
        )
        .unwrap();
        Self {
            renderer,
            output,
            width,
            height,
            instances,
            materials,
            card,
            atlas,
            paragraph,
            overlay_atlas,
            overlay_paragraph,
            previous: None,
            turns: Turns::default(),
            steam_box: Self::STEAM_BOX,
        }
    }

    pub fn camera(&self, pose: Pose) -> Camera {
        let eye = eye(pose);
        let view = look(eye, TARGET);
        let projection = projection(self.width, self.height, pose.zoom);
        let current = multiply(projection, view);
        Camera {
            view,
            projection,
            previous_view_projection: self.previous.unwrap_or(current),
            position: eye,
        }
    }

    pub const STEAM_BOX: ([f32; 3], [f32; 3]) = (
        [
            STEAM_SOURCE[0] - 0.1,
            STEAM_SOURCE[1],
            STEAM_SOURCE[2] - 0.1,
        ],
        [
            STEAM_SOURCE[0] + 0.1,
            STEAM_SOURCE[1] + 0.32,
            STEAM_SOURCE[2] + 0.1,
        ],
    );

    pub fn steam(&self) -> Volume {
        Volume {
            kind: VolumeKind::Plume(Plume::at(STEAM_SOURCE)),
            lo: self.steam_box.0,
            hi: self.steam_box.1,
            color: [0.82, 0.85, 0.88],
            density: 1.6,
            anisotropy: 0.25,
            time_scale: 1.0,
        }
    }

    pub fn with<R>(
        &mut self,
        pose: Pose,
        time: f32,
        show: Show,
        run: impl FnOnce(&mut Renderer, &Next<'_>, &wgpu::TextureView) -> R,
    ) -> R {
        let camera = self.camera(pose);
        let mut instances = self.instances.clone();
        let card = &mut instances[self.card];
        card.model[3][0] += show.card_shift;
        card.previous_model = card.model;
        let mut materials = self.materials.clone();
        materials[5].base[2] -= show.card_tint;
        let scene = Scene {
            camera,
            time,
            seed: SEED,
            sun: Sun::from_daylight(
                Daylight {
                    hour: HOUR,
                    day: Daylight::DAY,
                    latitude: Daylight::LATITUDE,
                    heading: Daylight::HEADING,
                },
                REFERENCE_HOUR,
            ),
            instances: &instances,
            materials: &materials,
            deformers: &[],
            wind: SceneWind::default(),
        };
        let view_projection = multiply(camera.projection, camera.view);
        let model = flat([-0.145 + show.text_shift, TOP + 0.0045, 0.09]);
        let surface = vec![TextItem {
            atlas: &self.atlas,
            paragraph: &self.paragraph,
            color: [0.03, 0.025, 0.02, 1.0],
            space: TextSpace::Surface {
                model_view_projection: multiply(view_projection, model),
            },
            id: 40,
        }];
        let overlay = if show.overlay {
            vec![TextItem {
                atlas: &self.overlay_atlas,
                paragraph: &self.overlay_paragraph,
                color: [0.95, 0.93, 0.88, 1.0],
                space: TextSpace::Overlay {
                    width: self.width,
                    height: self.height,
                },
                id: 0,
            }]
        } else {
            Vec::new()
        };
        let text = Text {
            surface,
            overlay,
            surface_quads: Vec::new(),
            overlay_quads: Vec::new(),
        };
        let effects = Effects {
            volume: show.steam.then(|| self.steam()),
            ..Effects::default()
        };
        let mut next = Next::new(&scene, &text, &effects, Finish::Standard);
        next.content = show.content;
        next.overlay = show.overlay_revision;
        let result = run(&mut self.renderer, &next, &self.output.view);
        self.previous = Some(view_projection);
        result
    }

    pub fn needed(&mut self, pose: Pose, time: f32, show: Show) -> Need {
        let previous = self.previous;
        let need = self.with(pose, time, show, |renderer, next, _| {
            renderer.frame_needed(next)
        });
        self.previous = previous;
        need
    }

    pub fn draw(
        &mut self,
        pose: Pose,
        time: f32,
        show: Show,
        need: Option<Need>,
    ) -> (Need, Vec<PassTiming>) {
        let mut turns = std::mem::take(&mut self.turns);
        let result = self.with(pose, time, show, |renderer, next, output| {
            let need = need.unwrap_or_else(|| renderer.frame_needed(next));
            let timings = turns.time(|| renderer.render_needed(next, &need, output).unwrap());
            (need, timings.unwrap_or_default())
        });
        self.turns = turns;
        result
    }

    pub fn bridge(&mut self, pose: Pose, time: f32, show: Show) -> Option<Vec<PassTiming>> {
        let previous = self.previous;
        let timings = self.with(pose, time, show, |renderer, next, output| {
            renderer.render_bridge(next, output).unwrap()
        });
        self.previous = previous;
        timings
    }

    pub fn settle(&mut self, pose: Pose, time: f32, show: Show) -> u32 {
        let mut drawn = 0;
        loop {
            let (need, _) = self.draw(pose, time, show, None);
            if need == Need::None {
                return drawn;
            }
            drawn += 1;
            assert!(drawn < 64, "the desk never settles: {need:?}");
        }
    }

    pub fn pixels(&mut self) -> Vec<u8> {
        self.turns.turn();
        self.renderer.gpu().readback_rgba8(&self.output).unwrap()
    }

    pub fn region_of_steam(&self, pose: Pose) -> Region {
        let camera = self.camera(pose);
        let steam = self.steam();
        pfx_live::demand::box_region(
            &multiply(camera.projection, camera.view),
            steam.lo,
            steam.hi,
            8 + pfx_live::demand::MARGIN,
            self.width,
            self.height,
        )
        .unwrap()
    }
}

pub fn save_png(path: &std::path::Path, width: u32, height: u32, pixels: &[u8]) {
    let file = std::fs::File::create(path).unwrap();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(pixels)
        .unwrap();
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Error {
    pub mean: f64,
    pub visible: f64,
    pub worst: u8,
}

pub fn error(a: &[u8], b: &[u8]) -> Error {
    let mut total = 0u64;
    let mut visible = 0u64;
    let mut worst = 0u8;
    let pixels = a.len() / 4;
    for (x, y) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
        let mut largest = 0u8;
        for c in 0..3 {
            let d = x[c].abs_diff(y[c]);
            total += u64::from(d);
            largest = largest.max(d);
        }
        if largest > 12 {
            visible += 1;
        }
        worst = worst.max(largest);
    }
    Error {
        mean: total as f64 / (pixels * 3) as f64,
        visible: visible as f64 / pixels as f64,
        worst,
    }
}

pub fn kuwahara_finish() -> pfx_post::Chain {
    use pfx_post::Pass;
    use pfx_post::grade::Vignette;
    pfx_post::Chain {
        passes: vec![
            Pass::Kuwahara(3),
            Pass::Vignette(Vignette::power(0.4, 2.0, 1.0, 1.0)),
            Pass::Tone(pfx_post::Tone::aces()),
        ],
        frame: 0,
        seed: 31,
    }
}

impl Desk {
    pub fn fit_steam(&mut self, width: u32, height: u32) -> Region {
        let source = STEAM_SOURCE;
        let search = |wide: bool, goal: u32, desk: &mut Self| {
            let (mut lo, mut hi) = (0.0005f32, 0.5f32);
            for _ in 0..40 {
                let mid = 0.5 * (lo + hi);
                if wide {
                    desk.steam_box.0[0] = source[0] - mid;
                    desk.steam_box.0[2] = source[2] - mid;
                    desk.steam_box.1[0] = source[0] + mid;
                    desk.steam_box.1[2] = source[2] + mid;
                } else {
                    desk.steam_box.1[1] = source[1] + mid;
                }
                let region = desk.region_of_steam(Pose::default());
                let size = if wide { region.width } else { region.height };
                if size > goal {
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
        };
        for _ in 0..4 {
            search(true, width, self);
            search(false, height, self);
        }
        self.region_of_steam(Pose::default())
    }
}
