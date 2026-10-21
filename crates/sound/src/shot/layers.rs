use super::ambience::City;
use super::dsp::{Noise, OnePole, Pink, Svf, Wander, decay_per_sample, pan, smooth_step};
use super::engine::Model;
use super::recording::{Bank, Range};
use super::surface::{self, Cobbles, Gravel};
use super::tyre::{TyreVoice, Whine};
use super::{CONTROL, Common, Ctx, Kind, Knob, RATE, Surface};
use crate::DriveCurve;
use crate::drive::shape;
use crate::filter::{Filter, Kind as FilterKind};

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
        Kind::Road(r) => {
            let pink = Pink::new(seeds.seed());
            let white = Noise::new(seeds.seed());
            let uses = |s: Surface| r.surface.iter().any(|k| k.1 == s);
            let surfaces = if r.surface.iter().all(|k| k.1 == Surface::Asphalt) {
                Vec::new()
            } else {
                r.surface.clone()
            };
            let gravel = uses(Surface::Gravel).then(|| Gravel::new(seeds.seed()));
            let cobbles = uses(Surface::Cobbles).then(|| Cobbles::new(seeds.seed(), r.spacing));
            let wet = uses(Surface::Wet).then(|| Hiss::new(&mut seeds));
            Box::new(RoadVoice {
                speed: ctx.knob(&r.speed),
                roughness: r.roughness,
                pink,
                white,
                brown: OnePole::new(60.0),
                body: Svf::new(400.0, 0.6),
                hiss: Svf::new(2_000.0, 0.9),
                surfaces,
                load: ctx.knob(&r.load),
                gravel,
                cobbles,
                wet,
                scratch: [0.0; CONTROL],
            })
        }
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
            whine: Whine::new(&mut seeds),
        }),
        Kind::Tyre(t) => Box::new(TyreVoice::new(t, &mut seeds, ctx)),
        Kind::Spray(s) => Box::new(SprayVoice {
            speed: ctx.knob(&s.speed),
            wet: ctx.knob(&s.wet),
            hiss: Hiss::new(&mut seeds),
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
    let mut passes = Passes::new(common, ctx, frame_to_seconds(at));
    while at < last {
        let len = CONTROL.min(last - at);
        let t = frame_to_seconds(at);
        let g1 = level(frame_to_seconds(at + len));
        let block = &mut scratch[..len];
        block.fill([0.0, 0.0]);
        voice.block(ctx, t, block);
        passes.apply(ctx, frame_to_seconds(at + len), block, stereo);
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

struct Passes {
    filters: Vec<(Filter, Knob)>,
    buf: [f32; CONTROL * 2],
}

impl Passes {
    fn new(common: &Common, ctx: &Ctx, t: f32) -> Self {
        let filters = [
            (FilterKind::HighPass, &common.highpass),
            (FilterKind::LowPass, &common.lowpass),
        ]
        .into_iter()
        .filter_map(|(kind, pass)| {
            let pass = pass.as_ref()?;
            let knob = ctx.knob(&pass.hz);
            Some((Filter::starting(kind, ctx.get(knob, t), pass.q, RATE), knob))
        })
        .collect();
        Self {
            filters,
            buf: [0.0; CONTROL * 2],
        }
    }

    fn apply(&mut self, ctx: &Ctx, end: f32, block: &mut [[f32; 2]], stereo: bool) {
        if self.filters.is_empty() {
            return;
        }
        let channels = if stereo { 2 } else { 1 };
        let len = block.len();
        let buf = &mut self.buf[..len * channels];
        for (i, frame) in block.iter().enumerate() {
            buf[i * channels..(i + 1) * channels].copy_from_slice(&frame[..channels]);
        }
        for (filter, knob) in &mut self.filters {
            filter.set(Some(ctx.get(*knob, end)), len as u32, RATE);
            filter.process(buf, len, channels, RATE);
        }
        for (i, frame) in block.iter_mut().enumerate() {
            frame[..channels].copy_from_slice(&buf[i * channels..(i + 1) * channels]);
        }
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
    surfaces: Vec<(f32, Surface)>,
    load: Knob,
    gravel: Option<Gravel>,
    cobbles: Option<Cobbles>,
    wet: Option<Hiss>,
    scratch: [f32; CONTROL],
}

const ASPHALT_UNDER: [f32; 4] = [1.0, 0.4, 1.0, 0.7];

impl Voice for RoadVoice {
    fn block(&mut self, ctx: &Ctx, t: f32, out: &mut [[f32; 2]]) {
        let speed = ctx.get(self.speed, t).max(0.0);
        let level = pfx_core::sim::powf((speed / 25.0).min(3.0), 1.3);
        self.body.tune(180.0 + 28.0 * speed, 0.6);
        self.hiss.tune(1_500.0 + 45.0 * speed, 0.9);
        let hiss = 0.25 * self.roughness;
        if self.surfaces.is_empty() {
            for frame in out.iter_mut() {
                let w = self.white.white();
                let rumble = self.brown.tick(w) * 8.0;
                let y = self.body.low(self.pink.tick()) * 1.6 + rumble + self.hiss.band(w) * hiss;
                frame[0] = y * level * 0.45;
            }
            return;
        }
        let weights = surface::weights(&self.surfaces, t);
        let under: f32 = weights.iter().zip(ASPHALT_UNDER).map(|(w, u)| w * u).sum();
        for frame in out.iter_mut() {
            let w = self.white.white();
            let rumble = self.brown.tick(w) * 8.0;
            let y = self.body.low(self.pink.tick()) * 1.6 + rumble + self.hiss.band(w) * hiss;
            frame[0] = y * level * 0.45 * under;
        }
        let scratch = &mut self.scratch[..out.len()];
        let gravel_w = weights[Surface::Gravel as usize];
        if let Some(gravel) = &mut self.gravel {
            let load = ctx.get(self.load, t);
            gravel.block(speed, load, scratch);
            for (frame, y) in out.iter_mut().zip(scratch.iter()) {
                frame[0] += y * gravel_w;
            }
        }
        let cobbles_w = weights[Surface::Cobbles as usize];
        if let Some(cobbles) = &mut self.cobbles {
            cobbles.block(speed, scratch);
            for (frame, y) in out.iter_mut().zip(scratch.iter()) {
                frame[0] += y * cobbles_w;
            }
        }
        let wet_w = weights[Surface::Wet as usize];
        if let Some(wet) = &mut self.wet {
            let level = Hiss::level(speed, 1.0) * 0.8 * wet_w;
            wet.tune(speed);
            for frame in out.iter_mut() {
                frame[0] += wet.tick() * level;
            }
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
    whine: Whine,
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
        self.whine.tune(base);
        for frame in out.iter_mut() {
            frame[0] = self.whine.tick(base, gate);
        }
    }
}

struct Hiss {
    white: Noise,
    high: Svf,
    low: Svf,
    flutter: Wander,
}

impl Hiss {
    fn new(seeds: &mut Noise) -> Self {
        let white = Noise::new(seeds.seed());
        Self {
            white,
            high: Svf::new(1_800.0, 0.7),
            low: Svf::new(6_000.0, 0.7),
            flutter: Wander::new(seeds.seed(), 18.0),
        }
    }

    fn level(speed: f32, wet: f32) -> f32 {
        wet * pfx_core::sim::powf((speed / 25.0).min(2.0), 1.2)
    }

    fn tune(&mut self, speed: f32) {
        self.low.tune(5_000.0 + 120.0 * speed, 0.7);
    }

    fn tick(&mut self) -> f32 {
        let y = self.low.low(self.high.high(self.white.white()));
        let flutter = 0.7 + 0.3 * self.flutter.tick();
        y * flutter
    }
}

struct SprayVoice {
    speed: Knob,
    wet: Knob,
    hiss: Hiss,
}

impl Voice for SprayVoice {
    fn block(&mut self, ctx: &Ctx, t: f32, out: &mut [[f32; 2]]) {
        let speed = ctx.get(self.speed, t).max(0.0);
        let wet = ctx.get(self.wet, t).clamp(0.0, 1.0);
        let level = Hiss::level(speed, wet);
        self.hiss.tune(speed);
        for frame in out.iter_mut() {
            frame[0] = self.hiss.tick() * level * 0.8;
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
