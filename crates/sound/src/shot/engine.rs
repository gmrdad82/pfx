use super::RATE;
use super::dsp::{Noise, OnePole, fract, sin_turns};

const GUIDE_MAX_AMP: f32 = 20.0;

const PIPES: [[f32; 3]; 10] = [
    [0.000_25, 0.002_166_666_6, 0.000_583_333_3],
    [0.000_416_666_68, 0.000_395_833_34, 0.001_875],
    [0.000_458_333_32, 0.000_312_5, 0.000_562_5],
    [0.000_354_166_67, 0.000_208_333_34, 0.001_166_666_6],
    [0.000_25, 0.000_312_5, 0.000_854_166_7],
    [0.000_354_166_67, 0.000_312_5, 0.001_312_5],
    [0.000_333_333_33, 0.000_354_166_67, 0.000_562_5],
    [0.000_458_333_32, 0.000_5, 0.001_458_333_3],
    [0.000_208_333_34, 0.000_541_666_64, 0.000_812_5],
    [0.000_458_333_32, 0.000_666_666_66, 0.001_812_5],
];

const MUFFLERS: [f32; 4] = [
    0.000_270_833_32,
    0.000_25,
    0.000_166_666_66,
    0.000_145_833_33,
];

const INTAKE_VOLUME: f32 = 0.371_133_48;
const EXHAUST_VOLUME: f32 = 0.590_194_76;
const VIBRATION_VOLUME: f32 = 0.038_671_78;
const INTAKE_NOISE: f32 = 0.255_650_25;
const INTAKE_NOISE_HZ: f32 = 1.0 / 0.001_316_329_7;
const VIBRATION_HZ: f32 = 1.0 / 0.000_258_907_3;
const INTAKE_VALVE_SHIFT: f32 = 0.017_913_7;
const EXHAUST_VALVE_SHIFT: f32 = 0.182_139_52;
const INTAKE_OPEN: f32 = 0.012_284_279;
const INTAKE_CLOSED: f32 = 0.804_102_54;
const EXHAUST_OPEN: f32 = -0.011_291_921;
const EXHAUST_CLOSED: f32 = 0.962_894_44;
const PISTON: f32 = 3.947_230_3;
const IGNITION: f32 = 1.443_675_6;
const IGNITION_TIME: f32 = 0.083_376_56;
const VOLUME: f32 = 0.1;
const DC_HZ: f32 = 0.5;

struct Line {
    data: Vec<f32>,
    pos: usize,
}

impl Line {
    fn new(seconds: f32) -> Self {
        let len = ((seconds * RATE as f32) as usize).max(1);
        Self {
            data: vec![0.0; len],
            pos: 0,
        }
    }

    fn pop(&self) -> f32 {
        let next = self.pos + 1;
        self.data[if next == self.data.len() { 0 } else { next }]
    }

    fn push(&mut self, value: f32) {
        self.data[self.pos] = value;
        self.pos += 1;
        if self.pos == self.data.len() {
            self.pos = 0;
        }
    }
}

struct Guide {
    c0: Line,
    c1: Line,
    alpha: f32,
    beta: f32,
    c0_out: f32,
    c1_out: f32,
}

impl Guide {
    fn new(seconds: f32, alpha: f32, beta: f32) -> Self {
        Self {
            c0: Line::new(seconds),
            c1: Line::new(seconds),
            alpha,
            beta,
            c0_out: 0.0,
            c1_out: 0.0,
        }
    }

    fn pop(&mut self) -> (f32, f32) {
        self.c1_out = dampen(self.c1.pop());
        self.c0_out = dampen(self.c0.pop());
        (
            self.c1_out * (1.0 - self.alpha.abs()),
            self.c0_out * (1.0 - self.beta.abs()),
        )
    }

    fn push(&mut self, x0: f32, x1: f32) {
        let c0_in = self.c1_out * self.alpha + x0;
        let c1_in = self.c0_out * self.beta + x1;
        self.c0.push(c0_in);
        self.c1.push(c1_in);
    }
}

fn dampen(sample: f32) -> f32 {
    let magnitude = sample.abs();
    if magnitude > GUIDE_MAX_AMP {
        (-1.0 / (magnitude - GUIDE_MAX_AMP + 1.0) + 1.0 + GUIDE_MAX_AMP).copysign(sample)
    } else {
        sample
    }
}

struct Cylinder {
    offset: f32,
    exhaust: Guide,
    intake: Guide,
    extractor: Guide,
    sound: f32,
    extractor_exhaust: f32,
}

impl Cylinder {
    fn pop(&mut self, crank: f32, collector: f32, ignition: f32) -> (f32, f32, f32) {
        let crank = fract(crank + self.offset);
        self.sound = piston_motion(crank) * PISTON
            + fuel_ignition(crank, IGNITION_TIME) * IGNITION * ignition;
        let ex_valve = exhaust_valve(fract(crank + EXHAUST_VALVE_SHIFT));
        let in_valve = intake_valve(fract(crank + INTAKE_VALVE_SHIFT));
        self.exhaust.alpha = EXHAUST_CLOSED + (EXHAUST_OPEN - EXHAUST_CLOSED) * ex_valve;
        self.intake.alpha = INTAKE_CLOSED + (INTAKE_OPEN - INTAKE_CLOSED) * in_valve;
        let ex = self.exhaust.pop();
        let inn = self.intake.pop();
        let extractor = self.extractor.pop();
        self.extractor_exhaust = extractor.0;
        self.extractor.push(ex.1, collector);
        (inn.1, extractor.1, self.sound)
    }

    fn push(&mut self, intake: f32) {
        let ex_in = (1.0 - self.exhaust.alpha.abs()) * self.sound * 0.5;
        self.exhaust.push(ex_in, self.extractor_exhaust);
        let in_in = (1.0 - self.intake.alpha.abs()) * self.sound * 0.5;
        self.intake.push(in_in, intake);
    }
}

pub(crate) struct Model {
    cylinders: Vec<Cylinder>,
    straight: Guide,
    mufflers: Vec<Guide>,
    noise: Noise,
    noise_lp: OnePole,
    vibration_lp: OnePole,
    dc: OnePole,
    crank: f32,
    exhaust_collector: f32,
    intake_collector: f32,
}

impl Model {
    pub(crate) fn new(cylinders: usize, seed: u64) -> Self {
        let n = cylinders.clamp(1, 16);
        let cylinders = (0..n)
            .map(|i| {
                let [exhaust, intake, extractor] = PIPES[i % PIPES.len()];
                Cylinder {
                    offset: i as f32 / n as f32,
                    exhaust: Guide::new(exhaust, EXHAUST_CLOSED, 0.06),
                    intake: Guide::new(intake, INTAKE_CLOSED, -0.304_824_65),
                    extractor: Guide::new(extractor, 0.0, 0.016_535_282),
                    sound: 0.0,
                    extractor_exhaust: 0.0,
                }
            })
            .collect();
        Self {
            cylinders,
            straight: Guide::new(0.002_354_166_7, 0.123_455_05, -0.239_606_92),
            mufflers: MUFFLERS
                .iter()
                .map(|&d| Guide::new(d, 0.0, 0.073_564_77))
                .collect(),
            noise: Noise::new(seed),
            noise_lp: OnePole::new(INTAKE_NOISE_HZ),
            vibration_lp: OnePole::new(VIBRATION_HZ),
            dc: OnePole::new(DC_HZ),
            crank: 0.0,
            exhaust_collector: 0.0,
            intake_collector: 0.0,
        }
    }

    pub(crate) fn tick(&mut self, rpm: f32, load: f32) -> f32 {
        let step = rpm.max(0.0) / (RATE as f32 * 120.0);
        self.crank = fract(self.crank + step);
        let load = load.clamp(0.0, 1.0);
        let (intake, vibration, exhaust) = self.generate(0.35 + 0.65 * load);
        let mixed = (intake * INTAKE_VOLUME * (0.4 + 0.6 * load)
            + vibration * VIBRATION_VOLUME
            + exhaust * EXHAUST_VOLUME * (0.6 + 0.4 * load))
            * VOLUME;
        mixed - self.dc.tick(mixed)
    }

    fn generate(&mut self, ignition: f32) -> (f32, f32, f32) {
        let intake_noise = self.noise_lp.tick(self.noise.white()) * INTAKE_NOISE;
        let count = self.cylinders.len() as f32;
        let last_exhaust = self.exhaust_collector / count;
        self.exhaust_collector = 0.0;
        self.intake_collector = 0.0;
        let mut vibration = 0.0;
        for cylinder in &mut self.cylinders {
            let (inn, ex, vib) = cylinder.pop(self.crank, last_exhaust, ignition);
            self.intake_collector += inn;
            self.exhaust_collector += ex;
            vibration += vib;
        }
        let straight = self.straight.pop();
        let mut muffled = (0.0, 0.0);
        for muffler in &mut self.mufflers {
            let out = muffler.pop();
            muffled.0 += out.0;
            muffled.1 += out.1;
        }
        let intake_share = self.intake_collector / count;
        for cylinder in &mut self.cylinders {
            let valve = intake_valve(fract(self.crank + cylinder.offset));
            cylinder.push(intake_share + intake_noise * valve);
        }
        self.straight.push(self.exhaust_collector, muffled.0);
        self.exhaust_collector += straight.0;
        let elements = self.mufflers.len() as f32;
        for muffler in &mut self.mufflers {
            muffler.push(straight.1 / elements, 0.0);
        }
        let vibration = self.vibration_lp.tick(vibration);
        (self.intake_collector, vibration, muffled.1)
    }
}

fn exhaust_valve(crank: f32) -> f32 {
    if 0.75 < crank && crank < 1.0 {
        -sin_turns(crank * 2.0)
    } else {
        0.0
    }
}

fn intake_valve(crank: f32) -> f32 {
    if 0.0 < crank && crank < 0.25 {
        sin_turns(crank * 2.0)
    } else {
        0.0
    }
}

fn piston_motion(crank: f32) -> f32 {
    sin_turns(crank * 2.0 + 0.25)
}

fn fuel_ignition(crank: f32, time: f32) -> f32 {
    if 0.5 < crank && crank < time / 2.0 + 0.5 {
        sin_turns((crank - 0.5) / time)
    } else {
        0.0
    }
}
