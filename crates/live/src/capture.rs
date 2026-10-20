use pfx_bake::FileEntry;
use pfx_bake::reflection::{
    FACES, REFLECTION_FORMAT, ReflectionArtifact, ReflectionManifest, ReflectionSpec, prefilter,
};
use pfx_gpu::Gpu;
use pfx_load::scene::{Draws, Scene};
use pfx_trace::bvh::{Bvh, Ray, Triangle};
use pfx_trace::detail::Lens;
use pfx_trace::sky::Environment;
use pfx_trace::stage::Staged;
use pfx_trace::{Camera, Projection};
use std::sync::mpsc::{Receiver, TryRecvError, sync_channel};

use crate::sky::SkySource;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneReflections {
    pub resolution: u32,
    pub samples: u32,
    pub fade: f32,
    pub margin: f32,
    pub seed: u32,
}

impl Default for SceneReflections {
    fn default() -> Self {
        Self {
            resolution: 256,
            samples: 16,
            fade: 0.25,
            margin: 0.5,
            seed: 7,
        }
    }
}

impl SceneReflections {
    pub fn valid(&self) -> bool {
        (4..=1024).contains(&self.resolution)
            && self.resolution.is_power_of_two()
            && self.samples > 0
            && self.fade.is_finite()
            && self.fade >= 0.0
            && self.margin.is_finite()
            && self.margin >= 0.0
    }
}

fn transform(model: &[[f32; 4]; 4], p: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|k| {
        model[0][k] * p[0] + model[1][k] * p[1] + model[2][k] * p[2] + model[3][k]
    })
}

pub fn world_triangles(draws: &Draws) -> Result<Vec<Triangle>, String> {
    let count = draws
        .items
        .iter()
        .map(|draw| draw.geometry.indices.len() / 3)
        .sum();
    let mut out = pfx_core::memory::vec(count, "the scene reflections' triangles")?;
    for draw in &draws.items {
        let geometry = &draw.geometry;
        let world: Vec<[f32; 3]> = geometry
            .positions
            .iter()
            .map(|&p| transform(&draw.model, p))
            .collect();
        for tri in geometry.indices.chunks_exact(3) {
            out.push(Triangle {
                vertices: [0, 1, 2].map(|k| world[tri[k] as usize]),
                material: draw.material,
            });
        }
    }
    Ok(out)
}

pub fn bounds(triangles: &[Triangle]) -> Option<[[f32; 3]; 2]> {
    let mut points = triangles.iter().flat_map(|t| t.vertices);
    let first = points.next()?;
    Some(points.fold([first, first], |[lo, hi], p| {
        [
            std::array::from_fn(|k| lo[k].min(p[k])),
            std::array::from_fn(|k| hi[k].max(p[k])),
        ]
    }))
}

fn face_texels(
    face: usize,
    bytes: &[u8],
    size: u32,
    eye: [f32; 3],
    bvh: &Bvh,
    triangles: &[Triangle],
    sky: &SkySource,
) -> Vec<[f32; 4]> {
    bytes
        .chunks_exact(16)
        .enumerate()
        .map(|(index, pixel)| {
            let direction = pfx_bake::reflection::face_direction(
                face,
                index as u32 % size,
                index as u32 / size,
                size,
            );
            let ray = Ray {
                origin: eye,
                direction,
            };
            match bvh.intersect(triangles, ray, f32::INFINITY) {
                Some(hit) => {
                    let rgb: [f32; 3] = std::array::from_fn(|c| {
                        f32::from_le_bytes(pixel[c * 4..c * 4 + 4].try_into().unwrap()).max(0.0)
                    });
                    [rgb[0], rgb[1], rgb[2], 1.0 / hit.distance.max(0.001)]
                }
                None => {
                    let open = sky.diffuse(direction);
                    [open[0], open[1], open[2], 0.0]
                }
            }
        })
        .collect()
}

pub struct Prepared {
    staged: Staged<Environment>,
    triangles: Vec<Triangle>,
    eye: [f32; 3],
}

pub fn prepare(scene: &Scene) -> Result<Option<Prepared>, String> {
    let triangles = world_triangles(&scene.draws())?;
    if bounds(&triangles).is_none() {
        return Ok(None);
    }
    Ok(Some(Prepared {
        staged: pfx_trace::stage::scene_at(scene, 1.0, 0.0)?,
        triangles,
        eye: scene.camera_or_default().at,
    }))
}

impl Prepared {
    pub fn capture(
        self,
        gpu: &Gpu,
        sky: &SkySource,
        settings: &SceneReflections,
        hour: f32,
    ) -> Result<Option<ReflectionArtifact>, String> {
        capture_staged(
            gpu,
            self.staged,
            &self.triangles,
            self.eye,
            (sky, settings),
            hour,
        )
    }
}

pub fn capture(
    gpu: &Gpu,
    scene: &Scene,
    sky: &SkySource,
    settings: &SceneReflections,
    hour: f32,
) -> Result<Option<ReflectionArtifact>, String> {
    match prepare(scene)? {
        Some(prepared) => prepared.capture(gpu, sky, settings, hour),
        None => Ok(None),
    }
}

pub type Captured = Result<Option<ReflectionArtifact>, String>;

pub struct Pending {
    arrival: Receiver<Captured>,
    pub hour: f32,
}

impl Pending {
    pub fn spawn(
        gpu: Gpu,
        prepared: Prepared,
        sky: SkySource,
        settings: SceneReflections,
        hour: f32,
    ) -> Result<Self, String> {
        let (sender, arrival) = sync_channel(1);
        std::thread::Builder::new()
            .name("pfx scene reflections".into())
            .spawn(move || {
                let _ = sender.send(prepared.capture(&gpu, &sky, &settings, hour));
            })
            .map_err(|error| format!("the scene reflections' thread: {error}"))?;
        Ok(Self { arrival, hour })
    }

    pub fn try_take(&self) -> Option<Captured> {
        match self.arrival.try_recv() {
            Ok(captured) => Some(captured),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                Some(Err("the scene reflections' capture stopped".into()))
            }
        }
    }

    pub fn take(&self) -> Captured {
        self.arrival
            .recv()
            .unwrap_or_else(|_| Err("the scene reflections' capture stopped".into()))
    }
}

pub fn capture_staged(
    gpu: &Gpu,
    staged: Staged<Environment>,
    triangles: &[Triangle],
    eye: [f32; 3],
    (sky, settings): (&SkySource, &SceneReflections),
    hour: f32,
) -> Result<Option<ReflectionArtifact>, String> {
    if !settings.valid() {
        return Err("scene reflections need a power-of-two resolution from 4 to 1024, samples and a finite fade and margin".into());
    }
    let Some([lo, hi]) = bounds(triangles) else {
        return Ok(None);
    };
    let min = std::array::from_fn(|k| lo[k].min(eye[k]) - settings.margin);
    let max = std::array::from_fn(|k| hi[k].max(eye[k]) + settings.margin);
    let size = settings.resolution;
    let mut trace = staged.trace(gpu, size, size).map_err(|e| e.to_string())?;
    trace.set_projection(Projection::Perspective)?;
    trace.set_lens(Lens::default())?;
    let bvh = Bvh::try_build(triangles).map_err(|e| format!("the scene reflections: {e}"))?;
    let mut colours = Vec::with_capacity(6);
    for (face, &(forward, right, up)) in FACES.iter().enumerate() {
        trace.set_camera(Camera {
            origin: eye,
            forward,
            right,
            up,
        })?;
        for pass in 0..settings.samples {
            trace
                .sample(
                    gpu,
                    1,
                    settings
                        .seed
                        .wrapping_add(face as u32 * 0x9e37)
                        .wrapping_add(pass),
                )
                .map_err(|e| e.to_string())?;
        }
        colours.push(trace.readback_color(gpu)?);
    }
    let bvh = &bvh;
    let faces: Vec<Vec<[f32; 4]>> = std::thread::scope(|scope| {
        let workers: Vec<_> = colours
            .iter()
            .enumerate()
            .map(|(face, bytes)| {
                scope.spawn(move || face_texels(face, bytes, size, eye, bvh, triangles, sky))
            })
            .collect();
        workers
            .into_iter()
            .map(|worker| worker.join().expect("scene reflection face worker"))
            .collect()
    });
    let base = faces.concat();
    let cube = prefilter(base, size)?;
    let spec = ReflectionSpec {
        name: Some("scene".into()),
        position: eye,
        min,
        max,
        resolution: size,
        anchors: None,
        fade: settings.fade,
        priority: 0.0,
    };
    Ok(Some(ReflectionArtifact {
        manifest: ReflectionManifest {
            schema_version: 1,
            scene_hash: String::new(),
            spec,
            anchors: vec![hour],
            sample_count: settings.samples,
            seed: settings.seed,
            format: REFLECTION_FORMAT.into(),
            color_space: "linear_rec709_radiance".into(),
            coordinates: "right_handed_y_up_px_nx_py_ny_pz_nz".into(),
            files: Vec::<FileEntry>::new(),
        },
        cubes: vec![cube],
    }))
}
