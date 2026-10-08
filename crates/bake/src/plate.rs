pub mod codec;
mod run;

use std::collections::BTreeMap;
use std::path::Path;

use pfx_load::scene::{Camera as SceneCamera, Projection as SceneProjection, TraceSettings};
use pfx_trace::Adaptive;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub use run::{
    Geometry, Pass, PlateOutcome, PlateRun, StagedPlate, bake_plates, plate_key, probe_plate,
    stage_plate,
};

pub const SCHEMA_VERSION: u32 = 1;
pub const FORMAT: &str = "plate";
pub const CODE_VERSION: u32 = 1;
pub const MAX_ID: u32 = u16::MAX as u32;
pub const R16_RANGE: f32 = 64.0;
pub const DEFAULT_VIEW: &str = "main";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Layers {
    pub normal: bool,
    pub id: bool,
    pub sun: bool,
    pub transfer: bool,
}

impl Layers {
    pub const ALL: Layers = Layers {
        normal: true,
        id: true,
        sun: true,
        transfer: true,
    };

    pub fn names(&self) -> Vec<&'static str> {
        let mut names = vec!["color", "depth"];
        for (on, name) in [
            (self.normal, "normal"),
            (self.id, "id"),
            (self.sun, "sun"),
            (self.transfer, "transfer"),
        ] {
            if on {
                names.push(name);
            }
        }
        names
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct ViewKeys {
    pub at: Option<[f32; 3]>,
    pub look_at: Option<[f32; 3]>,
    pub up: Option<[f32; 3]>,
    pub fov: Option<f32>,
    pub focal: Option<f32>,
    pub sensor: Option<f32>,
    pub shift: Option<[f32; 2]>,
    pub near: Option<f32>,
    pub far: Option<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ViewRecipe {
    pub name: String,
    pub keys: ViewKeys,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlateRecipe {
    pub scene: String,
    pub size: [u32; 2],
    pub overscan: f32,
    pub scale: f32,
    pub anchors: Option<Vec<f32>>,
    pub layers: Layers,
    pub adaptive: Adaptive,
    pub seed: Option<u32>,
    pub key: Option<String>,
    pub clamp_indirect: Option<f32>,
    pub filter_glossy: Option<f32>,
    pub views: Vec<ViewRecipe>,
}

#[derive(Deserialize)]
struct Outer {
    plates: Option<toml::Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPlates {
    scene: String,
    size: [u32; 2],
    overscan: Option<f32>,
    scale: Option<f32>,
    anchors: Option<Vec<f32>>,
    layers: Option<Vec<String>>,
    threshold: Option<f32>,
    min_samples: Option<u32>,
    max_samples: Option<u32>,
    growth: Option<f32>,
    seed: Option<u32>,
    key: Option<String>,
    clamp_indirect: Option<f32>,
    filter_glossy: Option<f32>,
    #[serde(default)]
    view: Vec<RawView>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawView {
    name: String,
    at: Option<[f32; 3]>,
    look_at: Option<[f32; 3]>,
    up: Option<[f32; 3]>,
    fov: Option<f32>,
    focal: Option<f32>,
    sensor: Option<f32>,
    shift: Option<[f32; 2]>,
    near: Option<f32>,
    far: Option<f32>,
}

fn label(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

pub fn check_anchors(anchors: &[f32]) -> Result<(), String> {
    if anchors.is_empty()
        || anchors
            .iter()
            .any(|hour| !hour.is_finite() || !(0.0..=24.0).contains(hour))
        || anchors.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return Err("plate anchors must be strictly increasing hours from 0 through 24".into());
    }
    Ok(())
}

pub fn parse_plates(bytes: &[u8]) -> Result<Option<PlateRecipe>, String> {
    let text = std::str::from_utf8(bytes).map_err(|e| format!("bake.toml: {e}"))?;
    let outer: Outer = toml::from_str(text).map_err(|e| format!("bake.toml: {e}"))?;
    let Some(value) = outer.plates else {
        return Ok(None);
    };
    let raw: RawPlates = value
        .try_into()
        .map_err(|e: toml::de::Error| format!("bake.toml [plates]: {}", e.message()))?;
    let [width, height] = raw.size;
    if !(16..=16384).contains(&width) || !(16..=16384).contains(&height) {
        return Err("plates size must be 16 to 16384 pixels on each side".into());
    }
    let overscan = raw.overscan.unwrap_or(0.1);
    if !(0.0..=1.0).contains(&overscan) {
        return Err("plates overscan must be 0 to 1".into());
    }
    let scale = raw.scale.unwrap_or(1.0);
    if !(0.25..=4.0).contains(&scale) {
        return Err("plates scale must be 0.25 to 4".into());
    }
    if let Some(anchors) = &raw.anchors {
        check_anchors(anchors)?;
    }
    let mut layers = Layers::default();
    let mut seen = Vec::new();
    for name in raw.layers.unwrap_or_else(|| {
        ["color", "depth", "normal", "id", "sun", "transfer"]
            .map(String::from)
            .to_vec()
    }) {
        if seen.contains(&name) {
            return Err(format!("plates layer {name} is named twice"));
        }
        match name.as_str() {
            "color" | "depth" => {}
            "normal" => layers.normal = true,
            "id" => layers.id = true,
            "sun" => layers.sun = true,
            "transfer" => layers.transfer = true,
            other => {
                return Err(format!(
                    "plates layer {other} is unknown; layers are color, depth, normal, id, sun and transfer"
                ));
            }
        }
        seen.push(name);
    }
    if !seen.iter().any(|name| name == "color") || !seen.iter().any(|name| name == "depth") {
        return Err("plates layers need color and depth".into());
    }
    if layers.transfer && !(layers.sun && layers.normal) {
        return Err("plates layer transfer needs sun and normal".into());
    }
    let adaptive = Adaptive {
        threshold: raw.threshold.unwrap_or(0.01),
        min_samples: raw.min_samples.unwrap_or(16),
        max_samples: raw.max_samples.unwrap_or(1024),
        growth: raw.growth.unwrap_or(2.0),
    };
    if !adaptive.valid() || adaptive.max_samples > 65536 {
        return Err("plates sampling needs a positive threshold, a growth over 1 and 2 to 65536 samples, min_samples at most max_samples".into());
    }
    TraceSettings {
        clamp_indirect: raw.clamp_indirect.unwrap_or(0.0),
        filter_glossy: raw.filter_glossy.unwrap_or(0.0),
        ..TraceSettings::default()
    }
    .check()
    .map_err(|message| format!("plates {message}"))?;
    let mut views = Vec::new();
    for view in raw.view {
        if !label(&view.name) {
            return Err(format!(
                "plates view name {:?} must be an ASCII label",
                view.name
            ));
        }
        if views
            .iter()
            .any(|other: &ViewRecipe| other.name == view.name)
        {
            return Err(format!("plates view {} is named twice", view.name));
        }
        if view.fov.is_some() && (view.focal.is_some() || view.sensor.is_some()) {
            return Err(format!(
                "plates view {} takes fov or focal and sensor, not both",
                view.name
            ));
        }
        views.push(ViewRecipe {
            name: view.name,
            keys: ViewKeys {
                at: view.at,
                look_at: view.look_at,
                up: view.up,
                fov: view.fov,
                focal: view.focal,
                sensor: view.sensor,
                shift: view.shift,
                near: view.near,
                far: view.far,
            },
        });
    }
    if views.is_empty() {
        views.push(ViewRecipe {
            name: DEFAULT_VIEW.into(),
            keys: ViewKeys::default(),
        });
    }
    Ok(Some(PlateRecipe {
        scene: raw.scene,
        size: raw.size,
        overscan,
        scale,
        anchors: raw.anchors,
        layers,
        adaptive,
        seed: raw.seed,
        key: raw.key,
        clamp_indirect: raw.clamp_indirect,
        filter_glossy: raw.filter_glossy,
        views,
    }))
}

impl PlateRecipe {
    pub fn trace(&self, scene: TraceSettings) -> TraceSettings {
        TraceSettings {
            clamp_indirect: self.clamp_indirect.unwrap_or(scene.clamp_indirect),
            filter_glossy: self.filter_glossy.unwrap_or(scene.filter_glossy),
            ..scene
        }
    }
}

impl ViewKeys {
    pub fn camera(&self, base: &SceneCamera) -> Result<SceneCamera, String> {
        let mut camera = *base;
        if let Some(at) = self.at {
            camera.at = at;
        }
        if let Some(look_at) = self.look_at {
            camera.look_at = look_at;
        }
        if let Some(up) = self.up {
            camera.up = up;
        }
        if let Some(shift) = self.shift {
            camera.shift = shift;
        }
        if let Some(near) = self.near {
            camera.near = near;
        }
        if let Some(far) = self.far {
            camera.far = far;
        }
        if let Some(fov) = self.fov {
            camera.projection = SceneProjection::Perspective { fov };
        } else if let Some(focal) = self.focal {
            let sensor = self.sensor.unwrap_or(24.0);
            camera.projection = SceneProjection::Perspective {
                fov: (2.0 * (sensor * 0.5 / focal).atan()).to_degrees(),
            };
        }
        let finite = |values: &[f32]| values.iter().all(|value| value.is_finite());
        if !finite(&camera.at) || !finite(&camera.look_at) || !finite(&camera.up) {
            return Err("plate view camera is not finite".into());
        }
        if !(camera.near > 0.0 && camera.far > camera.near) {
            return Err("plate view needs 0 < near < far".into());
        }
        match camera.projection {
            SceneProjection::Perspective { fov } if fov > 0.0 && fov < 179.0 => Ok(camera),
            SceneProjection::Perspective { .. } => {
                Err("plate view fov must be between 0 and 179 degrees".into())
            }
            SceneProjection::Orthographic { .. } => {
                Err("a plate is perspective; an orthographic camera is refused".into())
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlateCamera {
    pub origin: [f32; 3],
    pub forward: [f32; 3],
    pub right: [f32; 3],
    pub up: [f32; 3],
    pub shift: [f32; 2],
    pub near: f32,
    pub far: f32,
}

pub fn plate_size(size: [u32; 2], overscan: f32, scale: f32) -> [u32; 2] {
    size.map(|side| 2 * ((side as f32 * scale * (1.0 + overscan) * 0.5).round() as u32).max(1))
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

impl PlateCamera {
    pub fn new(camera: &SceneCamera, size: [u32; 2], overscan: f32, scale: f32) -> Self {
        let [width, height] = plate_size(size, overscan, scale);
        let tan = camera.tan_half_height().unwrap_or(1.0);
        let aspect = size[0] as f32 / size[1] as f32;
        let grow = [
            width as f32 / (size[0] as f32 * scale),
            height as f32 / (size[1] as f32 * scale),
        ];
        let (forward, right, up) = camera.axes();
        Self {
            origin: camera.at,
            forward,
            right: right.map(|v| v * tan * aspect * grow[0]),
            up: up.map(|v| v * tan * grow[1]),
            shift: [camera.shift[0] / grow[0], camera.shift[1] / grow[1]],
            near: camera.near,
            far: camera.far,
        }
    }

    pub fn ray(&self, texel: [f32; 2], size: [u32; 2]) -> [f32; 3] {
        let x = texel[0] / size[0] as f32 * 2.0 - 1.0;
        let y = texel[1] / size[1] as f32 * 2.0 - 1.0;
        std::array::from_fn(|k| {
            self.forward[k]
                + self.right[k] * (x + self.shift[0])
                + self.up[k] * (-y + self.shift[1])
        })
    }

    pub fn project(&self, world: [f32; 3], size: [u32; 2]) -> Option<([f32; 2], f32)> {
        let v = [
            world[0] - self.origin[0],
            world[1] - self.origin[1],
            world[2] - self.origin[2],
        ];
        let z = dot(v, self.forward);
        if z <= 0.0 || z.is_nan() {
            return None;
        }
        let a = dot(v, self.right) / (dot(self.right, self.right) * z);
        let b = dot(v, self.up) / (dot(self.up, self.up) * z);
        let x = a - self.shift[0];
        let y = self.shift[1] - b;
        Some((
            [
                (x + 1.0) * 0.5 * size[0] as f32,
                (y + 1.0) * 0.5 * size[1] as f32,
            ],
            z,
        ))
    }

    pub fn trace_camera(&self) -> pfx_trace::Camera {
        pfx_trace::Camera {
            origin: self.origin,
            forward: self.forward,
            right: self.right,
            up: self.up,
        }
    }

    pub fn strip(&self, size: [u32; 2], first: u32, rows: u32) -> (pfx_trace::Camera, [f32; 2]) {
        let k = rows as f32 / size[1] as f32;
        let centre = (2.0 * first as f32 + rows as f32) / size[1] as f32 - 1.0;
        (
            pfx_trace::Camera {
                origin: self.origin,
                forward: self.forward,
                right: self.right,
                up: self.up.map(|v| v * k),
            },
            [self.shift[0], (self.shift[1] - centre) / k],
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bounds {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sampling {
    pub threshold: f32,
    pub min_samples: u32,
    pub max_samples: u32,
    pub growth: f32,
    pub seed: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PassRecord {
    pub mean: f64,
    pub max: u32,
    pub rounds: Vec<[u64; 2]>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnchorRecord {
    pub hour: f32,
    pub color: PassRecord,
    pub sun: Option<PassRecord>,
    pub transfer: Option<PassRecord>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SunRecord {
    pub direction: [f32; 3],
    pub color: [f32; 3],
    pub intensity: f32,
}

impl SunRecord {
    pub fn light(&self) -> [f32; 3] {
        self.color.map(|channel| channel * self.intensity)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdEntry {
    pub id: u32,
    pub object: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlateFile {
    pub name: String,
    pub layer: String,
    pub anchor: Option<usize>,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlateManifest {
    pub schema_version: u32,
    pub format: String,
    pub engine: String,
    pub key: String,
    pub caller_key: Option<String>,
    pub view: String,
    pub size: [u32; 2],
    pub overscan: f32,
    pub scale: f32,
    pub width: u32,
    pub height: u32,
    pub camera: PlateCamera,
    pub depth_near: f32,
    pub bounds: Bounds,
    pub anchors: Vec<f32>,
    pub suns: Vec<SunRecord>,
    pub layers: BTreeMap<String, String>,
    pub sampling: Sampling,
    pub samples: Vec<AnchorRecord>,
    pub ids: Vec<IdEntry>,
    pub files: Vec<PlateFile>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Depth {
    Inverse(Vec<u16>),
    Float(Vec<f32>),
}

impl Depth {
    pub fn at(&self, index: usize, near: f32) -> Option<f32> {
        match self {
            Self::Inverse(codes) => codec::from_inverse_depth(codes[index], near),
            Self::Float(depths) => (depths[index] > 0.0).then_some(depths[index]),
        }
    }

    pub fn format(&self) -> &'static str {
        match self {
            Self::Inverse(_) => "r16-inverse",
            Self::Float(_) => "r32f",
        }
    }

    fn bytes(&self) -> Vec<u8> {
        match self {
            Self::Inverse(codes) => codes.iter().flat_map(|code| code.to_le_bytes()).collect(),
            Self::Float(depths) => depths
                .iter()
                .flat_map(|depth| depth.to_le_bytes())
                .collect(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Plate {
    pub manifest: PlateManifest,
    pub depth: Depth,
    pub normal: Option<Vec<u32>>,
    pub id: Option<Vec<u16>>,
    pub color: Vec<Vec<u32>>,
    pub sun: Option<Vec<Vec<u32>>>,
    pub transfer: Option<Vec<Vec<u32>>>,
}

impl Plate {
    pub fn texels(&self) -> usize {
        self.manifest.width as usize * self.manifest.height as usize
    }

    pub fn bytes_per_anchor(&self) -> u64 {
        let layers = 1 + u64::from(self.sun.is_some()) + u64::from(self.transfer.is_some());
        self.texels() as u64 * 4 * layers
    }

    pub fn shared_bytes(&self) -> u64 {
        let depth = match self.depth {
            Depth::Inverse(_) => 2,
            Depth::Float(_) => 4,
        };
        let normal = if self.normal.is_some() { 4 } else { 0 };
        let id = if self.id.is_some() { 2 } else { 0 };
        self.texels() as u64 * (depth + normal + id)
    }

    pub fn depth_at(&self, x: u32, y: u32) -> Option<f32> {
        let index = y as usize * self.manifest.width as usize + x as usize;
        self.depth.at(index, self.manifest.depth_near)
    }

    pub fn blend(&self, hour: f32) -> (usize, usize, f32) {
        anchor_blend(&self.manifest.anchors, hour)
    }
}

pub fn anchor_blend(anchors: &[f32], hour: f32) -> (usize, usize, f32) {
    let last = anchors.len().saturating_sub(1);
    if anchors.is_empty() || !hour.is_finite() || hour <= anchors[0] {
        return (0, 0, 0.0);
    }
    if hour >= anchors[last] {
        return (last, last, 0.0);
    }
    let next = anchors
        .iter()
        .position(|&anchor| anchor > hour)
        .unwrap_or(last);
    let before = next - 1;
    let t = (hour - anchors[before]) / (anchors[next] - anchors[before]);
    (before, next, t)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn sha(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn words<T: Copy>(bytes: &[u8], size: usize, read: impl Fn(&[u8]) -> T) -> Vec<T> {
    bytes.chunks_exact(size).map(read).collect()
}

pub fn write_plate(dir: &Path, plate: &Plate) -> Result<PlateManifest, String> {
    let mut manifest = plate.manifest.clone();
    let mut files = Vec::new();
    let mut out: Vec<(String, Vec<u8>)> = Vec::new();
    let mut add = |name: String, layer: &str, anchor: Option<usize>, bytes: Vec<u8>| {
        files.push(PlateFile {
            name: name.clone(),
            layer: layer.into(),
            anchor,
            bytes: bytes.len() as u64,
            sha256: sha(&bytes),
        });
        out.push((name, bytes));
    };
    let mut layers = BTreeMap::new();
    layers.insert("color".to_string(), "rgb9e5".to_string());
    layers.insert("depth".to_string(), plate.depth.format().to_string());
    add("depth.bin".into(), "depth", None, plate.depth.bytes());
    if let Some(normal) = &plate.normal {
        layers.insert("normal".into(), "oct16".into());
        add(
            "normal.bin".into(),
            "normal",
            None,
            normal.iter().flat_map(|v| v.to_le_bytes()).collect(),
        );
    }
    if let Some(id) = &plate.id {
        layers.insert("id".into(), "r16".into());
        add(
            "id.bin".into(),
            "id",
            None,
            id.iter().flat_map(|v| v.to_le_bytes()).collect(),
        );
    }
    for (anchor, color) in plate.color.iter().enumerate() {
        add(
            format!("color-{anchor:03}.bin"),
            "color",
            Some(anchor),
            color.iter().flat_map(|v| v.to_le_bytes()).collect(),
        );
    }
    for (name, layer) in [("sun", &plate.sun), ("transfer", &plate.transfer)] {
        if let Some(anchors) = layer {
            layers.insert(name.into(), "rgb9e5".into());
            for (anchor, values) in anchors.iter().enumerate() {
                add(
                    format!("{name}-{anchor:03}.bin"),
                    name,
                    Some(anchor),
                    values.iter().flat_map(|v| v.to_le_bytes()).collect(),
                );
            }
        }
    }
    manifest.layers = layers;
    manifest.files = files;
    check(&manifest, plate)?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for (name, bytes) in &out {
        std::fs::write(dir.join(name), bytes).map_err(|e| format!("{name}: {e}"))?;
    }
    let json = serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("manifest.json"), &json).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("manifest.sha256"), format!("{}\n", sha(&json)))
        .map_err(|e| e.to_string())?;
    Ok(manifest)
}

fn check(manifest: &PlateManifest, plate: &Plate) -> Result<(), String> {
    let texels = plate.texels();
    let anchors = manifest.anchors.len();
    let depth = match &plate.depth {
        Depth::Inverse(codes) => codes.len(),
        Depth::Float(depths) => depths.len(),
    };
    if texels == 0
        || depth != texels
        || manifest.suns.len() != anchors
        || plate.color.len() != anchors
        || plate.color.iter().any(|layer| layer.len() != texels)
        || plate.normal.as_ref().is_some_and(|v| v.len() != texels)
        || plate.id.as_ref().is_some_and(|v| v.len() != texels)
        || [&plate.sun, &plate.transfer].iter().any(|layer| {
            layer.as_ref().is_some_and(|layer| {
                layer.len() != anchors || layer.iter().any(|v| v.len() != texels)
            })
        })
    {
        return Err("plate layers do not match its size and anchors".into());
    }
    check_anchors(&manifest.anchors)
}

pub fn read_plate(dir: &Path) -> Result<Plate, String> {
    read_plate_from(&|name| std::fs::read(dir.join(name)).ok())
}

pub fn read_plate_from(get: &dyn Fn(&str) -> Option<Vec<u8>>) -> Result<Plate, String> {
    let fetch = |name: &str| get(name).ok_or_else(|| format!("missing plate file {name}"));
    let json = fetch("manifest.json")?;
    let recorded = fetch("manifest.sha256")?;
    if String::from_utf8_lossy(&recorded).trim() != sha(&json) {
        return Err("plate manifest.sha256 does not match manifest.json".into());
    }
    let manifest: PlateManifest =
        serde_json::from_slice(&json).map_err(|e| format!("plate manifest: {e}"))?;
    if manifest.schema_version != SCHEMA_VERSION || manifest.format != FORMAT {
        return Err(format!(
            "plate manifest is {} schema {}; this engine reads {FORMAT} schema {SCHEMA_VERSION}",
            manifest.format, manifest.schema_version
        ));
    }
    let mut loaded: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for file in &manifest.files {
        let bytes = fetch(&file.name)?;
        if bytes.len() as u64 != file.bytes || sha(&bytes) != file.sha256 {
            return Err(format!(
                "plate file {} does not match its manifest",
                file.name
            ));
        }
        loaded.insert(file.name.clone(), bytes);
    }
    let mut take = |name: &str| loaded.remove(name);
    let u32s = |bytes: Vec<u8>| words(&bytes, 4, |c| u32::from_le_bytes(c.try_into().unwrap()));
    let u16s = |bytes: Vec<u8>| words(&bytes, 2, |c| u16::from_le_bytes(c.try_into().unwrap()));
    let layer = |name: &str| manifest.layers.get(name).map(String::as_str);
    let depth_bytes = take("depth.bin").ok_or("missing plate file depth.bin")?;
    let depth = match layer("depth") {
        Some("r16-inverse") => Depth::Inverse(u16s(depth_bytes)),
        Some("r32f") => Depth::Float(words(&depth_bytes, 4, |c| {
            f32::from_le_bytes(c.try_into().unwrap())
        })),
        other => return Err(format!("plate depth format {other:?} is unknown")),
    };
    if layer("color") != Some("rgb9e5") {
        return Err("plate colour must be rgb9e5".into());
    }
    let anchors = manifest.anchors.len();
    let mut color = Vec::with_capacity(anchors);
    for anchor in 0..anchors {
        let name = format!("color-{anchor:03}.bin");
        color.push(u32s(
            take(&name).ok_or(format!("missing plate file {name}"))?,
        ));
    }
    let mut per_anchor = |kind: &str| -> Result<Option<Vec<Vec<u32>>>, String> {
        match layer(kind) {
            None => Ok(None),
            Some("rgb9e5") => {
                let mut layers = Vec::with_capacity(anchors);
                for anchor in 0..anchors {
                    let name = format!("{kind}-{anchor:03}.bin");
                    layers.push(u32s(
                        take(&name).ok_or(format!("missing plate file {name}"))?,
                    ));
                }
                Ok(Some(layers))
            }
            Some(other) => Err(format!("plate {kind} format {other} is unknown")),
        }
    };
    let sun = per_anchor("sun")?;
    let transfer = per_anchor("transfer")?;
    let normal = match layer("normal") {
        None => None,
        Some("oct16") => Some(u32s(
            take("normal.bin").ok_or("missing plate file normal.bin")?,
        )),
        Some(other) => return Err(format!("plate normal format {other} is unknown")),
    };
    let id = match layer("id") {
        None => None,
        Some("r16") => Some(u16s(take("id.bin").ok_or("missing plate file id.bin")?)),
        Some(other) => return Err(format!("plate id format {other} is unknown")),
    };
    let plate = Plate {
        manifest,
        depth,
        normal,
        id,
        color,
        sun,
        transfer,
    };
    check(&plate.manifest, &plate)?;
    Ok(plate)
}

#[cfg(test)]
mod tests;
