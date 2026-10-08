use std::collections::BTreeMap;

use pfx_core::anim::{AnimError, Animator, Fired, Playback, SpritePlayer};
use pfx_load::scene::{Camera, Matrix, Posed, Scene, billboard};

use super::{ObjectId, World};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnimationEvent {
    pub object: ObjectId,
    pub clip: String,
    pub event: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Defaults {
    speed: f32,
    playback: Playback,
}

impl Default for Defaults {
    fn default() -> Self {
        Self {
            speed: 1.0,
            playback: Playback::Loop,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Animations {
    animators: BTreeMap<usize, Animator>,
    sprites: BTreeMap<usize, SpritePlayer>,
    defaults: BTreeMap<usize, Defaults>,
    events: Vec<AnimationEvent>,
    errors: Vec<AnimError>,
}

fn object_error(name: &str, error: AnimError) -> String {
    format!("object {name}: {error}")
}

impl Animations {
    pub(crate) fn new(scene: &Scene, rate: u32) -> Result<Self, String> {
        let mut out = Self::default();
        for (index, object) in scene.objects.iter().enumerate() {
            let data = scene.animations.get(&object.name);
            let defaults = data.map_or_else(Defaults::default, |data| Defaults {
                speed: data.speed,
                playback: data.playback,
            });
            out.defaults.insert(index, defaults);
            if let Some(sprite) = data.and_then(|data| data.sprite.as_ref()) {
                let mut player = SpritePlayer::new(sprite.sheet.clone(), rate);
                if let Some(data) = data
                    && let Some(clip) = &data.clip
                {
                    player
                        .play_from(clip, data.start, 0)
                        .and_then(|()| player.set_speed(data.speed, 0))
                        .map_err(|error| object_error(&object.name, error))?;
                }
                out.sprites.insert(index, player);
                continue;
            }
            let rig = data.and_then(|data| data.rig.clone()).or_else(|| {
                scene
                    .meshes
                    .get(&object.mesh)
                    .and_then(|mesh| mesh.rig.clone())
            });
            let Some(rig) = rig else {
                continue;
            };
            let mut animator = Animator::new(rig, rate);
            if let Some(data) = data.filter(|data| data.plays()) {
                let clips: Vec<(&str, f32)> = if data.blend.is_empty() {
                    data.clip.iter().map(|clip| (clip.as_str(), 1.0)).collect()
                } else {
                    data.blend
                        .iter()
                        .map(|(clip, weight)| (clip.as_str(), *weight))
                        .collect()
                };
                animator
                    .play_blend(&clips, 0)
                    .map_err(|error| object_error(&object.name, error))?;
                for (clip, _) in &clips {
                    animator
                        .set_playback(clip, data.playback)
                        .and_then(|()| animator.set_start(clip, data.start, 0))
                        .and_then(|()| animator.set_speed(clip, data.speed, 0))
                        .map_err(|error| object_error(&object.name, error))?;
                }
            }
            out.animators.insert(index, animator);
        }
        Ok(out)
    }
}

pub struct Animate<'a> {
    world: &'a mut World,
    id: ObjectId,
    error: Option<AnimError>,
}

impl Animate<'_> {
    fn each(mut self, change: impl FnOnce(&mut World, ObjectId) -> Result<(), AnimError>) -> Self {
        if self.error.is_none()
            && let Err(error) = change(self.world, self.id)
        {
            self.world.animations.errors.push(error.clone());
            self.error = Some(error);
        }
        self
    }

    pub fn blend(self, clip: &str, weight: f32) -> Self {
        self.each(|world, id| {
            let tick = world.ticks;
            let defaults = world.defaults(id);
            match world.animations.animators.get_mut(&id.index()) {
                Some(animator) => {
                    animator.blend(clip, weight, tick)?;
                    animator.set_playback(clip, defaults.playback)?;
                    animator.set_speed(clip, defaults.speed, tick)
                }
                None => Err(world.not_animated(id, "blends only skeletal clips")),
            }
        })
    }

    pub fn fade(self, seconds: f32) -> Self {
        self.each(
            |world, id| match world.animations.animators.get_mut(&id.index()) {
                Some(animator) => animator.set_fade(seconds),
                None => Err(world.not_animated(id, "crossfades only skeletal clips")),
            },
        )
    }

    pub fn speed(self, speed: f32) -> Self {
        self.each(|world, id| {
            let tick = world.ticks;
            if let Some(player) = world.animations.sprites.get_mut(&id.index()) {
                return player.set_speed(speed, tick);
            }
            let Some(animator) = world.animations.animators.get_mut(&id.index()) else {
                return Err(world.not_animated(id, "has no animation"));
            };
            for clip in playing(animator) {
                animator.set_speed(&clip, speed, tick)?;
            }
            Ok(())
        })
    }

    pub fn start(self, seconds: f32) -> Self {
        self.each(|world, id| {
            let tick = world.ticks;
            if let Some(player) = world.animations.sprites.get_mut(&id.index()) {
                let Some(clip) = player.clip().map(|clip| clip.name().to_string()) else {
                    return Ok(());
                };
                return player.play_from(&clip, seconds, tick);
            }
            let Some(animator) = world.animations.animators.get_mut(&id.index()) else {
                return Err(world.not_animated(id, "has no animation"));
            };
            for clip in playing(animator) {
                animator.set_start(&clip, seconds, tick)?;
            }
            Ok(())
        })
    }

    pub fn playback(self, playback: Playback) -> Self {
        self.each(|world, id| {
            let Some(animator) = world.animations.animators.get_mut(&id.index()) else {
                return Err(world.not_animated(id, "sets a sprite clip's loop in the scene"));
            };
            for clip in playing(animator) {
                animator.set_playback(&clip, playback)?;
            }
            Ok(())
        })
    }

    pub fn once(self) -> Self {
        self.playback(Playback::Once)
    }

    pub fn looped(self) -> Self {
        self.playback(Playback::Loop)
    }

    pub fn ping_pong(self) -> Self {
        self.playback(Playback::PingPong)
    }

    pub fn done(self) -> Result<(), AnimError> {
        match self.error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl World {
    fn defaults(&self, id: ObjectId) -> Defaults {
        self.animations
            .defaults
            .get(&id.index())
            .copied()
            .unwrap_or_default()
    }

    fn not_animated(&self, id: ObjectId, reason: &str) -> AnimError {
        AnimError::NotAnimated(format!(
            "object {} {reason}",
            self.objects
                .get(id.index())
                .map_or("(gone)", |entry| entry.name.as_str())
        ))
    }

    fn start_clip(&mut self, id: ObjectId, clip: &str) -> Result<(), AnimError> {
        let tick = self.ticks;
        let defaults = self.defaults(id);
        if let Some(player) = self.animations.sprites.get_mut(&id.index()) {
            return player.play(clip, tick);
        }
        let Some(animator) = self.animations.animators.get_mut(&id.index()) else {
            return Err(self.not_animated(id, "has no animation"));
        };
        animator.play(clip, tick)?;
        animator.set_playback(clip, defaults.playback)?;
        animator.set_speed(clip, defaults.speed, tick)
    }

    pub fn play_clip(&mut self, id: ObjectId, clip: &str) -> Result<(), AnimError> {
        let result = self.start_clip(id, clip);
        if let Err(error) = &result {
            self.animations.errors.push(error.clone());
        }
        result
    }

    pub fn animate(&mut self, id: ObjectId, clip: &str) -> Animate<'_> {
        let error = self.play_clip(id, clip).err();
        Animate {
            world: self,
            id,
            error,
        }
    }

    pub fn stop_animation(&mut self, id: ObjectId) -> bool {
        if let Some(player) = self.animations.sprites.get_mut(&id.index()) {
            player.stop();
            return true;
        }
        match self.animations.animators.get_mut(&id.index()) {
            Some(animator) => {
                animator.stop();
                true
            }
            None => false,
        }
    }

    pub fn animator(&self, id: ObjectId) -> Option<&Animator> {
        self.animations.animators.get(&id.index())
    }

    pub fn animator_mut(&mut self, id: ObjectId) -> Option<&mut Animator> {
        self.animations.animators.get_mut(&id.index())
    }

    pub fn sprite(&self, id: ObjectId) -> Option<&SpritePlayer> {
        self.animations.sprites.get(&id.index())
    }

    pub fn sprite_mut(&mut self, id: ObjectId) -> Option<&mut SpritePlayer> {
        self.animations.sprites.get_mut(&id.index())
    }

    pub fn palette(&self, id: ObjectId) -> Option<Vec<Matrix>> {
        self.animator(id)
            .map(|animator| animator.palette(self.ticks))
    }

    pub fn sprite_frame(&self, id: ObjectId) -> Option<[f32; 4]> {
        let player = self.sprite(id)?;
        player
            .uv(self.ticks)
            .or_else(|| player.sheet().atlas().uv(0))
    }

    pub fn animation_events(&self) -> &[AnimationEvent] {
        &self.animations.events
    }

    pub fn animation_errors(&self) -> &[AnimError] {
        &self.animations.errors
    }

    pub(crate) fn palettes(&self) -> BTreeMap<usize, Vec<Matrix>> {
        self.animations
            .animators
            .iter()
            .map(|(&index, animator)| (index, animator.palette(self.ticks)))
            .collect()
    }

    pub(crate) fn sprite_frames(&self) -> BTreeMap<usize, [f32; 4]> {
        self.animations
            .sprites
            .keys()
            .filter_map(|&index| Some((index, self.sprite_frame(ObjectId::from_index(index))?)))
            .collect()
    }

    pub(crate) fn faced_models(&self, camera: &Camera) -> Vec<Matrix> {
        let mut models = self.models();
        for (model, object) in models.iter_mut().zip(&self.scene.objects) {
            if object.face_camera {
                *model = billboard(*model, camera);
            }
        }
        models
    }

    pub fn posed(&self) -> Posed {
        Posed {
            time: self.time() as f32,
            models: self.faced_models(&self.fx.camera(&self.camera)),
            palettes: self.palettes(),
            frames: self.sprite_frames(),
        }
    }

    pub(crate) fn begin_animations(&mut self) {
        self.animations.errors.clear();
        self.animations.events.clear();
    }

    pub(crate) fn step_animations(&mut self) {
        let tick = self.ticks;
        let mut fired: Vec<(usize, Fired)> = Vec::new();
        for (&index, animator) in &mut self.animations.animators {
            fired.extend(animator.advance(tick).into_iter().map(|one| (index, one)));
        }
        for (&index, player) in &mut self.animations.sprites {
            fired.extend(player.advance(tick).into_iter().map(|one| (index, one)));
        }
        fired.sort_by_key(|(index, _)| *index);
        self.animations.events = fired
            .into_iter()
            .map(|(index, one)| AnimationEvent {
                object: ObjectId::from_index(index),
                clip: one.clip,
                event: one.event,
            })
            .collect();
    }

    pub(crate) fn rate_of(clock: &pfx_core::clock::Clock) -> u32 {
        clock.tick_rate().round().max(1.0) as u32
    }
}

fn playing(animator: &Animator) -> Vec<String> {
    animator
        .playing()
        .into_iter()
        .map(|(clip, _)| clip.to_string())
        .collect()
}
