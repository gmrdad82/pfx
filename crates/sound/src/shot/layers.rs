use super::ambience::City;
use super::dsp::{
    Noise, OnePole, Pink, Svf, Wander, decay_per_sample, fract, pan, sin_turns, smooth_step,
};
use super::engine::Model;
use super::recording::{Bank, Range};
use super::{CONTROL, Common, Ctx, Kind, Knob, RATE};
use crate::DriveCurve;
use crate::drive::shape;

const ENGINE_LEVEL: f32 = 4.0;
const WARMUP: usize = RATE as usize / 2;
const SPEED_OF_SOUND: f32 = 343.0;

pub(crate) trait Voice {
    fn stereo(&self) -> bool {
        false
    }

    fn block(&mut self, ctx: &Ctx, t: f32, out: &mut [[f32; 2]]);
}

pub(crate) fn voice(
    kind: &Kind,
    common: &Common,
    seed: u64,
    ctx: &Ctx,
) -> Result<Box<dyn Voice>, String> {
    let mut seeds = Noise::new(seed);
    Ok(match kind {
        Kind::Engine(e) => {
            let model = Model::new(e.cylinders.unwrap_or(8) as usize, seeds.seed());
            let pops = Noise::new(seeds.seed());
            let rpm = ctx.knob(&e.rpm);
            let bank = match &e.sample {
                Some(spec) => Some(Bank::load(
                    spec,
                    seeds.seed(),
                    &rpm_range(rpm, common, ctx),
                    e.cylinders.is_some(),
                )?),
                None => None,
            };
            Box::new(EngineVoice {
                model: e.cylinders.map(|_| model),
                bank,
                blend: ctx.knob(&e.blend),
                at: None,
                rpm,
                load: ctx.knob(&e.load),
                drive_db: e.drive,
                lowpass: e.lowpass,
                crackle: e.crackle,
                filter: Svf::new(e.lowpass, 0.707),
                floor: Svf::new(25.0, 0.707),
                last_load: None,
                lift: 0.0,
                pops,
                pop_level: 0.0,
                pop_decay: 0.0,
                pop_filter: Svf::new(1_200.0, 1.2),
            })
        }
        Kind::Road(r) => Box::new(RoadVoice {
            speed: ctx.knob(&r.speed),
            roughness: r.roughness,
            pink: Pink::new(seeds.seed()),
            white: Noise::new(seeds.seed()),
            brown: OnePole::new(60.0),
            body: Svf::new(400.0, 0.6),
            hiss: Svf::new(2_000.0, 0.9),
        }),
        Kind::Squeal(s) => Box::new(SquealVoice {
            at: s.at.clone(),
            curve: s
                .curve
                .as_ref()
                .map(|c| ctx.knob(&super::Value::Curve(c.clone()))),
            threshold: s.threshold,
            range: s.range,
            hz: ctx.knob(&s.hz),
            gate: 0.0,
            phase: 0.0,
            white: Noise::new(seeds.seed()),
            ring: Svf::new(1_400.0, 14.0),
            wobble: Wander::new(seeds.seed(), 6.0),
            flutter: Wander::new(seeds.seed(), 28.0),
        }),
        Kind::Spray(s) => Box::new(SprayVoice {
            speed: ctx.knob(&s.speed),
            wet: ctx.knob(&s.wet),
            white: Noise::new(seeds.seed()),
            high: Svf::new(1_800.0, 0.7),
            low: Svf::new(6_000.0, 0.7),
            flutter: Wander::new(seeds.seed(), 18.0),
        }),
        Kind::Whoosh(w) => Box::new(WhooshVoice {
            passes: w
                .at
                .iter()
                .map(|&at| Pass {
                    at,
                    pink: Pink::new(seeds.seed()),
                    band: Svf::new(w.hz, 0.9),
                    low: Svf::new(350.0, 0.7),
                })
                .collect(),
            speed: w.speed,
            distance: w.distance,
            hz: w.hz,
            sweep: w.sweep,
            length: w.length,
        }),
        Kind::Ambience(a) => Box::new(City::new(a, seeds.seed(), ctx.seconds)),
    })
}

fn rpm_range(rpm: Knob, common: &Common, ctx: &Ctx) -> Range {
    let start = common.start.min(ctx.seconds);
    let end = common
        .end
        .unwrap_or(ctx.seconds)
        .min(ctx.seconds)
        .max(start);
    let steps = ((end - start) * 100.0).ceil().max(1.0) as usize;
    let (mut low, mut high) = (f32::MAX, 0.0f32);
    for i in 0..=steps {
        let v = ctx
            .get(rpm, start + (end - start) * i as f32 / steps as f32)
            .max(1.0);
        low = low.min(v);
        high = high.max(v);
    }
    Range {
        low,
        high,
        start,
        end,
    }
}

pub(crate) fn run(mut voice: Box<dyn Voice>, common: &Common, ctx: &Ctx, out: &mut [f32]) {
    let frames = out.len() / 2;
    let first = seconds_to_frame(common.start).min(frames);
    let last = common
        .end
        .map_or(frames, |end| seconds_to_frame(end).min(frames));
    if first >= last {
        return;
    }
    let gain = ctx.knob(&common.gain);
    let pan_knob = ctx.knob(&common.pan);
    let stereo = voice.stereo();
    let window = |t: f32| {
        let mut w = 1.0;
        if common.fade_in > 0.0 {
            w *= smooth_step((t - common.start) / common.fade_in);
        }
        if common.fade_out > 0.0 {
            let end = common.end.unwrap_or(ctx.seconds);
            w *= smooth_step((end - t) / common.fade_out);
        }
        w
    };
    let level = |t: f32| ctx.get(gain, t) * window(t);
    let mut scratch = [[0.0f32; 2]; CONTROL];
    let mut at = first;
    let mut g0 = level(frame_to_seconds(at));
    while at < last {
        let len = CONTROL.min(last - at);
        let t = frame_to_seconds(at);
        let g1 = level(frame_to_seconds(at + len));
        let block = &mut scratch[..len];
        block.fill([0.0, 0.0]);
        voice.block(ctx, t, block);
        let p = ctx.get(pan_knob, t).clamp(-1.0, 1.0);
        let (pl, pr) = if stereo {
            ((1.0 - p).min(1.0), (1.0 + p).min(1.0))
        } else {
            pan(p)
        };
        let step = (g1 - g0) / len as f32;
        for (i, frame) in block.iter().enumerate() {
            let g = g0 + step * i as f32;
            let (l, r) = if stereo {
                (frame[0], frame[1])
            } else {
                (frame[0], frame[0])
            };
            let dst = &mut out[(at + i) * 2..(at + i) * 2 + 2];
            dst[0] += l * pl * g;
            dst[1] += r * pr * g;
        }
        g0 = g1;
        at += len;
    }
}

fn seconds_to_frame(seconds: f32) -> usize {
    (f64::from(seconds.max(0.0)) * f64::from(RATE)).round() as usize
}

fn frame_to_seconds(frame: usize) -> f32 {
    (frame as f64 / f64::from(RATE)) as f32
}

struct EngineVoice {
    model: Option<Model>,
    bank: Option<Bank>,
    blend: Knob,
    at: Option<u64>,
    rpm: Knob,
    load: Knob,
    drive_db: f32,
    lowpass: f32,
    crackle: f32,
    filter: Svf,
    floor: Svf,
    last_load: Option<f32>,
    lift: f32,
    pops: Noise,
    pop_level: f32,
    pop_decay: f32,
    pop_filter: Svf,
}

impl Voice for EngineVoice {
    fn block(&mut self, ctx: &Ctx, t: f32, out: &mut [[f32; 2]]) {
        let rpm = ctx.get(self.rpm, t).max(0.0);
        let load = ctx.get(self.load, t).clamp(0.0, 1.0);
        let dt = out.len() as f32 / RATE as f32;
        if self.last_load.is_none()
            && let Some(model) = &mut self.model
        {
            for _ in 0..WARMUP {
                let x = model.tick(rpm, load);
                self.floor.high(x);
            }
        }
        let at = *self.at.get_or_insert(seconds_to_frame(t) as u64);
        self.at = Some(at + out.len() as u64);
        let blend = ctx.get(self.blend, t);
        let mix = self.bank.as_ref().map(|bank| {
            let share = if self.model.is_some() {
                bank.share(blend, rpm)
            } else {
                0.0
            };
            (
                bank.weights(rpm),
                pfx_core::sim::sqrtf(share),
                pfx_core::sim::sqrtf(1.0 - share),
            )
        });
        let drop = (self.last_load.unwrap_or(load) - load).max(0.0) / dt;
        self.last_load = Some(load);
        let keep = super::dsp::exp_seconds(-dt / 0.4);
        self.lift = (self.lift * keep).max((drop / 3.0).min(1.0));
        let revving = ((rpm - 2_500.0) / 3_500.0).clamp(0.0, 1.0);
        let overrun = (1.0 - load) * (1.0 - load) * revving;
        let pop_chance = self.crackle * (self.lift + 0.35 * overrun) * 60.0 / RATE as f32;
        self.filter.tune(
            self.lowpass * (0.3 + 0.7 * load),
            std::f32::consts::FRAC_1_SQRT_2,
        );
        let drive = pfx_core::sim::powf(10.0, self.drive_db * (0.4 + 0.6 * load) / 20.0);
        let trim = 1.0 / pfx_core::sim::sqrtf(drive);
        for (i, frame) in out.iter_mut().enumerate() {
            let synth = match &mut self.model {
                Some(model) => self.floor.high(model.tick(rpm, load)) * ENGINE_LEVEL,
                None => 0.0,
            };
            let x = match (&mut self.bank, &mix) {
                (Some(bank), Some((weights, of_synth, of_sample))) => {
                    let tick = bank.tick(rpm, weights, at + i as u64);
                    tick.run * (synth * of_synth + tick.sample * of_sample) + tick.shots
                }
                _ => synth,
            };
            let mut y = self.filter.low(shape(DriveCurve::Tanh, x * drive) * trim);
            if pop_chance > 0.0 {
                if self.pops.unit() < pop_chance {
                    self.pop_level = self.pops.range(0.4, 1.0);
                    self.pop_decay = decay_per_sample(self.pops.range(0.003, 0.012));
                    self.pop_filter.tune(self.pops.range(700.0, 2_500.0), 1.2);
                }
                if self.pop_level > 1e-4 {
                    let n = self.pops.white();
                    y += self.pop_level * (self.pop_filter.band(n) + n * 0.15);
                    self.pop_level *= self.pop_decay;
                }
            }
            frame[0] = y;
        }
    }
}

struct RoadVoice {
    speed: Knob,
    roughness: f32,
    pink: Pink,
    white: Noise,
    brown: OnePole,
    body: Svf,
    hiss: Svf,
}

impl Voice for RoadVoice {
    fn block(&mut self, ctx: &Ctx, t: f32, out: &mut [[f32; 2]]) {
        let speed = ctx.get(self.speed, t).max(0.0);
        let level = pfx_core::sim::powf((speed / 25.0).min(3.0), 1.3);
        self.body.tune(180.0 + 28.0 * speed, 0.6);
        self.hiss.tune(1_500.0 + 45.0 * speed, 0.9);
        let hiss = 0.25 * self.roughness;
        for frame in out.iter_mut() {
            let w = self.white.white();
            let rumble = self.brown.tick(w) * 8.0;
            let y = self.body.low(self.pink.tick()) * 1.6 + rumble + self.hiss.band(w) * hiss;
            frame[0] = y * level * 0.45;
        }
    }
}

struct SquealVoice {
    at: Vec<[f32; 2]>,
    curve: Option<Knob>,
    threshold: f32,
    range: f32,
    hz: Knob,
    gate: f32,
    phase: f32,
    white: Noise,
    ring: Svf,
    wobble: Wander,
    flutter: Wander,
}

impl Voice for SquealVoice {
    fn block(&mut self, ctx: &Ctx, t: f32, out: &mut [[f32; 2]]) {
        let mut target: f32 = if self.at.iter().any(|w| t >= w[0] && t < w[1]) {
            1.0
        } else {
            0.0
        };
        if let Some(curve) = self.curve {
            let v = ctx.get(curve, t);
            target = target.max(((v - self.threshold) / self.range).clamp(0.0, 1.0));
        }
        let dt = out.len() as f32 / RATE as f32;
        let time = if target > self.gate { 0.04 } else { 0.12 };
        let k = 1.0 - super::dsp::exp_seconds(-dt / time);
        self.gate += (target - self.gate) * k;
        if self.gate < 1e-4 && target == 0.0 {
            self.gate = 0.0;
            return;
        }
        let gate = self.gate;
        let base = ctx.get(self.hz, t).max(20.0) * (0.92 + 0.12 * gate);
        self.ring.tune(base, 14.0);
        for frame in out.iter_mut() {
            let hz = base * (1.0 + 0.02 * self.wobble.tick());
            self.phase += hz / RATE as f32;
            self.phase = fract(self.phase);
            let tone = sin_turns(self.phase) + 0.3 * sin_turns(self.phase * 2.0);
            let ring = self.ring.band(self.white.white()) * 0.6;
            let flutter = 0.75 + 0.25 * self.flutter.tick();
            frame[0] = gate * flutter * 0.5 * (tone * 0.6 + ring);
        }
    }
}

struct SprayVoice {
    speed: Knob,
    wet: Knob,
    white: Noise,
    high: Svf,
    low: Svf,
    flutter: Wander,
}

impl Voice for SprayVoice {
    fn block(&mut self, ctx: &Ctx, t: f32, out: &mut [[f32; 2]]) {
        let speed = ctx.get(self.speed, t).max(0.0);
        let wet = ctx.get(self.wet, t).clamp(0.0, 1.0);
        let level = wet * pfx_core::sim::powf((speed / 25.0).min(2.0), 1.2);
        self.low.tune(5_000.0 + 120.0 * speed, 0.7);
        for frame in out.iter_mut() {
            let y = self.low.low(self.high.high(self.white.white()));
            let flutter = 0.7 + 0.3 * self.flutter.tick();
            frame[0] = y * flutter * level * 0.8;
        }
    }
}

struct Pass {
    at: f32,
    pink: Pink,
    band: Svf,
    low: Svf,
}

struct WhooshVoice {
    passes: Vec<Pass>,
    speed: f32,
    distance: f32,
    hz: f32,
    sweep: [f32; 2],
    length: f32,
}

impl Voice for WhooshVoice {
    fn stereo(&self) -> bool {
        true
    }

    fn block(&mut self, _ctx: &Ctx, t: f32, out: &mut [[f32; 2]]) {
        let half = self.length * 0.5;
        for pass in &mut self.passes {
            let since = t - pass.at;
            if since.abs() > half {
                continue;
            }
            let x = self.speed * since;
            let r = pfx_core::sim::sqrtf(x * x + self.distance * self.distance);
            let toward = self.speed * x / r;
            let doppler = SPEED_OF_SOUND / (SPEED_OF_SOUND + toward);
            let edge = smooth_step((half - since.abs()) / (self.length * 0.15));
            let amp = self.distance / r * edge;
            let side = 0.5 + 0.5 * x / r;
            let (pl, pr) = pan(self.sweep[0] + (self.sweep[1] - self.sweep[0]) * side);
            pass.band.tune(self.hz * doppler, 0.9);
            pass.low.tune(350.0 * doppler, 0.7);
            for frame in out.iter_mut() {
                let n = pass.pink.tick();
                let y = amp * (pass.band.band(n) * 3.0 + pass.low.low(n) * 1.6);
                frame[0] += y * pl;
                frame[1] += y * pr;
            }
        }
    }
}
