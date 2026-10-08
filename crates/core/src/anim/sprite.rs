use std::sync::Arc;

use super::AnimError;
use super::animator::{Fired, MAX_REPEATS, Playback};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Atlas {
    size: [u32; 2],
    frames: Vec<[u32; 4]>,
}

impl Atlas {
    pub fn grid(size: [u32; 2], cell: [u32; 2], count: Option<usize>) -> Result<Self, AnimError> {
        if size.contains(&0) || cell.contains(&0) || cell[0] > size[0] || cell[1] > size[1] {
            return Err(AnimError::Atlas(format!(
                "a grid of {}x{} cells does not fit a {}x{} atlas",
                cell[0], cell[1], size[0], size[1]
            )));
        }
        let columns = size[0] / cell[0];
        let rows = size[1] / cell[1];
        let whole = (columns * rows) as usize;
        let count = count.unwrap_or(whole);
        if count == 0 || count > whole {
            return Err(AnimError::Atlas(format!(
                "the grid holds {whole} cells, not {count}"
            )));
        }
        let frames = (0..count as u32)
            .map(|index| {
                [
                    index % columns * cell[0],
                    index / columns * cell[1],
                    cell[0],
                    cell[1],
                ]
            })
            .collect();
        Ok(Self { size, frames })
    }

    pub fn rects(size: [u32; 2], frames: Vec<[u32; 4]>) -> Result<Self, AnimError> {
        if size.contains(&0) || frames.is_empty() {
            return Err(AnimError::Atlas("an atlas needs a size and frames".into()));
        }
        for (index, rect) in frames.iter().enumerate() {
            let inside = rect[2] > 0
                && rect[3] > 0
                && rect[0]
                    .checked_add(rect[2])
                    .is_some_and(|end| end <= size[0])
                && rect[1]
                    .checked_add(rect[3])
                    .is_some_and(|end| end <= size[1]);
            if !inside {
                return Err(AnimError::Atlas(format!(
                    "frame {index} ({}, {}, {}x{}) is outside the {}x{} atlas",
                    rect[0], rect[1], rect[2], rect[3], size[0], size[1]
                )));
            }
        }
        Ok(Self { size, frames })
    }

    pub fn size(&self) -> [u32; 2] {
        self.size
    }

    pub fn frames(&self) -> &[[u32; 4]] {
        &self.frames
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    pub fn uv(&self, frame: usize) -> Option<[f32; 4]> {
        let [x, y, w, h] = *self.frames.get(frame)?;
        let width = self.size[0] as f32;
        let height = self.size[1] as f32;
        Some([
            x as f32 / width,
            y as f32 / height,
            (x + w) as f32 / width,
            (y + h) as f32 / height,
        ])
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FrameEvent {
    pub frame: usize,
    pub name: String,
}

impl FrameEvent {
    pub fn new(frame: usize, name: impl Into<String>) -> Self {
        Self {
            frame,
            name: name.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpriteClip {
    name: String,
    frames: Vec<usize>,
    fps: f32,
    playback: Playback,
    events: Vec<FrameEvent>,
}

impl SpriteClip {
    pub fn new(
        name: impl Into<String>,
        frames: Vec<usize>,
        fps: f32,
        playback: Playback,
    ) -> Result<Self, AnimError> {
        let name = name.into();
        if frames.is_empty() {
            return Err(AnimError::Sprite {
                clip: name,
                reason: "it has no frames".into(),
            });
        }
        if !fps.is_finite() || fps <= 0.0 {
            return Err(AnimError::Sprite {
                clip: name,
                reason: format!("fps {fps} is not above 0"),
            });
        }
        Ok(Self {
            name,
            frames,
            fps,
            playback,
            events: Vec::new(),
        })
    }

    pub fn with_events(mut self, mut events: Vec<FrameEvent>) -> Result<Self, AnimError> {
        if let Some(event) = events.iter().find(|event| event.frame >= self.frames.len()) {
            return Err(AnimError::Sprite {
                clip: self.name.clone(),
                reason: format!(
                    "event {} is on frame {} of {}",
                    event.name,
                    event.frame,
                    self.frames.len()
                ),
            });
        }
        events.sort_by(|a, b| a.frame.cmp(&b.frame).then_with(|| a.name.cmp(&b.name)));
        self.events = events;
        Ok(self)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn frames(&self) -> &[usize] {
        &self.frames
    }

    pub fn fps(&self) -> f32 {
        self.fps
    }

    pub fn playback(&self) -> Playback {
        self.playback
    }

    pub fn events(&self) -> &[FrameEvent] {
        &self.events
    }

    pub fn position(&self, step: u64) -> usize {
        let len = self.frames.len() as u64;
        match self.playback {
            Playback::Loop => (step % len) as usize,
            Playback::Once => step.min(len - 1) as usize,
            Playback::PingPong if len == 1 => 0,
            Playback::PingPong => {
                let at = step % (2 * len - 2);
                (if at < len { at } else { 2 * len - 2 - at }) as usize
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpriteSheet {
    atlas: Atlas,
    clips: Vec<SpriteClip>,
}

impl SpriteSheet {
    pub fn new(atlas: Atlas, clips: Vec<SpriteClip>) -> Result<Self, AnimError> {
        for (index, clip) in clips.iter().enumerate() {
            if clips[..index].iter().any(|other| other.name == clip.name) {
                return Err(AnimError::DuplicateClip(clip.name.clone()));
            }
            if let Some(frame) = clip.frames.iter().find(|frame| **frame >= atlas.len()) {
                return Err(AnimError::Sprite {
                    clip: clip.name.clone(),
                    reason: format!("frame {frame} is past the atlas's {}", atlas.len()),
                });
            }
        }
        Ok(Self { atlas, clips })
    }

    pub fn atlas(&self) -> &Atlas {
        &self.atlas
    }

    pub fn clips(&self) -> &[SpriteClip] {
        &self.clips
    }

    pub fn clip(&self, name: &str) -> Option<usize> {
        self.clips.iter().position(|clip| clip.name == name)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpritePlayer {
    sheet: Arc<SpriteSheet>,
    rate: u32,
    clip: Option<usize>,
    start: u64,
    offset: f64,
    speed: f32,
    heard: Option<u64>,
}

impl SpritePlayer {
    pub fn new(sheet: Arc<SpriteSheet>, rate: u32) -> Self {
        Self {
            sheet,
            rate: rate.max(1),
            clip: None,
            start: 0,
            offset: 0.0,
            speed: 1.0,
            heard: None,
        }
    }

    pub fn sheet(&self) -> &SpriteSheet {
        &self.sheet
    }

    pub fn play(&mut self, clip: &str, tick: u64) -> Result<(), AnimError> {
        self.play_from(clip, 0.0, tick)
    }

    pub fn play_from(&mut self, clip: &str, seconds: f32, tick: u64) -> Result<(), AnimError> {
        let index = self
            .sheet
            .clip(clip)
            .ok_or_else(|| AnimError::UnknownClip(clip.to_string()))?;
        if !seconds.is_finite() || seconds < 0.0 {
            return Err(AnimError::Start(seconds));
        }
        self.clip = Some(index);
        self.start = tick;
        self.offset = seconds as f64;
        self.heard = None;
        Ok(())
    }

    pub fn stop(&mut self) {
        self.clip = None;
        self.heard = None;
    }

    pub fn set_speed(&mut self, speed: f32, tick: u64) -> Result<(), AnimError> {
        if !speed.is_finite() || speed < 0.0 {
            return Err(AnimError::Speed(speed));
        }
        self.offset = self.elapsed(tick);
        self.start = tick.max(self.start);
        self.speed = speed;
        Ok(())
    }

    pub fn clip(&self) -> Option<&SpriteClip> {
        self.clip.map(|index| &self.sheet.clips()[index])
    }

    fn elapsed(&self, tick: u64) -> f64 {
        let ticks = tick.saturating_sub(self.start) as f64;
        self.offset + ticks * self.speed as f64 / self.rate as f64
    }

    pub fn step(&self, tick: u64) -> Option<u64> {
        let clip = self.clip()?;
        let fps = clip.fps as f64;
        let ticks = tick.saturating_sub(self.start) as f64;
        let steps = self.offset * fps + ticks * self.speed as f64 * fps / self.rate as f64;
        Some(steps.floor() as u64)
    }

    pub fn frame(&self, tick: u64) -> Option<usize> {
        let clip = self.clip()?;
        let step = self.step(tick)?;
        Some(clip.frames[clip.position(step)])
    }

    pub fn uv(&self, tick: u64) -> Option<[f32; 4]> {
        self.sheet.atlas.uv(self.frame(tick)?)
    }

    pub fn finished(&self, tick: u64) -> bool {
        match (self.clip(), self.step(tick)) {
            (Some(clip), Some(step)) => {
                clip.playback == Playback::Once && step + 1 >= clip.frames.len() as u64
            }
            _ => false,
        }
    }

    pub fn advance(&mut self, tick: u64) -> Vec<Fired> {
        let (Some(clip), Some(now)) = (self.clip(), self.step(tick)) else {
            return Vec::new();
        };
        let first = match self.heard {
            None => (self.offset * clip.fps as f64).floor() as u64,
            Some(heard) if heard >= now => return Vec::new(),
            Some(heard) => heard + 1,
        };
        let len = clip.frames.len() as u64;
        let last = match clip.playback {
            Playback::Loop | Playback::PingPong => now,
            Playback::Once => now.min(len - 1),
        };
        let first = first.max(last.saturating_sub(MAX_REPEATS * len - 1));
        let mut fired = Vec::new();
        if first <= last {
            for step in first..=last {
                let position = clip.position(step);
                for event in clip.events.iter().filter(|event| event.frame == position) {
                    fired.push(Fired {
                        clip: clip.name.clone(),
                        event: event.name.clone(),
                        time: event.frame as f32 / clip.fps,
                        tick,
                    });
                }
            }
        }
        self.heard = Some(now);
        fired
    }
}
