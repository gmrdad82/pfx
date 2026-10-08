use std::any::Any;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use pfx_core::clock::{Clock, HitStop};
use pfx_core::fx::{Comfort, Shake, ShakeTuning};
use pfx_core::sim::Rng;
use pfx_gpu::screens::DisplaySettings;
use pfx_input::{PadId, Rumble};
use pfx_live::renderer::invert;
use pfx_load::scene::{
    Body, BodyKind, BodyShape, Camera, Cue, Light, Matrix, Scene, model, multiply,
};
use pfx_materials::Material;
use pfx_physics::rigid::{
    self, BodyDesc, BodyId, Character, CharacterDesc, ColliderDesc, ColliderId, Combine, Event,
    Filter, Friction, Pose, Shape, WorldDesc,
};
use pfx_sound::{ChannelId, Clip, Level, Mixer, MusicTrack, PcmSender, VoiceId};

use crate::PlayError;
use crate::events::{Phase, TriggerEvent, WorldEvent};
use crate::layers::Layers;
use crate::math;
use crate::rigs::Rigs;
use crate::tunables::{FromTunable, TunableValue, Tunables};

mod anim;
mod characters;
mod edits;
mod query;
mod rig;

pub use anim::{Animate, AnimationEvent};
pub use query::{Hit, Overlap, Query, QueryError};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SoundLevel {
    pub volume: f32,
    pub pan: f32,
    pub looping: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Physical {
    id: BodyId,
    collider: ColliderId,
    desc: Body,
    groups: rigid::Layers,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Sensor {
    body: BodyId,
    follows: bool,
    pose: Pose,
    shape: BodyShape,
    offset: [f32; 3],
    detects: u32,
}

pub(crate) fn shape(shape: BodyShape) -> Shape {
    match shape {
        BodyShape::Box { half } => Shape::Box { half },
        BodyShape::Sphere { radius } => Shape::Sphere { radius },
        BodyShape::Capsule {
            half_height,
            radius,
        } => Shape::Capsule {
            half_height,
            radius,
        },
        BodyShape::Cylinder {
            half_height,
            radius,
        } => Shape::Cylinder {
            half_height,
            radius,
        },
    }
}

pub fn body_volume(shape: BodyShape) -> f32 {
    match shape {
        BodyShape::Box { half } => 8.0 * half[0] * half[1] * half[2],
        BodyShape::Sphere { radius } => 4.0 / 3.0 * std::f32::consts::PI * radius.powi(3),
        BodyShape::Capsule {
            half_height,
            radius,
        } => {
            std::f32::consts::PI * radius * radius * 2.0 * half_height
                + 4.0 / 3.0 * std::f32::consts::PI * radius.powi(3)
        }
        BodyShape::Cylinder {
            half_height,
            radius,
        } => std::f32::consts::PI * radius * radius * 2.0 * half_height,
    }
}

pub(crate) fn collider(body: &Body) -> ColliderDesc {
    ColliderDesc::new(shape(body.shape))
        .with_offset(Pose::at(body.offset))
        .with_density(body.density)
        .with_friction(Friction::uniform(body.friction))
        .with_restitution(body.restitution, Combine::Average)
}

pub(crate) fn exposure_of(scene: &Scene) -> f32 {
    scene
        .finish
        .as_ref()
        .and_then(|finish| finish.chain.passes.first())
        .and_then(|pass| match pass {
            pfx_post::Pass::Exposure(value) => Some(*value),
            _ => None,
        })
        .unwrap_or(1.0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectId(usize);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RumbleRequest {
    Active(Rumble),
    Pad(PadId, Rumble),
}

impl ObjectId {
    pub(crate) fn from_index(place: usize) -> Self {
        Self(place)
    }

    pub fn index(self) -> usize {
        self.0
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Look {
    pub material: Option<String>,
    pub color: Option<[f32; 3]>,
    pub emission: Option<[f32; 3]>,
}

impl Look {
    pub fn is_plain(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Contact {
    pub a: ObjectId,
    pub b: ObjectId,
    pub impulse: f32,
}

#[derive(Clone, Debug, PartialEq)]
struct Entry {
    name: String,
    parent: Option<usize>,
    local: Matrix,
    scale: [f32; 3],
    hidden: bool,
    drawn: bool,
    look: Look,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fx {
    pub shake: Shake,
    pub comfort: Comfort,
}

impl Default for Fx {
    fn default() -> Self {
        Self {
            shake: Shake::new(ShakeTuning::default()),
            comfort: Comfort::FULL,
        }
    }
}

impl Fx {
    pub fn camera(&self, camera: &Camera) -> Camera {
        let offset = self.shake.offset(self.comfort.amount());
        if offset.is_zero() {
            return *camera;
        }
        let (_, right, up) = camera.axes();
        let shift: [f32; 3] = std::array::from_fn(|axis| {
            right[axis] * offset.translation[0] + up[axis] * offset.translation[1]
        });
        let (sin, cos) = offset.roll.sin_cos();
        Camera {
            at: std::array::from_fn(|axis| camera.at[axis] + shift[axis]),
            look_at: std::array::from_fn(|axis| camera.look_at[axis] + shift[axis]),
            up: std::array::from_fn(|axis| up[axis] * cos - right[axis] * sin),
            ..*camera
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Exit {
    Quit(u8),
    Failed(String),
}

impl Exit {
    pub fn code(&self) -> u8 {
        match self {
            Exit::Quit(code) => *code,
            Exit::Failed(_) => 1,
        }
    }
}

pub struct PcmStream {
    voice: VoiceId,
    sender: PcmSender,
}

impl PcmStream {
    pub fn push(&self, samples: &[f32]) -> usize {
        self.sender.push(samples)
    }

    pub fn room(&self) -> usize {
        self.sender.room_frames()
    }

    pub fn queued(&self) -> usize {
        self.sender.queued_frames()
    }

    pub fn capacity(&self) -> usize {
        self.sender.capacity_frames()
    }

    pub fn channels(&self) -> u16 {
        self.sender.channels()
    }

    pub fn voice(&self) -> VoiceId {
        self.voice
    }
}

pub struct Sounds {
    mixer: Mixer,
    clips: BTreeMap<String, Clip>,
    voices: BTreeMap<String, VoiceId>,
    log: Vec<(u64, String)>,
    audio: Vec<f32>,
    keep: usize,
    rendered: u64,
    now: u64,
}

impl Sounds {
    fn new(scene: &Scene, rate: u32, channels: u16, seed: u64) -> Result<Self, PlayError> {
        let mut mixer = Mixer::new(rate.max(1), channels.max(1));
        mixer.set_seed(seed);
        let mut clips = BTreeMap::new();
        for (name, sound) in &scene.sounds {
            let clip = if sound.bytes.starts_with(b"OggS") {
                Clip::from_vorbis(&sound.bytes).map_err(|message| {
                    PlayError::Sound(format!(
                        "sound {name}: {} does not decode as Ogg Vorbis: {message}",
                        sound.file.display()
                    ))
                })?
            } else {
                Clip::from_wav(&sound.bytes).ok_or_else(|| {
                    PlayError::Sound(format!(
                        "sound {name}: {} does not decode as a WAV",
                        sound.file.display()
                    ))
                })?
            };
            clips.insert(name.clone(), clip);
        }
        Ok(Self {
            keep: rate.max(1) as usize * channels.max(1) as usize * 2,
            mixer,
            clips,
            voices: BTreeMap::new(),
            log: Vec::new(),
            audio: Vec::new(),
            rendered: 0,
            now: 0,
        })
    }

    pub fn play_clip(&mut self, name: &str, level: Level, looping: bool) -> bool {
        let Some(clip) = self.clips.get(name) else {
            return false;
        };
        let voice = if looping {
            self.mixer
                .play_music(&MusicTrack::looping(clip.clone()), 0.0, level)
        } else {
            self.mixer.play(clip, level)
        };
        self.voices.insert(name.to_string(), voice);
        self.log.push((self.now, name.to_string()));
        true
    }

    pub fn stop(&mut self, name: &str, fade_seconds: f32) -> bool {
        match self.voices.remove(name) {
            Some(voice) => {
                self.mixer.stop(voice, fade_seconds);
                true
            }
            None => false,
        }
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.clips.keys().map(String::as_str)
    }

    pub fn stream(
        &mut self,
        channel: ChannelId,
        channels: u16,
        buffer_seconds: f32,
        level: Level,
    ) -> PcmStream {
        let (voice, sender) = self
            .mixer
            .pcm_stream(channel, channels, buffer_seconds, level);
        PcmStream { voice, sender }
    }

    pub fn end_stream(&mut self, stream: PcmStream, fade_seconds: f32) {
        stream.sender.finish();
        self.mixer.stop(stream.voice, fade_seconds);
    }

    pub fn log(&self) -> &[(u64, String)] {
        &self.log
    }

    pub fn mixer(&mut self) -> &mut Mixer {
        &mut self.mixer
    }

    pub fn rate(&self) -> u32 {
        self.mixer.rate()
    }

    pub fn channels(&self) -> u16 {
        self.mixer.channels()
    }

    pub fn take_audio(&mut self) -> Vec<f32> {
        std::mem::take(&mut self.audio)
    }

    fn render_until(&mut self, frame: u64) {
        if frame <= self.rendered {
            return;
        }
        let frames = (frame - self.rendered) as usize;
        self.rendered = frame;
        let start = self.audio.len();
        self.audio
            .resize(start + frames.saturating_mul(self.channels() as usize), 0.0);
        self.mixer.render_into(&mut self.audio[start..]);
        if self.audio.len() > self.keep {
            let extra = self.audio.len() - self.keep;
            let channels = self.channels().max(1) as usize;
            self.audio.drain(..extra.div_ceil(channels) * channels);
        }
    }
}

pub struct World {
    scene: Arc<Scene>,
    objects: Vec<Entry>,
    index: BTreeMap<String, usize>,
    pub camera: Camera,
    pub physics: rigid::World,
    bodies: BTreeMap<usize, Physical>,
    characters: BTreeMap<usize, Character>,
    roster: BTreeMap<String, CharacterDesc>,
    owners: BTreeMap<ColliderId, usize>,
    sensors: BTreeMap<usize, Sensor>,
    sensor_owners: BTreeMap<ColliderId, usize>,
    inside: BTreeSet<(usize, usize)>,
    movers: Vec<usize>,
    layers: Layers,
    events: Vec<WorldEvent>,
    contacts: Vec<Contact>,
    pub sounds: Sounds,
    pub fx: Fx,
    pub rng: Rng,
    clock: Clock,
    ticks: u64,
    state: Option<Box<dyn Any>>,
    rumble: Vec<RumbleRequest>,
    poses: BTreeMap<usize, [[f32; 3]; 3]>,
    materials: BTreeMap<String, Material>,
    lights: BTreeMap<String, Light>,
    lit: u64,
    exposure: f32,
    exposed: u64,
    levels: BTreeMap<String, SoundLevel>,
    tunables: Tunables,
    tuned: BTreeMap<String, TunableValue>,
    driver: rig::Driver,
    animations: anim::Animations,
    exit: Option<Exit>,
    display: DisplaySettings,
}

impl World {
    pub(crate) fn new(
        scene: Arc<Scene>,
        seed: u64,
        audio: (u32, u16),
        tunables: Tunables,
        rigs: Rigs,
    ) -> Result<Self, PlayError> {
        let layers = Layers::from_scene(&scene.layers).map_err(PlayError::Layers)?;
        let settings = scene.physics_or_default();
        let clock = Clock::new(f64::from(settings.rate))
            .map_err(|error| PlayError::Clock(format!("{error:?}")))?;
        let index: BTreeMap<String, usize> = scene
            .objects
            .iter()
            .enumerate()
            .map(|(place, object)| (object.name.clone(), place))
            .collect();
        let objects: Vec<Entry> = scene
            .objects
            .iter()
            .map(|object| Entry {
                name: object.name.clone(),
                parent: object
                    .parent
                    .as_deref()
                    .and_then(|parent| index.get(parent).copied()),
                local: model(object.at, object.rotate, object.scale),
                scale: object.scale,
                hidden: object.hidden,
                drawn: !object.hidden,
                look: Look::default(),
            })
            .collect();
        let mut physics = rigid::World::new(WorldDesc {
            rate: settings.rate,
            substeps: settings.substeps,
            iterations: settings.iterations,
            gravity: settings.gravity,
            ccd: true,
        });
        let mut bodies = BTreeMap::new();
        let mut owners = BTreeMap::new();
        for (name, body) in &scene.bodies {
            let Some(&place) = index.get(name) else {
                continue;
            };
            let pose = math::pose(scene.objects[place].model);
            let desc = match body.kind {
                BodyKind::Dynamic => BodyDesc::dynamic(pose),
                BodyKind::Kinematic => BodyDesc::kinematic(pose),
                BodyKind::Fixed => BodyDesc::fixed(pose),
            };
            let mut desc = desc
                .with_velocity(body.velocity)
                .with_spin(body.spin.map(f32::to_radians))
                .with_damping(body.damping[0], body.damping[1])
                .with_gravity_scale(body.gravity_scale);
            if body.ccd {
                desc = desc.with_ccd();
            }
            let groups =
                match scene.body_layers.get(name) {
                    Some(layer) => layers.groups(layers.require(layer).map_err(|message| {
                        PlayError::Layers(format!("object {name}: {message}"))
                    })?),
                    None => rigid::Layers::new(u32::MAX, u32::MAX),
                };
            let id = physics.add_body(desc);
            let collider = physics
                .add_collider(id, collider(body).with_layers(groups))
                .ok_or_else(|| PlayError::Body(format!("[body.{name}] has no usable shape")))?;
            bodies.insert(
                place,
                Physical {
                    id,
                    collider,
                    desc: *body,
                    groups,
                },
            );
            owners.insert(collider, place);
        }
        let mut roster = BTreeMap::new();
        for (name, character) in &scene.characters {
            let groups =
                match &character.layer {
                    Some(layer) => layers.groups(layers.require(layer).map_err(|message| {
                        PlayError::Layers(format!("object {name}: {message}"))
                    })?),
                    None => rigid::Layers::ALL,
                };
            roster.insert(name.clone(), crate::characters::desc_of(character, groups));
        }
        let mut sensors = BTreeMap::new();
        let mut sensor_owners = BTreeMap::new();
        for (name, trigger) in &scene.triggers {
            let Some(&place) = index.get(name) else {
                continue;
            };
            let layer = match &trigger.layer {
                Some(layer) => layers
                    .require(layer)
                    .map_err(|message| PlayError::Layers(format!("object {name}: {message}")))?,
                None => u32::MAX,
            };
            let detects = match (&trigger.mask, &trigger.layer) {
                (Some(mask), _) => layers
                    .mask(mask)
                    .map_err(|message| PlayError::Layers(format!("object {name}: {message}")))?,
                (None, Some(_)) => layers.collision_mask(layer),
                (None, None) => u32::MAX,
            };
            let pose = math::pose(scene.objects[place].model);
            let (body, follows) = match bodies.get(&place) {
                Some(physical) => (physical.id, false),
                None => (physics.add_body(BodyDesc::kinematic(pose)), true),
            };
            let desc = ColliderDesc::new(shape(trigger.shape))
                .with_offset(Pose::at(trigger.offset))
                .with_density(0.0)
                .with_layers(rigid::Layers::new(layer, 0))
                .as_sensor();
            let collider = physics
                .add_collider(body, desc)
                .ok_or_else(|| PlayError::Body(format!("[trigger.{name}] has no usable shape")))?;
            sensors.insert(
                place,
                Sensor {
                    body,
                    follows,
                    pose,
                    shape: trigger.shape,
                    offset: trigger.offset,
                    detects,
                },
            );
            sensor_owners.insert(collider, place);
        }
        let sounds = Sounds::new(&scene, audio.0, audio.1, seed)?;
        let levels = scene
            .sounds
            .iter()
            .map(|(name, sound)| {
                (
                    name.clone(),
                    SoundLevel {
                        volume: sound.volume,
                        pan: sound.pan,
                        looping: sound.looping,
                    },
                )
            })
            .collect();
        let tuned = tunables
            .iter()
            .map(|tunable| (tunable.name.clone(), tunable.default))
            .collect();
        let animations =
            anim::Animations::new(&scene, Self::rate_of(&clock)).map_err(PlayError::Animation)?;
        let movers = mover_order(&scene, &objects);
        let mut world = Self {
            animations,
            movers,
            poses: BTreeMap::new(),
            materials: BTreeMap::new(),
            lights: scene.lights.clone(),
            lit: 0,
            exposure: exposure_of(&scene),
            exposed: 0,
            levels,
            tunables,
            tuned,
            driver: rig::Driver::new(rigs),
            camera: scene.camera_or_default(),
            objects,
            index,
            physics,
            bodies,
            characters: BTreeMap::new(),
            roster,
            owners,
            sensors,
            sensor_owners,
            inside: BTreeSet::new(),
            layers,
            events: Vec::new(),
            contacts: Vec::new(),
            sounds,
            fx: Fx::default(),
            rng: Rng::new(seed),
            clock,
            ticks: 0,
            state: None,
            rumble: Vec::new(),
            exit: None,
            display: DisplaySettings::default(),
            scene,
        };
        world.physics.refresh_queries();
        Ok(world)
    }

    pub fn scene(&self) -> &Scene {
        &self.scene
    }

    pub fn len(&self) -> usize {
        self.objects.len()
    }

    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    pub fn object(&self, name: &str) -> Option<ObjectId> {
        self.index.get(name).copied().map(ObjectId)
    }

    pub fn ids(&self) -> impl Iterator<Item = ObjectId> {
        (0..self.objects.len()).map(ObjectId)
    }

    pub fn name(&self, id: ObjectId) -> &str {
        &self.objects[id.0].name
    }

    pub fn parent(&self, id: ObjectId) -> Option<ObjectId> {
        self.objects[id.0].parent.map(ObjectId)
    }

    pub fn rest(&self, id: ObjectId) -> Matrix {
        self.scene.objects[id.0].model
    }

    pub fn local(&self, id: ObjectId) -> Matrix {
        self.objects[id.0].local
    }

    pub fn set_local(&mut self, id: ObjectId, local: Matrix) {
        self.objects[id.0].local = local;
    }

    pub fn model(&self, id: ObjectId) -> Matrix {
        model_in(&self.objects, id.0)
    }

    pub fn models(&self) -> Vec<Matrix> {
        self.ids().map(|id| self.model(id)).collect()
    }

    pub fn set_model(&mut self, id: ObjectId, world: Matrix) -> bool {
        let local = match self.parent(id) {
            None => world,
            Some(parent) => match invert(self.model(parent)) {
                Some(inverse) => multiply(inverse, world),
                None => return false,
            },
        };
        self.objects[id.0].local = local;
        true
    }

    pub fn at(&self, id: ObjectId) -> [f32; 3] {
        math::position(self.model(id))
    }

    pub fn place(&mut self, id: ObjectId, at: [f32; 3], rotation: [f32; 4]) -> bool {
        let scale = self.objects[id.0].scale;
        self.set_model(id, math::compose(at, rotation, scale))
    }

    pub fn move_to(&mut self, id: ObjectId, at: [f32; 3]) -> bool {
        let mut world = self.model(id);
        world[3] = [at[0], at[1], at[2], 1.0];
        self.set_model(id, world)
    }

    pub fn hidden(&self, id: ObjectId) -> bool {
        self.objects[id.0].hidden
    }

    pub fn set_hidden(&mut self, id: ObjectId, hidden: bool) -> bool {
        let entry = &mut self.objects[id.0];
        if !entry.drawn {
            return false;
        }
        entry.hidden = hidden;
        true
    }

    pub fn look(&self, id: ObjectId) -> &Look {
        &self.objects[id.0].look
    }

    pub fn set_look(&mut self, id: ObjectId, look: Look) {
        self.objects[id.0].look = look;
    }

    pub fn body(&self, id: ObjectId) -> Option<BodyId> {
        self.bodies.get(&id.0).map(|physical| physical.id)
    }

    pub fn body_desc(&self, id: ObjectId) -> Option<&Body> {
        self.bodies.get(&id.0).map(|physical| &physical.desc)
    }

    pub fn material(&self, name: &str) -> Option<Material> {
        self.materials
            .get(name)
            .or_else(|| self.scene.library.get(name))
            .copied()
    }

    pub fn edited_materials(&self) -> &BTreeMap<String, Material> {
        &self.materials
    }

    pub fn light(&self, name: &str) -> Option<&Light> {
        self.lights.get(name)
    }

    pub fn lights(&self) -> &BTreeMap<String, Light> {
        &self.lights
    }

    pub fn set_light(&mut self, name: &str, light: Light) -> bool {
        let Some(slot) = self.lights.get_mut(name) else {
            return false;
        };
        if *slot != light {
            *slot = light;
            self.lit += 1;
        }
        true
    }

    pub(crate) fn lights_revision(&self) -> u64 {
        self.lit
    }

    pub fn exposure(&self) -> f32 {
        self.exposure
    }

    pub fn set_exposure(&mut self, exposure: f32) -> bool {
        if !exposure.is_finite() || exposure <= 0.0 {
            return false;
        }
        if exposure != self.exposure {
            self.exposure = exposure;
            self.exposed += 1;
        }
        true
    }

    pub(crate) fn exposure_revision(&self) -> u64 {
        self.exposed
    }

    pub fn sound_level(&self, name: &str) -> Option<SoundLevel> {
        self.levels.get(name).copied()
    }

    pub fn tunables(&self) -> &Tunables {
        &self.tunables
    }

    pub fn tunable<T: FromTunable>(&self, name: &str) -> Option<T> {
        T::from_tunable(*self.tuned.get(name)?)
    }

    pub fn set_tunable(&mut self, name: &str, value: TunableValue) -> Result<TunableValue, String> {
        let value = self.tunables.check(name, value)?;
        self.tuned.insert(name.to_string(), value);
        Ok(value)
    }

    pub fn contacts(&self) -> &[Contact] {
        &self.contacts
    }

    pub fn ticks(&self) -> u64 {
        self.ticks
    }

    pub fn time(&self) -> f64 {
        self.ticks as f64 / self.clock.tick_rate()
    }

    pub fn clock(&self) -> &Clock {
        &self.clock
    }

    pub fn hit_stop(&mut self, stop: HitStop) {
        self.clock.hit_stop(self.fx.comfort.hit_stop(stop));
    }

    pub fn set_speed(&mut self, speed: f32) {
        self.clock.set_speed(speed);
    }

    pub fn state<T: Any>(&self) -> Option<&T> {
        self.state.as_ref()?.downcast_ref()
    }

    pub fn state_mut<T: Any>(&mut self) -> Option<&mut T> {
        self.state.as_mut()?.downcast_mut()
    }

    pub fn set_state<T: Any>(&mut self, state: T) {
        self.state = Some(Box::new(state));
    }

    pub fn play_sound(&mut self, name: &str) -> bool {
        let Some(sound) = self.levels.get(name).copied() else {
            return false;
        };
        let level = Level {
            volume: sound.volume,
            pan: sound.pan,
            ..Level::default()
        };
        self.sounds.play_clip(name, level, sound.looping)
    }

    pub fn play_cued(&mut self, cue: Cue) -> usize {
        let scene = Arc::clone(&self.scene);
        scene
            .sounds
            .iter()
            .filter(|(name, sound)| sound.cue == cue && self.play_sound(name))
            .count()
    }

    pub fn play_hits(&mut self) -> usize {
        let scene = Arc::clone(&self.scene);
        let mut names: Vec<&String> = Vec::new();
        for contact in &self.contacts {
            for (name, sound) in &scene.sounds {
                let Some(object) = &sound.object else {
                    continue;
                };
                if sound.cue == Cue::Hit
                    && (self.objects[contact.a.0].name == *object
                        || self.objects[contact.b.0].name == *object)
                    && !names.contains(&name)
                {
                    names.push(name);
                }
            }
        }
        names.iter().filter(|name| self.play_sound(name)).count()
    }

    pub fn pose_movers(&mut self) {
        if self.movers.is_empty() {
            return;
        }
        let posed = self
            .scene
            .posed(&self.scene.camera_or_default(), self.time() as f32);
        for at in 0..self.movers.len() {
            let place = self.movers[at];
            self.set_model(ObjectId(place), posed[place]);
        }
    }

    pub fn rumble(&mut self, rumble: Rumble) {
        self.rumble.push(RumbleRequest::Active(rumble));
    }

    pub fn rumble_pad(&mut self, pad: PadId, rumble: Rumble) {
        self.rumble.push(RumbleRequest::Pad(pad, rumble));
    }

    pub fn rumble_queue(&self) -> &[RumbleRequest] {
        &self.rumble
    }

    pub(crate) fn take_rumble(&mut self) -> Vec<RumbleRequest> {
        std::mem::take(&mut self.rumble)
    }

    pub(crate) fn begin_tick(&mut self) {
        self.begin_animations();
        self.ticks += 1;
        self.sounds.now = self.ticks;
    }

    pub(crate) fn refresh_physics(&mut self) {
        self.sync_triggers();
        self.physics.refresh_queries();
    }

    fn sync_triggers(&mut self) {
        for (&place, sensor) in self.sensors.iter_mut().filter(|(_, sensor)| sensor.follows) {
            let pose = math::pose(model_in(&self.objects, place));
            if sensor.pose != pose {
                sensor.pose = pose;
                self.physics.set_pose(sensor.body, pose);
            }
        }
    }

    pub(crate) fn step_physics(&mut self) {
        self.events.clear();
        if self.bodies.is_empty() && self.sensors.is_empty() && self.characters.is_empty() {
            return;
        }
        self.sync_triggers();
        self.step_characters();
        self.physics.step();
        self.place_characters();
        for (&place, physical) in &self.bodies {
            if physical.desc.kind == BodyKind::Fixed {
                continue;
            }
            if let Some(pose) = self.physics.pose(physical.id) {
                let entry = &mut self.objects[place];
                entry.local = math::compose(pose.position, pose.rotation, entry.scale);
            }
        }
        self.contacts.clear();
        for event in self.physics.events() {
            if let Event::ContactStarted { a, b, impulse } = *event
                && let (Some(&a), Some(&b)) = (self.owners.get(&a), self.owners.get(&b))
            {
                self.contacts.push(Contact {
                    a: ObjectId(a),
                    b: ObjectId(b),
                    impulse,
                });
            }
        }
        let mut now = BTreeSet::new();
        for (&place, sensor) in &self.sensors {
            let Some(pose) = self.physics.pose(sensor.body) else {
                continue;
            };
            let shift = math::rotate(pose.rotation, sensor.offset);
            let at = std::array::from_fn(|axis| pose.position[axis] + shift[axis]);
            let filter = Filter::layers(sensor.detects)
                .with_sensors()
                .excluding(sensor.body);
            for collider in self.physics.shape_overlaps(
                &shape(sensor.shape),
                Pose::new(at, pose.rotation),
                filter,
            ) {
                if let Some(&other) = self
                    .owners
                    .get(&collider)
                    .or_else(|| self.sensor_owners.get(&collider))
                {
                    now.insert((place, other));
                }
            }
        }
        self.events
            .extend(self.contacts.iter().copied().map(WorldEvent::Contact));
        let event = |phase| {
            move |&(trigger, other): &(usize, usize)| {
                WorldEvent::Trigger(TriggerEvent {
                    phase,
                    trigger: ObjectId(trigger),
                    other: ObjectId(other),
                })
            }
        };
        self.events
            .extend(now.difference(&self.inside).map(event(Phase::Enter)));
        self.events
            .extend(now.intersection(&self.inside).map(event(Phase::Stay)));
        self.events
            .extend(self.inside.difference(&now).map(event(Phase::Exit)));
        self.inside = now;
    }

    pub fn events(&self) -> &[WorldEvent] {
        &self.events
    }

    pub fn layers(&self) -> &Layers {
        &self.layers
    }

    pub fn is_trigger(&self, id: ObjectId) -> bool {
        self.sensors.contains_key(&id.0)
    }

    pub fn inside(&self, trigger: ObjectId) -> Vec<ObjectId> {
        self.inside
            .iter()
            .filter(|(sensor, _)| *sensor == trigger.0)
            .map(|(_, other)| ObjectId(*other))
            .collect()
    }

    pub fn quit(&mut self, code: u8) {
        if self.exit.is_none() {
            self.exit = Some(Exit::Quit(code));
        }
    }

    pub fn fail(&mut self, error: impl std::fmt::Display) {
        if !matches!(self.exit, Some(Exit::Failed(_))) {
            self.exit = Some(Exit::Failed(error.to_string()));
        }
    }

    pub fn exit(&self) -> Option<&Exit> {
        self.exit.as_ref()
    }

    pub fn display(&self) -> &DisplaySettings {
        &self.display
    }

    pub(crate) fn set_display(&mut self, display: &DisplaySettings) {
        if self.display != *display {
            self.display = display.clone();
        }
    }

    fn audio_frame(&self) -> u64 {
        (self.ticks as f64 * f64::from(self.sounds.rate()) / self.clock.tick_rate()).floor() as u64
    }

    pub fn audio_due(&self) -> usize {
        usize::try_from(self.audio_frame().saturating_sub(self.sounds.rendered))
            .unwrap_or(usize::MAX)
    }

    pub(crate) fn render_audio(&mut self) {
        self.sounds.render_until(self.audio_frame());
    }

    pub(crate) fn clock_mut(&mut self) -> &mut Clock {
        &mut self.clock
    }
}

fn mover_order(scene: &Scene, objects: &[Entry]) -> Vec<usize> {
    if scene.movers.is_empty() {
        return Vec::new();
    }
    let owned: Vec<bool> = scene
        .objects
        .iter()
        .map(|object| {
            scene
                .movers
                .values()
                .any(|mover| mover.objects.contains(&object.name))
        })
        .collect();
    let mut order: Vec<(usize, usize)> = Vec::new();
    for place in 0..objects.len() {
        let mut depth = 0;
        let mut moved = owned[place];
        let mut at = objects[place].parent;
        while let Some(parent) = at {
            depth += 1;
            if depth > objects.len() {
                break;
            }
            moved |= owned[parent];
            at = objects[parent].parent;
        }
        if moved {
            order.push((depth, place));
        }
    }
    order.sort();
    order.into_iter().map(|(_, place)| place).collect()
}

fn model_in(objects: &[Entry], place: usize) -> Matrix {
    let entry = &objects[place];
    let mut matrix = entry.local;
    let mut at = entry.parent;
    let mut steps = 0;
    while let Some(parent) = at {
        steps += 1;
        if steps > objects.len() {
            break;
        }
        matrix = multiply(objects[parent].local, matrix);
        at = objects[parent].parent;
    }
    matrix
}
