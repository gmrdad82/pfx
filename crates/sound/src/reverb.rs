const COMBS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASSES: [usize; 4] = [556, 441, 341, 225];
const SPREAD: usize = 23;
const TUNED_RATE: f64 = 44_100.0;
const FIXED_GAIN: f32 = 0.015;
const SCALE_WET: f32 = 3.0;
const SCALE_DAMP: f32 = 0.4;
const SCALE_ROOM: f32 = 0.28;
const OFFSET_ROOM: f32 = 0.7;
const ALLPASS_FEEDBACK: f32 = 0.5;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Room {
    pub size: f32,
    pub damp: f32,
    pub wet: f32,
    pub width: f32,
}

struct Comb {
    buffer: Vec<f32>,
    at: usize,
    store: f32,
}

impl Comb {
    fn new(len: usize) -> Comb {
        Comb {
            buffer: vec![0.0; len.max(1)],
            at: 0,
            store: 0.0,
        }
    }

    fn process(&mut self, input: f32, feedback: f32, damp: f32) -> f32 {
        let output = self.buffer[self.at];
        self.store = output * (1.0 - damp) + self.store * damp;
        self.buffer[self.at] = input + self.store * feedback;
        self.at = (self.at + 1) % self.buffer.len();
        output
    }
}

struct Allpass {
    buffer: Vec<f32>,
    at: usize,
}

impl Allpass {
    fn new(len: usize) -> Allpass {
        Allpass {
            buffer: vec![0.0; len.max(1)],
            at: 0,
        }
    }

    fn process(&mut self, input: f32) -> f32 {
        let held = self.buffer[self.at];
        self.buffer[self.at] = input + held * ALLPASS_FEEDBACK;
        self.at = (self.at + 1) % self.buffer.len();
        held - input
    }
}

struct Side {
    combs: Vec<Comb>,
    allpasses: Vec<Allpass>,
}

impl Side {
    fn new(rate: u32, spread: usize) -> Side {
        let scale =
            |len: usize| ((len + spread) as f64 * f64::from(rate) / TUNED_RATE).round() as usize;
        Side {
            combs: COMBS.iter().map(|&n| Comb::new(scale(n))).collect(),
            allpasses: ALLPASSES.iter().map(|&n| Allpass::new(scale(n))).collect(),
        }
    }

    fn process(&mut self, input: f32, feedback: f32, damp: f32) -> f32 {
        let mut out = 0.0;
        for comb in &mut self.combs {
            out += comb.process(input, feedback, damp);
        }
        for allpass in &mut self.allpasses {
            out = allpass.process(out);
        }
        out
    }
}

pub fn reverb(dry: &[[f32; 2]], rate: u32, room: Room) -> Vec<[f32; 2]> {
    let unit = |value: f32| {
        if value.is_nan() {
            0.0
        } else {
            value.clamp(0.0, 1.0)
        }
    };
    let feedback = unit(room.size) * SCALE_ROOM + OFFSET_ROOM;
    let damp = unit(room.damp) * SCALE_DAMP;
    let wet = if room.wet.is_finite() {
        room.wet.max(0.0) * SCALE_WET
    } else {
        0.0
    };
    let width = unit(room.width);
    let wet1 = wet * (width / 2.0 + 0.5);
    let wet2 = wet * ((1.0 - width) / 2.0);
    let mut left = Side::new(rate, 0);
    let mut right = Side::new(rate, SPREAD);
    dry.iter()
        .map(|[l, r]| {
            let input = (l + r) * FIXED_GAIN;
            let a = left.process(input, feedback, damp);
            let b = right.process(input, feedback, damp);
            [a * wet1 + b * wet2, b * wet1 + a * wet2]
        })
        .collect()
}
