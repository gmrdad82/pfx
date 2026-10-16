use crate::duck::{self, Duck, Law, Side};
use crate::score::{Play, Sounds};
use crate::script::{Kinds, Script};
use std::collections::VecDeque;

const STEREO: usize = 2;
const OPEN: f32 = 0.98;
const STILL: f64 = 0.02;
const GLIDE: f32 = 0.002;

#[derive(Clone, Debug, PartialEq)]
struct Due {
    at: u64,
    name: String,
    play: Play,
}

pub struct Mixdown<'a> {
    rate: u32,
    fifo: VecDeque<[f32; 2]>,
    read: f64,
    low: [f32; 2],
    level: f32,
    score: &'a mut dyn Sounds,
    due: Vec<Due>,
    duck: Option<(Duck, Vec<(u64, Law)>)>,
    key: Vec<f32>,
    bed: Vec<f32>,
    sides: Vec<Side>,
    split: Option<Split>,
    ceiling: f32,
    limit: f32,
    pub track: Vec<f32>,
}

struct Split {
    masks: Vec<Vec<bool>>,
    stems: Vec<Vec<f32>>,
    game: Vec<f32>,
}

fn named(names: &[String]) -> (bool, Vec<String>) {
    let all = names.iter().any(|n| n == "score");
    (
        all,
        names.iter().filter(|n| *n != "score").cloned().collect(),
    )
}

fn sum(stems: &[Vec<f32>], len: usize, pick: impl Fn(usize) -> bool) -> Vec<f32> {
    let mut out = vec![0.0f32; len];
    for (_, stem) in stems.iter().enumerate().filter(|(i, _)| pick(*i)) {
        for (o, v) in out.iter_mut().zip(stem) {
            *o += v;
        }
    }
    out
}

fn ducking<K: Kinds>(script: &Script<K>, rate: u32) -> Option<(Duck, Vec<(u64, Law)>)> {
    let scened = script.scenes.iter().any(|s| s.duck.is_some());
    let duck = match &script.duck {
        Some(d) => d.clone(),
        None if scened => Duck {
            law: Law {
                depth_db: 0.0,
                ..Law::default()
            },
            ..Duck::default()
        },
        None => return None,
    };
    let mut laws = Vec::with_capacity(script.scenes.len());
    let mut start = 0.0;
    for scene in &script.scenes {
        let law = scene
            .duck
            .as_ref()
            .map_or(duck.law, |part| duck.law.with(part));
        laws.push(((start * f64::from(rate)).round() as u64, law));
        start += scene.seconds;
    }
    Some((duck, laws))
}

fn queue<K: Kinds>(script: &Script<K>, rate: u32) -> Vec<Due> {
    let mut out = Vec::new();
    let mut start = 0.0;
    for scene in &script.scenes {
        for sound in &scene.sound {
            for t in sound.times(scene.seconds) {
                let at = ((start + t) * f64::from(rate)).round() as u64;
                out.push(Due {
                    at,
                    name: sound.sound.clone(),
                    play: Play {
                        gain: if scene.score_db == 0.0 {
                            sound.gain
                        } else {
                            sound.gain * 10f32.powf(scene.score_db / 20.0)
                        },
                        pitch: sound.pitch,
                        seconds: sound.seconds,
                    },
                });
            }
        }
        start += scene.seconds;
    }
    out.sort_by_key(|d| std::cmp::Reverse(d.at));
    out
}

impl<'a> Mixdown<'a> {
    pub fn new<K: Kinds>(script: &Script<K>, rate: u32, score: &'a mut dyn Sounds) -> Mixdown<'a> {
        Mixdown {
            rate,
            fifo: VecDeque::new(),
            read: 0.0,
            low: [0.0; 2],
            level: 0.0,
            score,
            due: queue(script, rate),
            duck: ducking(script, rate),
            key: Vec::new(),
            bed: Vec::new(),
            sides: script.sides.clone(),
            split: None,
            ceiling: script.film.ceiling,
            limit: script.film.limit,
            track: Vec::new(),
        }
    }

    fn selectors(&self) -> Vec<Vec<String>> {
        let mut out = Vec::new();
        for side in &self.sides {
            out.push(named(&side.under).1);
            out.push(side.beds.clone());
        }
        if let Some((d, _)) = &self.duck {
            out.push(named(&d.under).1);
            out.push(d.beds.clone());
        }
        out
    }

    pub fn route(&mut self) -> Result<(), String> {
        if !self.sides.is_empty() {
            let masks = self.score.stems(&self.selectors())?;
            self.split = Some(Split {
                stems: vec![Vec::new(); masks.len()],
                masks,
                game: Vec::new(),
            });
            return Ok(());
        }
        match &self.duck {
            Some((d, _)) => self.score.route(&d.under, &d.beds),
            None => Ok(()),
        }
    }

    pub fn cut(&mut self) {
        self.fifo.clear();
        self.read = 0.0;
    }

    pub fn feed(&mut self, fresh: &[f32]) {
        for pair in fresh.chunks_exact(STEREO) {
            self.fifo.push_back([pair[0], pair[1]]);
        }
    }

    fn game(&mut self, frames: usize, speed: f32, gain: f32) -> Vec<f32> {
        let mut out = Vec::with_capacity(frames * STEREO);
        let speed = f64::from(speed.max(0.0));
        let cutoff = 250.0 + 17_750.0 * (speed as f32).powf(1.6);
        let a = 1.0 - (-std::f32::consts::TAU * cutoff / self.rate as f32).exp();
        let open = speed as f32 >= OPEN;
        for _ in 0..frames {
            let i = self.read.floor() as usize;
            let t = (self.read - i as f64) as f32;
            let s = match (self.fifo.get(i), self.fifo.get(i + 1)) {
                _ if speed < STILL => [0.0; 2],
                (Some(x), Some(y)) => [x[0] + (y[0] - x[0]) * t, x[1] + (y[1] - x[1]) * t],
                (Some(x), None) if t == 0.0 => *x,
                _ => [0.0; 2],
            };
            self.level += (gain - self.level) * GLIDE;
            let available = self.fifo.len() as f64;
            if self.read + speed <= available {
                self.read += speed;
            }
            for (low, v) in self.low.iter_mut().zip(s) {
                *low += a * (v - *low);
                out.push(if open { v } else { *low } * self.level);
            }
        }
        let used = self.read.floor() as usize;
        self.fifo.drain(..used.min(self.fifo.len()));
        self.read -= used as f64;
        out
    }

    pub fn frame(&mut self, frames: usize, speed: f32, gain: f32) -> Result<(), String> {
        let end = self.score.clock() + frames as u64;
        while self.due.last().is_some_and(|d| d.at < end) {
            if let Some(d) = self.due.pop() {
                self.score.cue(&d.name, d.at, d.play)?;
            }
        }
        let game = self.game(frames, speed, gain);
        if let Some(split) = &mut self.split {
            let fresh = self.score.render_stems(frames);
            for (i, stem) in split.stems.iter_mut().enumerate() {
                match fresh.get(i).filter(|f| !f.is_empty()) {
                    Some(f) => stem.extend(f),
                    None => stem.resize(stem.len() + game.len(), 0.0),
                }
            }
            split.game.extend(game);
            return Ok(());
        }
        if self.duck.is_none() {
            let score = self.score.render(frames);
            self.track
                .extend(game.iter().zip(&score).map(|(g, s)| g + s));
            return Ok(());
        }
        let [key, bed, dry] = self.score.render_apart(frames);
        let at = |v: &[f32], i: usize| v.get(i).copied().unwrap_or(0.0);
        for (i, g) in game.iter().enumerate() {
            self.key.push(at(&key, i));
            self.bed.push(g + at(&bed, i));
            self.track.push(at(&dry, i));
        }
        Ok(())
    }

    fn sidechain(&mut self) {
        let Some(mut split) = self.split.take() else {
            return;
        };
        let len = split.game.len();
        for (j, side) in self.sides.iter().enumerate() {
            let all = named(&side.under).0;
            let (under, beds) = (2 * j, 2 * j + 1);
            let masks = &split.masks;
            let key = sum(&split.stems, len, |i| {
                !masks[i][beds] && (masks[i][under] || all)
            });
            let gains = duck::gains(&key, self.rate, &[(0, side.law)]);
            for (i, stem) in split.stems.iter_mut().enumerate() {
                if !masks[i][beds] {
                    continue;
                }
                for (frame, g) in stem.chunks_exact_mut(STEREO).zip(&gains) {
                    frame[0] *= g;
                    frame[1] *= g;
                }
            }
        }
        let Some((d, _)) = &self.duck else {
            self.track = sum(&split.stems, len, |_| true);
            for (t, g) in self.track.iter_mut().zip(&split.game) {
                *t += g;
            }
            return;
        };
        let all = named(&d.under).0;
        let n = 2 * self.sides.len();
        let masks = &split.masks;
        let bedded = |i: usize| masks[i][n + 1];
        let keyed = |i: usize| !bedded(i) && (all || masks[i][n]);
        self.key = sum(&split.stems, len, keyed);
        self.bed = sum(&split.stems, len, bedded);
        for (b, g) in self.bed.iter_mut().zip(&split.game) {
            *b += g;
        }
        self.track = sum(&split.stems, len, |i| !keyed(i) && !bedded(i));
    }

    pub fn master(mut self) -> Vec<f32> {
        self.sidechain();
        if let Some((_, laws)) = &self.duck {
            duck::apply(&mut self.bed, &self.key, self.rate, laws);
            for ((t, b), k) in self.track.iter_mut().zip(&self.bed).zip(&self.key) {
                *t += b + k;
            }
        }
        let knee = self.limit;
        for s in &mut self.track {
            let a = s.abs();
            if a > knee {
                let over = (a - knee) / (1.0 - knee);
                *s = s.signum() * (knee + (1.0 - knee) * over.tanh());
            }
        }
        let peak = self.track.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        if peak > self.ceiling {
            let k = self.ceiling / peak;
            for s in &mut self.track {
                *s *= k;
            }
        }
        self.track
    }
}
