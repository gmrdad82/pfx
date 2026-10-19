mod recipe;

pub use recipe::{Curve, Kind, Layer, MOST_PITCH, MOST_SAMPLE_GAIN, Recipe, Saturate, Table, hz};

use pfx_sound::{ChannelId, Clip, Envelope, Level, Mixer, Prepared, Shape, VoiceId, Wave};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

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

    fn stems(&mut self, _selectors: &[Vec<String>]) -> Result<Vec<Vec<bool>>, String> {
        Err("[duck.<name>] needs a score that renders its sounds on stems".into())
    }

    fn render_stems(&mut self, frames: usize) -> Vec<Vec<f32>> {
        vec![self.render(frames)]
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
    Stop(VoiceId, f32, usize),
}

pub struct Score {
    table: Table,
    mixer: Mixer,
    stems: Vec<Option<Mixer>>,
    routes: BTreeMap<String, usize>,
    to: usize,
    seed: u64,
    rate: u32,
    clock: u64,
    acts: Vec<(u64, Act)>,
    next: u32,
    samples: BTreeMap<(String, usize), Prepared>,
}

fn trimmed(clip: &Clip, layer: &Layer, rate: u32) -> Result<Clip, String> {
    let clip = clip.for_rate(rate);
    let channels = usize::from(clip.channels.max(1));
    let frames = clip.frames() as usize;
    let at = |ms: f32| (f64::from(ms) * f64::from(rate) / 1000.0).round() as usize;
    let start = at(layer.start);
    let end = layer.end.map_or(frames, |ms| at(ms).min(frames));
    if start >= end {
        return Err(format!(
            "the sample is {:.1} ms long, and its start is past its end",
            frames as f64 * 1000.0 / f64::from(rate)
        ));
    }
    Ok(Clip {
        rate,
        channels: clip.channels,
        samples: clip.samples[start * channels..end * channels].to_vec(),
    })
}

fn load(
    table: &Table,
    rate: u32,
    dir: &Path,
) -> Result<BTreeMap<(String, usize), Prepared>, String> {
    let mut files: BTreeMap<PathBuf, Clip> = BTreeMap::new();
    let mut out = BTreeMap::new();
    for (name, recipe) in &table.recipes {
        for (i, layer) in recipe.layers.iter().enumerate() {
            let Some(file) = &layer.sample else {
                continue;
            };
            let path = dir.join(file);
            let at = |e: String| format!("score: {name}: {}: {e}", path.display());
            if !files.contains_key(&path) {
                let bytes = std::fs::read(&path).map_err(|e| at(e.to_string()))?;
                let clip = Clip::from_wav(&bytes)
                    .ok_or_else(|| at("not a WAV of 8-bit, 16-bit or float samples".into()))?;
                files.insert(path.clone(), clip);
            }
            let clip = trimmed(&files[&path], layer, rate).map_err(at)?;
            out.insert((name.clone(), i), Prepared::new(&clip, rate));
        }
    }
    Ok(out)
}

impl Score {
    pub fn new(text: &str, bare: &[&str], seed: u64, rate: u32) -> Result<Score, String> {
        Score::in_dir(text, bare, seed, rate, Path::new(""))
    }

    pub fn in_dir(
        text: &str,
        bare: &[&str],
        seed: u64,
        rate: u32,
        dir: &Path,
    ) -> Result<Score, String> {
        let table = Table::parse(text, bare)?;
        let mixer = Score::mixer(&table, seed, rate);
        let samples = load(&table, rate.max(1), dir)?;
        Ok(Score {
            samples,
            table,
            mixer,
            stems: Vec::new(),
            routes: BTreeMap::new(),
            to: 0,
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
            match bus {
                Bus::Key => None,
                Bus::Bed => self.routes.insert(name.clone(), 1),
                Bus::Dry => self.routes.insert(name.clone(), 2),
            };
        }
        let uses = |i: usize| self.routes.values().any(|v| *v == i);
        let used = [false, uses(1), uses(2)];
        self.make(&used);
        Ok(())
    }

    fn make(&mut self, used: &[bool]) {
        self.stems = used
            .iter()
            .enumerate()
            .map(|(i, u)| {
                (i > 0 && *u)
                    .then(|| Score::mixer(&self.table, self.seed.wrapping_add(i as u64), self.rate))
            })
            .collect();
    }

    fn holds(&self, name: &str, r: &Recipe, names: &[String]) -> bool {
        names
            .iter()
            .any(|n| n == name || r.kind.as_ref() == Some(n))
    }

    pub fn stems(&mut self, selectors: &[Vec<String>]) -> Result<Vec<Vec<bool>>, String> {
        for u in selectors.iter().flatten() {
            let named = self.table.recipes.contains_key(u);
            let kind = self
                .table
                .recipes
                .values()
                .any(|r| r.kind.as_deref() == Some(u.as_str()));
            if !(named || kind) {
                return Err(format!("[duck] '{u}' is no sound or kind in the score"));
            }
        }
        let mut masks: Vec<Vec<bool>> = vec![vec![false; selectors.len()]];
        let mut routes = BTreeMap::new();
        for (name, r) in &self.table.recipes {
            let mask: Vec<bool> = selectors.iter().map(|s| self.holds(name, r, s)).collect();
            if !mask.iter().any(|m| *m) {
                continue;
            }
            let i = match masks.iter().position(|m| *m == mask) {
                Some(i) => i,
                None => {
                    masks.push(mask);
                    masks.len() - 1
                }
            };
            routes.insert(name.clone(), i);
        }
        self.routes = routes;
        self.make(&vec![true; masks.len()]);
        Ok(masks)
    }

    fn mix(&mut self, stem: usize) -> &mut Mixer {
        match self.stems.get_mut(stem) {
            Some(Some(m)) => m,
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
        self.acts.push((end, Act::Stop(id, END, 0)));
    }

    pub fn cue(&mut self, name: &str, at: u64, play: Play) -> Result<(), String> {
        let recipe = self.recipe(name)?;
        let at = at.max(self.clock);
        self.to = self.routes.get(name).copied().unwrap_or(0);
        let channel = self.channel();
        let stretch = play
            .seconds
            .map_or(1.0, |s| (s * 1000.0 / recipe.length).max(0.01));
        self.sweeps(&recipe, channel, at, stretch);
        for (i, layer) in recipe.layers.iter().enumerate() {
            match self.samples.get(&(name.to_string(), i)).cloned() {
                Some(clip) => self.sample(&clip, layer, channel, at, stretch, play),
                None => self.layer(layer, channel, at, stretch, play),
            }
        }
        self.to = 0;
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

    fn sample(
        &mut self,
        clip: &Prepared,
        l: &Layer,
        channel: ChannelId,
        at: u64,
        stretch: f32,
        play: Play,
    ) {
        let start = at + self.frames(l.at * stretch);
        let level = Level {
            volume: (l.volume * play.gain).max(0.0),
            pan: l.pan,
            fade_in: self.frames(l.attack),
            fade_out: self.frames(l.release),
        };
        let bus = self.to;
        self.mix(bus)
            .play_prepared_pitched_at(channel, start, clip, l.rate(play.pitch), level);
    }

    fn layer(&mut self, l: &Layer, channel: ChannelId, at: u64, stretch: f32, play: Play) {
        let start = at + self.frames(l.at * stretch);
        let hz = l.pitch() * 2f32.powf(play.pitch / 12.0);
        let volume = (l.volume * play.gain).max(0.0);
        let (bus, offset) = (self.to, start - self.clock);
        let mixer = self.mix(bus);
        let wave = l.wave.map_or(Wave::Sine, Kind::wave);
        let id = mixer.tone_in(offset, channel, wave, hz, volume);
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
        let mut stems = self.render_stems(frames).into_iter();
        let mut next = || stems.next().unwrap_or_default();
        [next(), next(), next()]
    }

    pub fn render_stems(&mut self, frames: usize) -> Vec<Vec<f32>> {
        let mut out = Vec::with_capacity(frames * usize::from(CHANNELS));
        let mut parts = vec![Vec::new(); self.stems.len().saturating_sub(1)];
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
            for (part, m) in parts.iter_mut().zip(self.stems.iter_mut().skip(1)) {
                if let Some(m) = m {
                    part.extend(m.render(n));
                }
            }
            self.clock = next;
        }
        let mut all = Vec::with_capacity(parts.len() + 1);
        all.push(out);
        all.extend(parts);
        all
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

    fn stems(&mut self, selectors: &[Vec<String>]) -> Result<Vec<Vec<bool>>, String> {
        Score::stems(self, selectors)
    }

    fn render_stems(&mut self, frames: usize) -> Vec<Vec<f32>> {
        Score::render_stems(self, frames)
    }
}
