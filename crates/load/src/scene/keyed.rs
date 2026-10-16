use std::collections::BTreeMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

use pfx_post::{Chain, Pass};
use pfx_scene::types::{self as file, Key, KeyShape, Keys};
use sha2::{Digest, Sha256};

use super::{Camera, Environment, Haze, Projection, Spot, Sun};
use crate::Sky;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkyLook {
    pub backdrop: f32,
    pub horizon_color: [f32; 3],
    pub horizon_width: f32,
    pub horizon_amount: f32,
}

impl Default for SkyLook {
    fn default() -> Self {
        Self {
            backdrop: file::Sky::BACKDROP,
            horizon_color: [0.0; 3],
            horizon_width: file::Sky::HORIZON_WIDTH,
            horizon_amount: 0.0,
        }
    }
}

impl SkyLook {
    pub fn plain(&self) -> bool {
        self.backdrop == 1.0 && self.horizon_amount <= 0.0
    }

    pub(super) fn hash(&self, hasher: &mut Sha256) {
        if *self == Self::default() {
            return;
        }
        hasher.update(b"look");
        for value in [self.backdrop, self.horizon_width, self.horizon_amount]
            .into_iter()
            .chain(self.horizon_color)
        {
            hasher.update(value.to_le_bytes());
        }
    }
}

fn tinted(sky: &file::Sky) -> bool {
    sky.horizon_color.is_some()
        || sky
            .keys
            .as_ref()
            .is_some_and(|keys| keys.has("horizon_color"))
}

pub(super) fn look_of(sky: &file::Sky) -> SkyLook {
    SkyLook {
        backdrop: sky.backdrop.unwrap_or(file::Sky::BACKDROP),
        horizon_color: sky.horizon_color.unwrap_or([0.0; 3]),
        horizon_width: sky.horizon_width.unwrap_or(file::Sky::HORIZON_WIDTH),
        horizon_amount: if tinted(sky) {
            sky.horizon_amount.unwrap_or(0.0)
        } else {
            0.0
        },
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum PlanSource {
    Analytic(Box<file::Sky>),
    Image(Arc<Sky>),
    Mix(Vec<(f32, SkyPlan)>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct SkyPlan {
    pub source: PlanSource,
    pub intensity: f32,
    pub keys: Option<Keys>,
}

impl SkyPlan {
    fn intensity_at(&self, time: f32) -> f32 {
        self.keys
            .as_ref()
            .and_then(|keys| number(keys, "intensity", time))
            .unwrap_or(self.intensity)
    }

    fn stamp(&self, time: f32, hasher: &mut DefaultHasher) {
        self.intensity_at(time).to_bits().hash(hasher);
        if let PlanSource::Mix(layers) = &self.source {
            for (_, layer) in layers {
                layer.stamp(time, hasher);
            }
        }
    }

    pub fn environment(&self, sun: Option<&Sun>, time: f32) -> Environment {
        let intensity = self.intensity_at(time);
        match &self.source {
            PlanSource::Analytic(sky) => {
                Environment::Analytic(super::build::analytic_of(sky, sun, intensity))
            }
            PlanSource::Image(base) => {
                Environment::Hdr(Arc::new((**base).clone().exposed(intensity)))
            }
            PlanSource::Mix(layers) => {
                let layers: Vec<(f32, Environment)> = layers
                    .iter()
                    .map(|(weight, layer)| (*weight, layer.environment(sun, time)))
                    .collect();
                Environment::Hdr(Arc::new(super::mix::bake(&layers)))
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LightAt {
    pub intensity: Option<f32>,
    pub color: Option<[f32; 3]>,
    pub range: Option<f32>,
    pub cone: Option<[f32; 2]>,
}

impl LightAt {
    pub fn tune(
        &self,
        intensity: &mut f32,
        color: &mut [f32; 3],
        range: &mut f32,
        spot: &mut Option<Spot>,
    ) {
        if let Some(value) = self.intensity {
            *intensity = value;
        }
        if let Some(value) = self.color {
            *color = value;
        }
        if let Some(value) = self.range {
            *range = value;
        }
        if let (Some([inner, outer]), Some(spot)) = (self.cone, spot.as_mut()) {
            spot.inner = inner;
            spot.outer = outer;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FinishAt {
    pub gain: f32,
    pub lut_amount: Option<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CameraKeys {
    pub keys: Keys,
    pub sensor: f32,
    pub focal: Option<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SunKeys {
    pub keys: Keys,
    pub sun: file::Sun,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HazeKeys {
    pub keys: Keys,
    pub haze: file::Haze,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FinishKeys {
    pub keys: Keys,
    pub exposure: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SkyKeys {
    pub keys: Option<Keys>,
    pub look: SkyLook,
    pub tinted: bool,
    pub plan: Option<SkyPlan>,
    pub sun: Option<Sun>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SceneKeys {
    pub lights: BTreeMap<String, Keys>,
    pub camera: Option<CameraKeys>,
    pub sun: Option<SunKeys>,
    pub sky: Option<SkyKeys>,
    pub haze: Option<HazeKeys>,
    pub finish: Option<FinishKeys>,
}

fn number(keys: &Keys, name: &str, time: f32) -> Option<f32> {
    keys.sample(name, time)?.number()
}

fn pair(keys: &Keys, name: &str, time: f32) -> Option<[f32; 2]> {
    keys.sample(name, time)?.pair()
}

fn triple(keys: &Keys, name: &str, time: f32) -> Option<[f32; 3]> {
    keys.sample(name, time)?.triple()
}

pub fn gather(
    keyed: &[(&str, KeyShape)],
    track: impl Fn(&str) -> Option<Vec<Key>>,
) -> Option<Keys> {
    let tracks: BTreeMap<String, Vec<Key>> = keyed
        .iter()
        .filter_map(|(name, _)| {
            let track = track(name).filter(|track| !track.is_empty())?;
            Some((name.to_string(), track))
        })
        .collect();
    (!tracks.is_empty()).then_some(Keys(tracks))
}

pub fn first_number(track: Option<Vec<Key>>) -> Option<f32> {
    track?.first().and_then(|(_, value)| value.number())
}

pub(super) fn keeps(sky: &file::Sky, sun: Option<&file::Sun>) -> bool {
    fn keyed(sky: &file::Sky) -> bool {
        sky.keys.as_ref().is_some_and(|keys| keys.has("intensity")) || sky.layer.iter().any(keyed)
    }
    fn analytic(sky: &file::Sky) -> bool {
        sky.kind == file::SkyKind::Analytic || sky.layer.iter().any(analytic)
    }
    let hourly = sun.is_some_and(|sun| sun.keys.as_ref().is_some_and(|keys| keys.has("hour")));
    keyed(sky) || (hourly && analytic(sky))
}

pub(super) fn sky_keys(
    sky: &file::Sky,
    plan: Option<SkyPlan>,
    sun: Option<Sun>,
) -> Option<SkyKeys> {
    let keys = gather(file::Sky::KEYED, |name| sky.track(name));
    if keys.is_none() && plan.is_none() {
        return None;
    }
    Some(SkyKeys {
        keys,
        look: look_of(sky),
        tinted: tinted(sky),
        plan,
        sun,
    })
}

pub(super) fn finish_exposure(chain: &Chain) -> f32 {
    chain
        .passes
        .iter()
        .find_map(|pass| match pass {
            Pass::Exposure(value) => Some(*value),
            _ => None,
        })
        .unwrap_or(1.0)
}

impl SceneKeys {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    pub fn light(&self, name: &str, time: f32) -> Option<LightAt> {
        let keys = self.lights.get(name)?;
        Some(LightAt {
            intensity: number(keys, "intensity", time),
            color: triple(keys, "color", time),
            range: number(keys, "range", time),
            cone: pair(keys, "cone", time),
        })
    }

    pub fn camera(&self, camera: &Camera, time: f32) -> Camera {
        let mut out = camera.clone();
        let Some(keyed) = &self.camera else {
            return out;
        };
        let keys = &keyed.keys;
        let fov = number(keys, "fov", time);
        if let (Some(fov), Projection::Perspective { .. }) = (fov, out.projection) {
            out.projection = Projection::Perspective { fov };
        }
        if let Some(depth) = out.depth_of_field.as_mut() {
            if let Some(fstop) = number(keys, "fstop", time) {
                depth.fstop = fstop;
            }
            if let Some(focus) = number(keys, "focus", time) {
                depth.distance = focus;
            }
            if let (Some(fov), None) = (fov, keyed.focal) {
                depth.focal = keyed.sensor / (2.0 * (fov.to_radians() * 0.5).tan()) / 1000.0;
            }
        }
        out
    }

    pub fn sun(&self, time: f32) -> Option<Sun> {
        let keyed = self.sun.as_ref()?;
        let hour = number(&keyed.keys, "hour", time)?;
        super::build::sun_of(&keyed.sun, Some(f64::from(hour))).ok()
    }

    pub fn haze(&self, time: f32) -> Option<Haze> {
        let keyed = self.haze.as_ref()?;
        let mut haze = keyed.haze.clone();
        if let Some(amount) = number(&keyed.keys, "amount", time) {
            haze.amount = Some(amount);
        }
        if let Some(color) = triple(&keyed.keys, "color", time) {
            haze.color = Some(file::Color::Rgb(color));
        }
        Some(super::build::build_haze(&haze))
    }

    pub fn finish(&self, time: f32) -> Option<FinishAt> {
        let keyed = self.finish.as_ref()?;
        let gain = number(&keyed.keys, "exposure", time).map_or(1.0, |exposure| {
            if keyed.exposure != 0.0 {
                exposure / keyed.exposure
            } else {
                1.0
            }
        });
        Some(FinishAt {
            gain,
            lut_amount: number(&keyed.keys, "lut_amount", time),
        })
    }

    pub fn look(&self, time: f32) -> Option<SkyLook> {
        let keyed = self.sky.as_ref()?;
        let keys = keyed.keys.as_ref()?;
        let mut look = keyed.look;
        if let Some(value) = number(keys, "backdrop", time) {
            look.backdrop = value;
        }
        if let Some(value) = triple(keys, "horizon_color", time) {
            look.horizon_color = value;
        }
        if let Some(value) = number(keys, "horizon_width", time) {
            look.horizon_width = value;
        }
        if let Some(value) = number(keys, "horizon_amount", time)
            && keyed.tinted
        {
            look.horizon_amount = value;
        }
        Some(look)
    }

    pub fn sky_stamp(&self, time: f32) -> Option<u64> {
        let plan = self.sky.as_ref()?.plan.as_ref()?;
        let mut hasher = DefaultHasher::new();
        if let Some(sun) = self.sun(time) {
            sun.hour.to_bits().hash(&mut hasher);
        }
        plan.stamp(time, &mut hasher);
        Some(hasher.finish())
    }

    pub fn environment(&self, time: f32) -> Option<Environment> {
        let keyed = self.sky.as_ref()?;
        let plan = keyed.plan.as_ref()?;
        let sun = self.sun(time).or(keyed.sun);
        Some(plan.environment(sun.as_ref(), time))
    }
}
