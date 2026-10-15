use crate::score::{Play, Sounds};
use crate::script::{Kinds, Script};
use std::collections::VecDeque;

const STEREO: usize = 2;
const CEILING: f32 = 0.89;
const OPEN: f32 = 0.98;
const STILL: f64 = 0.02;
const KNEE: f32 = 0.72;
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
    pub track: Vec<f32>,
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
                        gain: sound.gain,
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
            track: Vec::new(),
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
        let score = self.score.render(frames);
        self.track
            .extend(game.iter().zip(&score).map(|(g, s)| g + s));
        Ok(())
    }

    pub fn master(mut self) -> Vec<f32> {
        for s in &mut self.track {
            let a = s.abs();
            if a > KNEE {
                let over = (a - KNEE) / (1.0 - KNEE);
                *s = s.signum() * (KNEE + (1.0 - KNEE) * over.tanh());
            }
        }
        let peak = self.track.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        if peak > CEILING {
            let k = CEILING / peak;
            for s in &mut self.track {
                *s *= k;
            }
        }
        self.track
    }
}
