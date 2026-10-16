mod recipe;

pub use recipe::{Curve, Kind, Layer, Recipe, Saturate, Table, hz};

use pfx_sound::{ChannelId, Envelope, Mixer, Shape, VoiceId, Wave};
use std::collections::BTreeMap;

pub const CHANNELS: u16 = 2;
const FIRST: u32 = 4096;
const END: f32 = 0.005;

pub trait Sounds {
    fn knows(&self, name: &str) -> bool;
    fn clock(&self) -> u64;
    fn cue(&mut self, name: &str, at: u64, play: Play) -> Result<(), String>;
    fn render(&mut self, frames: usize) -> Vec<f32>;

    fn route(&mut self, _under: &[String], _beds: &[String]) -> Result<(), String> {
        Ok(())
    }

    fn render_apart(&mut self, frames: usize) -> [Vec<f32>; 3] {
        [self.render(frames), Vec::new(), Vec::new()]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Bus {
    Key,
    Bed,
    Dry,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Play {
    pub gain: f32,
    pub pitch: f32,
    pub seconds: Option<f32>,
}

impl Default for Play {
    fn default() -> Play {
        Play {
            gain: 1.0,
            pitch: 0.0,
            seconds: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Act {
    Stop(VoiceId, f32, Bus),
}

pub struct Score {
    table: Table,
    mixer: Mixer,
    bed: Option<Mixer>,
    dry: Option<Mixer>,
    routes: BTreeMap<String, Bus>,
    to: Bus,
    seed: u64,
    rate: u32,
    clock: u64,
    acts: Vec<(u64, Act)>,
    next: u32,
}

impl Score {
    pub fn new(text: &str, bare: &[&str], seed: u64, rate: u32) -> Result<Score, String> {
        let table = Table::parse(text, bare)?;
        let mixer = Score::mixer(&table, seed, rate);
        Ok(Score {
            table,
            mixer,
            bed: None,
            dry: None,
            routes: BTreeMap::new(),
            to: Bus::Key,
            seed,
            rate: rate.max(1),
            clock: 0,
            acts: Vec::new(),
            next: 0,
        })
    }

    fn mixer(table: &Table, seed: u64, rate: u32) -> Mixer {
        let mut mixer = Mixer::new(rate.max(1), CHANNELS);
        mixer.set_seed(seed);
        mixer.set_limiter(Some(table.limiter));
        mixer.set_master_volume(table.master);
        mixer
    }

    pub fn knows(&self, name: &str) -> bool {
        self.table.recipes.contains_key(name)
    }

    pub fn route(&mut self, under: &[String], beds: &[String]) -> Result<(), String> {
        let all = under.iter().any(|u| u == "score");
        for u in under.iter().filter(|u| *u != "score") {
            let named = self.table.recipes.contains_key(u);
            let kind = self
                .table
                .recipes
                .values()
                .any(|r| r.kind.as_deref() == Some(u.as_str()));
            if !(named || kind) {
                return Err(format!(
                    "[duck] under names '{u}', which is no sound or kind in the score"
                ));
            }
        }
        self.routes.clear();
        for (name, r) in &self.table.recipes {
            let keyed =
                all || under.contains(name) || r.kind.as_ref().is_some_and(|k| under.contains(k));
            let bus = if beds.contains(name) {
                Bus::Bed
            } else if keyed {
                Bus::Key
            } else {
                Bus::Dry
            };
            if bus != Bus::Key {
                self.routes.insert(name.clone(), bus);
            }
        }
        let uses = |b: Bus| self.routes.values().any(|v| *v == b);
        let (bed, dry) = (uses(Bus::Bed), uses(Bus::Dry));
        self.bed = bed.then(|| Score::mixer(&self.table, self.seed.wrapping_add(1), self.rate));
        self.dry = dry.then(|| Score::mixer(&self.table, self.seed.wrapping_add(2), self.rate));
        Ok(())
    }

    fn mix(&mut self, bus: Bus) -> &mut Mixer {
        match (bus, &mut self.bed, &mut self.dry) {
            (Bus::Bed, Some(m), _) | (Bus::Dry, _, Some(m)) => m,
            _ => &mut self.mixer,
        }
    }

    pub fn names(&self) -> Vec<&str> {
        self.table.recipes.keys().map(String::as_str).collect()
    }

    pub fn clock(&self) -> u64 {
        self.clock
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    pub fn recipe(&self, name: &str) -> Result<Recipe, String> {
        self.table
            .recipes
            .get(name)
            .cloned()
            .ok_or_else(|| format!("the score has no sound '{name}'"))
    }

    pub fn frames(&self, ms: f32) -> u64 {
        (f64::from(ms.max(0.0)) * f64::from(self.rate) / 1000.0).round() as u64
    }

    pub fn channel(&mut self) -> ChannelId {
        self.next = self.next.wrapping_add(1);
        ChannelId::custom(FIRST + self.next)
    }

    pub fn note(&mut self, channel: ChannelId, at: u64, wave: Wave, tone: [f32; 2], decay: f32) {
        let [hz, volume] = tone;
        let offset = at.saturating_sub(self.clock);
        let id = self.mixer.tone_in(offset, channel, wave, hz, volume);
        self.mixer.set_envelope(
            id,
            Some(Envelope {
                attack_ms: 0.0,
                decay_ms: decay * 1000.0,
                sustain: 0.0,
                release_ms: 0.0,
                shape: Shape::Linear,
            }),
        );
        let end = at + self.frames(decay * 1000.0) + 1;
        self.acts.push((end, Act::Stop(id, END, Bus::Key)));
    }

    pub fn cue(&mut self, name: &str, at: u64, play: Play) -> Result<(), String> {
        let recipe = self.recipe(name)?;
        let at = at.max(self.clock);
        self.to = self.routes.get(name).copied().unwrap_or(Bus::Key);
        let channel = self.channel();
        let stretch = play
            .seconds
            .map_or(1.0, |s| (s * 1000.0 / recipe.length).max(0.01));
        self.sweeps(&recipe, channel, at, stretch);
        for layer in &recipe.layers {
            self.layer(layer, channel, at, stretch, play);
        }
        self.to = Bus::Key;
        Ok(())
    }

    pub fn sweeps(&mut self, r: &Recipe, channel: ChannelId, at: u64, stretch: f32) {
        let (to, bus) = (self.to, self.to);
        if let Some(drive) = &r.drive {
            self.mix(to).set_channel_drive(channel, Some(drive.stage()));
        }
        let drive = r.drive.as_ref().map_or(&[][..], |d| &d.sweep[..]);
        for (points, way) in [(&r.lowpass[..], 0), (&r.highpass[..], 1), (drive, 2)] {
            let Some(first) = points.first() else {
                continue;
            };
            match way {
                0 => self.mix(to).set_channel_lowpass(channel, Some(first[1])),
                1 => self.mix(to).set_channel_highpass(channel, Some(first[1])),
                _ => {}
            }
            for pair in points.windows(2) {
                let [from, to] = [pair[0], pair[1]];
                let when = at + self.frames(from[0] * stretch);
                let seconds = (to[0] - from[0]) * stretch / 1000.0;
                match way {
                    0 => {
                        self.mix(bus)
                            .glide_channel_lowpass_at(when, channel, Some(to[1]), seconds)
                    }
                    1 => {
                        self.mix(bus)
                            .glide_channel_highpass_at(when, channel, Some(to[1]), seconds)
                    }
                    _ => self
                        .mix(bus)
                        .glide_channel_drive_at(when, channel, to[1], seconds),
                }
            }
        }
    }

    fn layer(&mut self, l: &Layer, channel: ChannelId, at: u64, stretch: f32, play: Play) {
        let start = at + self.frames(l.at * stretch);
        let hz = l.pitch() * 2f32.powf(play.pitch / 12.0);
        let volume = (l.volume * play.gain).max(0.0);
        let (bus, offset) = (self.to, start - self.clock);
        let mixer = self.mix(bus);
        let id = mixer.tone_in(offset, channel, l.wave.wave(), hz, volume);
        mixer.set_pan(id, l.pan);
        let shape = if l.linear {
            Shape::Linear
        } else {
            Shape::Exponential
        };
        let held = l.sustain > 0.0;
        self.mix(bus).set_envelope(
            id,
            Some(Envelope {
                attack_ms: l.attack * stretch,
                decay_ms: if held { 1.0 } else { l.decay * stretch },
                sustain: if held { l.sustain } else { 0.0 },
                release_ms: 0.0,
                shape,
            }),
        );
        if l.bend != 0.0 {
            let to = hz * 2f32.powf(l.bend / 12.0);
            let glide = if l.glide > 0.0 { l.glide } else { l.span() };
            self.mix(bus)
                .glide_tone_frequency_at(start, id, to, glide * stretch / 1000.0);
        }
        let (end, fade) = if held {
            (
                start + self.frames((l.attack + l.hold) * stretch),
                (l.release * stretch / 1000.0).max(END),
            )
        } else {
            (start + self.frames(l.span() * stretch) + 1, END)
        };
        self.acts
            .push((end.max(start + 1), Act::Stop(id, fade, bus)));
    }

    fn apply(&mut self, act: Act) {
        match act {
            Act::Stop(id, fade, bus) => self.mix(bus).stop(id, fade),
        }
    }

    fn due(&mut self) {
        let now = self.clock;
        let mut ready: Vec<(u64, Act)> = Vec::new();
        self.acts.retain(|&(at, act)| {
            if at <= now {
                ready.push((at, act));
                false
            } else {
                true
            }
        });
        ready.sort_by_key(|&(at, _)| at);
        for (_, act) in ready {
            self.apply(act);
        }
    }

    pub fn render(&mut self, frames: usize) -> Vec<f32> {
        let [mut out, bed, dry] = self.render_apart(frames);
        for part in [bed, dry] {
            for (o, v) in out.iter_mut().zip(part) {
                *o += v;
            }
        }
        out
    }

    pub fn render_apart(&mut self, frames: usize) -> [Vec<f32>; 3] {
        let mut out = Vec::with_capacity(frames * usize::from(CHANNELS));
        let (mut bed, mut dry) = (Vec::new(), Vec::new());
        let end = self.clock + frames as u64;
        while self.clock < end {
            self.due();
            let next = self
                .acts
                .iter()
                .map(|&(at, _)| at)
                .filter(|&at| at > self.clock)
                .min()
                .unwrap_or(end)
                .min(end);
            let n = (next - self.clock) as usize;
            out.extend(self.mixer.render(n));
            if let Some(m) = &mut self.bed {
                bed.extend(m.render(n));
            }
            if let Some(m) = &mut self.dry {
                dry.extend(m.render(n));
            }
            self.clock = next;
        }
        [out, bed, dry]
    }
}

impl Sounds for Score {
    fn knows(&self, name: &str) -> bool {
        Score::knows(self, name)
    }

    fn clock(&self) -> u64 {
        self.clock
    }

    fn cue(&mut self, name: &str, at: u64, play: Play) -> Result<(), String> {
        Score::cue(self, name, at, play)
    }

    fn render(&mut self, frames: usize) -> Vec<f32> {
        Score::render(self, frames)
    }

    fn route(&mut self, under: &[String], beds: &[String]) -> Result<(), String> {
        Score::route(self, under, beds)
    }

    fn render_apart(&mut self, frames: usize) -> [Vec<f32>; 3] {
        Score::render_apart(self, frames)
    }
}
