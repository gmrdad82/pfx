use super::dsp::{
    Noise, Pink, Svf, Wander, decay_per_sample, exp_seconds, fract, sin_turns, smooth_step,
};
use super::layers::Voice;
use super::{Ctx, Knob, RATE, Tyre};

pub(crate) struct Whine {
    phase: f32,
    white: Noise,
    ring: Svf,
    wobble: Wander,
    flutter: Wander,
}

impl Whine {
    pub(crate) fn new(seeds: &mut Noise) -> Self {
        let white = Noise::new(seeds.seed());
        let wobble = Wander::new(seeds.seed(), 6.0);
        let flutter = Wander::new(seeds.seed(), 28.0);
        Self {
            phase: 0.0,
            white,
            ring: Svf::new(1_400.0, 14.0),
            wobble,
            flutter,
        }
    }

    pub(crate) fn tune(&mut self, base: f32) {
        self.ring.tune(base, 14.0);
    }

    pub(crate) fn tick(&mut self, base: f32, gate: f32) -> f32 {
        let hz = base * (1.0 + 0.02 * self.wobble.tick());
        self.phase += hz / RATE as f32;
        self.phase = fract(self.phase);
        let tone = sin_turns(self.phase) + 0.3 * sin_turns(self.phase * 2.0);
        let ring = self.ring.band(self.white.white()) * 0.6;
        let flutter = 0.75 + 0.25 * self.flutter.tick();
        gate * flutter * 0.5 * (tone * 0.6 + ring)
    }
}

const SNAP_HZ: f32 = 2_600.0;
const CHIRP_ATTACK: f32 = 0.002;
const QUIET: f32 = 1e-4;

pub(crate) struct TyreVoice {
    slip: Knob,
    load: Knob,
    grip: f32,
    threshold: f32,
    range: f32,
    hz: f32,
    rise: f32,
    scrub_level: f32,
    chirp_level: f32,
    squeal_level: f32,
    pink: Pink,
    white: Noise,
    bright: Svf,
    floor: Svf,
    grain: Wander,
    gate: f32,
    whine: Whine,
    sliding: Option<bool>,
    last_slip: f32,
    events: Noise,
    snap_band: Svf,
    chirp: Option<Chirp>,
}

struct Chirp {
    level: f32,
    age: f32,
    decay: f32,
    glide: f32,
    glide_decay: f32,
    hz: f32,
    phase: f32,
    snap: f32,
    snap_decay: f32,
}

impl TyreVoice {
    pub(crate) fn new(tyre: &Tyre, seeds: &mut Noise, ctx: &Ctx) -> Self {
        let pink = Pink::new(seeds.seed());
        let white = Noise::new(seeds.seed());
        let grain = Wander::new(seeds.seed(), 60.0);
        let whine = Whine::new(seeds);
        let events = Noise::new(seeds.seed());
        Self {
            slip: ctx.knob(&tyre.slip),
            load: ctx.knob(&tyre.load),
            grip: tyre.grip,
            threshold: tyre.threshold,
            range: tyre.range,
            hz: tyre.hz,
            rise: tyre.rise,
            scrub_level: tyre.scrub,
            chirp_level: tyre.chirp,
            squeal_level: tyre.squeal,
            pink,
            white,
            bright: Svf::new(900.0, 0.7),
            floor: Svf::new(120.0, 0.7),
            grain,
            gate: 0.0,
            whine,
            sliding: None,
            last_slip: 0.0,
            events,
            snap_band: Svf::new(SNAP_HZ, 2.0),
            chirp: None,
        }
    }

    fn onset(&mut self, slip: f32, dt: f32) -> Option<f32> {
        let rate = (slip - self.last_slip) / dt;
        self.last_slip = slip;
        let Some(sliding) = self.sliding else {
            self.sliding = Some(slip >= self.grip);
            return None;
        };
        if !sliding && slip >= self.grip {
            self.sliding = Some(true);
            return Some(0.5 + 0.5 * smooth_step(rate / 10.0));
        }
        if sliding && slip < self.grip * 0.5 {
            self.sliding = Some(false);
        }
        None
    }

    fn start_chirp(&mut self, strength: f32, load: f32) {
        let length = self.events.range(0.04, 0.08);
        let hz = self.hz * self.events.range(0.94, 1.06) * pitch_by_load(load);
        self.chirp = Some(Chirp {
            level: self.chirp_level * strength * level_by_load(load),
            age: 0.0,
            decay: decay_per_sample(length),
            glide: 0.5,
            glide_decay: decay_per_sample(length * 0.4),
            hz,
            phase: 0.0,
            snap: self.events.range(0.6, 1.0),
            snap_decay: decay_per_sample(0.008),
        });
    }
}

fn level_by_load(load: f32) -> f32 {
    pfx_core::sim::sqrtf(load)
}

fn pitch_by_load(load: f32) -> f32 {
    (1.0 + 0.15 * (load - 1.0)).clamp(0.8, 1.3)
}

impl Voice for TyreVoice {
    fn block(&mut self, ctx: &Ctx, t: f32, out: &mut [[f32; 2]]) {
        let slip = ctx.get(self.slip, t).max(0.0);
        let load = ctx.get(self.load, t).clamp(0.0, 4.0);
        let dt = out.len() as f32 / RATE as f32;
        if let Some(strength) = self.onset(slip, dt)
            && self.chirp_level > 0.0
        {
            self.start_chirp(strength, load);
        }
        let by_load = level_by_load(load);
        let scrub = self.scrub_level * pfx_core::sim::powf((slip / 10.0).min(4.0), 0.8) * by_load;
        if scrub > 0.0 {
            self.bright.tune((500.0 + 400.0 * slip).min(14_000.0), 0.7);
            for frame in out.iter_mut() {
                let x = self.pink.tick() * 1.4 + self.white.white() * 0.35;
                let y = self.floor.high(self.bright.low(x));
                let grain = 0.7 + 0.3 * self.grain.tick();
                frame[0] += y * grain * scrub * 0.5;
            }
        }
        let target = if slip > self.threshold {
            smooth_step((slip - self.threshold) / self.range)
        } else {
            0.0
        };
        let time = if target > self.gate { 0.04 } else { 0.12 };
        self.gate += (target - self.gate) * (1.0 - exp_seconds(-dt / time));
        if self.gate < QUIET && target == 0.0 {
            self.gate = 0.0;
        }
        if self.gate > 0.0 && self.squeal_level > 0.0 {
            let over = (slip - self.threshold).max(0.0);
            let base = (self.hz + self.rise * over) * pitch_by_load(load);
            let gate = self.gate * self.squeal_level * by_load;
            self.whine.tune(base);
            for frame in out.iter_mut() {
                frame[0] += self.whine.tick(base, gate);
            }
        }
        if let Some(chirp) = &mut self.chirp {
            let attack = CHIRP_ATTACK * RATE as f32;
            for frame in out.iter_mut() {
                let hz = chirp.hz * (1.0 + chirp.glide);
                chirp.phase = fract(chirp.phase + hz / RATE as f32);
                let tone = sin_turns(chirp.phase) + 0.25 * sin_turns(chirp.phase * 2.0);
                let rise = smooth_step(chirp.age / attack);
                let snap = self.snap_band.band(self.events.white()) * chirp.snap * 2.0;
                frame[0] += chirp.level * (tone * 0.6 + snap) * rise;
                chirp.age += 1.0;
                chirp.level *= chirp.decay;
                chirp.glide *= chirp.glide_decay;
                chirp.snap *= chirp.snap_decay;
            }
            if chirp.level < QUIET {
                self.chirp = None;
            }
        }
    }
}
