use crate::channel::Ramp;

const OPEN_LOW_PASS: f32 = 0.45;
const OPEN_HIGH_PASS: f32 = 5.0;
const MIN_HZ: f32 = 5.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    LowPass,
    HighPass,
}

pub(crate) struct Filter {
    kind: Kind,
    on: bool,
    closing: bool,
    target: Option<f32>,
    cutoff: Ramp,
    applied: f32,
    coef: [f64; 5],
    state: Vec<[f64; 2]>,
    primed: bool,
}

impl Filter {
    pub(crate) fn new(kind: Kind) -> Self {
        Self {
            kind,
            on: false,
            closing: false,
            target: None,
            cutoff: Ramp::new(0.0),
            applied: f32::NAN,
            coef: [1.0, 0.0, 0.0, 0.0, 0.0],
            state: Vec::new(),
            primed: false,
        }
    }

    pub(crate) fn target(&self) -> Option<f32> {
        self.target
    }

    fn open(&self, rate: u32) -> f32 {
        match self.kind {
            Kind::LowPass => OPEN_LOW_PASS * rate as f32,
            Kind::HighPass => OPEN_HIGH_PASS,
        }
    }

    pub(crate) fn set(&mut self, hz: Option<f32>, frames: u32, rate: u32) {
        let top = 0.49 * rate as f32;
        match hz {
            Some(hz) => {
                let hz = if hz.is_nan() {
                    top
                } else {
                    hz.clamp(MIN_HZ, top)
                };
                if !self.on {
                    self.on = true;
                    self.cutoff.snap(self.open(rate).ln());
                    self.applied = f32::NAN;
                    self.primed = false;
                }
                self.closing = false;
                self.target = Some(hz);
                self.cutoff.to(hz.ln(), frames);
            }
            None => {
                self.target = None;
                if self.on {
                    self.closing = true;
                    self.cutoff.to(self.open(rate).ln(), frames);
                }
            }
        }
    }

    pub(crate) fn idle(&mut self, frames: usize) {
        if self.on {
            self.cutoff.skip(frames);
            self.primed = false;
            if self.closing && self.cutoff.is_settled() {
                self.on = false;
                self.closing = false;
            }
        }
    }

    pub(crate) fn process(&mut self, buf: &mut [f32], frames: usize, channels: usize, rate: u32) {
        if !self.on {
            return;
        }
        if self.state.len() != channels {
            self.state = vec![[0.0; 2]; channels];
            self.primed = false;
        }
        for frame in 0..frames {
            let cutoff = self.cutoff.next();
            if cutoff != self.applied {
                self.recompute(cutoff, rate);
            }
            let [b0, b1, b2, a1, a2] = self.coef;
            let base = frame * channels;
            for channel in 0..channels {
                let x = f64::from(buf[base + channel]);
                let state = &mut self.state[channel];
                if !self.primed {
                    let y = x * (b0 + b1 + b2) / (1.0 + a1 + a2);
                    state[1] = b2 * x - a2 * y;
                    state[0] = b1 * x - a1 * y + state[1];
                }
                let y = b0 * x + state[0];
                state[0] = b1 * x - a1 * y + state[1];
                state[1] = b2 * x - a2 * y;
                buf[base + channel] = y as f32;
            }
            self.primed = true;
            if self.closing && self.cutoff.is_settled() {
                self.on = false;
                self.closing = false;
                self.primed = false;
                return;
            }
        }
    }

    fn recompute(&mut self, cutoff: f32, rate: u32) {
        self.applied = cutoff;
        let hz = f64::from(cutoff).exp().min(f64::from(rate) * 0.49);
        let w0 = std::f64::consts::TAU * hz / f64::from(rate);
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / std::f64::consts::SQRT_2;
        let a0 = 1.0 + alpha;
        let (b0, b1, b2) = match self.kind {
            Kind::LowPass => ((1.0 - cos) / 2.0, 1.0 - cos, (1.0 - cos) / 2.0),
            Kind::HighPass => ((1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0),
        };
        self.coef = [
            b0 / a0,
            b1 / a0,
            b2 / a0,
            -2.0 * cos / a0,
            (1.0 - alpha) / a0,
        ];
    }
}
