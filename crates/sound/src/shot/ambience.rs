use super::dsp::{Noise, OnePole, Pink, Svf, Wander, fract, pan, sin_turns};
use super::layers::Voice;
use super::{Ambience, Ctx, RATE};
use crate::synth::{Synth, Wave};

#[derive(Clone, Copy)]
enum Call {
    Horn { hz: f32 },
    Chirp { hz: f32, sweep: f32 },
}

#[derive(Clone, Copy)]
struct Note {
    start: usize,
    len: usize,
    gain: f32,
    pan: f32,
    call: Call,
}

struct Playing {
    note: Note,
    at: usize,
    low: Synth,
    high: Synth,
    filter: Svf,
    phase: f32,
}

pub(crate) struct City {
    bed: f32,
    rumble: f32,
    pink: [Pink; 2],
    bed_low: [Svf; 2],
    air: [Svf; 2],
    white: Noise,
    brown: [OnePole; 2],
    rumble_low: [Svf; 2],
    swell: Wander,
    notes: Vec<Note>,
    next: usize,
    playing: Vec<Playing>,
    clock: usize,
}

impl City {
    pub(crate) fn new(a: &Ambience, seed: u64, seconds: f32) -> Self {
        let mut seeds = Noise::new(seed);
        let mut events = Noise::new(seeds.seed());
        let mut notes = Vec::new();
        for t in arrivals(&mut events, a.horns / 60.0, seconds) {
            let hz = events.range(340.0, 470.0);
            let gain = events.range(0.25, 0.6);
            let pan = events.range(-0.9, 0.9);
            let len = events.range(0.18, 0.6);
            notes.push(note(t, len, gain, pan, Call::Horn { hz }));
            if events.unit() < 0.35 {
                notes.push(note(
                    t + len + 0.08,
                    len * 0.6,
                    gain,
                    pan,
                    Call::Horn { hz },
                ));
            }
        }
        for t in arrivals(&mut events, a.birds / 60.0, seconds) {
            let count = 2 + (events.unit() * 3.99) as usize;
            let hz = events.range(2_600.0, 4_800.0);
            let gain = events.range(0.1, 0.3);
            let pan = events.range(-0.9, 0.9);
            let mut at = t;
            for _ in 0..count {
                let len = events.range(0.04, 0.12);
                let sweep = events.range(-0.45, 0.35);
                let hz = hz * events.range(0.92, 1.08);
                notes.push(note(at, len, gain, pan, Call::Chirp { hz, sweep }));
                at += len + events.range(0.05, 0.16);
            }
        }
        notes.sort_by_key(|n| n.start);
        Self {
            bed: a.bed,
            rumble: a.rumble,
            pink: [Pink::new(seeds.seed()), Pink::new(seeds.seed())],
            bed_low: [Svf::new(700.0, 0.6), Svf::new(700.0, 0.6)],
            air: [Svf::new(2_400.0, 0.5), Svf::new(2_400.0, 0.5)],
            white: Noise::new(seeds.seed()),
            brown: [OnePole::new(50.0), OnePole::new(50.0)],
            rumble_low: [Svf::new(110.0, 0.7), Svf::new(110.0, 0.7)],
            swell: Wander::new(seeds.seed(), 0.15),
            notes,
            next: 0,
            playing: Vec::new(),
            clock: 0,
        }
    }
}

fn note(t: f32, len: f32, gain: f32, pan: f32, call: Call) -> Note {
    Note {
        start: (t * RATE as f32) as usize,
        len: ((len * RATE as f32) as usize).max(1),
        gain,
        pan,
        call,
    }
}

fn arrivals(rng: &mut Noise, per_second: f32, seconds: f32) -> Vec<f32> {
    let mut out = Vec::new();
    if per_second <= 0.0 {
        return out;
    }
    let mut t = 0.0;
    loop {
        let u = rng.unit().max(1e-6);
        t += -pfx_core::sim::lnf(u) / per_second;
        if t >= seconds {
            return out;
        }
        out.push(t);
    }
}

impl Voice for City {
    fn stereo(&self) -> bool {
        true
    }

    fn block(&mut self, _ctx: &Ctx, t: f32, out: &mut [[f32; 2]]) {
        let start = (f64::from(t) * f64::from(RATE)).round() as usize;
        if start != self.clock {
            self.clock = start;
            self.next = self.notes.partition_point(|n| n.start < start);
            self.playing.clear();
        }
        for frame in out.iter_mut() {
            while self.next < self.notes.len() && self.notes[self.next].start <= self.clock {
                let note = self.notes[self.next];
                let (low, high, filter) = match note.call {
                    Call::Horn { hz } => (
                        Synth::new(Wave::Square, hz, 0),
                        Synth::new(Wave::Saw, hz * 1.26, 0),
                        Svf::new(1_400.0, 0.7),
                    ),
                    Call::Chirp { .. } => (
                        Synth::new(Wave::Sine, 0.0, 0),
                        Synth::new(Wave::Sine, 0.0, 0),
                        Svf::new(8_000.0, 0.7),
                    ),
                };
                self.playing.push(Playing {
                    note,
                    at: 0,
                    low,
                    high,
                    filter,
                    phase: 0.0,
                });
                self.next += 1;
            }
            let swell = 0.65 + 0.35 * self.swell.tick();
            let mut l = 0.0;
            let mut r = 0.0;
            for side in 0..2 {
                let p = self.pink[side].tick();
                let bed = self.bed_low[side].low(p) * 1.2 + self.air[side].band(p) * 0.15;
                let w = self.white.white();
                let rumble = self.rumble_low[side].low(self.brown[side].tick(w) * 10.0);
                let y = bed * self.bed * 0.25 + rumble * self.rumble * swell * 0.5;
                if side == 0 { l = y } else { r = y }
            }
            for play in &mut self.playing {
                let n = play.note;
                let p = play.at as f32 / n.len as f32;
                let y = match n.call {
                    Call::Horn { .. } => {
                        let edge =
                            (play.at.min(n.len - play.at) as f32 / (0.01 * RATE as f32)).min(1.0);
                        let raw = play.low.next(1.0, RATE) + play.high.next(1.0, RATE);
                        play.filter.low(raw) * edge * 0.15
                    }
                    Call::Chirp { hz, sweep } => {
                        play.phase += hz * (1.0 + sweep * p) / RATE as f32;
                        play.phase = fract(play.phase);
                        sin_turns(play.phase) * sin_turns(p * 0.5)
                    }
                };
                let (pl, pr) = pan(n.pan);
                l += y * n.gain * pl;
                r += y * n.gain * pr;
                play.at += 1;
            }
            self.playing.retain(|play| play.at < play.note.len);
            frame[0] = l;
            frame[1] = r;
            self.clock += 1;
        }
    }
}
