const MIN_EDGE_SECONDS: f32 = 0.002;
const CURVE_RATE: f32 = 5.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Linear,
    Exponential,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Envelope {
    pub attack_ms: f32,
    pub decay_ms: f32,
    pub sustain: f32,
    pub release_ms: f32,
    pub shape: Shape,
}

impl Default for Envelope {
    fn default() -> Self {
        Self {
            attack_ms: 0.0,
            decay_ms: 0.0,
            sustain: 1.0,
            release_ms: 0.0,
            shape: Shape::Linear,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Attack,
    Decay,
    Sustain,
    Release,
    Done,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct EnvState {
    shape: Shape,
    sustain: f32,
    attack: u64,
    decay: u64,
    release: u64,
    stage: Stage,
    pos: u64,
    from: f32,
    level: f32,
}

impl EnvState {
    pub(crate) fn new(envelope: Envelope, rate: u32, playing: bool) -> Self {
        let floor = (f64::from(MIN_EDGE_SECONDS) * f64::from(rate))
            .round()
            .max(1.0) as u64;
        let frames = |ms: f32| -> u64 {
            if ms.is_nan() || ms <= 0.0 {
                0
            } else {
                (f64::from(ms) * f64::from(rate) / 1000.0).round() as u64
            }
        };
        let sustain = if envelope.sustain.is_nan() {
            1.0
        } else {
            envelope.sustain.clamp(0.0, 1.0)
        };
        let mut state = Self {
            shape: envelope.shape,
            sustain,
            attack: frames(envelope.attack_ms).max(floor),
            decay: frames(envelope.decay_ms),
            release: frames(envelope.release_ms).max(floor),
            stage: Stage::Attack,
            pos: 0,
            from: 0.0,
            level: 0.0,
        };
        if playing {
            state.begin_decay();
            state.level = 1.0;
        }
        state
    }

    fn begin_decay(&mut self) {
        self.pos = 0;
        self.from = 1.0;
        self.stage = if self.decay == 0 {
            Stage::Sustain
        } else {
            Stage::Decay
        };
    }

    pub(crate) fn next(&mut self) -> f32 {
        let level = match self.stage {
            Stage::Attack => {
                let level = self.ease(self.pos, self.attack);
                self.pos += 1;
                if self.pos >= self.attack {
                    self.begin_decay();
                }
                level
            }
            Stage::Decay => {
                let level =
                    self.from + (self.sustain - self.from) * self.ease(self.pos, self.decay);
                self.pos += 1;
                if self.pos >= self.decay {
                    self.stage = Stage::Sustain;
                }
                level
            }
            Stage::Sustain => self.sustain,
            Stage::Release => {
                let level = self.from * (1.0 - self.ease(self.pos, self.release));
                self.pos += 1;
                if self.pos >= self.release {
                    self.stage = Stage::Done;
                }
                level
            }
            Stage::Done => 0.0,
        };
        self.level = level;
        level
    }

    fn ease(&self, pos: u64, len: u64) -> f32 {
        let p = (pos as f64 / len.max(1) as f64) as f32;
        match self.shape {
            Shape::Linear => p,
            Shape::Exponential => (1.0 - (-CURVE_RATE * p).exp()) / (1.0 - (-CURVE_RATE).exp()),
        }
    }

    pub(crate) fn release(&mut self, at_least: u64) {
        if matches!(self.stage, Stage::Release | Stage::Done) {
            return;
        }
        self.release = self.release.max(at_least);
        self.from = self.level;
        self.pos = 0;
        self.stage = Stage::Release;
    }

    pub(crate) fn level(&self) -> f32 {
        self.level
    }

    pub(crate) fn done(&self) -> bool {
        self.stage == Stage::Done
    }

    pub(crate) fn releasing(&self) -> bool {
        matches!(self.stage, Stage::Release | Stage::Done)
    }
}
