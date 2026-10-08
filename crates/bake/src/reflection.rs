use std::fs;
use std::path::Path;

use half::f16;
use pfx_gpu::{Gpu, wgpu};
use pfx_trace::bvh::{Bvh, Ray};
use pfx_trace::shapes::intersect;
use pfx_trace::{Camera, Scene, Trace};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{Anchor, BakeScene, Paced, SceneSource, artifact::FileEntry};

pub const REFLECTION_FORMAT: &str = "cube_array_rgba16f_ggx_mips_inverse_distance_face_major_le_v2";
pub const REFLECTION_FORMAT_V1: &str = "cube_array_rgba16f_ggx_mips_face_major_le_v1";

const FACES: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
    ([1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]),
    ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
    ([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]),
    ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
    ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
    ([0.0, 0.0, -1.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReflectionSpec {
    pub name: Option<String>,
    pub position: [f32; 3],
    pub min: [f32; 3],
    pub max: [f32; 3],
    #[serde(default = "default_resolution")]
    pub resolution: u32,
    pub anchors: Option<Vec<f32>>,
    #[serde(default = "default_fade")]
    pub fade: f32,
    #[serde(default)]
    pub priority: f32,
}

fn default_resolution() -> u32 {
    256
}
fn default_fade() -> f32 {
    0.25
}

impl ReflectionSpec {
    pub fn name(&self, index: usize) -> String {
        self.name
            .clone()
            .unwrap_or_else(|| format!("reflection-{index:03}"))
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.resolution < 4 || self.resolution > 1024 || !self.resolution.is_power_of_two() {
            return Err("reflection resolution must be a power of two from 4 to 1024".into());
        }
        if self.name.as_ref().is_some_and(|name| {
            name.is_empty()
                || !name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        }) {
            return Err("reflection name must be an ASCII label".into());
        }
        if !self.fade.is_finite() || self.fade < 0.0 || !self.priority.is_finite() {
            return Err("reflection fade and priority must be finite".into());
        }
        for axis in 0..3 {
            if !self.position[axis].is_finite()
                || !self.min[axis].is_finite()
                || !self.max[axis].is_finite()
                || self.min[axis] >= self.max[axis]
                || self.position[axis] < self.min[axis]
                || self.position[axis] > self.max[axis]
            {
                return Err("reflection position must be inside finite box bounds".into());
            }
        }
        if let Some(hours) = &self.anchors
            && (hours.is_empty()
                || hours
                    .iter()
                    .any(|v| !v.is_finite() || !(0.0..=24.0).contains(v))
                || hours.windows(2).any(|v| v[0] >= v[1]))
        {
            return Err("reflection anchors must be sorted distinct hours from 0 to 24".into());
        }
        Ok(())
    }

    pub fn hours<'a>(&'a self, defaults: &'a [f32]) -> &'a [f32] {
        self.anchors.as_deref().unwrap_or(defaults)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReflectionManifest {
    pub schema_version: u32,
    pub scene_hash: String,
    pub spec: ReflectionSpec,
    pub anchors: Vec<f32>,
    pub sample_count: u32,
    pub seed: u32,
    pub format: String,
    pub color_space: String,
    pub coordinates: String,
    pub files: Vec<FileEntry>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Cube {
    pub resolution: u32,
    pub levels: Vec<Vec<[f32; 4]>>,
}

impl Cube {
    pub fn new(resolution: u32, levels: Vec<Vec<[f32; 4]>>) -> Result<Self, String> {
        let mut size = resolution;
        if size < 4 || !size.is_power_of_two() {
            return Err("invalid cube resolution".into());
        }
        for level in &levels {
            if level.len() != 6 * size as usize * size as usize
                || level.iter().flatten().any(|v| !v.is_finite() || *v < 0.0)
            {
                return Err("invalid cube level".into());
            }
            size = (size / 2).max(1);
        }
        if levels.len() != resolution.ilog2() as usize + 1 {
            return Err("invalid cube mip count".into());
        }
        Ok(Self { resolution, levels })
    }

    pub fn bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"PITOREF1");
        bytes.extend_from_slice(&self.resolution.to_le_bytes());
        bytes.extend_from_slice(&(self.levels.len() as u32).to_le_bytes());
        for level in &self.levels {
            for pixel in level {
                for value in pixel {
                    bytes.extend_from_slice(
                        &f16::from_f32(value.min(65504.0)).to_bits().to_le_bytes(),
                    );
                }
            }
        }
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < 16 || &bytes[..8] != b"PITOREF1" {
            return Err("invalid reflection header".into());
        }
        let resolution = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        let count = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
        if !(4..=1024).contains(&resolution)
            || !resolution.is_power_of_two()
            || count != resolution.ilog2() + 1
        {
            return Err("invalid reflection dimensions".into());
        }
        let mut at = 16;
        let mut size = resolution as usize;
        let mut levels = Vec::new();
        for _ in 0..count {
            let len = 6 * size * size;
            if bytes.len().saturating_sub(at) < len * 8 {
                return Err("short reflection payload".into());
            }
            let mut level = Vec::with_capacity(len);
            for pixel in bytes[at..at + len * 8].chunks_exact(8) {
                level.push(std::array::from_fn(|channel| {
                    f16::from_bits(u16::from_le_bytes(
                        pixel[channel * 2..channel * 2 + 2].try_into().unwrap(),
                    ))
                    .to_f32()
                }));
            }
            levels.push(level);
            at += len * 8;
            size = (size / 2).max(1);
        }
        if at != bytes.len() {
            return Err("trailing reflection payload".into());
        }
        Self::new(resolution, levels)
    }
}

pub struct ReflectionArtifact {
    pub manifest: ReflectionManifest,
    pub cubes: Vec<Cube>,
}

fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub fn reflection_hash(
    source: &SceneSource,
    scene: &BakeScene,
    spec: &ReflectionSpec,
    samples: u32,
) -> Result<String, String> {
    let grid = crate::GridSpec {
        min: spec.position,
        max: spec.position,
        spacing: 1.0,
    };
    let mut h = Sha256::new();
    h.update(b"pito-engine-reflection-v2");
    h.update(source.hash(grid, samples)?.as_bytes());
    h.update(crate::scene_hash(scene, grid, samples, source.recipe.seed)?.as_bytes());
    h.update(serde_json::to_vec(spec).map_err(|e| e.to_string())?);
    Ok(hex::encode(h.finalize()))
}

pub fn face_direction(face: usize, x: u32, y: u32, size: u32) -> [f32; 3] {
    let (forward, right, up) = FACES[face];
    let u = (x as f32 + 0.5) * 2.0 / size as f32 - 1.0;
    let v = 1.0 - (y as f32 + 0.5) * 2.0 / size as f32;
    normalize(std::array::from_fn(|i| {
        forward[i] + u * right[i] + v * up[i]
    }))
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let length = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    v.map(|x| x / length)
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|i| a[i] * b[i]).sum()
}

fn face_uv(direction: [f32; 3]) -> (usize, f32, f32) {
    let axis = (0..3)
        .max_by(|&a, &b| direction[a].abs().total_cmp(&direction[b].abs()))
        .unwrap();
    let face = axis * 2 + usize::from(direction[axis] < 0.0);
    let (forward, right, up) = FACES[face];
    let scale = dot(direction, forward);
    (
        face,
        dot(direction, right) / scale,
        dot(direction, up) / scale,
    )
}

fn lookup(base: &[[f32; 4]], size: u32, direction: [f32; 3]) -> [f32; 4] {
    let (face, u, v) = face_uv(direction);
    let x = (((u + 1.0) * 0.5 * size as f32) as i32).clamp(0, size as i32 - 1) as usize;
    let y = (((1.0 - v) * 0.5 * size as f32) as i32).clamp(0, size as i32 - 1) as usize;
    base[face * size as usize * size as usize + y * size as usize + x]
}

fn radical_inverse(mut value: u32) -> f32 {
    value = value.reverse_bits();
    value as f32 * (1.0 / 4294967296.0)
}

pub fn prefilter(base: Vec<[f32; 4]>, resolution: u32) -> Result<Cube, String> {
    if resolution < 4
        || !resolution.is_power_of_two()
        || base.len() != 6 * resolution as usize * resolution as usize
    {
        return Err("invalid base cube".into());
    }
    let count = resolution.ilog2() + 1;
    let mut levels = vec![base.clone()];
    for mip in 1..count {
        let size = resolution >> mip;
        let roughness = mip as f32 / (count - 1) as f32;
        let alpha2 = roughness.powi(4).max(0.0001);
        let mut pixels = Vec::with_capacity(6 * size as usize * size as usize);
        for face in 0..6 {
            for y in 0..size {
                for x in 0..size {
                    let n = face_direction(face, x, y, size);
                    let helper = if n[1].abs() < 0.99 {
                        [0.0, 1.0, 0.0]
                    } else {
                        [1.0, 0.0, 0.0]
                    };
                    let tangent = normalize([
                        n[1] * helper[2] - n[2] * helper[1],
                        n[2] * helper[0] - n[0] * helper[2],
                        n[0] * helper[1] - n[1] * helper[0],
                    ]);
                    let bitangent = [
                        n[1] * tangent[2] - n[2] * tangent[1],
                        n[2] * tangent[0] - n[0] * tangent[2],
                        n[0] * tangent[1] - n[1] * tangent[0],
                    ];
                    let mut sum = [0.0f32; 4];
                    let mut weight = 0.0f32;
                    for sample in 0..64 {
                        let phi = std::f32::consts::TAU * sample as f32 / 64.0;
                        let xi = radical_inverse(sample);
                        let cos_theta = ((1.0 - xi) / (1.0 + (alpha2 - 1.0) * xi)).sqrt();
                        let sin_theta = (1.0 - cos_theta * cos_theta).max(0.0).sqrt();
                        let h = std::array::from_fn(|i| {
                            tangent[i] * phi.cos() * sin_theta
                                + bitangent[i] * phi.sin() * sin_theta
                                + n[i] * cos_theta
                        });
                        let l = std::array::from_fn(|i| 2.0 * dot(n, h) * h[i] - n[i]);
                        let w = dot(n, l).max(0.0);
                        if w > 0.0 {
                            let radiance = lookup(&base, resolution, l);
                            for channel in 0..4 {
                                sum[channel] += radiance[channel] * w;
                            }
                            weight += w;
                        }
                    }
                    pixels.push(sum.map(|v| v / weight));
                }
            }
        }
        levels.push(pixels);
    }
    Cube::new(resolution, levels)
}

pub fn bake_reflection(
    gpu: &Gpu,
    scene: &BakeScene,
    spec: &ReflectionSpec,
    samples: u32,
    seed: u32,
    anchor_index: usize,
) -> Result<Cube, String> {
    Paced::quiet(|paced| {
        bake_reflection_paced(gpu, scene, spec, samples, seed, anchor_index, paced)
    })
}

pub fn bake_reflection_paced(
    gpu: &Gpu,
    scene: &BakeScene,
    spec: &ReflectionSpec,
    samples: u32,
    seed: u32,
    anchor_index: usize,
    paced: &mut Paced<'_>,
) -> Result<Cube, String> {
    spec.validate()?;
    if samples == 0 {
        return Err("reflection samples must be positive".into());
    }
    let anchor = scene
        .anchors
        .get(anchor_index)
        .ok_or("reflection anchor is missing")?;
    let size = spec.resolution;
    let mut base = Vec::with_capacity(6 * size as usize * size as usize);
    let trace_scene = Scene {
        triangles: scene.triangles.clone(),
        shapes: scene.shapes.clone(),
        materials: scene.materials.clone(),
        sky: anchor.sky.clone(),
        camera: Camera {
            origin: spec.position,
            forward: FACES[0].0,
            right: FACES[0].1,
            up: FACES[0].2,
        },
        sun: anchor.sun,
    };
    let mut trace = Trace::new(gpu, &trace_scene, size, size)?;
    let bvh = Bvh::build(&scene.triangles);
    for (face, &(forward, right, up)) in FACES.iter().enumerate() {
        trace.set_camera(Camera {
            origin: spec.position,
            forward,
            right,
            up,
        })?;
        let ray_seed = seed
            .wrapping_add((anchor_index as u32).wrapping_mul(0x9e37_79b9))
            .wrapping_add(face as u32);
        let mut failed = None::<String>;
        let between = &mut *paced.between;
        trace.sample_paced(gpu, samples, ray_seed, paced.pacer, |ms| {
            match between(ms) {
                Ok(turned) => turned,
                Err(error) => {
                    failed.get_or_insert(error);
                    false
                }
            }
        })?;
        if let Some(error) = failed {
            return Err(error);
        }
        let bytes = trace.readback_color(gpu)?;
        for (index, pixel) in bytes.chunks_exact(16).enumerate() {
            let rgb: [f32; 3] = std::array::from_fn(|channel| {
                f32::from_le_bytes(pixel[channel * 4..channel * 4 + 4].try_into().unwrap())
            });
            let (x, y) = (index as u32 % size, index as u32 / size);
            let inverse =
                inverse_distance(scene, &bvh, spec.position, face_direction(face, x, y, size));
            base.push([rgb[0].max(0.0), rgb[1].max(0.0), rgb[2].max(0.0), inverse]);
        }
    }
    prefilter(base, size)
}

fn inverse_distance(scene: &BakeScene, bvh: &Bvh, origin: [f32; 3], direction: [f32; 3]) -> f32 {
    let ray = Ray { origin, direction };
    let mesh = bvh
        .intersect(&scene.triangles, ray, f32::INFINITY)
        .map(|hit| hit.distance);
    let limit = mesh.unwrap_or(f32::INFINITY);
    intersect(&scene.shapes, ray, limit)
        .map(|hit| hit.distance)
        .or(mesh)
        .map_or(0.0, |distance| 1.0 / distance.max(0.001))
}

pub struct ReflectionWrite<'a> {
    pub tmp_root: &'a Path,
    pub relative: &'a Path,
    pub scene_hash: String,
    pub spec: &'a ReflectionSpec,
    pub anchors: &'a [Anchor],
    pub cubes: &'a [Cube],
    pub samples: u32,
    pub seed: u32,
}

pub fn write_reflection_artifact(input: ReflectionWrite<'_>) -> Result<ReflectionManifest, String> {
    let ReflectionWrite {
        tmp_root,
        relative,
        scene_hash,
        spec,
        anchors,
        cubes,
        samples,
        seed,
    } = input;
    spec.validate()?;
    if cubes.is_empty()
        || cubes.len() != anchors.len()
        || cubes.iter().any(|v| v.resolution != spec.resolution)
    {
        return Err("reflection cubes and anchors do not match".into());
    }
    let out = tmp_root.join(relative);
    if out.exists()
        || !out.starts_with(tmp_root)
        || relative
            .components()
            .any(|v| !matches!(v, std::path::Component::Normal(_)))
    {
        return Err("invalid reflection output folder".into());
    }
    fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let mut files = Vec::new();
    for (index, cube) in cubes.iter().enumerate() {
        let path = format!("cube-{index:03}.bin");
        let bytes = cube.bytes();
        fs::write(out.join(&path), &bytes).map_err(|e| e.to_string())?;
        files.push(FileEntry {
            path,
            bytes: bytes.len() as u64,
            sha256: digest(&bytes),
        });
    }
    let manifest = ReflectionManifest {
        schema_version: 1,
        scene_hash,
        spec: spec.clone(),
        anchors: anchors.iter().map(|v| v.hour).collect(),
        sample_count: samples,
        seed,
        format: REFLECTION_FORMAT.into(),
        color_space: "linear_rec709_radiance".into(),
        coordinates: "right_handed_y_up_px_nx_py_ny_pz_nz".into(),
        files,
    };
    let json = serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?;
    fs::write(out.join("manifest.json"), &json).map_err(|e| e.to_string())?;
    fs::write(out.join("manifest.sha256"), format!("{}\n", digest(&json)))
        .map_err(|e| e.to_string())?;
    Ok(manifest)
}

pub fn read_reflection_artifact(dir: &Path) -> Result<ReflectionArtifact, String> {
    read_reflection_artifact_from(&|name| fs::read(dir.join(name)).ok())
}

pub fn read_reflection_artifact_from(
    files: &dyn Fn(&str) -> Option<Vec<u8>>,
) -> Result<ReflectionArtifact, String> {
    let fetch = |name: &str| files(name).ok_or_else(|| format!("missing reflection file {name}"));
    let json = fetch("manifest.json")?;
    if String::from_utf8(fetch("manifest.sha256")?)
        .map_err(|e| e.to_string())?
        .trim()
        != digest(&json)
    {
        return Err("reflection manifest checksum mismatch".into());
    }
    let manifest: ReflectionManifest = serde_json::from_slice(&json).map_err(|e| e.to_string())?;
    manifest.spec.validate()?;
    if manifest.schema_version != 1
        || manifest.sample_count == 0
        || manifest.anchors.is_empty()
        || manifest.anchors.len() != manifest.files.len()
        || (manifest.format != REFLECTION_FORMAT && manifest.format != REFLECTION_FORMAT_V1)
        || manifest.color_space != "linear_rec709_radiance"
        || manifest.coordinates != "right_handed_y_up_px_nx_py_ny_pz_nz"
        || manifest
            .anchors
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=24.0).contains(v))
        || manifest.anchors.windows(2).any(|v| v[0] >= v[1])
    {
        return Err("invalid reflection manifest".into());
    }
    let mut cubes = Vec::new();
    for (index, file) in manifest.files.iter().enumerate() {
        if file.path != format!("cube-{index:03}.bin") {
            return Err("invalid reflection file name".into());
        }
        let bytes = fetch(&file.path)?;
        if file.bytes != bytes.len() as u64 || file.sha256 != digest(&bytes) {
            return Err("reflection checksum mismatch".into());
        }
        let cube = Cube::from_bytes(&bytes)?;
        if cube.resolution != manifest.spec.resolution {
            return Err("reflection resolution mismatch".into());
        }
        cubes.push(cube);
    }
    Ok(ReflectionArtifact { manifest, cubes })
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProbeRecord {
    pub center: [f32; 3],
    pub layer: i32,
    pub box_min: [f32; 3],
    pub priority: f32,
    pub box_max: [f32; 3],
    pub fade: f32,
}

pub struct ReflectionArray {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub records: Vec<ProbeRecord>,
    pub max_mip: f32,
}

pub fn upload_reflections(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    artifacts: &[ReflectionArtifact],
    hour: f32,
) -> Result<ReflectionArray, String> {
    if artifacts.is_empty() || !hour.is_finite() {
        return Err("reflection array needs probes and an hour".into());
    }
    let size = artifacts[0].manifest.spec.resolution;
    if artifacts.iter().any(|a| a.manifest.spec.resolution != size) {
        return Err("reflection resolutions must match".into());
    }
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("baked local reflections"),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: (artifacts.len() * 6) as u32,
        },
        mip_level_count: size.ilog2() + 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let mut records = Vec::new();
    for (index, artifact) in artifacts.iter().enumerate() {
        let blend = crate::blend_anchors(&artifact.manifest.anchors, hour)?;
        let distance = artifact.manifest.format == REFLECTION_FORMAT;
        let lower = &artifact.cubes[blend.lower];
        let upper = &artifact.cubes[blend.upper];
        for (mip, (a, b)) in lower.levels.iter().zip(&upper.levels).enumerate() {
            let side = size >> mip;
            for face in 0..6 {
                let start = face * side as usize * side as usize;
                let end = start + side as usize * side as usize;
                let mut bytes = Vec::with_capacity((end - start) * 8);
                for (pa, pb) in a[start..end].iter().zip(&b[start..end]) {
                    for channel in 0..4 {
                        let mut value = pa[channel] * (1.0 - blend.upper_weight)
                            + pb[channel] * blend.upper_weight;
                        if channel == 3 && distance {
                            value = -1.0 - value.max(0.0);
                        }
                        bytes.extend_from_slice(&f16::from_f32(value).to_bits().to_le_bytes());
                    }
                }
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &texture,
                        mip_level: mip as u32,
                        origin: wgpu::Origin3d {
                            x: 0,
                            y: 0,
                            z: (index * 6 + face) as u32,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    &bytes,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(side * 8),
                        rows_per_image: Some(side),
                    },
                    wgpu::Extent3d {
                        width: side,
                        height: side,
                        depth_or_array_layers: 1,
                    },
                );
            }
        }
        let spec = &artifact.manifest.spec;
        records.push(ProbeRecord {
            center: spec.position,
            layer: index as i32,
            box_min: spec.min,
            priority: spec.priority,
            box_max: spec.max,
            fade: spec.fade,
        });
    }
    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::CubeArray),
        ..Default::default()
    });
    Ok(ReflectionArray {
        texture,
        view,
        records,
        max_mip: size.ilog2() as f32,
    })
}

pub fn nearest_two(records: &[ProbeRecord], world: [f32; 3]) -> [(usize, f32); 2] {
    let mut weighted = records
        .iter()
        .enumerate()
        .filter_map(|(index, probe)| {
            let mut edge = f32::INFINITY;
            for (axis, coordinate) in world.iter().enumerate() {
                if *coordinate < probe.box_min[axis] || *coordinate > probe.box_max[axis] {
                    return None;
                }
                edge = edge
                    .min(*coordinate - probe.box_min[axis])
                    .min(probe.box_max[axis] - *coordinate);
            }
            let distance = (0..3)
                .map(|axis| (world[axis] - probe.center[axis]).powi(2))
                .sum::<f32>()
                .sqrt();
            let seam = if probe.fade == 0.0 {
                1.0
            } else {
                (edge / probe.fade).clamp(0.0, 1.0)
            };
            Some((
                index,
                seam * (1.0 + probe.priority.max(0.0)) / (distance + 0.001),
                seam,
            ))
        })
        .collect::<Vec<_>>();
    weighted.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let a = weighted.first().copied().unwrap_or((0, 0.0, 0.0));
    let b = weighted.get(1).copied().unwrap_or((0, 0.0, 0.0));
    let total = a.1 + b.1;
    let coverage = a.2.max(b.2);
    if total == 0.0 {
        [(a.0, 0.0), (b.0, 0.0)]
    } else {
        [(a.0, coverage * a.1 / total), (b.0, coverage * b.1 / total)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pfx_load::Sky;
    use pfx_materials::Material;
    use pfx_trace::{Sun, shapes::Shape};

    #[test]
    fn face_axes_match_cpu_reference() {
        let expected = [
            [1.0, 0.0, 0.0],
            [-1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, -1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, -1.0],
        ];
        for (face, axis) in expected.into_iter().enumerate() {
            let direction = face_direction(face, 127, 127, 256);
            assert!(dot(direction, axis) > 0.999);
            let (actual, _, _) = face_uv(direction);
            assert_eq!(actual, face);
        }
        let corners = [
            [1.0, 0.75, -0.75],
            [-1.0, 0.75, 0.75],
            [0.75, 1.0, -0.75],
            [0.75, -1.0, 0.75],
            [0.75, 0.75, 1.0],
            [-0.75, 0.75, -1.0],
        ];
        for (face, reference) in corners.into_iter().enumerate() {
            let actual = face_direction(face, 3, 0, 4);
            let reference = normalize(reference);
            assert!(dot(actual, reference) > 0.99999, "face {face}: {actual:?}");
        }
        assert_eq!(face_uv([1.0, 0.0, -0.5]).0, 0);
        assert_eq!(face_uv([0.0, -1.0, 0.5]).0, 3);
    }

    #[test]
    fn prefilter_spreads_bright_texel_and_is_deterministic() {
        let mut base = vec![[0.0, 0.0, 0.0, 1.0]; 6 * 16 * 16];
        base[7 * 16 + 7] = [8.0, 1.0, 1.0, 1.0];
        let cube = prefilter(base.clone(), 16).unwrap();
        assert_eq!(cube.bytes(), prefilter(base, 16).unwrap().bytes());
        assert_eq!(
            Cube::from_bytes(&cube.bytes()).unwrap().bytes(),
            cube.bytes()
        );
        let peak =
            |level: &Vec<[f32; 4]>| level.iter().map(|pixel| pixel[0]).fold(0.0f32, f32::max);
        assert!(
            cube.levels
                .windows(2)
                .all(|pair| peak(&pair[1]) <= peak(&pair[0]) + 0.01)
        );
    }

    #[test]
    fn nearest_blends_at_overlap_and_fades_at_edge() {
        assert_eq!(std::mem::size_of::<ProbeRecord>(), 48);
        let record = ProbeRecord {
            center: [0.0; 3],
            layer: 0,
            box_min: [-1.0; 3],
            priority: 0.0,
            box_max: [1.0; 3],
            fade: 0.25,
        };
        let mut other = record;
        other.center = [0.2, 0.0, 0.0];
        other.layer = 1;
        let selected = nearest_two(&[record, other], [0.1, 0.0, 0.0]);
        assert!((selected[0].1 - 0.5).abs() < 0.01);
        assert!((selected[1].1 - 0.5).abs() < 0.01);
        assert!(nearest_two(&[record], [0.99, 0.0, 0.0])[0].1 < 0.1);
        assert_eq!(nearest_two(&[record], [2.0, 0.0, 0.0])[0].1, 0.0);
        other.priority = 2.0;
        let selected = nearest_two(&[record, other], [0.1, 0.0, 0.0]);
        assert_eq!(selected[0].0, 1);
        assert!(selected[0].1 > selected[1].1);
    }

    #[test]
    fn recipe_parses_reflection_defaults_and_rejects_bad_bounds() {
        let text = "scene = 'room.glb'\napp = 'test'\nanchors = [12.0]\nsamples = 2\n[[reflection]]\nposition = [0.0, 1.0, 0.0]\nmin = [-2.0, 0.0, -2.0]\nmax = [2.0, 2.0, 2.0]\n";
        let recipe = crate::Recipe::parse(text.as_bytes()).unwrap();
        assert_eq!(recipe.reflections[0].resolution, 256);
        assert!(recipe.volumes.is_empty());
        assert!(
            crate::Recipe::parse(text.replace("2.0, 2.0, 2.0", "-2.0, 2.0, 2.0").as_bytes())
                .is_err()
        );
    }

    #[test]
    fn artifact_checks_every_cube() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp");
        fs::create_dir_all(&root).unwrap();
        let folder = format!("reflection-artifact-test-{}", std::process::id());
        let target = root.join(&folder);
        if target.exists() {
            fs::remove_dir_all(&target).unwrap();
        }
        let spec = ReflectionSpec {
            name: None,
            position: [0.0; 3],
            min: [-1.0; 3],
            max: [1.0; 3],
            resolution: 4,
            anchors: None,
            fade: 0.25,
            priority: 0.0,
        };
        let cube = prefilter(vec![[1.0, 0.0, 0.0, 1.0]; 6 * 4 * 4], 4).unwrap();
        let anchors = vec![Anchor {
            hour: 12.0,
            sky: Sky {
                width: 1,
                height: 1,
                texels: vec![[0.0; 4]],
            },
            sun: Sun {
                direction: [0.0, 1.0, 0.0],
                color: [0.0; 3],
                intensity: 0.0,
            },
        }];
        let manifest = write_reflection_artifact(ReflectionWrite {
            tmp_root: &root,
            relative: Path::new(&folder),
            scene_hash: "example".into(),
            spec: &spec,
            anchors: &anchors,
            cubes: std::slice::from_ref(&cube),
            samples: 2,
            seed: 7,
        })
        .unwrap();
        assert_eq!(manifest.files.len(), 1);
        assert_eq!(
            read_reflection_artifact(&target).unwrap().cubes[0].bytes(),
            cube.bytes()
        );
        let files: std::collections::HashMap<String, Vec<u8>> = fs::read_dir(&target)
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                (
                    entry.file_name().to_string_lossy().into_owned(),
                    fs::read(entry.path()).unwrap(),
                )
            })
            .collect();
        let from_bytes = read_reflection_artifact_from(&|file| files.get(file).cloned()).unwrap();
        let from_path = read_reflection_artifact(&target).unwrap();
        assert_eq!(from_bytes.manifest, from_path.manifest);
        assert_eq!(from_bytes.cubes[0].bytes(), cube.bytes());
        assert_eq!(from_path.manifest.format, REFLECTION_FORMAT);
        let mut older = files.clone();
        let json = String::from_utf8(older["manifest.json"].clone())
            .unwrap()
            .replace(REFLECTION_FORMAT, REFLECTION_FORMAT_V1)
            .into_bytes();
        older.insert(
            "manifest.sha256".into(),
            format!("{}\n", digest(&json)).into_bytes(),
        );
        older.insert("manifest.json".into(), json);
        let older = read_reflection_artifact_from(&|file| older.get(file).cloned()).unwrap();
        assert_eq!(older.manifest.format, REFLECTION_FORMAT_V1);
        assert_eq!(older.cubes[0].bytes(), cube.bytes());
        for file in ["manifest.json", "cube-000.bin"] {
            let mut changed = files.clone();
            changed.get_mut(file).unwrap()[10] ^= 1;
            let error = read_reflection_artifact_from(&|file| changed.get(file).cloned())
                .err()
                .unwrap();
            assert!(error.contains("checksum"), "{file}: {error}");
            let mut missing = files.clone();
            missing.remove(file);
            let error = read_reflection_artifact_from(&|file| missing.get(file).cloned())
                .err()
                .unwrap();
            assert!(error.contains("missing reflection file"), "{file}: {error}");
        }
        let path = target.join("cube-000.bin");
        let mut bytes = fs::read(&path).unwrap();
        bytes[20] ^= 1;
        fs::write(path, bytes).unwrap();
        assert!(
            read_reflection_artifact(&target)
                .err()
                .unwrap()
                .contains("checksum")
        );
        fs::remove_dir_all(target).unwrap();
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn red_box_appears_in_positive_x_face() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let scene = BakeScene {
            triangles: Vec::new(),
            shapes: vec![Shape::RoundedBox {
                center: [2.0, 0.0, 0.0],
                half: [0.5, 0.5, 0.5],
                radius: 0.01,
                material: 0,
            }],
            materials: vec![Material {
                base: [1.0, 0.0, 0.0],
                emission: [8.0, 0.0, 0.0],
                ..Material::default()
            }],
            anchors: vec![Anchor {
                hour: 12.0,
                sky: Sky {
                    width: 1,
                    height: 1,
                    texels: vec![[0.0; 4]],
                },
                sun: Sun {
                    direction: [0.0, 1.0, 0.0],
                    color: [0.0; 3],
                    intensity: 0.0,
                },
            }],
        };
        let spec = ReflectionSpec {
            name: None,
            position: [0.0; 3],
            min: [-3.0; 3],
            max: [3.0; 3],
            resolution: 16,
            anchors: None,
            fade: 0.25,
            priority: 0.0,
        };
        let first = bake_reflection(&gpu, &scene, &spec, 4, 9, 0).unwrap();
        let second = bake_reflection(&gpu, &scene, &spec, 4, 9, 0).unwrap();
        assert_eq!(first.bytes(), second.bytes());
        let pixel = first.levels[0][8 * 16 + 8];
        assert!(pixel[0] > pixel[1] * 4.0 + 0.1, "red box pixel: {pixel:?}");
        assert!(
            (1.0 / pixel[3] - 1.5).abs() < 0.02,
            "red box distance: {pixel:?}"
        );
        let behind = first.levels[0][16 * 16 + 8 * 16 + 8];
        assert_eq!(behind[3], 0.0, "open sky distance: {behind:?}");
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn split_samples_bake_the_same_cube() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let scene = BakeScene {
            triangles: Vec::new(),
            shapes: vec![Shape::RoundedBox {
                center: [2.0, 0.0, 0.0],
                half: [0.5, 0.5, 0.5],
                radius: 0.01,
                material: 0,
            }],
            materials: vec![Material {
                base: [0.6, 0.3, 0.2],
                emission: [2.0, 0.5, 0.0],
                ..Material::default()
            }],
            anchors: vec![Anchor {
                hour: 12.0,
                sky: Sky {
                    width: 2,
                    height: 1,
                    texels: vec![[0.2, 0.3, 0.5, 1.0], [0.1, 0.1, 0.1, 1.0]],
                },
                sun: Sun {
                    direction: [0.3, 0.8, 0.5],
                    color: [1.0; 3],
                    intensity: 4.0,
                },
            }],
        };
        let spec = ReflectionSpec {
            name: None,
            position: [0.0; 3],
            min: [-3.0; 3],
            max: [3.0; 3],
            resolution: 32,
            anchors: None,
            fade: 0.25,
            priority: 0.0,
        };
        let bake = |fixed: Option<u32>| {
            let mut pacer = pfx_gpu::pace::Pacer::default();
            pacer.set_fixed_units(fixed);
            let mut slices = 0;
            let mut turns = pfx_gpu::pace::Turns::default();
            let mut between = |ms: f64| {
                slices += 1;
                Ok(turns.add(ms))
            };
            let cube = bake_reflection_paced(
                &gpu,
                &scene,
                &spec,
                48,
                9,
                0,
                &mut Paced {
                    pacer: &mut pacer,
                    between: &mut between,
                },
            )
            .unwrap();
            (cube.bytes(), slices)
        };
        let (whole, whole_slices) = bake(Some(32 * 48));
        assert_eq!(whole_slices, 6);
        let (single, single_slices) = bake(Some(8));
        assert_eq!(single_slices, 6 * 32 * 48 / 8);
        assert!(single == whole, "one band of eight rows per submission");
        assert!(bake(Some(40)).0 == whole, "bands across samples");
        assert!(bake(None).0 == whole, "paced submissions");
    }
}
