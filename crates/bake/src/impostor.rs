use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

use half::f16;
use pfx_gpu::Gpu;
use pfx_load::Mesh;
use pfx_materials::Material;
use pfx_trace::bvh::{Bvh, Ray, Triangle};
use pfx_trace::shapes::{Shape, intersect};
use pfx_trace::{Camera, Scene, Trace};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{Anchor, BakeScene, FileEntry};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PropSpec {
    pub name: String,
    pub node: Option<String>,
    pub file: Option<String>,
    pub frames_per_side: u32,
    pub atlas_size: u32,
    #[serde(default)]
    pub hemisphere: bool,
    #[serde(default)]
    pub anchors: Vec<f32>,
}

#[derive(Deserialize)]
struct PropList {
    #[serde(default, rename = "prop")]
    props: Vec<PropSpec>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PropBounds {
    pub center: [f32; 3],
    pub radius: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PropManifest {
    pub schema_version: u32,
    pub name: String,
    pub scene_hash: String,
    pub frames_per_side: u32,
    pub atlas_size: u32,
    pub hemisphere: bool,
    pub bounds: PropBounds,
    pub anchors: Vec<f32>,
    pub sample_count: u32,
    pub seed: u32,
    pub format: String,
    pub color_space: String,
    pub coordinates: String,
    pub files: Vec<FileEntry>,
}

#[derive(Debug)]
pub struct PropArtifact {
    pub manifest: PropManifest,
    pub radiance: Vec<Vec<u8>>,
    pub geometry: Vec<u8>,
}

fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn label(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

impl PropSpec {
    pub fn validate(&self, default_anchors: &[f32]) -> Result<(), String> {
        if !label(&self.name) {
            return Err("prop name must be an ASCII label".into());
        }
        if self.node.is_some() == self.file.is_some() {
            return Err(format!("prop {} needs exactly one node or file", self.name));
        }
        if self.node.as_ref().is_some_and(String::is_empty) {
            return Err("prop node must not be empty".into());
        }
        if self
            .file
            .as_ref()
            .is_some_and(|v| !(v.ends_with(".gltf") || v.ends_with(".glb")))
        {
            return Err("prop file must be glTF or GLB".into());
        }
        if self.frames_per_side < 2
            || self.frames_per_side > 32
            || self.atlas_size < 16
            || self.atlas_size > 8192
            || !self.atlas_size.is_multiple_of(self.frames_per_side)
            || self.atlas_size / self.frames_per_side < 8
        {
            return Err(
                "prop atlas needs 2..32 frames per side and at least 8 texels per frame".into(),
            );
        }
        let hours = if self.anchors.is_empty() {
            default_anchors
        } else {
            &self.anchors
        };
        if hours.is_empty()
            || hours
                .iter()
                .any(|v| !v.is_finite() || !(0.0..=24.0).contains(v))
            || hours.windows(2).any(|v| v[0] >= v[1])
        {
            return Err("prop anchors must be sorted, distinct hours from 0 to 24".into());
        }
        Ok(())
    }

    pub fn hours<'a>(&'a self, defaults: &'a [f32]) -> &'a [f32] {
        if self.anchors.is_empty() {
            defaults
        } else {
            &self.anchors
        }
    }
}

pub fn parse_props(bytes: &[u8], default_anchors: &[f32]) -> Result<Vec<PropSpec>, String> {
    let text = std::str::from_utf8(bytes).map_err(|e| format!("bake.toml: {e}"))?;
    let parsed: PropList = toml::from_str(text).map_err(|e| format!("bake.toml: {e}"))?;
    let mut names = BTreeSet::new();
    for prop in &parsed.props {
        prop.validate(default_anchors)?;
        if !names.insert(&prop.name) {
            return Err("prop names must be unique".into());
        }
    }
    Ok(parsed.props)
}

fn inside(root: &Path, path: &str) -> Result<PathBuf, String> {
    let relative = Path::new(path);
    if relative.components().next().is_none()
        || relative
            .components()
            .any(|v| !matches!(v, Component::Normal(_)))
    {
        return Err(format!("{path} is outside the scene folder"));
    }
    let full = root
        .join(relative)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if !full.starts_with(root) || !full.is_file() {
        return Err(format!("{path} is outside the scene folder"));
    }
    Ok(full)
}

fn transform(matrix: [[f32; 4]; 4], point: [f32; 3]) -> [f32; 3] {
    [0, 1, 2].map(|axis| matrix[3][axis] + (0..3).map(|k| matrix[k][axis] * point[k]).sum::<f32>())
}

fn mesh_triangles(mesh: &Mesh, node: Option<&str>) -> Result<Vec<Triangle>, String> {
    let roots = if let Some(name) = node {
        let matches = mesh
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, item)| item.name == name)
            .map(|(index, _)| index as u32)
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            return Err(format!("prop node {name} must name exactly one node"));
        }
        matches
    } else {
        mesh.roots.clone()
    };
    let mut stack = roots;
    let mut seen = BTreeSet::new();
    let mut result = Vec::new();
    while let Some(index) = stack.pop() {
        if !seen.insert(index) {
            return Err("prop scene has repeated nodes".into());
        }
        let item = mesh
            .nodes
            .get(index as usize)
            .ok_or("prop node is missing")?;
        stack.extend(item.children.iter().copied());
        let Some(group) = item.group else { continue };
        let matrix = mesh.world(index).ok_or("prop transform is invalid")?;
        for primitive in &mesh.groups[group as usize].primitives {
            for indices in primitive.indices.chunks_exact(3) {
                result.push(Triangle {
                    vertices: [indices[0], indices[1], indices[2]]
                        .map(|id| transform(matrix, primitive.positions[id as usize])),
                    material: primitive.material.unwrap_or(mesh.materials.len() as u32),
                });
            }
        }
    }
    if result.is_empty() {
        return Err("prop has no triangles".into());
    }
    Ok(result)
}

fn gltf_materials(bytes: &[u8]) -> Result<Vec<Material>, String> {
    let gltf = gltf::Gltf::from_slice(bytes).map_err(|e| e.to_string())?;
    let mut result = Vec::new();
    for item in gltf.materials() {
        let pbr = item.pbr_metallic_roughness();
        let base = pbr.base_color_factor();
        result.push(Material {
            base: [base[0], base[1], base[2]],
            roughness: pbr.roughness_factor(),
            metalness: pbr.metallic_factor(),
            emission: item.emissive_factor(),
            ..Material::default()
        });
    }
    result.push(Material::default());
    Ok(result)
}

pub fn load_prop_scene(
    folder: &Path,
    main_file: &str,
    spec: &PropSpec,
    anchors: &[Anchor],
) -> Result<(BakeScene, PropBounds), String> {
    let root = folder.canonicalize().map_err(|e| e.to_string())?;
    let file = spec.file.as_deref().unwrap_or(main_file);
    let bytes = fs::read(inside(&root, file)?).map_err(|e| e.to_string())?;
    let mesh = Mesh::parse_with(&bytes, |uri| {
        let parent = Path::new(file).parent().unwrap_or(Path::new(""));
        let relative = parent.join(uri);
        let name = relative.to_str().ok_or("prop buffer name is invalid")?;
        fs::read(inside(&root, name)?).map_err(|e| e.to_string())
    })?;
    let triangles = mesh_triangles(&mesh, spec.node.as_deref())?;
    let bounds = bounds(&triangles, &[])?;
    Ok((
        BakeScene {
            triangles,
            shapes: Vec::new(),
            materials: gltf_materials(&bytes)?,
            anchors: anchors.to_vec(),
        },
        bounds,
    ))
}

pub fn bounds(triangles: &[Triangle], shapes: &[Shape]) -> Result<PropBounds, String> {
    if triangles.is_empty() && shapes.is_empty() {
        return Err("prop has no geometry".into());
    }
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for point in triangles.iter().flat_map(|v| v.vertices) {
        for axis in 0..3 {
            lo[axis] = lo[axis].min(point[axis]);
            hi[axis] = hi[axis].max(point[axis]);
        }
    }
    for shape in shapes {
        let (center, half) = match *shape {
            Shape::RoundedBox { center, half, .. } => (center, half),
            Shape::Ellipsoid { center, radii, .. } => (center, radii),
            Shape::Capsule { a, b, radius, .. } => {
                let center = [0, 1, 2].map(|i| (a[i] + b[i]) * 0.5);
                let half = [0, 1, 2].map(|i| (a[i] - b[i]).abs() * 0.5 + radius);
                (center, half)
            }
            Shape::RoundCone {
                a,
                b,
                radius_a,
                radius_b,
                ..
            } => {
                let center = [0, 1, 2].map(|i| (a[i] + b[i]) * 0.5);
                let radius = radius_a.max(radius_b);
                let half = [0, 1, 2].map(|i| (a[i] - b[i]).abs() * 0.5 + radius);
                (center, half)
            }
            Shape::SmoothUnion { .. } => continue,
        };
        for axis in 0..3 {
            lo[axis] = lo[axis].min(center[axis] - half[axis]);
            hi[axis] = hi[axis].max(center[axis] + half[axis]);
        }
    }
    if lo.iter().chain(hi.iter()).any(|v| !v.is_finite()) {
        return Err("prop bounds are invalid".into());
    }
    let center = [0, 1, 2].map(|i| (lo[i] + hi[i]) * 0.5);
    let radius = (0..3)
        .map(|i| (hi[i] - center[i]).powi(2))
        .sum::<f32>()
        .sqrt();
    if radius <= 0.0 {
        return Err("prop bounds have no size".into());
    }
    Ok(PropBounds { center, radius })
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|i| a[i] * b[i]).sum()
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn normalize(v: [f32; 3]) -> [f32; 3] {
    let length = dot(v, v).sqrt().max(1e-20);
    v.map(|x| x / length)
}

fn oct_sign(value: f32) -> f32 {
    if value < 0.0 { -1.0 } else { 1.0 }
}

pub fn oct_encode(direction: [f32; 3]) -> [f32; 2] {
    let scale = direction.iter().map(|v| v.abs()).sum::<f32>().max(1e-20);
    let mut p = [direction[0] / scale, direction[2] / scale];
    if direction[1] < 0.0 {
        p = [
            (1.0 - p[1].abs()) * oct_sign(p[0]),
            (1.0 - p[0].abs()) * oct_sign(p[1]),
        ];
    }
    p
}

pub fn oct_decode(p: [f32; 2]) -> [f32; 3] {
    let y = 1.0 - p[0].abs() - p[1].abs();
    if y < 0.0 {
        normalize([
            (1.0 - p[1].abs()) * oct_sign(p[0]),
            y,
            (1.0 - p[0].abs()) * oct_sign(p[1]),
        ])
    } else {
        normalize([p[0], y, p[1]])
    }
}

pub fn frame_direction(spec: &PropSpec, x: u32, y: u32) -> [f32; 3] {
    let n = spec.frames_per_side as f32;
    let p = [
        (x as f32 + 0.5) * 2.0 / n - 1.0,
        (y as f32 + 0.5) * 2.0 / n - 1.0,
    ];
    let mut dir = oct_decode(p);
    if spec.hemisphere {
        dir[1] = dir[1].abs();
    }
    dir
}

fn camera(bounds: PropBounds, direction: [f32; 3]) -> Camera {
    let origin = [0, 1, 2].map(|i| bounds.center[i] + direction[i] * bounds.radius * 3.0);
    let forward = direction.map(|v| -v);
    let nominal_up = if direction[1].abs() > 0.95 {
        [0.0, 0.0, 1.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    let right = normalize(cross(forward, nominal_up));
    let up = normalize(cross(right, forward));
    let scale = bounds.radius / (bounds.radius * 3.0);
    Camera {
        origin,
        forward,
        right: right.map(|v| v * scale),
        up: up.map(|v| v * scale),
    }
}

fn ray_depth(scene: &BakeScene, bvh: &Bvh, camera: Camera, x: f32, y: f32, size: u32) -> f32 {
    let u = x * 2.0 / size as f32 - 1.0;
    let v = 1.0 - y * 2.0 / size as f32;
    let direction =
        normalize([0, 1, 2].map(|i| camera.forward[i] + camera.right[i] * u + camera.up[i] * v));
    let ray = Ray {
        origin: camera.origin,
        direction,
    };
    let mesh = bvh
        .intersect(&scene.triangles, ray, f32::INFINITY)
        .map(|v| v.distance)
        .unwrap_or(f32::INFINITY);
    intersect(&scene.shapes, ray, mesh)
        .map(|v| v.distance)
        .unwrap_or(mesh)
}

fn hit_depth(
    scene: &BakeScene,
    bvh: &Bvh,
    camera: Camera,
    x: u32,
    y: u32,
    size: u32,
    coverage: f32,
) -> f32 {
    let depth = ray_depth(scene, bvh, camera, x as f32 + 0.5, y as f32 + 0.5, size);
    if depth.is_finite() || coverage <= 0.0 {
        return depth;
    }
    for dy in [0.25, 0.75] {
        for dx in [0.25, 0.75] {
            let depth = ray_depth(scene, bvh, camera, x as f32 + dx, y as f32 + dy, size);
            if depth.is_finite() {
                return depth;
            }
        }
    }
    depth
}

fn sky_radiance(sky: &pfx_load::Sky, camera: Camera, x: u32, y: u32, size: u32) -> [f32; 3] {
    let u = (x as f32 + 0.5) * 2.0 / size as f32 - 1.0;
    let v = 1.0 - (y as f32 + 0.5) * 2.0 / size as f32;
    let direction =
        normalize([0, 1, 2].map(|i| camera.forward[i] + camera.right[i] * u + camera.up[i] * v));
    let longitude =
        (direction[0].atan2(-direction[2]) / std::f32::consts::TAU + 0.5).rem_euclid(1.0);
    let latitude = direction[1].clamp(-1.0, 1.0).acos() / std::f32::consts::PI;
    let sx = ((longitude * sky.width as f32) as u32).min(sky.width - 1);
    let sy = ((latitude * sky.height as f32) as u32).min(sky.height - 1);
    let texel = sky.texels[(sy * sky.width + sx) as usize];
    [texel[0], texel[1], texel[2]]
}

pub fn prop_hash(
    scene: &BakeScene,
    spec: &PropSpec,
    bounds: PropBounds,
    samples: u32,
    seed: u32,
) -> Result<String, String> {
    let mut h = Sha256::new();
    h.update(b"pito-engine-impostor-v1");
    h.update(env!("CARGO_PKG_VERSION"));
    h.update(pfx_trace::TRACE_WGSL);
    h.update(pfx_materials::BRDF);
    h.update(serde_json::to_vec(spec).map_err(|e| e.to_string())?);
    h.update(serde_json::to_vec(&bounds).map_err(|e| e.to_string())?);
    for triangle in &scene.triangles {
        for point in triangle.vertices {
            for value in point {
                h.update(value.to_bits().to_le_bytes());
            }
        }
        h.update(triangle.material.to_le_bytes());
    }
    for shape in &scene.shapes {
        h.update(format!("{shape:?}"));
    }
    let used = scene
        .triangles
        .iter()
        .map(|v| v.material)
        .chain(scene.shapes.iter().map(|v| v.material()))
        .collect::<BTreeSet<_>>();
    for index in used {
        h.update(index.to_le_bytes());
        let material = scene
            .materials
            .get(index as usize)
            .ok_or("prop references a missing material")?;
        h.update(serde_json::to_vec(material).map_err(|e| e.to_string())?);
    }
    for anchor in &scene.anchors {
        h.update(anchor.hour.to_bits().to_le_bytes());
        for value in anchor.sun.direction.into_iter().chain(anchor.sun.color) {
            h.update(value.to_bits().to_le_bytes());
        }
        h.update(anchor.sun.intensity.to_bits().to_le_bytes());
        h.update(anchor.sky.width.to_le_bytes());
        h.update(anchor.sky.height.to_le_bytes());
        for pixel in &anchor.sky.texels {
            for value in pixel {
                h.update(value.to_bits().to_le_bytes());
            }
        }
    }
    h.update(samples.to_le_bytes());
    h.update(seed.to_le_bytes());
    Ok(hex::encode(h.finalize()))
}

pub fn bake_prop(
    gpu: &Gpu,
    scene: &BakeScene,
    spec: &PropSpec,
    bounds: PropBounds,
    samples: u32,
    seed: u32,
) -> Result<PropArtifact, String> {
    bake_prop_with_turn(gpu, scene, spec, bounds, samples, seed, |_| Ok(()))
}

pub fn bake_prop_with_turn(
    gpu: &Gpu,
    scene: &BakeScene,
    spec: &PropSpec,
    bounds: PropBounds,
    samples: u32,
    seed: u32,
    mut turn: impl FnMut(usize) -> Result<(), String>,
) -> Result<PropArtifact, String> {
    spec.validate(&scene.anchors.iter().map(|a| a.hour).collect::<Vec<_>>())?;
    if samples == 0 || scene.anchors.is_empty() {
        return Err("prop needs samples and anchors".into());
    }
    if !spec.anchors.is_empty()
        && spec.anchors
            != scene
                .anchors
                .iter()
                .map(|anchor| anchor.hour)
                .collect::<Vec<_>>()
    {
        return Err("prop anchors do not match the trace scene".into());
    }
    let size = spec.atlas_size as usize;
    let tile = spec.atlas_size / spec.frames_per_side;
    let bvh = Bvh::build(&scene.triangles);
    let mut geometry = vec![0u8; size * size * 16];
    let mut radiance = Vec::new();
    for (anchor_index, anchor) in scene.anchors.iter().enumerate() {
        let mut atlas = vec![0u8; size * size * 8];
        for fy in 0..spec.frames_per_side {
            for fx in 0..spec.frames_per_side {
                let view = camera(bounds, frame_direction(spec, fx, fy));
                let trace_scene = Scene {
                    triangles: scene.triangles.clone(),
                    shapes: scene.shapes.clone(),
                    materials: scene.materials.clone(),
                    sky: anchor.sky.clone(),
                    camera: view,
                    sun: anchor.sun,
                };
                let mut tracer = Trace::new(gpu, &trace_scene, tile, tile)?;
                let frame = fy * spec.frames_per_side + fx;
                tracer.sample(
                    gpu,
                    samples,
                    seed.wrapping_add((anchor_index as u32).wrapping_mul(0x9e37_79b9))
                        .wrapping_add(frame),
                )?;
                let output = tracer.readback(gpu)?;
                for y in 0..tile {
                    for x in 0..tile {
                        let src = ((y * tile + x) * 16) as usize;
                        let dst_pixel =
                            ((fy * tile + y) * spec.atlas_size + fx * tile + x) as usize;
                        let coverage = f32::from_le_bytes(
                            output.normal[src + 12..src + 16].try_into().unwrap(),
                        )
                        .clamp(0.0, 1.0);
                        let depth = if anchor_index == 0 {
                            hit_depth(scene, &bvh, view, x, y, tile, coverage)
                        } else {
                            0.0
                        };
                        let sky = sky_radiance(&anchor.sky, view, x, y, tile);
                        for channel in 0..3 {
                            let color = f32::from_le_bytes(
                                output.color[src + channel * 4..src + channel * 4 + 4]
                                    .try_into()
                                    .unwrap(),
                            )
                            .max(0.0);
                            let premultiplied = (color - sky[channel] * (1.0 - coverage)).max(0.0);
                            let encoded = f16::from_f32(premultiplied).to_bits().to_le_bytes();
                            atlas[dst_pixel * 8 + channel * 2..dst_pixel * 8 + channel * 2 + 2]
                                .copy_from_slice(&encoded);
                            if anchor_index == 0 {
                                geometry[dst_pixel * 16 + channel * 4
                                    ..dst_pixel * 16 + channel * 4 + 4]
                                    .copy_from_slice(
                                        &output.normal[src + channel * 4..src + channel * 4 + 4],
                                    );
                            }
                        }
                        atlas[dst_pixel * 8 + 6..dst_pixel * 8 + 8]
                            .copy_from_slice(&f16::from_f32(coverage).to_bits().to_le_bytes());
                        if anchor_index == 0 {
                            geometry[dst_pixel * 16 + 12..dst_pixel * 16 + 16]
                                .copy_from_slice(&depth.to_le_bytes());
                        }
                    }
                }
            }
        }
        radiance.push(atlas);
        if anchor_index + 1 < scene.anchors.len() {
            turn(anchor_index)?;
        }
    }
    let manifest = PropManifest {
        schema_version: SCHEMA_VERSION,
        name: spec.name.clone(),
        scene_hash: prop_hash(scene, spec, bounds, samples, seed)?,
        frames_per_side: spec.frames_per_side,
        atlas_size: spec.atlas_size,
        hemisphere: spec.hemisphere,
        bounds,
        anchors: scene.anchors.iter().map(|a| a.hour).collect(),
        sample_count: samples,
        seed,
        format: "radiance_rgba16f_geometry_normal_xyz_depth_f32_le_v1".into(),
        color_space: "linear_rec709_radiance".into(),
        coordinates: "right_handed_y_up_octahedral_full_or_upper_hemisphere".into(),
        files: Vec::new(),
    };
    Ok(PropArtifact {
        manifest,
        radiance,
        geometry,
    })
}

pub fn write_prop_artifact(
    tmp: &Path,
    relative: &Path,
    artifact: &PropArtifact,
) -> Result<PropManifest, String> {
    let root = tmp.canonicalize().map_err(|e| e.to_string())?;
    if root.file_name().is_none_or(|v| v != "tmp")
        || relative.components().next().is_none()
        || relative
            .components()
            .any(|v| !matches!(v, Component::Normal(_)))
    {
        return Err("prop output must be relative to caller tmp".into());
    }
    let mut out = root;
    for component in relative.components() {
        out.push(component.as_os_str());
        if fs::symlink_metadata(&out).is_ok_and(|meta| meta.file_type().is_symlink()) {
            return Err("prop output contains a symlink".into());
        }
    }
    if out.exists() {
        return Err("prop artifact folder already exists".into());
    }
    let parent = out.parent().ok_or("prop output has no parent")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let stage = parent.join(format!(
        ".{}.prop-partial",
        out.file_name().unwrap().to_string_lossy()
    ));
    fs::create_dir(&stage).map_err(|e| e.to_string())?;
    let mut manifest = artifact.manifest.clone();
    manifest.files.clear();
    for (path, bytes) in std::iter::once(("geometry.bin".to_string(), &artifact.geometry)).chain(
        artifact
            .radiance
            .iter()
            .enumerate()
            .map(|(i, bytes)| (format!("radiance-{i:03}.bin"), bytes)),
    ) {
        fs::write(stage.join(&path), bytes).map_err(|e| e.to_string())?;
        manifest.files.push(FileEntry {
            path,
            bytes: bytes.len() as u64,
            sha256: digest(bytes),
        });
    }
    let json = serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?;
    fs::write(stage.join("manifest.json"), &json).map_err(|e| e.to_string())?;
    fs::write(
        stage.join("manifest.sha256"),
        format!("{}\n", digest(&json)),
    )
    .map_err(|e| e.to_string())?;
    fs::rename(stage, out).map_err(|e| e.to_string())?;
    Ok(manifest)
}

pub fn read_prop_artifact(folder: &Path) -> Result<PropArtifact, String> {
    let json = fs::read(folder.join("manifest.json")).map_err(|e| e.to_string())?;
    let checksum = fs::read_to_string(folder.join("manifest.sha256")).map_err(|e| e.to_string())?;
    if checksum.trim() != digest(&json) {
        return Err("prop manifest checksum mismatch".into());
    }
    let manifest: PropManifest = serde_json::from_slice(&json).map_err(|e| e.to_string())?;
    if manifest.schema_version != SCHEMA_VERSION
        || !label(&manifest.name)
        || manifest.frames_per_side < 2
        || manifest.frames_per_side > 32
        || manifest.atlas_size < 16
        || manifest.atlas_size > 8192
        || !manifest.atlas_size.is_multiple_of(manifest.frames_per_side)
        || manifest.atlas_size / manifest.frames_per_side < 8
        || manifest.sample_count == 0
        || manifest.scene_hash.len() != 64
        || !manifest.scene_hash.bytes().all(|b| b.is_ascii_hexdigit())
        || manifest.bounds.center.iter().any(|v| !v.is_finite())
        || !manifest.bounds.radius.is_finite()
        || manifest.bounds.radius <= 0.0
        || manifest.anchors.is_empty()
        || manifest.anchors.windows(2).any(|v| v[0] >= v[1])
        || manifest
            .anchors
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=24.0).contains(v))
        || manifest.format != "radiance_rgba16f_geometry_normal_xyz_depth_f32_le_v1"
        || manifest.color_space != "linear_rec709_radiance"
        || manifest.coordinates != "right_handed_y_up_octahedral_full_or_upper_hemisphere"
        || manifest.files.len() != manifest.anchors.len() + 1
    {
        return Err("unsupported or invalid prop manifest".into());
    }
    let pixels = manifest.atlas_size as usize * manifest.atlas_size as usize;
    let mut data = Vec::new();
    for (i, entry) in manifest.files.iter().enumerate() {
        let expected = if i == 0 {
            "geometry.bin".to_string()
        } else {
            format!("radiance-{:03}.bin", i - 1)
        };
        let size = pixels * if i == 0 { 16 } else { 8 };
        if entry.path != expected || entry.bytes != size as u64 {
            return Err("prop file entry is invalid".into());
        }
        if entry.sha256.len() != 64 || !entry.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("prop file checksum is invalid".into());
        }
        let bytes = fs::read(folder.join(&entry.path)).map_err(|e| e.to_string())?;
        if bytes.len() != size || digest(&bytes) != entry.sha256 {
            return Err("prop file checksum mismatch".into());
        }
        data.push(bytes);
    }
    let geometry = data.remove(0);
    Ok(PropArtifact {
        manifest,
        radiance: data,
        geometry,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn octahedral_round_trip() {
        for direction in [
            [1.0, 0.0, 0.0],
            [-1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, -1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, -1.0],
        ] {
            assert!(dot(direction, oct_decode(oct_encode(direction))) > 0.9999);
        }
        for x in -8..=8 {
            for y in -8..=8 {
                let direction = normalize([x as f32 + 0.13, y as f32 + 0.27, 2.5]);
                let decoded = oct_decode(oct_encode(direction));
                assert!(dot(direction, decoded) > 0.9999);
            }
        }
    }

    #[test]
    fn recipe_parses_props() {
        let recipe = b"scene = 'room.glb'\nanchors = [8.0, 16.0]\n[[prop]]\nname = 'desk'\nnode = 'Desk'\nframes_per_side = 4\natlas_size = 256\nanchors = [9.0, 17.0]\n";
        let props = parse_props(recipe, &[8.0, 16.0]).unwrap();
        assert_eq!(props[0].hours(&[8.0, 16.0]), &[9.0, 17.0]);
        let bad = std::str::from_utf8(recipe)
            .unwrap()
            .replace("node = 'Desk'", "file = 'desk.glb'\nnode = 'Desk'");
        assert!(parse_props(bad.as_bytes(), &[8.0]).is_err());
    }

    #[test]
    fn prop_artifact_verifies_bytes() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp")
            .join(format!("impostor-artifact-test-{}", std::process::id()));
        if root.exists() {
            fs::remove_dir_all(&root).unwrap();
        }
        fs::create_dir_all(&root).unwrap();
        let tmp = root.join("tmp");
        fs::create_dir_all(&tmp).unwrap();
        let artifact = PropArtifact {
            manifest: PropManifest {
                schema_version: SCHEMA_VERSION,
                name: "box".into(),
                scene_hash: "a".repeat(64),
                frames_per_side: 2,
                atlas_size: 16,
                hemisphere: false,
                bounds: PropBounds {
                    center: [0.0; 3],
                    radius: 1.0,
                },
                anchors: vec![12.0],
                sample_count: 4,
                seed: 7,
                format: "radiance_rgba16f_geometry_normal_xyz_depth_f32_le_v1".into(),
                color_space: "linear_rec709_radiance".into(),
                coordinates: "right_handed_y_up_octahedral_full_or_upper_hemisphere".into(),
                files: Vec::new(),
            },
            radiance: vec![vec![0; 16 * 16 * 8]],
            geometry: vec![0; 16 * 16 * 16],
        };
        write_prop_artifact(&tmp, Path::new("box"), &artifact).unwrap();
        let folder = tmp.join("box");
        assert_eq!(
            read_prop_artifact(&folder).unwrap().radiance,
            artifact.radiance
        );
        fs::write(folder.join("radiance-000.bin"), vec![1; 16 * 16 * 8]).unwrap();
        assert!(
            read_prop_artifact(&folder)
                .unwrap_err()
                .contains("checksum")
        );
        fs::remove_dir_all(root).unwrap();
    }
}
