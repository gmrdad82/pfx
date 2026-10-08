use std::collections::{BTreeMap, BTreeSet};
use std::f32::consts::TAU;
use std::fs;
use std::path::{Component, Path, PathBuf};

use pfx_core::daylight::{Authored, DampedClock, Daylight, REFERENCE_HOUR, SunArc};
use pfx_load::{Mesh, RoomSky, Sky};
use pfx_materials::{Library, Material};
use pfx_trace::Sun;
use pfx_trace::bvh::Triangle;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::compose::{self, Files, Palette};
use crate::reflection::ReflectionSpec;
use crate::{Anchor, BakeScene, GridSpec, emitter_hash, scene_hash};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Volume {
    pub name: String,
    #[serde(flatten)]
    pub grid: GridSpec,
}

#[derive(Clone, Debug)]
pub struct Recipe {
    pub scene: String,
    pub product: String,
    pub materials: Option<Materials>,
    pub fallback: Option<String>,
    pub tint: BTreeMap<String, [f32; 3]>,
    pub anchors: Vec<f32>,
    pub samples: u32,
    pub seed: u32,
    pub day: f64,
    pub latitude: f64,
    pub heading: f64,
    pub reference_hour: f64,
    pub sky: SkyRecipe,
    pub sun: SunModel,
    pub detail: Option<DetailRecipe>,
    pub shadow_only_nodes: Vec<String>,
    pub volumes: Vec<Volume>,
    pub reflections: Vec<ReflectionSpec>,
    pub assets_env: Option<String>,
    pub files: Vec<FileRecipe>,
    pub poses: Vec<PoseRecipe>,
    pub shadow_only_materials: Vec<String>,
    pub drop_degenerate: bool,
    pub emitters: Vec<EmitterRecipe>,
    pub emitter_layer: Option<EmitterLayer>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Materials {
    Library(String),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    scene: String,
    app: Option<String>,
    product: Option<String>,
    materials: Option<String>,
    fallback: Option<String>,
    #[serde(default)]
    tint: BTreeMap<String, [f32; 3]>,
    anchors: Vec<f32>,
    samples: u32,
    #[serde(default = "default_seed")]
    seed: u32,
    #[serde(default = "default_day")]
    day: f64,
    #[serde(default = "default_latitude")]
    latitude: f64,
    #[serde(default = "default_heading")]
    heading: f64,
    #[serde(default = "default_reference_hour")]
    reference_hour: f64,
    #[serde(default)]
    sky: RawSky,
    sun: Option<RawSun>,
    detail: Option<DetailRecipe>,
    #[serde(default)]
    shadow_only_nodes: Vec<String>,
    #[serde(default)]
    volumes: Vec<Volume>,
    #[serde(default, rename = "reflection")]
    reflections: Vec<ReflectionSpec>,
    assets_env: Option<String>,
    clock: Option<RawClock>,
    #[serde(default, rename = "file")]
    files: Vec<FileRecipe>,
    #[serde(default, rename = "pose")]
    poses: Vec<PoseRecipe>,
    #[serde(default)]
    shadow_only_materials: Vec<String>,
    #[serde(default)]
    drop_degenerate: bool,
    #[serde(default, rename = "emitter")]
    emitters: Vec<EmitterRecipe>,
    emitter_layer: Option<EmitterLayer>,
    #[serde(default, rename = "plates")]
    _plates: Option<toml::Value>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SunRecipe {
    pub toward: [f64; 3],
    pub color: [f64; 3],
    pub irradiance: f32,
    pub sky_fill: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SunModel {
    Solar,
    Fixed(SunRecipe),
    Authored(SunRecipe),
    Clock { clock: DampedClock, base: [f64; 3] },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSun {
    model: Option<String>,
    toward: Option<[f64; 3]>,
    color: Option<[f64; 3]>,
    irradiance: Option<f32>,
    sky_fill: Option<f64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawClock {
    toward: [f64; 3],
    sun: Option<f64>,
    tint: Option<f64>,
    elevation: Option<f64>,
    azimuth: Option<f64>,
    skywarm: Option<f64>,
    lamp: Option<f64>,
    exposure: Option<f64>,
    warmth: Option<f64>,
    turn: Option<f64>,
    arc: Option<RawArc>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawArc {
    start: f64,
    span: f64,
    elevation: f64,
    azimuth: f64,
    sweep: f64,
    reference: f64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileRecipe {
    pub path: String,
    pub translation: Option<[f32; 3]>,
    pub keep_prefix: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PoseRecipe {
    pub file: Option<String>,
    pub node: String,
    #[serde(default)]
    pub translation: [f32; 3],
    #[serde(default)]
    pub yaw: f32,
    #[serde(default = "unit_scale")]
    pub scale: [f32; 3],
    #[serde(default)]
    pub materials: BTreeMap<String, String>,
}

fn unit_scale() -> [f32; 3] {
    [1.0; 3]
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmitterRecipe {
    pub node: Option<String>,
    pub position: Option<[f32; 3]>,
    pub radius: Option<f32>,
    #[serde(alias = "colour")]
    pub color: [f32; 3],
    pub intensity: f32,
    pub scales: Option<Vec<f32>>,
    pub preset: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmitterLayer {
    #[serde(default)]
    pub direct: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DetailRecipe {
    #[serde(default = "default_detail_size")]
    pub size: u32,
    #[serde(default = "default_detail_padding")]
    pub padding: u32,
    #[serde(default)]
    pub dynamic_nodes: Vec<String>,
    #[serde(default)]
    pub parts: Vec<String>,
    #[serde(default)]
    pub sizes: BTreeMap<String, u32>,
    #[serde(default)]
    pub view: Option<DetailView>,
    #[serde(default = "default_detail_page")]
    pub page: u32,
}

fn default_detail_page() -> u32 {
    crate::detail::atlas::PAGE
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DetailView {
    pub position: [f32; 3],
    pub target: [f32; 3],
    pub fov_y_deg: f32,
    #[serde(default)]
    pub shift: [f32; 2],
    #[serde(default = "default_view_width")]
    pub width: u32,
    #[serde(default = "default_view_height")]
    pub height: u32,
}

fn default_view_width() -> u32 {
    3840
}

fn default_view_height() -> u32 {
    2160
}

impl DetailRecipe {
    pub fn check(&self) -> Result<(), String> {
        let valid = |size: u32| {
            (crate::detail::FLOOR..=crate::detail::LIMIT).contains(&size) && size.is_power_of_two()
        };
        if !valid(self.size) || self.padding > self.size / 4 {
            return Err(format!(
                "detail needs a power-of-two size from {} to {} and valid padding",
                crate::detail::FLOOR,
                crate::detail::LIMIT
            ));
        }
        if !(crate::detail::FLOOR..=crate::detail::atlas::PAGE_LIMIT).contains(&self.page)
            || !self.page.is_power_of_two()
        {
            return Err(format!(
                "detail page needs a power of two from {} to {}",
                crate::detail::FLOOR,
                crate::detail::atlas::PAGE_LIMIT
            ));
        }
        if self.dynamic_nodes.iter().any(|name| name.is_empty()) {
            return Err("detail dynamic nodes need names".into());
        }
        if self.parts.iter().any(|name| name.is_empty()) {
            return Err("detail parts need names".into());
        }
        for (name, &size) in &self.sizes {
            if name.is_empty() || !valid(size) || self.padding > size / 4 {
                return Err(format!(
                    "detail size for {name:?} needs a power of two from {} to {} and valid padding",
                    crate::detail::FLOOR,
                    crate::detail::LIMIT
                ));
            }
        }
        if let Some(view) = &self.view
            && (view.width == 0
                || view.height == 0
                || !(view.fov_y_deg > 0.0 && view.fov_y_deg < 180.0)
                || view.position == view.target
                || view
                    .position
                    .iter()
                    .chain(&view.target)
                    .chain(&view.shift)
                    .any(|value| !value.is_finite()))
        {
            return Err("detail view needs a camera apart from its target, a field of view between 0 and 180 degrees and a size".into());
        }
        Ok(())
    }
}

fn default_detail_size() -> u32 {
    1024
}

fn default_detail_padding() -> u32 {
    8
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkyKind {
    Daylight,
    Hdr,
    Room,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ambient {
    Authored,
    Clock,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Prepare {
    pub cap: Option<f32>,
    pub balance: Option<[f32; 3]>,
    pub mean: Option<f32>,
}

impl Prepare {
    pub fn apply(&self, sky: Sky) -> Sky {
        let sky = match self.cap {
            Some(cap) => sky.capped_luminance(cap),
            None => sky,
        };
        let sky = match self.balance {
            Some(balance) => sky.balanced_to(balance),
            None => sky,
        };
        match self.mean {
            Some(mean) => sky.scaled_to_mean_luminance(mean),
            None => sky,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SkyRecipe {
    pub kind: SkyKind,
    pub path: Option<String>,
    pub prepare: Option<Prepare>,
    pub rotation_deg: f32,
    pub intensity: f32,
    pub shade: BTreeMap<String, [f32; 3]>,
    pub room: Option<RoomSky>,
    pub ambient: Option<Ambient>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSky {
    kind: Option<String>,
    path: Option<String>,
    prepare: Option<toml::Value>,
    balance: Option<[f32; 3]>,
    #[serde(default)]
    rotation_deg: f32,
    #[serde(default = "unit_intensity")]
    intensity: f32,
    #[serde(default)]
    shade: BTreeMap<String, [f32; 3]>,
    room: Option<RoomSky>,
    ambient: Option<String>,
}

impl Default for RawSky {
    fn default() -> Self {
        Self {
            kind: None,
            path: None,
            prepare: None,
            balance: None,
            rotation_deg: 0.0,
            intensity: 1.0,
            shade: BTreeMap::new(),
            room: None,
            ambient: None,
        }
    }
}

fn unit_intensity() -> f32 {
    1.0
}

fn default_seed() -> u32 {
    7
}
fn default_day() -> f64 {
    Daylight::DAY
}
fn default_latitude() -> f64 {
    Daylight::LATITUDE
}
fn default_heading() -> f64 {
    Daylight::HEADING
}
fn default_reference_hour() -> f64 {
    REFERENCE_HOUR
}

fn slug(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn rgb_valid(rgb: &[f32; 3]) -> bool {
    rgb.iter().all(|v| v.is_finite() && *v >= 0.0)
}

fn material_source(name: Option<&str>) -> Result<Option<Materials>, String> {
    let Some(name) = name else {
        return Ok(None);
    };
    if name.ends_with(".toml") {
        let path = Path::new(name);
        if path.is_absolute()
            || path.file_stem().is_none()
            || !path
                .components()
                .all(|part| matches!(part, Component::Normal(_) | Component::ParentDir))
        {
            return Err(
                "materials names a library .toml file by a path relative to the recipe".into(),
            );
        }
        return Ok(Some(Materials::Library(name.to_owned())));
    }
    Err("materials names a library .toml file relative to the recipe".into())
}

fn authored(sun: &RawSun, what: &str) -> Result<SunRecipe, String> {
    let (Some(toward), Some(color), Some(irradiance)) = (sun.toward, sun.color, sun.irradiance)
    else {
        return Err(format!("{what} needs toward, color and irradiance"));
    };
    if toward.iter().any(|v| !v.is_finite())
        || toward.iter().map(|v| v * v).sum::<f64>() <= 1e-12
        || color.iter().any(|v| !v.is_finite() || *v < 0.0)
        || !irradiance.is_finite()
        || irradiance < 0.0
    {
        return Err("sun needs finite toward, color and nonnegative irradiance".into());
    }
    let sky_fill = sun.sky_fill.unwrap_or(0.0);
    if !sky_fill.is_finite() || sky_fill < 0.0 {
        return Err("sun sky_fill must be finite and nonnegative".into());
    }
    Ok(SunRecipe {
        toward,
        color,
        irradiance,
        sky_fill,
    })
}

fn clock_model(clock: &RawClock) -> Result<SunModel, String> {
    let (
        Some(sun),
        Some(tint),
        Some(elevation),
        Some(azimuth),
        Some(skywarm),
        Some(lamp),
        Some(exposure),
        Some(warmth),
        Some(turn),
    ) = (
        clock.sun,
        clock.tint,
        clock.elevation,
        clock.azimuth,
        clock.skywarm,
        clock.lamp,
        clock.exposure,
        clock.warmth,
        clock.turn,
    )
    else {
        return Err("[clock] for sun model = 'clock' needs toward, sun, tint, elevation, azimuth, skywarm, lamp, exposure, warmth and turn".into());
    };
    let Some(arc) = &clock.arc else {
        return Err("[clock] needs a [clock.arc] table with start, span, elevation, azimuth, sweep and reference".into());
    };
    let arc = SunArc {
        start: arc.start,
        span: arc.span,
        elevation: arc.elevation,
        azimuth: arc.azimuth,
        sweep: arc.sweep,
        reference: arc.reference,
    };
    if [
        arc.start,
        arc.span,
        arc.elevation,
        arc.azimuth,
        arc.sweep,
        arc.reference,
    ]
    .iter()
    .any(|v| !v.is_finite())
        || arc.span <= 0.0
    {
        return Err("clock.arc needs finite values and a positive span".into());
    }
    let toward = clock.toward;
    if toward.iter().any(|v| !v.is_finite())
        || toward.iter().map(|v| v * v).sum::<f64>() <= 1e-12
        || [sun, tint, skywarm, lamp, exposure, warmth]
            .iter()
            .any(|v| !v.is_finite() || *v < 0.0)
        || [elevation, azimuth, turn].iter().any(|v| !v.is_finite())
    {
        return Err("clock needs a direction and finite, nonnegative values".into());
    }
    Ok(SunModel::Clock {
        clock: DampedClock {
            sun,
            tint,
            elevation,
            azimuth,
            skywarm,
            lamp,
            exposure,
            warmth,
            turn,
            arc,
        },
        base: toward,
    })
}

fn sun_model(raw: &Raw) -> Result<SunModel, String> {
    if let Some(sun) = &raw.sun
        && let Some(model) = sun.model.as_deref()
    {
        let values = sun.toward.is_some() || sun.color.is_some() || sun.irradiance.is_some();
        return match model {
            "solar" | "clock" if values || sun.sky_fill.is_some() => Err(format!(
                "sun model = '{model}' takes no toward, color, irradiance or sky_fill"
            )),
            "fixed" if sun.sky_fill.is_some() => {
                Err("sky_fill belongs to sun model = 'authored'".into())
            }
            "solar" | "fixed" | "authored" if raw.clock.is_some() => {
                Err("clock and sun cannot both set the light; use [sun] model = 'clock'".into())
            }
            "solar" => Ok(SunModel::Solar),
            "fixed" => Ok(SunModel::Fixed(authored(sun, "sun model = 'fixed'")?)),
            "authored" => Ok(SunModel::Authored(authored(sun, "sun model = 'authored'")?)),
            "clock" => match &raw.clock {
                Some(clock) => clock_model(clock),
                None => Err("sun model = 'clock' needs a [clock] table".into()),
            },
            other => Err(format!(
                "sun model is 'solar', 'fixed', 'authored' or 'clock', not '{other}'"
            )),
        };
    }
    match (&raw.sun, &raw.clock) {
        (Some(_), _) => Err(
            "[sun] needs model = 'solar', 'fixed', 'authored' or 'clock'; values alone need model = 'authored'"
                .into(),
        ),
        (None, Some(_)) => Err("[clock] needs [sun] model = 'clock'".into()),
        (None, None) => Ok(SunModel::Solar),
    }
}

fn sky_recipe(sky: RawSky, sun: &SunModel) -> Result<SkyRecipe, String> {
    let kind = match sky.kind.as_deref() {
        None | Some("daylight") | Some("analytic") => SkyKind::Daylight,
        Some("hdr") => SkyKind::Hdr,
        Some("room") => SkyKind::Room,
        Some(other) => {
            return Err(format!(
                "sky kind is 'daylight', 'hdr' with a path, or 'room' with a [sky.room] table, not '{other}'"
            ));
        }
    };
    let spelled = sky.kind.as_deref().unwrap_or("daylight");
    match (kind, sky.path.is_some()) {
        (SkyKind::Hdr, false) => return Err("sky kind = 'hdr' needs a path".into()),
        (SkyKind::Daylight | SkyKind::Room, true) => {
            return Err("a sky path needs kind = 'hdr'".into());
        }
        _ => {}
    }
    match (kind, &sky.room) {
        (SkyKind::Room, None) => {
            return Err(format!("sky kind = '{spelled}' needs a [sky.room] table"));
        }
        (SkyKind::Room, Some(room)) => {
            if sky.prepare.is_some() {
                return Err(format!("sky kind = '{spelled}' takes no prepare"));
            }
            if sky.balance.is_some() {
                return Err(format!("sky kind = '{spelled}' takes no balance"));
            }
            room.check().map_err(|e| format!("sky.room: {e}"))?;
        }
        (_, Some(_)) => return Err("[sky.room] needs kind = 'room'".into()),
        _ => {}
    }
    if !sky.rotation_deg.is_finite()
        || !sky.intensity.is_finite()
        || sky.intensity < 0.0
        || sky
            .balance
            .is_some_and(|balance| balance.into_iter().any(|v| !v.is_finite() || v <= 0.0))
    {
        return Err("sky preparation values must be finite and nonnegative".into());
    }
    if kind == SkyKind::Daylight
        && (sky.balance.is_some() || sky.rotation_deg != 0.0 || sky.intensity != 1.0)
    {
        return Err("sky preparation needs an HDR sky".into());
    }
    let prepare = match &sky.prepare {
        None => None,
        Some(toml::Value::String(name)) => {
            return Err(format!(
                "sky.prepare is a [sky.prepare] table of cap, balance and mean, not '{name}'"
            ));
        }
        Some(value) => {
            let steps: Prepare = value
                .clone()
                .try_into()
                .map_err(|e| format!("sky.prepare: {e}"))?;
            if kind != SkyKind::Hdr {
                return Err("[sky.prepare] needs kind = 'hdr'".into());
            }
            if sky.balance.is_some() {
                return Err("[sky.prepare] takes balance in its own table".into());
            }
            if steps == Prepare::default() {
                return Err("[sky.prepare] needs cap, balance or mean".into());
            }
            let positive = |v: f32| v.is_finite() && v > 0.0;
            if steps.cap.is_some_and(|v| !positive(v))
                || steps.mean.is_some_and(|v| !positive(v))
                || steps
                    .balance
                    .is_some_and(|balance| !balance.into_iter().all(positive))
            {
                return Err("[sky.prepare] values must be finite and positive".into());
            }
            Some(steps)
        }
    };
    if prepare.is_none() && sky.balance.is_some() {
        return Err("sky balance belongs in a [sky.prepare] table".into());
    }
    let ambient = match sky.ambient.as_deref() {
        None => None,
        Some("authored") => {
            if kind == SkyKind::Daylight || !matches!(sun, SunModel::Authored(_)) {
                return Err(
                    "sky.ambient = 'authored' needs an HDR or room sky and [sun] model = 'authored'"
                        .into(),
                );
            }
            Some(Ambient::Authored)
        }
        Some("clock") => {
            if kind != SkyKind::Hdr || !matches!(sun, SunModel::Clock { .. }) {
                return Err(
                    "sky.ambient = 'clock' needs an HDR sky and [sun] model = 'clock'".into(),
                );
            }
            Some(Ambient::Clock)
        }
        Some(other) => {
            return Err(format!(
                "sky.ambient is 'authored' or 'clock', not '{other}'"
            ));
        }
    };
    if ambient == Some(Ambient::Clock) {
        if !prepare.is_some_and(|p| p.mean.is_some() && p.cap.is_none() && p.balance.is_none()) {
            return Err("sky.ambient = 'clock' needs [sky.prepare] with mean alone".into());
        }
        if sky.rotation_deg != 0.0 || sky.intensity != 1.0 {
            return Err(
                "sky.ambient = 'clock' turns the sky by the clock, so it takes no rotation or intensity"
                    .into(),
            );
        }
    }
    Ok(SkyRecipe {
        kind,
        path: sky.path,
        prepare,
        rotation_deg: sky.rotation_deg,
        intensity: sky.intensity,
        shade: sky.shade,
        room: sky.room,
        ambient,
    })
}

pub struct SceneSource {
    pub recipe: Recipe,
    pub scene: BakeScene,
    pub root: PathBuf,
    pub emitters: Vec<usize>,
    source_digest: [u8; 32],
    prepared_sky: Option<Sky>,
}

pub(crate) fn read_inside(root: &Path, name: &str) -> Result<Vec<u8>, String> {
    let path = Path::new(name);
    if path.components().next().is_none()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(format!("{name} is outside the scene folder"));
    }
    let file = root
        .join(path)
        .canonicalize()
        .map_err(|e| format!("{name}: {e}"))?;
    if !file.starts_with(root) || !file.is_file() {
        return Err(format!("{name} is outside the scene folder"));
    }
    fs::read(file).map_err(|e| format!("{name}: {e}"))
}

impl Recipe {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let text = std::str::from_utf8(bytes).map_err(|e| format!("bake.toml: {e}"))?;
        let raw: Raw = toml::from_str(text).map_err(|e| format!("bake.toml: {e}"))?;
        let product = match (&raw.app, &raw.product) {
            (Some(_), Some(_)) => {
                return Err("app and product name the same thing; keep app".into());
            }
            (Some(app), None) | (None, Some(app)) => app.clone(),
            (None, None) => return Err("bake.toml needs app = \"<slug>\"".into()),
        };
        if !slug(&product) {
            return Err(
                "app (or product) must be a slug of lowercase letters, digits and hyphens".into(),
            );
        }
        let materials = material_source(raw.materials.as_deref())?;
        let library = matches!(materials, Some(Materials::Library(_)));
        if let Some(fallback) = &raw.fallback {
            if !library {
                return Err("fallback needs materials = \"<library.toml>\"".into());
            }
            if fallback.is_empty() {
                return Err("fallback needs a material name".into());
            }
        }
        if !raw.tint.is_empty() {
            if !library {
                return Err("[tint] needs materials = \"<library.toml>\"".into());
            }
            if raw
                .tint
                .iter()
                .any(|(name, rgb)| name.is_empty() || !rgb_valid(rgb))
            {
                return Err("tint names materials with finite, nonnegative colours".into());
            }
        }
        if !(0.0..=24.0).contains(&raw.reference_hour) {
            return Err(format!(
                "reference_hour = {} is outside 0 to 24",
                raw.reference_hour
            ));
        }
        let sun = sun_model(&raw)?;
        let sky = sky_recipe(raw.sky, &sun)?;
        let recipe = Self {
            scene: raw.scene,
            product,
            materials,
            fallback: raw.fallback,
            tint: raw.tint,
            anchors: raw.anchors,
            samples: raw.samples,
            seed: raw.seed,
            day: raw.day,
            latitude: raw.latitude,
            heading: raw.heading,
            reference_hour: raw.reference_hour,
            sky,
            sun,
            detail: raw.detail,
            shadow_only_nodes: raw.shadow_only_nodes,
            volumes: raw.volumes,
            reflections: raw.reflections,
            assets_env: raw.assets_env,
            files: raw.files,
            poses: raw.poses,
            shadow_only_materials: raw.shadow_only_materials,
            drop_degenerate: raw.drop_degenerate,
            emitters: raw.emitters,
            emitter_layer: raw.emitter_layer,
        };
        recipe.validate()?;
        Ok(recipe)
    }

    fn validate(&self) -> Result<(), String> {
        if !self.scene.ends_with(".gltf") && !self.scene.ends_with(".glb") {
            return Err("scene must name a .gltf or .glb file".into());
        }
        if self.samples == 0 {
            return Err("samples must be positive".into());
        }
        if self.anchors.is_empty()
            || self
                .anchors
                .iter()
                .any(|v| !v.is_finite() || !(0.0..=24.0).contains(v))
            || self.anchors.windows(2).any(|v| v[0] >= v[1])
        {
            return Err("anchors must be sorted, distinct hours from 0 to 24".into());
        }
        Daylight::parse([
            (
                "hour",
                pfx_core::daylight::Reading::Number(self.anchors[0] as f64),
            ),
            ("day", pfx_core::daylight::Reading::Number(self.day)),
            (
                "latitude",
                pfx_core::daylight::Reading::Number(self.latitude),
            ),
            ("heading", pfx_core::daylight::Reading::Number(self.heading)),
        ])?;
        if self.volumes.is_empty() && self.reflections.is_empty() && self.detail.is_none() {
            return Err("at least one probe volume or reflection is required".into());
        }
        if let Some(detail) = &self.detail {
            detail.check()?;
        }
        if self.shadow_only_nodes.iter().any(|name| name.is_empty()) {
            return Err("shadow-only nodes need names".into());
        }
        let mut names = BTreeSet::new();
        for volume in &self.volumes {
            if volume.name.is_empty()
                || !volume
                    .name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                || !names.insert(volume.name.clone())
            {
                return Err("volume names must be unique ASCII labels".into());
            }
            volume.grid.dimensions()?;
        }
        for (index, reflection) in self.reflections.iter().enumerate() {
            reflection.validate()?;
            let name = reflection.name(index);
            if !names.insert(name) {
                return Err("reflection names must be unique from volume names".into());
            }
        }
        self.validate_composition()
    }

    fn validate_composition(&self) -> Result<(), String> {
        let library = matches!(self.materials, Some(Materials::Library(_)));
        for (key, shade) in &self.sky.shade {
            let hour: f32 = key
                .parse()
                .map_err(|_| format!("sky.shade key {key} is not an hour"))?;
            if !self.anchors.contains(&hour) {
                return Err(format!("sky.shade key {key} is not one of the anchors"));
            }
            if !rgb_valid(shade) {
                return Err("sky.shade values must be finite and nonnegative".into());
            }
        }
        let gltf = |name: &str| name.ends_with(".gltf") || name.ends_with(".glb");
        for file in &self.files {
            if !gltf(&file.path) {
                return Err("file must name a .gltf or .glb file".into());
            }
            if file
                .translation
                .is_some_and(|v| v.iter().any(|c| !c.is_finite()))
            {
                return Err("file translation must be finite".into());
            }
            if file.keep_prefix.as_deref().is_some_and(str::is_empty)
                || (file.keep_prefix.is_some() && file.translation.is_none())
            {
                return Err("file keep_prefix needs a name and a translation".into());
            }
        }
        if self
            .shadow_only_materials
            .iter()
            .any(|name| name.is_empty())
        {
            return Err("shadow-only materials need names".into());
        }
        for pose in &self.poses {
            if pose.node.is_empty() || pose.file.as_deref().is_some_and(|name| !gltf(name)) {
                return Err("pose needs a node and a .gltf or .glb file".into());
            }
            if pose
                .translation
                .iter()
                .chain(&pose.scale)
                .chain([&pose.yaw])
                .any(|v| !v.is_finite())
                || pose.scale.contains(&0.0)
            {
                return Err("pose values must be finite and its scale nonzero".into());
            }
            if !pose.materials.is_empty() && !library {
                return Err("pose materials need materials = \"<library.toml>\"".into());
            }
            if pose
                .materials
                .iter()
                .any(|(name, spec)| name.is_empty() || spec.is_empty())
            {
                return Err("pose materials map glTF material names to library names".into());
            }
        }
        for emitter in &self.emitters {
            let placed = match (&emitter.node, emitter.position, emitter.radius) {
                (Some(node), None, None) => !node.is_empty(),
                (None, Some(position), Some(radius)) => {
                    position.iter().all(|v| v.is_finite()) && radius.is_finite() && radius > 0.0
                }
                _ => false,
            };
            if !placed {
                return Err("emitter needs a node, or a position and a positive radius".into());
            }
            if !rgb_valid(&emitter.color)
                || !emitter.intensity.is_finite()
                || emitter.intensity < 0.0
            {
                return Err("emitter color and intensity must be finite and nonnegative".into());
            }
            if let Some(scales) = &emitter.scales
                && (scales.len() != self.anchors.len()
                    || scales.iter().any(|v| !v.is_finite() || *v < 0.0))
            {
                return Err("emitter scales need one nonnegative value per anchor".into());
            }
            if !library && emitter.preset.is_some() {
                return Err("emitter preset names a material in the recipe's library".into());
            }
        }
        if self.emitter_layer.is_some() {
            if self.emitters.is_empty() {
                return Err("emitter_layer needs at least one emitter".into());
            }
            self.layer_scales()?;
        }
        Ok(())
    }

    fn check_library(&self, path: &str, library: &Library) -> Result<(), String> {
        let missing = |name: &str| !library.contains(name);
        if let Some(name) = self.fallback.as_deref().filter(|name| missing(name)) {
            return Err(format!("fallback {name} is not in {path}"));
        }
        if let Some(name) = self.tint.keys().find(|name| missing(name)) {
            return Err(format!("tint names {name}, which {path} does not hold"));
        }
        if let Some(spec) = self
            .poses
            .iter()
            .flat_map(|pose| pose.materials.values())
            .find(|spec| missing(spec))
        {
            return Err(format!("pose material {spec} is not in {path}"));
        }
        if let Some(name) = self
            .emitters
            .iter()
            .filter_map(|emitter| emitter.preset.as_deref())
            .find(|name| missing(name))
        {
            return Err(format!("emitter preset {name} is not in {path}"));
        }
        Ok(())
    }

    pub(crate) fn emitter_base(&self, name: Option<&str>, palette: Option<&Palette>) -> Material {
        match (palette, name) {
            (Some(palette), Some(name)) => palette.listed(name).unwrap_or_default(),
            (Some(palette), None) => palette.fallback(),
            (None, _) => Material::default(),
        }
    }

    pub fn emitter_scales(&self) -> Vec<Vec<f32>> {
        self.emitters
            .iter()
            .map(|emitter| {
                emitter
                    .scales
                    .clone()
                    .unwrap_or_else(|| vec![1.0; self.anchors.len()])
            })
            .collect()
    }

    pub fn layer_scales(&self) -> Result<Vec<f32>, String> {
        let scales = self.emitter_scales();
        let first = scales.first().ok_or("recipe has no emitters")?;
        if scales.iter().any(|other| other != first) {
            return Err("emitter_layer needs every emitter to share its scales".into());
        }
        Ok(first.clone())
    }

    pub fn composed(&self) -> bool {
        !self.files.is_empty()
            || !self.poses.is_empty()
            || !self.emitters.is_empty()
            || !self.shadow_only_materials.is_empty()
            || self.drop_degenerate
    }
}

fn triangles(mesh: &Mesh, shadow_only_nodes: &[String]) -> Result<Vec<Triangle>, String> {
    let mut result = Vec::new();
    let mut stack: Vec<u32> = mesh.roots.iter().rev().copied().collect();
    let mut seen = BTreeSet::new();
    while let Some(index) = stack.pop() {
        if !seen.insert(index) {
            return Err("mesh scene has repeated nodes".into());
        }
        let node = mesh
            .nodes
            .get(index as usize)
            .ok_or("mesh scene has a missing node")?;
        stack.extend(node.children.iter().rev().copied());
        if shadow_only_nodes.contains(&node.name) {
            continue;
        }
        let Some(group) = node.group else { continue };
        let matrix = mesh.world(index).ok_or("mesh has invalid node hierarchy")?;
        for primitive in &mesh.groups[group as usize].primitives {
            for indices in primitive.indices.chunks_exact(3) {
                let vertices = [indices[0], indices[1], indices[2]].map(|id| {
                    let point = primitive.positions[id as usize];
                    [0, 1, 2].map(|axis| {
                        matrix[3][axis] + (0..3).map(|k| matrix[k][axis] * point[k]).sum::<f32>()
                    })
                });
                result.push(Triangle {
                    vertices,
                    material: primitive.material.unwrap_or(mesh.materials.len() as u32),
                });
            }
        }
    }
    if result.is_empty() {
        return Err("scene has no triangles".into());
    }
    Ok(result)
}

pub(crate) fn materials(bytes: &[u8], palette: Option<&Palette>) -> Result<Vec<Material>, String> {
    let gltf = gltf::Gltf::from_slice(bytes).map_err(|e| format!("materials: {e}"))?;
    let mut result = Vec::new();
    for item in gltf.materials() {
        if let Some(palette) = palette {
            result.push(palette.named(item.name().unwrap_or("")));
            continue;
        }
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
    result.push(palette.map_or_else(Material::default, Palette::fallback));
    Ok(result)
}

fn sun_at(model: &SunModel, daylight: &Daylight, hour: f32, reference_hour: f64) -> Sun {
    match model {
        SunModel::Solar => Sun::from_daylight(*daylight),
        SunModel::Fixed(authored) => {
            let norm = authored.toward.iter().map(|v| v * v).sum::<f64>().sqrt();
            Sun {
                direction: authored.toward.map(|v| (v / norm) as f32),
                color: authored.color.map(|v| v as f32),
                intensity: authored.irradiance * daylight.light(reference_hour).intensity as f32,
            }
        }
        SunModel::Authored(authored) => {
            let light = daylight.authored(
                &Authored {
                    toward: authored.toward,
                    colour: authored.color,
                    sky_fill: authored.sky_fill,
                },
                reference_hour,
            );
            let toward = light.toward.map(|v| v as f32);
            let length = toward.iter().map(|v| v * v).sum::<f32>().sqrt();
            Sun {
                direction: toward.map(|v| v / length),
                color: light.colour.map(|v| v as f32),
                intensity: authored.irradiance * light.intensity as f32,
            }
        }
        SunModel::Clock { clock, base } => {
            let damped = clock.at(f64::from(hour), *base);
            Sun {
                direction: compose::unit(damped.toward.map(|v| v as f32)),
                color: damped.colour.map(|v| v as f32),
                intensity: 1.0,
            }
        }
    }
}

fn anchors(recipe: &Recipe, sky: Option<&Sky>) -> Vec<Anchor> {
    recipe
        .anchors
        .iter()
        .map(|&hour| {
            let daylight = Daylight {
                hour: hour as f64,
                day: recipe.day,
                latitude: recipe.latitude,
                heading: recipe.heading,
            };
            let sun = sun_at(&recipe.sun, &daylight, hour, recipe.reference_hour);
            let sky = sky.cloned().unwrap_or_else(|| {
                let fill = daylight.light(recipe.reference_hour).sky as f32;
                Sky {
                    width: 4,
                    height: 2,
                    texels: vec![[fill * 0.5, fill * 0.7, fill, 1.0]; 8],
                }
            });
            let mut shade = recipe
                .sky
                .shade
                .iter()
                .find(|(key, _)| key.parse::<f32>() == Ok(hour))
                .map(|(_, shade)| *shade);
            let sky = match (recipe.sky.ambient, &recipe.sun, recipe.sky.prepare) {
                (Some(Ambient::Authored), SunModel::Authored(authored), _) => {
                    let light = daylight.authored(
                        &Authored {
                            toward: authored.toward,
                            colour: authored.color,
                            sky_fill: authored.sky_fill,
                        },
                        recipe.reference_hour,
                    );
                    sky.exposed(light.ambient as f32)
                }
                (
                    Some(Ambient::Clock),
                    SunModel::Clock { clock, base },
                    Some(Prepare {
                        mean: Some(mean), ..
                    }),
                ) => {
                    let damped = clock.at(f64::from(hour), *base);
                    shade = shade.or(Some(damped.shade.map(|v| v as f32)));
                    let current = sky.mean_luminance().max(1e-3);
                    sky.exposed(mean / current)
                        .rotated(-(damped.turn as f32) * TAU)
                }
                _ => sky,
            };
            let mut sky = sky;
            if let Some(shade) = shade {
                for texel in &mut sky.texels {
                    for (value, tint) in texel.iter_mut().zip(shade) {
                        *value *= tint;
                    }
                }
            }
            Anchor { hour, sky, sun }
        })
        .collect()
}

fn read_library(folder: &Path, name: &str) -> Result<(Vec<u8>, Library), String> {
    let file = folder
        .join(name)
        .canonicalize()
        .map_err(|e| format!("materials {name}: {e}"))?;
    let bytes = fs::read(&file).map_err(|e| format!("materials {name}: {e}"))?;
    let text = std::str::from_utf8(&bytes).map_err(|e| format!("materials {name}: {e}"))?;
    let library = Library::from_toml(text).map_err(|e| format!("materials {name}: {e}"))?;
    Ok((bytes, library))
}

impl SceneSource {
    pub fn with_hours(&self, hours: &[f32]) -> Result<Vec<Anchor>, String> {
        if hours.is_empty()
            || hours
                .iter()
                .any(|v| !v.is_finite() || !(0.0..=24.0).contains(v))
            || hours.windows(2).any(|v| v[0] >= v[1])
        {
            return Err("anchors must be sorted, distinct hours from 0 to 24".into());
        }
        if self.recipe.emitters.iter().any(|emitter| {
            emitter
                .scales
                .as_ref()
                .is_some_and(|s| s.len() != hours.len())
        }) {
            return Err(
                "emitter scales need one value per anchor, so --anchors cannot change their count"
                    .into(),
            );
        }
        let mut recipe = self.recipe.clone();
        recipe.anchors = hours.to_vec();
        Ok(anchors(&recipe, self.prepared_sky.as_ref()))
    }

    pub fn load(folder: &Path) -> Result<Self, String> {
        Self::load_with(folder, None)
    }

    pub fn load_with(folder: &Path, assets: Option<&Path>) -> Result<Self, String> {
        let folder = folder
            .canonicalize()
            .map_err(|e| format!("scene folder: {e}"))?;
        let recipe_bytes = read_inside(&folder, "bake.toml")?;
        let recipe = Recipe::parse(&recipe_bytes)?;
        let library = match &recipe.materials {
            Some(Materials::Library(name)) => {
                let (bytes, library) = read_library(&folder, name)?;
                recipe.check_library(name, &library)?;
                Some((name.clone(), bytes, library))
            }
            _ => None,
        };
        let root = match assets {
            Some(path) => path
                .canonicalize()
                .map_err(|e| format!("assets folder: {e}"))?,
            None => folder,
        };
        let mut files = Files::new(&root);
        files.load(&recipe.scene)?;
        if recipe.composed() {
            for file in &recipe.files {
                files.load(&file.path)?;
            }
            for pose in &recipe.poses {
                files.load(pose.file.as_deref().unwrap_or(&recipe.scene))?;
            }
        }
        let sky_bytes = recipe
            .sky
            .path
            .as_deref()
            .map(|name| read_inside(&root, name))
            .transpose()?;
        let sky = sky_bytes
            .as_deref()
            .map(Sky::parse)
            .transpose()?
            .or_else(|| recipe.sky.room.as_ref().map(Sky::room))
            .map(|sky| {
                if recipe.sky.ambient == Some(Ambient::Clock) {
                    return sky;
                }
                let sky = match &recipe.sky.prepare {
                    Some(prepare) => prepare.apply(sky),
                    None => sky,
                };
                sky.rotated(-recipe.sky.rotation_deg.to_radians())
                    .exposed(recipe.sky.intensity)
            });
        let mut hasher = Sha256::new();
        let scene_bytes = &files.sources[&recipe.scene];
        for (name, bytes) in std::iter::once(("bake.toml", &recipe_bytes))
            .chain(std::iter::once((recipe.scene.as_str(), scene_bytes)))
            .chain(
                files
                    .external
                    .iter()
                    .map(|(name, bytes)| (name.as_str(), bytes)),
            )
            .chain(
                files
                    .sources
                    .iter()
                    .filter(|(name, _)| **name != recipe.scene)
                    .map(|(name, bytes)| (name.as_str(), bytes)),
            )
        {
            hasher.update((name.len() as u64).to_le_bytes());
            hasher.update(name.as_bytes());
            hasher.update((bytes.len() as u64).to_le_bytes());
            hasher.update(bytes);
        }
        if let (Some(path), Some(bytes)) = (recipe.sky.path.as_deref(), sky_bytes.as_ref()) {
            hasher.update((path.len() as u64).to_le_bytes());
            hasher.update(path.as_bytes());
            hasher.update((bytes.len() as u64).to_le_bytes());
            hasher.update(bytes);
        }
        if let Some(room) = &recipe.sky.room {
            let bytes = serde_json::to_vec(room).map_err(|e| e.to_string())?;
            hasher.update(b"sky.room");
            hasher.update((bytes.len() as u64).to_le_bytes());
            hasher.update(&bytes);
        }
        if let Some((name, bytes, _)) = &library {
            hasher.update(b"materials");
            hasher.update((name.len() as u64).to_le_bytes());
            hasher.update(name.as_bytes());
            hasher.update((bytes.len() as u64).to_le_bytes());
            hasher.update(bytes);
        }
        hasher.update(env!("CARGO_PKG_VERSION").as_bytes());
        let source_digest = hasher.finalize().into();
        let palette = match (&recipe.materials, &library) {
            (Some(Materials::Library(_)), Some((_, _, library))) => Some(Palette {
                library,
                fallback: recipe.fallback.as_deref(),
                tint: &recipe.tint,
            }),
            _ => None,
        };
        let (triangles, materials, emitters) = if recipe.composed() {
            let composed = compose::compose(&recipe, &files, palette.as_ref())?;
            (composed.triangles, composed.materials, composed.emitters)
        } else {
            (
                triangles(files.mesh(&recipe.scene)?, &recipe.shadow_only_nodes)?,
                materials(scene_bytes, palette.as_ref())?,
                Vec::new(),
            )
        };
        let scene = BakeScene {
            triangles,
            shapes: Vec::new(),
            materials,
            anchors: anchors(&recipe, sky.as_ref()),
        };
        Ok(Self {
            recipe,
            scene,
            root,
            emitters,
            source_digest,
            prepared_sky: sky,
        })
    }

    pub fn anchor_scene(&self, index: usize) -> Result<Option<BakeScene>, String> {
        if self.emitters.is_empty() {
            return Ok(None);
        }
        let mut materials = self.scene.materials.clone();
        for (slot, scales) in self.emitters.iter().zip(self.recipe.emitter_scales()) {
            let scale = *scales.get(index).ok_or("an anchor has no emitter scale")?;
            if !scale.is_finite() || scale < 0.0 {
                return Err("emission scale must be finite and nonnegative".into());
            }
            materials[*slot].emission = materials[*slot].emission.map(|value| value * scale);
        }
        Ok(Some(BakeScene {
            triangles: self.scene.triangles.clone(),
            shapes: self.scene.shapes.clone(),
            materials,
            anchors: self.scene.anchors.clone(),
        }))
    }

    pub fn layer(&self) -> Result<Option<(Vec<f32>, bool)>, String> {
        match self.recipe.emitter_layer {
            Some(layer) => Ok(Some((self.recipe.layer_scales()?, layer.direct))),
            None => Ok(None),
        }
    }

    pub fn layer_hash(&self, spec: GridSpec, samples: u32) -> Result<String, String> {
        let (scales, direct) = self.layer()?.ok_or("recipe has no emitter layer")?;
        emitter_hash(
            &self.scene,
            spec,
            samples,
            self.recipe.seed,
            &scales,
            direct,
        )
    }

    pub fn hash(&self, spec: GridSpec, samples: u32) -> Result<String, String> {
        let mut h = Sha256::new();
        h.update(self.source_digest);
        h.update(scene_hash(&self.scene, spec, samples, self.recipe.seed)?.as_bytes());
        for scales in self.recipe.emitter_scales() {
            for scale in scales {
                h.update(scale.to_bits().to_le_bytes());
            }
        }
        Ok(hex::encode(h.finalize()))
    }
}

#[cfg(test)]
mod keys;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::emitter_hash;
    use pfx_materials::{Family, Library};
    use std::path::PathBuf;

    const ROOM: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/box-room");

    #[test]
    fn recipe_validates_hours_volumes_and_sky() {
        let bytes = fs::read(Path::new(ROOM).join("bake.toml")).unwrap();
        assert_eq!(
            Recipe::parse(&bytes).unwrap().volumes[0]
                .grid
                .dimensions()
                .unwrap(),
            [1; 3]
        );
        let text = String::from_utf8(bytes).unwrap();
        assert!(
            Recipe::parse(text.replace("[12.0]", "[16.0, 8.0]").as_bytes())
                .unwrap_err()
                .contains("anchors")
        );
        assert!(
            Recipe::parse(text.replace("spacing = 1.0", "spacing = 0.0").as_bytes())
                .unwrap_err()
                .contains("spacing")
        );
        assert!(
            Recipe::parse(text.replace("\"test\"", "\"Box Room\"").as_bytes())
                .unwrap_err()
                .contains("slug")
        );
        assert!(
            Recipe::parse(text.replace("samples = 1", "samples = 0").as_bytes())
                .unwrap_err()
                .contains("samples")
        );
    }

    #[test]
    fn prepared_sky_and_authored_sun_follow_recipe() {
        let original = fs::read_to_string(Path::new(ROOM).join("bake.toml")).unwrap();
        let recipe = original.replace("app = \"test\"", "app = \"test\"\nreference_hour = 14.0")
            + "\n[sky]\nkind = 'hdr'\npath = 'sky.hdr'\nrotation_deg = 169.4\nintensity = 4.2\nambient = 'authored'\n\n[sky.prepare]\ncap = 40.0\nbalance = [1.0, 0.92, 0.8]\nmean = 0.25\n\n[sun]\nmodel = 'authored'\ntoward = [0.45, 0.62, 0.64]\ncolor = [1.0, 0.9, 0.7]\nirradiance = 4.5\nsky_fill = 0.8\n";
        let parsed = Recipe::parse(recipe.as_bytes()).unwrap();
        assert_eq!(parsed.materials, None);
        assert_eq!(
            parsed.sky.prepare,
            Some(Prepare {
                cap: Some(40.0),
                balance: Some([1.0, 0.92, 0.8]),
                mean: Some(0.25),
            })
        );
        assert_eq!(parsed.sky.ambient, Some(Ambient::Authored));
        assert!(matches!(parsed.sun, SunModel::Authored(sun) if sun.irradiance == 4.5));
        assert!(
            Recipe::parse(
                recipe
                    .replace("intensity = 4.2", "intensity = -1")
                    .as_bytes()
            )
            .is_err()
        );
        assert!(Recipe::parse(recipe.replace("cap = 40.0", "cap = 0.0").as_bytes()).is_err());
        let scratch = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp")
            .join(format!("bake-sky-test-{}", std::process::id()));
        fs::create_dir_all(&scratch).unwrap();
        fs::write(scratch.join("bake.toml"), recipe).unwrap();
        fs::copy(Path::new(ROOM).join("room.gltf"), scratch.join("room.gltf")).unwrap();
        let sky_bytes = pfx_load::fixture::SKY_HDR;
        fs::write(scratch.join("sky.hdr"), sky_bytes).unwrap();
        let source = SceneSource::load(&scratch).unwrap();
        let prepared = Sky::parse(sky_bytes)
            .unwrap()
            .capped_luminance(40.0)
            .balanced_to([1.0, 0.92, 0.8])
            .scaled_to_mean_luminance(0.25)
            .rotated(-169.4_f32.to_radians())
            .exposed(4.2);
        let daylight = Daylight {
            hour: 12.0,
            day: Daylight::DAY,
            latitude: Daylight::LATITUDE,
            heading: Daylight::HEADING,
        };
        let authored = Authored {
            toward: [0.45, 0.62, 0.64],
            colour: [1.0, 0.9, 0.7],
            sky_fill: 0.8,
        };
        let light = daylight.authored(&authored, 14.0);
        assert_eq!(
            source.scene.anchors[0].sky,
            prepared.exposed(light.ambient as f32)
        );
        assert_eq!(
            source.scene.anchors[0].sun.intensity,
            4.5 * light.intensity as f32
        );
        assert_eq!(
            source.with_hours(&[12.0]).unwrap()[0].sky,
            source.scene.anchors[0].sky
        );
    }

    #[test]
    fn scene_hash_is_stable_and_changes_with_inputs() {
        let source = SceneSource::load(Path::new(ROOM)).unwrap();
        let spec = source.recipe.volumes[0].grid;
        let first = source.hash(spec, 1).unwrap();
        assert_eq!(first, source.hash(spec, 1).unwrap());
        assert_ne!(first, source.hash(spec, 2).unwrap());
        let mut changed = SceneSource::load(Path::new(ROOM)).unwrap();
        changed.scene.anchors = changed.with_hours(&[8.0, 12.0]).unwrap();
        assert_ne!(first, changed.hash(spec, 1).unwrap());
        let scratch = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp")
            .join(format!("bake-hash-test-{}", std::process::id()));
        fs::create_dir_all(&scratch).unwrap();
        fs::copy(Path::new(ROOM).join("bake.toml"), scratch.join("bake.toml")).unwrap();
        fs::copy(Path::new(ROOM).join("room.gltf"), scratch.join("room.gltf")).unwrap();
        assert_eq!(
            SceneSource::load(&scratch).unwrap().hash(spec, 1).unwrap(),
            first
        );
        let recipe = fs::read(scratch.join("bake.toml")).unwrap();
        let mut edited = recipe.clone();
        edited.push(b'\n');
        fs::write(scratch.join("bake.toml"), edited).unwrap();
        assert_ne!(
            SceneSource::load(&scratch).unwrap().hash(spec, 1).unwrap(),
            first
        );
        fs::write(scratch.join("bake.toml"), recipe).unwrap();
        let mut gltf = fs::read(scratch.join("room.gltf")).unwrap();
        gltf.push(b' ');
        fs::write(scratch.join("room.gltf"), gltf).unwrap();
        assert_ne!(
            SceneSource::load(&scratch).unwrap().hash(spec, 1).unwrap(),
            first
        );
    }

    const ROOM_SKY: &str = "\n[sky]\nkind = 'room'\nrotation_deg = 30.0\nintensity = 1.5\n\n[sky.room]\nwidth = 64\nfloor = [0.22, 0.135, 0.068]\nwall = [0.19, 0.13, 0.07]\nceiling = [0.28, 0.23, 0.16]\n\n[[sky.room.lights]]\nname = 'window'\naz = -64.0\nel = 14.0\nwidth = 37.5\nheight = 29.0\npower = 10.5\ncolor = [1.0, 0.9, 0.7]\nsoft = 0.05\nslats = 6.0\nopen = 0.7\n\n[[sky.room.lights]]\naz = 62.0\nel = 22.0\npower = 1.7\n";

    fn room_scene(name: &str, sky: &str) -> PathBuf {
        let scratch = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp")
            .join(format!("bake-{name}-{}", std::process::id()));
        fs::create_dir_all(&scratch).unwrap();
        let recipe = fs::read_to_string(Path::new(ROOM).join("bake.toml")).unwrap() + sky;
        fs::write(scratch.join("bake.toml"), recipe).unwrap();
        fs::copy(Path::new(ROOM).join("room.gltf"), scratch.join("room.gltf")).unwrap();
        scratch
    }

    #[test]
    fn a_room_sky_parses_and_is_refused_without_its_room_or_with_prepare() {
        let base = fs::read_to_string(Path::new(ROOM).join("bake.toml")).unwrap();
        let recipe = base.clone() + ROOM_SKY;
        let parsed = Recipe::parse(recipe.as_bytes()).unwrap();
        assert_eq!(parsed.sky.kind, SkyKind::Room);
        let room = parsed.sky.room.unwrap();
        assert_eq!(room.width, 64);
        assert_eq!(room.lights.len(), 2);
        assert_eq!(room.lights[0].slats, 6.0);
        assert_eq!(room.lights[1].width, 45.0);
        assert_eq!(room.lights[1].color, [1.0, 1.0, 1.0]);
        let bare = Recipe::parse((base.clone() + "\n[sky]\nkind = 'room'\n").as_bytes());
        assert!(bare.unwrap_err().contains("[sky.room]"));
        let prepared = recipe.replace(
            "kind = 'room'\n",
            "kind = 'room'\n\n[sky.prepare]\nmean = 1.0\n",
        );
        assert!(
            Recipe::parse(prepared.as_bytes())
                .unwrap_err()
                .contains("prepare")
        );
        let balanced = recipe.replace(
            "kind = 'room'\n",
            "kind = 'room'\nbalance = [1.0, 1.0, 1.0]\n",
        );
        assert!(Recipe::parse(balanced.as_bytes()).is_err());
        let hdr = recipe.replace("kind = 'room'\n", "kind = 'hdr'\npath = 'sky.hdr'\n");
        assert!(
            Recipe::parse(hdr.as_bytes())
                .unwrap_err()
                .contains("[sky.room]")
        );
        let pathed = recipe.replace("kind = 'room'\n", "kind = 'room'\npath = 'sky.hdr'\n");
        assert!(Recipe::parse(pathed.as_bytes()).is_err());
        let typo = recipe.replace("power = 1.7", "pwoer = 1.7");
        assert!(Recipe::parse(typo.as_bytes()).is_err());
        let narrow = recipe.replace("width = 64", "width = 1");
        assert!(
            Recipe::parse(narrow.as_bytes())
                .unwrap_err()
                .contains("sky.room")
        );
    }

    #[test]
    fn a_room_reaches_every_anchors_sky() {
        let scratch = room_scene("room-sky", ROOM_SKY);
        let source = SceneSource::load(&scratch).unwrap();
        let room = source.recipe.sky.room.clone().unwrap();
        let bare = Sky::room(&room);
        let prepared = bare.clone().rotated(-30.0_f32.to_radians()).exposed(1.5);
        assert_eq!(source.scene.anchors.len(), 1);
        let sky = &source.scene.anchors[0].sky;
        assert_eq!(*sky, prepared);
        assert_eq!((sky.width, sky.height), (64, 32));
        let want = bare.mean_color();
        let got = sky.mean_color();
        for k in 0..3 {
            assert!(
                (got[k] - want[k] * 1.5).abs() <= 1e-5 * want[k].max(1.0),
                "{got:?} against {want:?}"
            );
        }
        assert!(want[0] > room.wall[0]);
        let others = source.with_hours(&[8.0, 16.0]).unwrap();
        assert!(others.iter().all(|anchor| anchor.sky == *sky));
        fs::remove_dir_all(scratch).unwrap();
    }

    #[test]
    fn a_room_takes_the_authored_ambient_by_the_hour() {
        let sky = ROOM_SKY.replace(
            "intensity = 1.5\n",
            "intensity = 1.5\nambient = 'authored'\n",
        );
        let sun = "\n[sun]\nmodel = 'authored'\ntoward = [0.4, 0.6, 0.7]\ncolor = [1.0, 0.9, 0.7]\nirradiance = 2.0\nsky_fill = 0.5\n";
        let base = fs::read_to_string(Path::new(ROOM).join("bake.toml")).unwrap();
        let refused = |recipe: &str| Recipe::parse(recipe.as_bytes()).unwrap_err();
        assert!(refused(&(base.clone() + &sky)).contains("[sun] model = 'authored'"));
        assert!(
            refused(
                &(base.clone() + sun + &sky.replace("ambient = 'authored'", "ambient = 'sunny'"))
            )
            .contains("sky.ambient")
        );
        let hdr = "\n[sky]\nkind = 'hdr'\npath = 'sky.hdr'\nambient = 'sunny'\n";
        assert!(refused(&(base.clone() + sun + hdr)).contains("sky.ambient"));
        let scratch = room_scene("room-ambient", "");
        fs::write(scratch.join("bake.toml"), base + sun + &sky).unwrap();
        let source = SceneSource::load(&scratch).unwrap();
        let room = source.recipe.sky.room.clone().unwrap();
        let prepared = Sky::room(&room)
            .rotated(-30.0_f32.to_radians())
            .exposed(1.5);
        let authored = Authored {
            toward: [0.4, 0.6, 0.7],
            colour: [1.0, 0.9, 0.7],
            sky_fill: 0.5,
        };
        let hours = [6.0, 12.0, 16.0];
        let anchors = source.with_hours(&hours).unwrap();
        for (anchor, hour) in anchors.iter().zip(hours) {
            let light = Daylight {
                hour: f64::from(hour),
                day: Daylight::DAY,
                latitude: Daylight::LATITUDE,
                heading: Daylight::HEADING,
            }
            .authored(&authored, REFERENCE_HOUR);
            assert_eq!(anchor.sky, prepared.clone().exposed(light.ambient as f32));
        }
        assert_ne!(anchors[0].sky, anchors[1].sky);
        fs::remove_dir_all(scratch).unwrap();
    }

    #[test]
    fn the_older_product_spellings_are_refused_with_their_new_keys() {
        let base = fs::read_to_string(Path::new(ROOM).join("bake.toml")).unwrap();
        let refused = |extra: &str| Recipe::parse((base.clone() + extra).as_bytes()).unwrap_err();
        assert!(refused("\n[sky]\nkind = 'old'\n").contains("'room'"));
        assert!(
            refused("\n[sky]\nkind = 'hdr'\npath = 'sky.hdr'\nprepare = 'old'\n")
                .contains("[sky.prepare]")
        );
        assert!(
            refused("\n[sky]\nkind = 'hdr'\npath = 'sky.hdr'\nambient = 'old'\n")
                .contains("'authored'")
        );
        assert!(
            refused("\n[sky]\nkind = 'hdr'\npath = 'sky.hdr'\nbalance = [1.0, 1.0, 1.0]\n")
                .contains("[sky.prepare]")
        );
        assert!(
            refused(
                "\n[sun]\ntoward = [0.0, 1.0, 0.0]\ncolor = [1.0, 1.0, 1.0]\nirradiance = 1.0\n"
            )
            .contains("model = 'authored'")
        );
        let product = base.replace("app = \"test\"", "product = \"test\"");
        let parsed = Recipe::parse(product.as_bytes()).unwrap();
        assert_eq!(parsed.sun, SunModel::Solar);
    }

    #[test]
    fn a_rooms_lamp_power_changes_the_scene_hash() {
        let first = room_scene("room-hash-a", ROOM_SKY);
        let second = room_scene(
            "room-hash-b",
            &ROOM_SKY.replace("power = 1.7", "power = 1.8"),
        );
        let a = SceneSource::load(&first).unwrap();
        let b = SceneSource::load(&second).unwrap();
        let spec = a.recipe.volumes[0].grid;
        assert_eq!(
            a.hash(spec, 1).unwrap(),
            SceneSource::load(&first).unwrap().hash(spec, 1).unwrap()
        );
        assert_ne!(a.hash(spec, 1).unwrap(), b.hash(spec, 1).unwrap());
        assert_ne!(
            scene_hash(&a.scene, spec, 1, 73).unwrap(),
            scene_hash(&b.scene, spec, 1, 73).unwrap()
        );
        assert_ne!(a.source_digest, b.source_digest);
        fs::remove_dir_all(first).unwrap();
        fs::remove_dir_all(second).unwrap();
    }

    #[test]
    fn shadow_only_nodes_are_excluded_from_bake_geometry() {
        let bytes = include_str!("../tests/data/box-room/room.gltf");
        let named = bytes.replace("\"mesh\":0", "\"name\":\"occluder\",\"mesh\":0");
        let mesh = Mesh::parse_with(
            named.as_bytes(),
            |_| Err("unexpected external asset".into()),
        )
        .unwrap();
        assert_eq!(triangles(&mesh, &[]).unwrap().len(), 10);
        assert!(
            triangles(&mesh, &["occluder".into()])
                .unwrap_err()
                .contains("no triangles")
        );
    }

    fn encode(bytes: &[u8]) -> String {
        const SET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let n = chunk
                .iter()
                .enumerate()
                .fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
            for k in 0..4 {
                if k <= chunk.len() {
                    out.push(SET[(n >> (18 - 6 * k) & 63) as usize] as char);
                } else {
                    out.push('=');
                }
            }
        }
        out
    }

    fn gltf(names: &[&str], material: &str, positions: &[[f32; 3]], indices: &[u16]) -> String {
        let mut data: Vec<u8> = positions
            .iter()
            .flatten()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let split = data.len();
        data.extend(indices.iter().flat_map(|v| v.to_le_bytes()));
        let low: Vec<String> = (0..3)
            .map(|k| {
                positions
                    .iter()
                    .map(|p| p[k])
                    .fold(f32::MAX, f32::min)
                    .to_string()
            })
            .collect();
        let high: Vec<String> = (0..3)
            .map(|k| {
                positions
                    .iter()
                    .map(|p| p[k])
                    .fold(f32::MIN, f32::max)
                    .to_string()
            })
            .collect();
        let nodes: Vec<String> = names
            .iter()
            .map(|name| format!("{{\"name\":\"{name}\",\"mesh\":0}}"))
            .collect();
        let roots: Vec<String> = (0..names.len()).map(|i| i.to_string()).collect();
        format!(
            "{{\"asset\":{{\"version\":\"2.0\"}},\"scene\":0,\"scenes\":[{{\"nodes\":[{}]}}],\"nodes\":[{}],\"materials\":[{{\"name\":\"{material}\"}}],\"meshes\":[{{\"primitives\":[{{\"attributes\":{{\"POSITION\":0}},\"indices\":1,\"material\":0}}]}}],\"buffers\":[{{\"uri\":\"data:application/octet-stream;base64,{}\",\"byteLength\":{}}}],\"bufferViews\":[{{\"buffer\":0,\"byteOffset\":0,\"byteLength\":{split}}},{{\"buffer\":0,\"byteOffset\":{split},\"byteLength\":{}}}],\"accessors\":[{{\"bufferView\":0,\"componentType\":5126,\"count\":{},\"type\":\"VEC3\",\"min\":[{}],\"max\":[{}]}},{{\"bufferView\":1,\"componentType\":5123,\"count\":{},\"type\":\"SCALAR\"}}]}}",
            roots.join(","),
            nodes.join(","),
            encode(&data),
            data.len(),
            data.len() - split,
            positions.len(),
            low.join(","),
            high.join(","),
            indices.len()
        )
    }

    const QUAD: [[f32; 3]; 4] = [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 0.0, 1.0],
        [0.0, 0.0, 1.0],
    ];
    const QUAD_INDICES: [u16; 6] = [0, 1, 2, 0, 2, 3];

    fn scratch(label: &str) -> PathBuf {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp")
            .join(format!("bake-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("lib.toml"), pfx_materials::FIXTURE).unwrap();
        path
    }

    fn sample(top: &str, tables: &str) -> String {
        format!(
            "scene = \"scene.gltf\"\napp = \"test\"\nmaterials = \"lib.toml\"\nfallback = \"card\"\nanchors = [9.0, 22.0]\nsamples = 1\nseed = 3\n{top}\n[[volumes]]\nname = \"v\"\nmin = [0.0, 0.0, 0.0]\nmax = [0.0, 0.0, 0.0]\nspacing = 1.0\n{tables}"
        )
    }

    fn load(label: &str, recipe: &str, files: &[(&str, String)]) -> SceneSource {
        let folder = scratch(label);
        fs::write(folder.join("bake.toml"), recipe).unwrap();
        fs::write(
            folder.join("scene.gltf"),
            gltf(&["floor", "lamp"], "card", &QUAD, &QUAD_INDICES),
        )
        .unwrap();
        for (name, text) in files {
            fs::write(folder.join(name), text).unwrap();
        }
        SceneSource::load(&folder).unwrap()
    }

    #[test]
    fn composition_keys_parse_and_malformed_ones_are_refused() {
        let full = sample(
            "assets_env = \"SAMPLE_ASSETS\"\ndrop_degenerate = true\nshadow_only_materials = [\"occluder\"]\nemitter_layer = { direct = false }\n",
            "\n[sky.shade]\n\"9.0\" = [0.5, 0.6, 0.7]\n22 = [0.1, 0.1, 0.2]\n\n[[file]]\npath = \"plant.glb\"\ntranslation = [0.1, 0.0, 0.035]\nkeep_prefix = \"floor_\"\n\n[[pose]]\nfile = \"rolls.glb\"\nnode = \"roll\"\nyaw = 1.0\nscale = [1.0, 2.0, 1.0]\ntranslation = [0.0, 0.0, 1.0]\nmaterials = { roll_cover = \"felt\", roll_band = \"metal\" }\n\n[[emitter]]\nposition = [0.0, 1.0, 0.0]\nradius = 0.05\ncolour = [1.0, 0.7, 0.4]\nintensity = 0.22\nscales = [0.0, 1.0]\npreset = \"lamp\"\n",
        );
        let recipe = Recipe::parse(full.as_bytes()).unwrap();
        assert_eq!(recipe.assets_env.as_deref(), Some("SAMPLE_ASSETS"));
        assert!(recipe.drop_degenerate && recipe.composed());
        assert_eq!(recipe.shadow_only_materials, ["occluder"]);
        assert_eq!(recipe.sky.shade["22"], [0.1, 0.1, 0.2]);
        assert_eq!(recipe.files[0].keep_prefix.as_deref(), Some("floor_"));
        assert_eq!(recipe.poses[0].scale, [1.0, 2.0, 1.0]);
        assert_eq!(recipe.poses[0].materials["roll_band"], "metal");
        assert_eq!(recipe.emitters[0].color, [1.0, 0.7, 0.4]);
        assert_eq!(recipe.emitter_layer, Some(EmitterLayer { direct: false }));
        assert_eq!(recipe.layer_scales().unwrap(), vec![0.0, 1.0]);
        let refused = |from: &str, to: &str, wants: &str| {
            let error = Recipe::parse(full.replace(from, to).as_bytes()).unwrap_err();
            assert!(error.contains(wants), "{from} -> {to}: {error}");
        };
        refused("\"9.0\" = [0.5", "\"nine\" = [0.5", "not an hour");
        refused("\"9.0\" = [0.5", "\"10.0\" = [0.5", "anchors");
        refused("= [0.5, 0.6, 0.7]", "= [0.5, -0.6, 0.7]", "sky.shade");
        refused("plant.glb", "plant.obj", "file must name");
        refused(
            "keep_prefix = \"floor_\"",
            "keep_prefix = \"\"",
            "keep_prefix",
        );
        refused(
            "translation = [0.1, 0.0, 0.035]\nkeep",
            "keep",
            "keep_prefix",
        );
        refused(
            "scale = [1.0, 2.0, 1.0]",
            "scale = [1.0, 0.0, 1.0]",
            "scale nonzero",
        );
        refused("yaw = 1.0", "yaw = inf", "pose values");
        refused(
            "roll_band = \"metal\"",
            "roll_band = \"\"",
            "pose materials",
        );
        refused(
            "materials = \"lib.toml\"\nfallback = \"card\"\n",
            "",
            "pose materials need",
        );
        refused("[pose]]\nfile", "[pose]]\nstock = 1\nfile", "stock");
        refused("node = \"roll\"", "node = \"\"", "pose needs");
        refused("radius = 0.05", "radius = 0.0", "emitter needs");
        refused(
            "radius = 0.05",
            "node = \"lamp\"\nradius = 0.05",
            "emitter needs",
        );
        refused("intensity = 0.22", "intensity = -0.22", "emitter color");
        refused("scales = [0.0, 1.0]", "scales = [0.0]", "emitter scales");
        refused("direct = false", "direct = true\nlayer = 1", "layer");
        refused(
            "drop_degenerate = true\n",
            "drop_degenerate = true\n\n[stock]\nroughness = 0.5\n",
            "stock",
        );
        refused(
            "drop_degenerate = true\n",
            "drop_degenerate = true\n\n[backdrop]\nwall = [1.0, 1.0, 1.0]\n",
            "backdrop",
        );
        let uneven = full.replace(
            "[[emitter]]",
            "[[emitter]]\nnode = \"b\"\ncolor = [1.0, 1.0, 1.0]\nintensity = 1.0\n\n[[emitter]]",
        );
        assert!(
            Recipe::parse(uneven.as_bytes())
                .unwrap_err()
                .contains("share")
        );
        let none = full.replace("emitter_layer = { direct = false }\n", "");
        assert!(
            Recipe::parse(none.as_bytes())
                .unwrap()
                .emitter_layer
                .is_none()
        );
        assert!(
            Recipe::parse(sample("emitter_layer = {}\n", "").as_bytes())
                .unwrap_err()
                .contains("at least one emitter")
        );
    }

    #[test]
    fn a_pose_places_a_copy_of_a_node_and_moves_one_in_the_scene() {
        let tables = "\n[[pose]]\nnode = \"floor\"\nyaw = 0.5\nscale = [2.0, 1.0, 3.0]\ntranslation = [4.0, 5.0, 6.0]\n";
        let moved = load("pose-move", &sample("", tables), &[]);
        assert_eq!(moved.scene.triangles.len(), 4);
        let (s, c) = 0.5_f32.sin_cos();
        let expected = |p: [f32; 3]| {
            [
                c * 2.0 * p[0] + s * 3.0 * p[2] + 4.0,
                p[1] + 5.0,
                -s * 2.0 * p[0] + c * 3.0 * p[2] + 6.0,
            ]
        };
        let plain = load("pose-plain", &sample("", ""), &[]);
        let originals: Vec<[f32; 3]> = plain.scene.triangles[..2]
            .iter()
            .flat_map(|t| t.vertices)
            .collect();
        let posed: Vec<[f32; 3]> = moved.scene.triangles[2..]
            .iter()
            .flat_map(|t| t.vertices)
            .collect();
        for (from, to) in originals.iter().zip(&posed) {
            for (a, b) in expected(*from).iter().zip(to) {
                assert!((a - b).abs() < 1e-5, "{a} {b}");
            }
        }
        assert_eq!(moved.scene.triangles[..2], plain.scene.triangles[2..]);
        let copies = "\n[[pose]]\nfile = \"other.gltf\"\nnode = \"cup\"\ntranslation = [0.0, 1.0, 0.0]\nmaterials = { card = \"felt\" }\n\n[[pose]]\nfile = \"other.gltf\"\nnode = \"cup\"\ntranslation = [0.0, 2.0, 0.0]\n";
        let source = load(
            "pose-copy",
            &sample("", copies),
            &[("other.gltf", gltf(&["cup"], "card", &QUAD, &QUAD_INDICES))],
        );
        assert_eq!(source.scene.triangles.len(), 4 + 2 + 2);
        assert_eq!(source.scene.triangles[4].vertices[0], [0.0, 1.0, 0.0]);
        assert_eq!(source.scene.triangles[6].vertices[0], [0.0, 2.0, 0.0]);
        let first = source.scene.materials[source.scene.triangles[4].material as usize];
        let second = source.scene.materials[source.scene.triangles[6].material as usize];
        let fixture = Library::fixture();
        assert_eq!(first, *fixture.get("felt").unwrap());
        assert_eq!(first.family, Family::Cloth);
        assert_eq!(second, *fixture.get("card").unwrap());
        let missing = "\n[[pose]]\nfile = \"other.gltf\"\nnode = \"saucer\"\n";
        let folder = scratch("pose-missing");
        fs::write(folder.join("bake.toml"), sample("", missing)).unwrap();
        fs::write(
            folder.join("scene.gltf"),
            gltf(&["floor"], "card", &QUAD, &QUAD_INDICES),
        )
        .unwrap();
        fs::write(
            folder.join("other.gltf"),
            gltf(&["cup"], "card", &QUAD, &QUAD_INDICES),
        )
        .unwrap();
        assert!(SceneSource::load(&folder).err().unwrap().contains("saucer"));
    }

    #[test]
    fn extra_files_lift_their_nodes_except_the_kept_prefix() {
        let tables = "\n[[file]]\npath = \"plant.gltf\"\ntranslation = [10.0, 0.0, 0.0]\nkeep_prefix = \"floor_\"\n";
        let source = load(
            "files",
            &sample("", tables),
            &[(
                "plant.gltf",
                gltf(&["stem", "floor_dry"], "card", &QUAD, &QUAD_INDICES),
            )],
        );
        let at: Vec<f32> = source
            .scene
            .triangles
            .iter()
            .map(|t| t.vertices[0][0])
            .collect();
        assert_eq!(at, [0.0, 0.0, 0.0, 0.0, 10.0, 10.0, 0.0, 0.0]);
    }

    #[test]
    fn degenerate_triangles_and_shadow_only_materials_stay_out_of_the_bake() {
        let flat = [[0.0; 3], [1.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
        let kept = load("degenerate-kept", &sample("", ""), &[]);
        assert_eq!(kept.scene.triangles.len(), 4);
        let folder = scratch("degenerate");
        fs::write(
            folder.join("bake.toml"),
            sample(
                "drop_degenerate = true\nshadow_only_materials = [\"occluder\"]\n",
                "\n[[file]]\npath = \"rig.gltf\"\n",
            ),
        )
        .unwrap();
        fs::write(
            folder.join("scene.gltf"),
            gltf(&["a"], "card", &flat, &[0, 1, 2, 0, 2, 3]),
        )
        .unwrap();
        fs::write(
            folder.join("rig.gltf"),
            gltf(&["b"], "occluder", &QUAD, &QUAD_INDICES),
        )
        .unwrap();
        let source = SceneSource::load(&folder).unwrap();
        assert_eq!(source.scene.triangles.len(), 1);
        assert_eq!(source.scene.materials.len(), 2);
    }

    #[test]
    fn shade_multiplies_the_sky_of_each_anchor() {
        let plain = load("shade-plain", &sample("", ""), &[]);
        let shaded = load(
            "shade",
            &sample("", "\n[sky.shade]\n9 = [0.5, 0.25, 2.0]\n"),
            &[],
        );
        let (a, b) = (&plain.scene.anchors, &shaded.scene.anchors);
        for (x, y) in a[0].sky.texels.iter().zip(&b[0].sky.texels) {
            assert_eq!(y[0], x[0] * 0.5);
            assert_eq!(y[1], x[1] * 0.25);
            assert_eq!(y[2], x[2] * 2.0);
            assert_eq!(y[3], x[3]);
        }
        assert_eq!(a[1].sky, b[1].sky);
        assert_eq!(shaded.with_hours(&[9.0]).unwrap()[0].sky, b[0].sky);
        assert_eq!(shaded.with_hours(&[22.0]).unwrap()[0].sky, a[1].sky);
    }

    #[test]
    fn a_clock_sun_lights_turns_and_shades_the_hdr_by_the_hour() {
        let sky = pfx_load::fixture::SKY_HDR;
        let tables = "\n[sun]\nmodel = \"clock\"\n\n[clock]\ntoward = [0.2332867, 0.8678002, 0.43874848]\nsun = 2.0\ntint = 0.5\nelevation = 1.0\nazimuth = -2.0\nskywarm = 0.2\nlamp = 3.0\nexposure = 1.0\nwarmth = 0.5\nturn = 0.6\n\n[clock.arc]\nstart = 5.0\nspan = 15.0\nelevation = 50.0\nazimuth = -70.0\nsweep = 140.0\nreference = 10.0\n\n[sky]\nkind = \"hdr\"\npath = \"sky.hdr\"\nambient = \"clock\"\n\n[sky.prepare]\nmean = 0.44\n\n[sky.shade]\n\"22.0\" = [0.5, 0.5, 0.5]\n";
        let folder = scratch("clock-sky");
        fs::write(folder.join("bake.toml"), sample("", tables)).unwrap();
        fs::write(
            folder.join("scene.gltf"),
            gltf(&["a"], "card", &QUAD, &QUAD_INDICES),
        )
        .unwrap();
        fs::write(folder.join("sky.hdr"), sky).unwrap();
        let source = SceneSource::load(&folder).unwrap();
        let clock = DampedClock {
            sun: 2.0,
            tint: 0.5,
            elevation: 1.0,
            azimuth: -2.0,
            skywarm: 0.2,
            lamp: 3.0,
            exposure: 1.0,
            warmth: 0.5,
            turn: 0.6,
            arc: SunArc {
                start: 5.0,
                span: 15.0,
                elevation: 50.0,
                azimuth: -70.0,
                sweep: 140.0,
                reference: 10.0,
            },
        };
        let base = [0.2332867, 0.8678002, 0.43874848];
        let raw = Sky::parse(sky).unwrap();
        for anchor in &source.scene.anchors {
            let damped = clock.at(f64::from(anchor.hour), base);
            let mut expected = raw
                .clone()
                .exposed(0.44 / raw.mean_luminance().max(1e-3))
                .rotated(-(damped.turn as f32) * TAU);
            let shade = if anchor.hour == 22.0 {
                [0.5; 3]
            } else {
                damped.shade.map(|v| v as f32)
            };
            for texel in &mut expected.texels {
                for (value, tint) in texel.iter_mut().zip(shade) {
                    *value *= tint;
                }
            }
            assert_eq!(anchor.sky, expected, "sky at {} h", anchor.hour);
            assert_eq!(
                anchor.sun.direction,
                compose::unit(damped.toward.map(|v| v as f32))
            );
            assert_eq!(anchor.sun.color, damped.colour.map(|v| v as f32));
            assert_eq!(anchor.sun.intensity, 1.0);
        }
        assert_ne!(source.scene.anchors[0].sky, source.scene.anchors[1].sky);
    }

    #[test]
    fn emitters_become_spheres_or_nodes_and_scale_per_anchor() {
        let tables = "\n[[emitter]]\nposition = [0.0, 2.0, 0.0]\nradius = 0.1\ncolor = [1.0, 0.5, 0.25]\nintensity = 0.2\nscales = [0.0, 2.0]\n\n[[emitter]]\nnode = \"lamp\"\ncolor = [1.0, 1.0, 1.0]\nintensity = 3.0\nscales = [0.0, 2.0]\n";
        let source = load(
            "emitters",
            &sample("emitter_layer = { direct = false }\n", tables),
            &[],
        );
        let radiance = 0.2 / (std::f32::consts::PI * 0.1 * 0.1);
        assert_eq!(source.emitters.len(), 2);
        let sphere = source.scene.materials[source.emitters[0]];
        assert_eq!(sphere.emission, [radiance, radiance * 0.5, radiance * 0.25]);
        assert_eq!(sphere.base, [0.0; 3]);
        assert_eq!(sphere.roughness, 1.0);
        assert_eq!(
            source.scene.materials[source.emitters[1]].emission,
            [3.0; 3]
        );
        let lamp_triangles = source
            .scene
            .triangles
            .iter()
            .filter(|t| t.material as usize == source.emitters[1])
            .count();
        assert_eq!(lamp_triangles, 2);
        let sphere_triangles = source
            .scene
            .triangles
            .iter()
            .filter(|t| t.material as usize == source.emitters[0])
            .count();
        assert_eq!(sphere_triangles, 528);
        assert_eq!(source.scene.triangles.len(), 2 + 2 + 528);
        let dark = source.anchor_scene(0).unwrap().unwrap();
        assert_eq!(dark.materials[source.emitters[0]].emission, [0.0; 3]);
        assert_eq!(dark.materials[source.emitters[1]].emission, [0.0; 3]);
        let bright = source.anchor_scene(1).unwrap().unwrap();
        assert_eq!(
            bright.materials[source.emitters[0]].emission,
            sphere.emission.map(|v| v * 2.0)
        );
        assert_eq!(bright.materials[source.emitters[1]].emission, [6.0; 3]);
        assert_eq!(bright.triangles.len(), source.scene.triangles.len());
        assert!(source.anchor_scene(2).is_err());
        assert!(
            source
                .with_hours(&[9.0])
                .err()
                .unwrap()
                .contains("emitter scales")
        );
        assert_eq!(source.with_hours(&[8.0, 9.0]).unwrap().len(), 2);
        let none = load("emitters-none", &sample("", ""), &[]);
        assert!(none.anchor_scene(0).unwrap().is_none());
        assert!(none.layer().unwrap().is_none());
    }

    #[test]
    fn the_emitter_layer_is_hashed_with_its_scales_and_written_beside_the_probes() {
        let tables = "\n[[emitter]]\nposition = [0.0, 2.0, 0.0]\nradius = 0.1\ncolor = [1.0, 0.5, 0.25]\nintensity = 0.2\nscales = [0.0, 0.0]\n";
        let source = load(
            "layer",
            &sample("emitter_layer = { direct = false }\n", tables),
            &[],
        );
        let spec = source.recipe.volumes[0].grid;
        let (scales, direct) = source.layer().unwrap().unwrap();
        assert_eq!((scales.clone(), direct), (vec![0.0, 0.0], false));
        let hash = source.layer_hash(spec, 1).unwrap();
        assert_eq!(
            hash,
            emitter_hash(&source.scene, spec, 1, 3, &[0.0, 0.0], false).unwrap()
        );
        assert_ne!(hash, source.hash(spec, 1).unwrap());
        assert_ne!(hash, source.layer_hash(spec, 2).unwrap());
        let brighter = load(
            "layer-lit",
            &sample(
                "emitter_layer = { direct = false }\n",
                &tables.replace("[0.0, 0.0]", "[0.0, 1.0]"),
            ),
            &[],
        );
        assert_ne!(hash, brighter.layer_hash(spec, 1).unwrap());
        assert_ne!(
            source.hash(spec, 1).unwrap(),
            brighter.hash(spec, 1).unwrap()
        );
        let lit = load(
            "layer-direct",
            &sample("emitter_layer = { direct = true }\n", tables),
            &[],
        );
        assert_ne!(hash, lit.layer_hash(spec, 1).unwrap());
        let probe = crate::Probe::default();
        let grids: Vec<crate::Grid> = (0..2)
            .map(|_| crate::Grid::new(spec, vec![probe.clone()]).unwrap())
            .collect();
        let tmp = scratch("layer-out").join("tmp");
        fs::create_dir_all(&tmp).unwrap();
        let manifest = crate::write_artifact_with_emitters(
            &tmp,
            Path::new("probes"),
            &source.scene,
            &grids,
            &crate::Emitters {
                grid: grids[0].clone(),
                scales,
                direct,
            },
            1,
            3,
        )
        .unwrap();
        assert_eq!(manifest.scene_hash, hash);
        let read = crate::read_artifact(&tmp.join("probes")).unwrap();
        assert!(read.emitters.is_some());
        assert_eq!(read.manifest.emitters.unwrap().scales, vec![0.0, 0.0]);
    }

    #[test]
    fn the_scene_can_live_in_an_assets_folder_apart_from_the_recipe() {
        let recipe = scratch("assets-recipe");
        let assets = scratch("assets-files");
        fs::write(recipe.join("bake.toml"), sample("", "")).unwrap();
        assert!(SceneSource::load(&recipe).is_err());
        fs::write(
            assets.join("scene.gltf"),
            gltf(&["a"], "card", &QUAD, &QUAD_INDICES),
        )
        .unwrap();
        let source = SceneSource::load_with(&recipe, Some(&assets)).unwrap();
        assert_eq!(source.root, assets.canonicalize().unwrap());
        assert_eq!(source.scene.triangles.len(), 2);
        let spec = source.recipe.volumes[0].grid;
        let hash = source.hash(spec, 1).unwrap();
        assert_eq!(
            SceneSource::load_with(&recipe, Some(&assets))
                .unwrap()
                .hash(spec, 1)
                .unwrap(),
            hash
        );
        fs::write(
            assets.join("scene.gltf"),
            gltf(&["a"], "card", &QUAD, &[0, 2, 1, 0, 3, 2]),
        )
        .unwrap();
        assert_ne!(
            SceneSource::load_with(&recipe, Some(&assets))
                .unwrap()
                .hash(spec, 1)
                .unwrap(),
            hash
        );
        fs::write(
            recipe.join("bake.toml"),
            sample("", "").replace("scene.gltf", "../scene.gltf"),
        )
        .unwrap();
        assert!(
            SceneSource::load_with(&recipe, Some(&assets))
                .err()
                .unwrap()
                .contains("outside")
        );
        assert!(SceneSource::load_with(&recipe, Some(&assets.join("missing"))).is_err());
    }
}
