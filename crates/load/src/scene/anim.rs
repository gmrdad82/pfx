use std::collections::BTreeMap;
use std::sync::Arc;

use pfx_core::anim::{Atlas, ClipEvent, FrameEvent, Playback, Rig, SpriteClip, SpriteSheet};
use pfx_scene::Entry;
use pfx_scene::types as file;

use super::build::{Places, Reader, Step, content_from, inner};
use super::{Content, Matrix, Object, SceneError, SceneMesh};

pub const SPRITE_CLIP: &str = "";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Posed {
    pub time: f32,
    pub models: Vec<Matrix>,
    pub palettes: BTreeMap<usize, Vec<Matrix>>,
    pub frames: BTreeMap<usize, [f32; 4]>,
}

impl Posed {
    pub fn at(time: f32) -> Self {
        Self {
            time,
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Animation {
    pub clip: Option<String>,
    pub blend: Vec<(String, f32)>,
    pub speed: f32,
    pub playback: Playback,
    pub start: f32,
    pub rig: Option<Arc<Rig>>,
    pub sprite: Option<Sprite>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sprite {
    pub sheet: Arc<SpriteSheet>,
    pub content: String,
}

impl Animation {
    pub fn plays(&self) -> bool {
        self.clip.is_some() || !self.blend.is_empty()
    }

    pub fn still_frame(&self) -> Option<[f32; 4]> {
        let sprite = self.sprite.as_ref()?;
        let sheet = &sprite.sheet;
        let frame = self
            .clip
            .as_deref()
            .and_then(|clip| sheet.clip(clip))
            .map(|clip| {
                let clip = &sheet.clips()[clip];
                let step = (self.start as f64 * clip.fps() as f64).floor() as u64;
                clip.frames()[clip.position(step)]
            })
            .unwrap_or(0);
        sheet.atlas().uv(frame)
    }
}

pub fn surface(frame: [f32; 4]) -> ([f32; 2], [f32; 2], [f32; 4]) {
    (
        [frame[0], frame[1]],
        [frame[2] - frame[0], frame[3] - frame[1]],
        frame,
    )
}

fn playback(repeat: Option<file::Repeat>) -> Option<Playback> {
    repeat.map(|repeat| match repeat {
        file::Repeat::Loop => Playback::Loop,
        file::Repeat::Once => Playback::Once,
        file::Repeat::PingPong => Playback::PingPong,
    })
}

pub fn content_key(object: &str) -> String {
    format!("sprite:{object}")
}

pub(super) fn build_animations(
    places: &Places,
    entries: &[Entry<file::Object>],
    objects: &mut [Object],
    meshes: &BTreeMap<String, SceneMesh>,
    contents: &mut BTreeMap<String, Content>,
    reader: &mut Reader,
) -> Result<BTreeMap<String, Animation>, SceneError> {
    let mut out = BTreeMap::new();
    for (entry, object) in entries.iter().zip(objects.iter_mut()) {
        let Some(found) = &entry.value.animation else {
            continue;
        };
        let name = entry.key.as_str();
        let refuse = |key: Option<&str>, message: String| {
            let mut at = vec![Step::Named("object", inner(name)), Step::Key("animation")];
            if let Some(key) = key {
                at.push(Step::Key(key));
            }
            places.refusal(&entry.file, &at, format!("object {name}: {message}"))
        };
        let looped = playback(found.looping).unwrap_or_default();
        let mut animation = Animation {
            clip: found.clip.clone(),
            blend: found
                .blend
                .iter()
                .flatten()
                .map(|blend| {
                    (
                        blend.clip.clone(),
                        blend.weight.unwrap_or(file::BlendClip::WEIGHT),
                    )
                })
                .collect(),
            speed: found.speed.unwrap_or(file::Animation::SPEED),
            playback: looped,
            start: found.start.unwrap_or(file::Animation::START),
            rig: None,
            sprite: None,
        };
        if found.is_sprite() {
            let atlas_path = found.atlas.as_deref().unwrap_or_default();
            let path = places.root.join(atlas_path);
            let bytes = reader
                .read(&path)
                .map_err(|message| refuse(Some("atlas"), message))?;
            let content =
                content_from(path, &bytes).map_err(|message| refuse(Some("atlas"), message))?;
            if let Some(other) = &object.content {
                return Err(refuse(
                    None,
                    format!("it shows content {other} and a sprite; give it one"),
                ));
            }
            let size = [content.width, content.height];
            let atlas = match (&found.grid, &found.frames) {
                (Some([columns, rows]), _) => Atlas::grid(
                    size,
                    [size[0] / columns.max(&1), size[1] / rows.max(&1)],
                    Some((columns * rows) as usize),
                ),
                (None, Some(frames)) => Atlas::rects(size, frames.clone()),
                (None, None) => Atlas::grid(size, size, None),
            }
            .map_err(|error| {
                refuse(
                    Some(if found.grid.is_some() {
                        "grid"
                    } else {
                        "frames"
                    }),
                    error.to_string(),
                )
            })?;
            let fps = found.fps.unwrap_or(file::Animation::FPS);
            let mut clips: Vec<SpriteClip> = Vec::new();
            if found.clips.is_empty() {
                clips.push(
                    SpriteClip::new(SPRITE_CLIP, (0..atlas.len()).collect(), fps, looped)
                        .map_err(|error| refuse(None, error.to_string()))?,
                );
                if animation.clip.is_none() {
                    animation.clip = Some(SPRITE_CLIP.to_string());
                }
            }
            for (clip, range) in &found.clips {
                if range.to as usize >= atlas.len() {
                    return Err(refuse(
                        Some("clips"),
                        format!(
                            "sprite clip {clip} runs to frame {}, past the atlas's last frame {}",
                            range.to,
                            atlas.len().saturating_sub(1)
                        ),
                    ));
                }
                clips.push(
                    SpriteClip::new(
                        clip.clone(),
                        (range.from as usize..=range.to as usize).collect(),
                        range.fps.unwrap_or(fps),
                        playback(range.looping).unwrap_or(looped),
                    )
                    .map_err(|error| refuse(Some("clips"), error.to_string()))?,
                );
            }
            let mut marks: BTreeMap<String, Vec<FrameEvent>> = BTreeMap::new();
            for event in &found.events {
                let clip = event
                    .clip
                    .clone()
                    .or_else(|| animation.clip.clone())
                    .unwrap_or_default();
                let Some(index) = clips.iter().position(|candidate| candidate.name() == clip)
                else {
                    return Err(refuse(
                        Some("events"),
                        format!(
                            "event {} marks clip {clip}, which the sprite has not",
                            event.name
                        ),
                    ));
                };
                let target = &clips[index];
                let frame = match (event.frame, event.time) {
                    (Some(frame), _) => frame as usize,
                    (None, Some(time)) => (time as f64 * target.fps() as f64).floor() as usize,
                    (None, None) => continue,
                };
                if frame < target.frames().len() {
                    marks
                        .entry(clip)
                        .or_default()
                        .push(FrameEvent::new(frame, event.name.clone()));
                }
            }
            let clips = clips
                .into_iter()
                .map(|clip| match marks.remove(clip.name()) {
                    Some(events) => clip.with_events(events),
                    None => Ok(clip),
                })
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| refuse(Some("events"), error.to_string()))?;
            let sheet =
                SpriteSheet::new(atlas, clips).map_err(|error| refuse(None, error.to_string()))?;
            if let Some(clip) = &animation.clip
                && sheet.clip(clip).is_none()
            {
                return Err(refuse(
                    Some("clip"),
                    format!("the sprite has no clip {clip}"),
                ));
            }
            let key = content_key(name);
            contents.insert(key.clone(), content);
            object.content = Some(key.clone());
            animation.sprite = Some(Sprite {
                sheet: Arc::new(sheet),
                content: key,
            });
        } else {
            let Some(rig) = meshes.get(&object.mesh).and_then(|mesh| mesh.rig.clone()) else {
                return Err(refuse(
                    None,
                    format!("mesh {} holds no animation clips", object.mesh),
                ));
            };
            let known = |clip: &str| rig.clip(clip).is_some();
            if let Some(clip) = &animation.clip
                && !known(clip)
            {
                return Err(refuse(
                    Some("clip"),
                    format!("mesh {} has no clip {clip}", object.mesh),
                ));
            }
            if let Some((clip, _)) = animation.blend.iter().find(|(clip, _)| !known(clip)) {
                return Err(refuse(
                    Some("blend"),
                    format!("mesh {} has no clip {clip}", object.mesh),
                ));
            }
            let mut marks: BTreeMap<String, Vec<ClipEvent>> = BTreeMap::new();
            for event in &found.events {
                let Some(clip) = event.clip.clone().or_else(|| animation.clip.clone()) else {
                    return Err(refuse(
                        Some("events"),
                        format!("event {} names no clip", event.name),
                    ));
                };
                let Some(index) = rig.clip(&clip) else {
                    return Err(refuse(
                        Some("events"),
                        format!(
                            "event {} marks clip {clip}, which mesh {} has not",
                            event.name, object.mesh
                        ),
                    ));
                };
                let Some(time) = event.time else {
                    return Err(refuse(
                        Some("events"),
                        format!("event {} of a glTF clip takes a time", event.name),
                    ));
                };
                if time <= rig.clips()[index].duration() {
                    marks
                        .entry(clip)
                        .or_default()
                        .push(ClipEvent::new(time, event.name.clone()));
                }
            }
            let mut rig = Rig::clone(&rig);
            for (clip, events) in marks {
                rig = rig
                    .with_events(&clip, events)
                    .map_err(|error| refuse(Some("events"), error.to_string()))?;
            }
            animation.rig = Some(Arc::new(rig));
        }
        out.insert(name.to_string(), animation);
    }
    Ok(out)
}
