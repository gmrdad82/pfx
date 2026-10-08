mod anim;
mod bind;
mod build;
mod diff;
mod draws;
mod edit;
mod mix;
mod plates;
mod play;
mod pose;
mod rewrite;
pub mod tiles;
mod watch;

#[cfg(test)]
mod play_tests;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use pfx_core::anim::Rig;
use pfx_core::camera::Preset;
use pfx_core::daylight::Daylight;
use pfx_core::sky::AnalyticSky;
use pfx_materials::Library;
use pfx_post::Chain;
use pfx_text::{Anchor, Face, Representation, RichParagraph, Span, TextEngine};

use crate::Sky;
use crate::vec3::{cross, dot, sub};

pub use anim::{Animation, Posed, SPRITE_CLIP, Sprite, content_key, surface};
pub use bind::{bind, bound, filled};
pub use build::project_root;
pub use diff::{Changes, SceneDiff};
pub use draws::{Draw, Draws};
pub use edit::{EditError, Kind, Patch, PatchGroup, SceneEdit, Target, Value};
pub use pfx_scene::Id;
pub use pfx_scene::ProjectFile;
pub use pfx_scene::types::{
    Tunable as DeclaredTunable, TunableKind as DeclaredKind, TunableValue as DeclaredValue,
};
pub use plates::{PlateCasters, Plates, Proxies, Proxy, ProxyShape};
pub use play::{
    Body, BodyKind, BodyShape, Character, CharacterPlane, Cue, Physics, Sound, Trigger,
};
pub use pose::billboard;
pub use rewrite::{fix, migrate};
pub use watch::{DEBOUNCE, Reload, SceneWatch};

pub type Matrix = [[f32; 4]; 4];

pub const IDENTITY: Matrix = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

pub const DEFAULT_MATERIAL: &str = "(default)";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    pub file: PathBuf,
    pub line: usize,
    pub column: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SceneError {
    pub file: PathBuf,
    pub line: Option<usize>,
    pub column: Option<usize>,
    pub message: String,
    pub related: Vec<Location>,
}

impl SceneError {
    pub fn new(file: &Path, line: Option<usize>, message: impl Into<String>) -> Self {
        Self {
            file: file.to_path_buf(),
            line,
            column: None,
            message: message.into(),
            related: Vec::new(),
        }
    }
}

impl std::fmt::Display for SceneError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self.line, self.column) {
            (Some(line), Some(column)) => write!(
                f,
                "{}:{line}:{column}: {}",
                self.file.display(),
                self.message
            )?,
            (Some(line), None) => write!(f, "{}:{line}: {}", self.file.display(), self.message)?,
            _ => write!(f, "{}: {}", self.file.display(), self.message)?,
        }
        for place in &self.related {
            write!(
                f,
                "\n  {}:{}:{}: see here",
                place.file.display(),
                place.line,
                place.column
            )?;
        }
        Ok(())
    }
}

impl std::error::Error for SceneError {}

impl From<SceneError> for String {
    fn from(error: SceneError) -> Self {
        error.to_string()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shadow {
    Cast,
    Only,
    None,
}

impl Shadow {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "cast" => Some(Self::Cast),
            "only" => Some(Self::Only),
            "none" => Some(Self::None),
            _ => None,
        }
    }

    pub fn casts(self) -> bool {
        self != Self::None
    }

    pub fn drawn(self) -> bool {
        self != Self::Only
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Geometry {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub tangents: Vec<[f32; 4]>,
    pub uvs: Vec<[f32; 2]>,
    pub uvs1: Option<Vec<[f32; 2]>>,
    pub indices: Vec<u32>,
    pub skin: Option<Arc<GeometrySkin>>,
    pub hash: [u8; 32],
}

#[derive(Clone, Debug, PartialEq)]
pub struct GeometrySkin {
    pub joints: Vec<[u16; 4]>,
    pub weights: Vec<[f32; 4]>,
}

impl GeometrySkin {
    pub fn palette(&self) -> usize {
        self.joints
            .iter()
            .zip(&self.weights)
            .flat_map(|(joints, weights)| {
                joints
                    .iter()
                    .zip(weights)
                    .filter(|(_, weight)| **weight != 0.0)
                    .map(|(joint, _)| *joint as usize + 1)
            })
            .max()
            .unwrap_or(1)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NodeOverride {
    pub material: Option<String>,
    pub hidden: bool,
    pub shadow: Option<Shadow>,
    pub two_sided: Option<bool>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub node: String,
    pub joint: Option<usize>,
    pub transform: Matrix,
    pub geometry: Arc<Geometry>,
    pub material: Option<String>,
    pub overrides: NodeOverride,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SceneMesh {
    pub file: PathBuf,
    pub node: Option<String>,
    pub parts: Vec<Part>,
    pub rig: Option<Arc<Rig>>,
    pub hash: [u8; 32],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Object {
    pub name: String,
    pub mesh: String,
    pub at: [f32; 3],
    pub rotate: [f32; 3],
    pub scale: [f32; 3],
    pub parent: Option<String>,
    pub model: Matrix,
    pub material: Option<String>,
    pub materials: BTreeMap<String, String>,
    pub shadow: Shadow,
    pub two_sided: bool,
    pub hidden: bool,
    pub clip: Vec<[f32; 4]>,
    pub content: Option<String>,
    pub id: u32,
    pub alpha_cutoff: f32,
    pub face_camera: bool,
    pub dynamic: Option<bool>,
}

pub const ALPHA_CUTOFF: f32 = 0.5;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sun {
    pub direction: [f32; 3],
    pub color: [f32; 3],
    pub intensity: f32,
    pub hour: f32,
    pub daylight: Option<Daylight>,
    pub reference_hour: f64,
    pub radius: f32,
}

impl Sun {
    pub fn dark() -> Self {
        Self {
            direction: [0.0, 1.0, 0.0],
            color: [1.0; 3],
            intensity: 0.0,
            hour: 12.0,
            daylight: None,
            reference_hour: pfx_core::daylight::REFERENCE_HOUR,
            radius: 0.0,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Environment {
    Analytic(AnalyticSky),
    Hdr(Arc<Sky>),
}

impl Environment {
    pub fn black() -> Self {
        Self::Hdr(Arc::new(Sky {
            width: 1,
            height: 1,
            texels: vec![[0.0, 0.0, 0.0, 1.0]],
        }))
    }
}

#[derive(Clone, Debug)]
pub struct SceneSky {
    pub kind: String,
    pub environment: Environment,
    pub hash: [u8; 32],
}

impl PartialEq for SceneSky {
    fn eq(&self, other: &Self) -> bool {
        self.hash == other.hash
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Light {
    pub position: [f32; 3],
    pub color: [f32; 3],
    pub intensity: f32,
    pub radius: f32,
    pub range: f32,
    pub shadow: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Haze {
    pub lo: [f32; 3],
    pub hi: [f32; 3],
    pub fog: f32,
    pub smoke: f32,
    pub mist: f32,
    pub floor: f32,
    pub phase: f32,
    pub back: f32,
    pub gold: [f32; 3],
    pub ambient: [f32; 3],
    pub reach: f32,
    pub unmapped: f32,
    pub seed: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoverKind {
    Turn,
    Slide,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    Swing,
    Loop,
    Once,
}

impl Motion {
    pub fn at(self, u: f32) -> f32 {
        match self {
            Self::Swing => (1.0 - (std::f32::consts::TAU * u).cos()) * 0.5,
            Self::Loop => u - u.floor(),
            Self::Once => u.clamp(0.0, 1.0),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Mover {
    pub objects: Vec<String>,
    pub kind: MoverKind,
    pub pivot: [f32; 3],
    pub axis: [f32; 3],
    pub travel: [f32; 2],
    pub period: f32,
    pub motion: Motion,
    pub offset: f32,
    pub clip: Vec<[f32; 4]>,
}

impl Mover {
    pub fn value(&self, time: f32) -> f32 {
        let u = (time + self.offset) / self.period;
        let [from, to] = self.travel;
        from + (to - from) * self.motion.at(u)
    }

    pub fn transform(&self, time: f32) -> Matrix {
        let value = self.value(time);
        match self.kind {
            MoverKind::Slide => {
                let mut out = IDENTITY;
                out[3] = [
                    self.axis[0] * value,
                    self.axis[1] * value,
                    self.axis[2] * value,
                    1.0,
                ];
                out
            }
            MoverKind::Turn => {
                let [x, y, z] = self.axis;
                let (s, c) = value.to_radians().sin_cos();
                let t = 1.0 - c;
                let r: Matrix = [
                    [t * x * x + c, t * x * y + s * z, t * x * z - s * y, 0.0],
                    [t * x * y - s * z, t * y * y + c, t * y * z + s * x, 0.0],
                    [t * x * z + s * y, t * y * z - s * x, t * z * z + c, 0.0],
                    [0.0, 0.0, 0.0, 1.0],
                ];
                let mut to = IDENTITY;
                to[3] = [self.pivot[0], self.pivot[1], self.pivot[2], 1.0];
                let mut back = IDENTITY;
                back[3] = [-self.pivot[0], -self.pivot[1], -self.pivot[2], 1.0];
                multiply(to, multiply(r, back))
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Emitter {
    pub position: [f32; 3],
    pub radius: f32,
    pub color: [f32; 3],
    pub intensity: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Projection {
    Perspective { fov: f32 },
    Orthographic { height: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DepthOfField {
    pub fstop: f32,
    pub distance: f32,
    pub focal: f32,
}

impl DepthOfField {
    pub fn aperture(&self) -> f32 {
        self.focal / self.fstop
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub projection: Projection,
    pub at: [f32; 3],
    pub look_at: [f32; 3],
    pub up: [f32; 3],
    pub near: f32,
    pub far: f32,
    pub shift: [f32; 2],
    pub depth_of_field: Option<DepthOfField>,
    pub preset: Preset,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            projection: Projection::Perspective { fov: 40.0 },
            at: [0.0, 0.0, 5.0],
            look_at: [0.0; 3],
            up: [0.0, 1.0, 0.0],
            near: 0.05,
            far: 100.0,
            shift: [0.0; 2],
            depth_of_field: None,
            preset: Preset::default(),
        }
    }
}

impl Camera {
    pub fn axes(&self) -> ([f32; 3], [f32; 3], [f32; 3]) {
        let forward = unit(sub(self.look_at, self.at));
        let right = unit(cross(forward, self.up));
        let up = cross(right, forward);
        (forward, right, up)
    }

    pub fn view(&self) -> Matrix {
        let (forward, right, up) = self.axes();
        let eye = self.at;
        [
            [right[0], up[0], -forward[0], 0.0],
            [right[1], up[1], -forward[1], 0.0],
            [right[2], up[2], -forward[2], 0.0],
            [-dot(right, eye), -dot(up, eye), dot(forward, eye), 1.0],
        ]
    }

    pub fn projection(&self, aspect: f32) -> Matrix {
        let (near, far) = (self.near, self.far);
        let [sx, sy] = self.shift;
        match self.projection {
            Projection::Perspective { fov } => {
                let tan = (fov.to_radians() * 0.5).tan();
                [
                    [1.0 / (tan * aspect), 0.0, 0.0, 0.0],
                    [0.0, 1.0 / tan, 0.0, 0.0],
                    [sx, sy, far / (near - far), -1.0],
                    [0.0, 0.0, far * near / (near - far), 0.0],
                ]
            }
            Projection::Orthographic { height } => {
                let width = height * aspect;
                [
                    [2.0 / width, 0.0, 0.0, 0.0],
                    [0.0, 2.0 / height, 0.0, 0.0],
                    [0.0, 0.0, 1.0 / (near - far), 0.0],
                    [-sx, -sy, near / (near - far), 1.0],
                ]
            }
        }
    }

    pub fn tan_half_height(&self) -> Option<f32> {
        match self.projection {
            Projection::Perspective { fov } => Some((fov.to_radians() * 0.5).tan()),
            Projection::Orthographic { .. } => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Finish {
    pub chain: Chain,
    pub hash: [u8; 32],
}

impl PartialEq for Finish {
    fn eq(&self, other: &Self) -> bool {
        self.hash == other.hash
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TraceSettings {
    pub transmissive_shadows: bool,
    pub clamp_indirect: f32,
    pub filter_glossy: f32,
}

impl TraceSettings {
    pub const CLAMP_INDIRECT_MAX: f32 = 10_000.0;
    pub const FILTER_GLOSSY_MAX: f32 = 1.0;

    pub fn check(&self) -> Result<(), String> {
        if !(0.0..=Self::CLAMP_INDIRECT_MAX).contains(&self.clamp_indirect) {
            return Err(format!(
                "clamp_indirect = {} is outside 0 to {}",
                self.clamp_indirect,
                Self::CLAMP_INDIRECT_MAX
            ));
        }
        if !(0.0..=Self::FILTER_GLOSSY_MAX).contains(&self.filter_glossy) {
            return Err(format!(
                "filter_glossy = {} is outside 0 to {}",
                self.filter_glossy,
                Self::FILTER_GLOSSY_MAX
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Content {
    pub image: PathBuf,
    pub width: u32,
    pub height: u32,
    pub srgb: bool,
    pub rgba: Arc<Vec<u8>>,
    pub hash: [u8; 32],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Text {
    pub text: String,
    pub font: PathBuf,
    pub font_bytes: Arc<Vec<u8>>,
    pub family: String,
    pub lit: bool,
    pub size: f32,
    pub color: [f32; 4],
    pub at: [f32; 3],
    pub rotate: [f32; 3],
    pub place: Matrix,
    pub dynamic: bool,
}

pub const TEXT_NOMINAL: f32 = 1638.4;
pub const TEXT_WEIGHT: u16 = 400;
const OPEN: [f32; 4] = [-1e9, -1e9, 1e9, 1e9];
const FLIP: Matrix = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, -1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

impl Text {
    pub fn paragraph(&self) -> Result<RichParagraph, String> {
        let mut engine = TextEngine::new(&self.font_bytes)
            .map_err(|error| format!("{}: {error:?}", self.font.display()))?;
        let spans = [Span {
            text: self.text.clone(),
            face: Face {
                family: self.family.clone(),
                size: self.size,
                line: self.size * 1.25,
                weight: TEXT_WEIGHT,
                italic: false,
                spacing: 0.0,
            },
            color: self.color,
        }];
        let scale = TEXT_NOMINAL / self.size;
        let block = engine
            .layout_spans(&spans, None, scale, Anchor::Start)
            .map_err(|error| format!("{}: {error:?}", self.font.display()))?;
        let origin = [-block.width * 0.5, -block.height * 0.5];
        engine
            .render_spans(
                &spans,
                None,
                scale,
                Anchor::Start,
                Representation::Msdf,
                origin,
                1.0,
                OPEN,
                [0.0; 3],
            )
            .map_err(|error| format!("{}: {error:?}", self.font.display()))
    }

    pub fn model(&self) -> Matrix {
        multiply(
            self.place,
            multiply(model(self.at, self.rotate, [1.0; 3]), FLIP),
        )
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scene {
    pub path: PathBuf,
    pub files: BTreeMap<PathBuf, [u8; 32]>,
    pub library: Library,
    pub fallback: Option<String>,
    pub meshes: BTreeMap<String, SceneMesh>,
    pub objects: Vec<Object>,
    pub sun: Option<Sun>,
    pub sky: Option<SceneSky>,
    pub haze: Option<Haze>,
    pub movers: BTreeMap<String, Mover>,
    pub lights: BTreeMap<String, Light>,
    pub emitters: BTreeMap<String, Emitter>,
    pub camera: Option<Camera>,
    pub finish: Option<Finish>,
    pub trace: TraceSettings,
    pub contents: BTreeMap<String, Content>,
    pub texts: BTreeMap<String, Text>,
    pub plates: Option<Plates>,
    pub physics: Option<Physics>,
    pub bodies: BTreeMap<String, Body>,
    pub body_layers: BTreeMap<String, String>,
    pub characters: BTreeMap<String, Character>,
    pub animations: BTreeMap<String, Animation>,
    pub triggers: BTreeMap<String, Trigger>,
    pub layers: BTreeMap<String, Vec<String>>,
    pub sounds: BTreeMap<String, Sound>,
    pub warnings: Vec<SceneError>,
}

impl Scene {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self, SceneError> {
        build::open(path.as_ref(), None)
    }

    pub fn open_at(path: impl AsRef<Path>, hour: f64) -> Result<Self, SceneError> {
        build::open(path.as_ref(), Some(hour))
    }

    pub fn sun_or_dark(&self) -> Sun {
        self.sun.unwrap_or_else(Sun::dark)
    }

    pub fn physics_or_default(&self) -> Physics {
        self.physics.unwrap_or_default()
    }

    pub fn camera_or_default(&self) -> Camera {
        self.camera.unwrap_or_default()
    }

    pub fn environment(&self) -> Environment {
        self.sky
            .as_ref()
            .map(|sky| sky.environment.clone())
            .unwrap_or_else(Environment::black)
    }

    pub fn object(&self, name: &str) -> Option<&Object> {
        self.objects.iter().find(|object| object.name == name)
    }

    pub fn draws(&self) -> Draws {
        draws::draws(self)
    }

    pub fn diff(&self, newer: &Scene) -> SceneDiff {
        diff::diff(self, newer)
    }

    pub fn animated(&self) -> bool {
        !self.movers.is_empty()
            || self
                .objects
                .iter()
                .any(|object| object.face_camera && !object.hidden)
    }

    pub fn posed(&self, camera: &Camera, time: f32) -> Vec<Matrix> {
        pose::objects(self, camera, time)
    }
}

pub fn model(at: [f32; 3], rotate: [f32; 3], scale: [f32; 3]) -> Matrix {
    let [x, y, z] = rotate.map(f32::to_radians);
    let (sx, cx) = x.sin_cos();
    let (sy, cy) = y.sin_cos();
    let (sz, cz) = z.sin_cos();
    let rx: Matrix = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, cx, sx, 0.0],
        [0.0, -sx, cx, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let ry: Matrix = [
        [cy, 0.0, -sy, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [sy, 0.0, cy, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let rz: Matrix = [
        [cz, sz, 0.0, 0.0],
        [-sz, cz, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let s: Matrix = [
        [scale[0], 0.0, 0.0, 0.0],
        [0.0, scale[1], 0.0, 0.0],
        [0.0, 0.0, scale[2], 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let mut t = IDENTITY;
    t[3] = [at[0], at[1], at[2], 1.0];
    multiply(t, multiply(rz, multiply(ry, multiply(rx, s))))
}

pub fn multiply(a: Matrix, b: Matrix) -> Matrix {
    let mut out = [[0.0; 4]; 4];
    for column in 0..4 {
        for row in 0..4 {
            out[column][row] = (0..4).map(|k| a[k][row] * b[column][k]).sum();
        }
    }
    out
}

fn unit(v: [f32; 3]) -> [f32; 3] {
    let length = dot(v, v).sqrt();
    if length > 0.0 {
        v.map(|x| x / length)
    } else {
        v
    }
}
