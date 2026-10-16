use pfx_load::scene::{
    Camera as SceneCamera, FinishAt, Haze, Light, Scene, SceneKeys, SkyLook, Sun as SceneSun,
};

use crate::effects::RoomHaze;
use crate::height_fog::HeightFog;
use crate::lights::{LocalLight, local_light};
use crate::renderer::Renderer;
use crate::scene::{height_fog, room_haze, sky_source};
use crate::sky::SkySource;

#[derive(Clone, Debug, Default)]
pub struct Tuned {
    pub hour: Option<f32>,
    pub haze: Option<(Option<RoomHaze>, Option<HeightFog>)>,
    pub finish: Option<FinishAt>,
    pub look: Option<SkyLook>,
    pub sky: Option<(u64, SkySource)>,
}

pub fn look(look: &SkyLook) -> [[f32; 4]; 2] {
    pfx_core::sky::sky_look(
        look.backdrop,
        look.horizon_color,
        look.horizon_width,
        look.horizon_amount,
    )
}

pub fn scene_look(scene: &Scene) -> [[f32; 4]; 2] {
    look(&scene.sky.as_ref().map(|sky| sky.look).unwrap_or_default())
}

pub fn hazes(haze: Option<&Haze>) -> (Option<RoomHaze>, Option<HeightFog>) {
    (
        haze.filter(|haze| haze.outdoor.is_none()).map(room_haze),
        haze.and_then(height_fog),
    )
}

pub fn keyed_light(keys: &SceneKeys, name: &str, light: LocalLight, time: f32) -> LocalLight {
    let Some(at) = keys.light(name, time) else {
        return light;
    };
    let mut out = light;
    at.tune(
        &mut out.intensity,
        &mut out.colour,
        &mut out.range,
        &mut out.spot,
    );
    out
}

#[derive(Clone, Debug)]
pub struct Keyer {
    keys: SceneKeys,
    names: Vec<String>,
    lights: Vec<LocalLight>,
    tuned: Tuned,
}

impl Keyer {
    pub fn of(scene: &Scene) -> Option<Self> {
        if scene.keys.is_empty() {
            return None;
        }
        Some(Self {
            keys: scene.keys.clone(),
            names: scene.lights.keys().cloned().collect(),
            lights: scene.lights.values().map(local_light).collect(),
            tuned: Tuned::default(),
        })
    }

    pub fn keys(&self) -> &SceneKeys {
        &self.keys
    }

    pub fn lights(&self, posed: Option<&[LocalLight]>, time: f32) -> Option<Vec<LocalLight>> {
        if self.keys.lights.is_empty() {
            return None;
        }
        let base = posed.unwrap_or(&self.lights);
        Some(
            base.iter()
                .zip(&self.names)
                .map(|(light, name)| keyed_light(&self.keys, name, *light, time))
                .collect(),
        )
    }

    pub fn named_lights<'a>(
        &self,
        lights: impl IntoIterator<Item = (&'a String, &'a Light)>,
        time: f32,
    ) -> Vec<LocalLight> {
        lights
            .into_iter()
            .map(|(name, light)| keyed_light(&self.keys, name, local_light(light), time))
            .collect()
    }

    pub fn camera(&self, camera: &SceneCamera, time: f32) -> SceneCamera {
        self.keys.camera(camera, time)
    }

    pub fn sun(&self, sun: SceneSun, time: f32) -> SceneSun {
        self.keys.sun(time).unwrap_or(sun)
    }

    pub fn tune(&mut self, time: f32) -> &Tuned {
        let keys = &self.keys;
        self.tuned.hour = keys.sun(time).map(|sun| sun.hour);
        self.tuned.haze = keys.haze(time).map(|haze| hazes(Some(&haze)));
        self.tuned.finish = keys.finish(time);
        self.tuned.look = keys.look(time);
        match keys.sky_stamp(time) {
            Some(stamp) if self.tuned.sky.as_ref().map(|(known, _)| *known) != Some(stamp) => {
                self.tuned.sky = keys
                    .environment(time)
                    .map(|environment| (stamp, sky_source(environment)));
            }
            Some(_) => {}
            None => self.tuned.sky = None,
        }
        &self.tuned
    }

    pub fn tuned(&self) -> &Tuned {
        &self.tuned
    }

    pub fn apply(&self, renderer: &mut Renderer, time: f32) -> Result<(), String> {
        let keys = &self.keys;
        let sky = keys
            .sky_stamp(time)
            .filter(|stamp| renderer.tuned_sky() != Some(*stamp))
            .and_then(|stamp| {
                keys.environment(time)
                    .map(|environment| (stamp, sky_source(environment)))
            });
        renderer.tune(&Tuned {
            hour: keys.sun(time).map(|sun| sun.hour),
            haze: keys.haze(time).map(|haze| hazes(Some(&haze))),
            finish: keys.finish(time),
            look: keys.look(time),
            sky,
        })
    }
}

impl Renderer {
    pub fn tune(&mut self, tuned: &Tuned) -> Result<(), String> {
        if let Some(hour) = tuned.hour {
            self.set_plate_hour(hour);
        }
        match (&tuned.sky, tuned.hour) {
            (Some((stamp, source)), hour) => {
                let hour = hour.unwrap_or_else(|| self.hour());
                if self.tuned_sky() != Some(*stamp) {
                    self.request_tuned_sky(*stamp, source.clone(), hour)?;
                }
            }
            (None, Some(hour)) if self.tuned_sky().is_none() && hour != self.hour() => {
                self.set_hour(hour)?
            }
            _ => {}
        }
        if let Some((room, fog)) = tuned.haze {
            self.set_haze(room);
            self.set_height_fog(fog);
        }
        if let Some(finish) = tuned.finish {
            self.set_finish_tuning(finish.gain, finish.lut_amount);
        }
        if let Some(seen) = &tuned.look {
            self.set_sky_look(look(seen));
        }
        Ok(())
    }
}
