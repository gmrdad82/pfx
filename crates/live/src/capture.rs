use pfx_bake::FileEntry;
use pfx_bake::reflection::{
    FACES, REFLECTION_FORMAT, ReflectionArtifact, ReflectionManifest, ReflectionSpec, prefilter,
};
use pfx_gpu::Gpu;
use pfx_load::scene::{Draws, Scene};
use pfx_trace::bvh::{Bvh, Ray, Triangle};
use pfx_trace::detail::Lens;
use pfx_trace::{Camera, Projection};

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

pub fn world_triangles(draws: &Draws) -> Vec<Triangle> {
    let mut out = Vec::new();
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
    out
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

pub fn capture(
    gpu: &Gpu,
    scene: &Scene,
    sky: &SkySource,
    settings: &SceneReflections,
    hour: f32,
) -> Result<Option<ReflectionArtifact>, String> {
    if !settings.valid() {
        return Err("scene reflections need a power-of-two resolution from 4 to 1024, samples and a finite fade and margin".into());
    }
    let triangles = world_triangles(&scene.draws());
    let Some([lo, hi]) = bounds(&triangles) else {
        return Ok(None);
    };
    let eye = scene.camera_or_default().at;
    let min = std::array::from_fn(|k| lo[k].min(eye[k]) - settings.margin);
    let max = std::array::from_fn(|k| hi[k].max(eye[k]) + settings.margin);
    let size = settings.resolution;
    let staged = pfx_trace::stage::scene_at(scene, 1.0, 0.0)?;
    let mut trace = staged.trace(gpu, size, size).map_err(|e| e.to_string())?;
    trace.set_projection(Projection::Perspective)?;
    trace.set_lens(Lens::default())?;
    let bvh = Bvh::build(&triangles);
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
    let (bvh, triangles) = (&bvh, &triangles);
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
