use crate::desk::{
    Desk, EYE, FONT, HOUR, SEED, STEAM_SOURCE, TARGET, TOP, flat, look, material, projection, sub,
    translate,
};
use crate::output::output;
use pfx_core::daylight::{Daylight, REFERENCE_HOUR};
use pfx_geom::shapes::Shape;
use pfx_gpu::pace::Turns;
use pfx_gpu::{Gpu, OffscreenTarget, wgpu};
use pfx_live::effects::{ParticleInstance, Plume, VolumeKind, pack_particles};
use pfx_live::frame::{Camera, Instance, Matrix, MeshData, Scene, SceneWind, Sun, multiply};
use pfx_live::glass;
use pfx_live::renderer::{Effects, Exposure, Finish, Renderer, Text, TextItem, Volume};
use pfx_live::sky::{AnalyticSky, SkySource};
use pfx_live::text::{GpuAtlas, TextSpace};
use pfx_materials::Material;
use pfx_physics::{ParticleKind, Speck};
use pfx_post::dof::Lens;
use pfx_text::{Baseline, Paragraph, Representation, Style, TextEngine};

pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

pub struct Room {
    pub renderer: Renderer,
    pub output: OffscreenTarget,
    instances: Vec<Instance>,
    materials: Vec<Material>,
    glass: glass::MeshHandle,
    atlas: GpuAtlas,
    paragraph: Paragraph,
    overlay_atlas: GpuAtlas,
    overlay_paragraph: Paragraph,
    previous: Option<Matrix>,
    turns: Turns,
}

pub fn eye(frame: u32) -> [f32; 3] {
    let time = frame as f32 / 60.0;
    let yaw = (8.0 * (time * 0.9).sin()).to_radians();
    let offset = sub(EYE, TARGET);
    let (s, c) = yaw.sin_cos();
    [
        TARGET[0] + offset[0] * c + offset[2] * s,
        TARGET[1] + offset[1],
        TARGET[2] - offset[0] * s + offset[2] * c,
    ]
}

pub fn lens() -> Lens {
    Lens {
        focal_m: 0.05,
        sensor_m: Lens::sensor_for_view(0.05, 1.2, 0.36),
        fstop: 2.0,
        aperture: 0.5,
        focus_m: 1.2,
        near_m: 1.0,
        near_blur: 6.0,
        far_m: 1.6,
        far_blur: 10.0,
        ground_m: None,
    }
}

pub fn particles(time: f32) -> Vec<ParticleInstance> {
    let specks: Vec<Speck> = (0..48u32)
        .map(|index| {
            let k = index as f32;
            Speck {
                pos: [
                    -0.4 + 0.017 * k,
                    TOP + 0.05 + 0.006 * ((k * 1.7).sin() + 1.0) * 10.0,
                    0.1 + 0.05 * (k * 0.9).cos(),
                ],
                vel: [0.01, 0.02, 0.0],
                age: 0.2 + 0.01 * k,
                life: 4.0,
                seed: index * 7 + 3,
            }
        })
        .collect();
    pack_particles(&specks, ParticleKind::Dust, time, [0.95, 0.9, 0.8])
}

impl Room {
    pub fn new(gpu: Gpu, width: u32, height: u32) -> Self {
        let output = output(&gpu, width, height);
        let mut renderer = Renderer::new(gpu, width, height).unwrap();
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
        let mut instances = Vec::new();
        let materials = vec![
            material([0.42, 0.3, 0.2], 0.55),
            material([0.78, 0.76, 0.72], 0.9),
            Material {
                base: [0.36, 0.06, 0.05],
                roughness: 0.2,
                clearcoat: 1.0,
                ..Material::default()
            },
            Material {
                base: [0.92, 0.92, 0.92],
                roughness: 0.08,
                metalness: 1.0,
                ..Material::default()
            },
            material([0.9, 0.88, 0.82], 0.85),
            material([0.12, 0.4, 0.22], 0.6),
        ];
        let mut add = |renderer: &mut Renderer, shape: Shape, model: Matrix, material: u32| {
            let mesh = shape.mesh(0.004);
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
        add(
            &mut renderer,
            Shape::Ellipsoid {
                radius: 0.08,
                squash: 1.0,
            },
            translate([0.1, TOP + 0.08, -0.22]),
            3,
        );
        add(
            &mut renderer,
            block([0.12, 0.02, 0.16], 0.004),
            translate([-0.45, TOP + 0.02, -0.12]),
            5,
        );
        add(
            &mut renderer,
            block([0.16, 0.002, 0.1], 0.001),
            translate([0.0, TOP + 0.002, 0.16]),
            4,
        );
        let slab = Shape::RoundBox {
            half: [0.07, 0.09, 0.02],
            radius: 0.01,
        }
        .mesh(0.01);
        let glass = renderer.upload_glass(&slab).unwrap();
        let mut engine = TextEngine::new(FONT).unwrap();
        let paragraph = engine
            .layout(
                "A viewport keeps\nthe history",
                Style {
                    family: "EB Garamond",
                    size: 0.03,
                    line_height: 0.04,
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
                "VIEWPORT",
                Style {
                    family: "EB Garamond",
                    size: 18.0,
                    line_height: 22.0,
                    wrap_width: None,
                    pixels_per_unit: 1.0,
                    representation: Representation::Coverage,
                },
                &Baseline::Straight {
                    origin: [10.0, 20.0],
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
            instances,
            materials,
            glass,
            atlas,
            paragraph,
            overlay_atlas,
            overlay_paragraph,
            previous: None,
            turns: Turns::default(),
        }
    }

    pub fn draw(&mut self, frame: u32) {
        let (width, height) = self.renderer.size();
        let time = frame as f32 / 60.0;
        let eye = eye(frame);
        let view = look(eye, TARGET);
        let projection = projection(width, height, 0.0);
        let current = multiply(projection, view);
        let camera = Camera {
            view,
            projection,
            previous_view_projection: self.previous.unwrap_or(current),
            position: eye,
        };
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
            instances: &self.instances,
            materials: &self.materials,
            deformers: &[],
            wind: SceneWind::default(),
        };
        let surface = vec![TextItem {
            atlas: &self.atlas,
            paragraph: &self.paragraph,
            color: [0.03, 0.025, 0.02, 1.0],
            space: TextSpace::Surface {
                model_view_projection: multiply(current, flat([-0.14, TOP + 0.0045, 0.12])),
            },
            id: 40,
        }];
        let overlay = vec![TextItem {
            atlas: &self.overlay_atlas,
            paragraph: &self.overlay_paragraph,
            color: [0.95, 0.93, 0.88, 1.0],
            space: TextSpace::Overlay { width, height },
            id: 0,
        }];
        let text = Text {
            surface,
            overlay,
            surface_quads: Vec::new(),
            overlay_quads: Vec::new(),
        };
        let glass = [glass::Surface {
            mesh: self.glass,
            model: translate([-0.2, TOP + 0.09, -0.05]),
            material: pfx_materials::fixtures::glass(),
            liquid: false,
            fluid_height: 0.0,
            ripple_height: 0.0,
            caustic_strength: 0.0,
            tinted: None,
            id: 0,
            casts_shadow: false,
            shadow_only: false,
            clip: [[0.0; 4]; 2],
        }];
        let particles = particles(time);
        let effects = Effects {
            volume: Some(Volume {
                kind: VolumeKind::Plume(Plume::at(STEAM_SOURCE)),
                lo: Desk::STEAM_BOX.0,
                hi: Desk::STEAM_BOX.1,
                color: [0.82, 0.85, 0.88],
                density: 1.6,
                anisotropy: 0.25,
                time_scale: 1.0,
            }),
            particles: &particles,
            glass: &glass,
            ..Effects::default()
        };
        let renderer = &mut self.renderer;
        let output = &self.output.view;
        self.turns.time(|| {
            renderer
                .render(&scene, &text, &effects, Finish::Standard, output)
                .unwrap()
        });
        self.previous = Some(current);
    }

    pub fn pixels(&mut self) -> Vec<u16> {
        self.turns.turn();
        self.renderer.gpu().readback_rgba16(&self.output).unwrap()
    }
}
