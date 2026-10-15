use crate::channel::Ramp;

pub const DRIVE_MIN_DB: f32 = -24.0;
pub const DRIVE_MAX_DB: f32 = 60.0;
const OUTPUT_MIN_DB: f32 = -60.0;
const OUTPUT_MAX_DB: f32 = 24.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DriveCurve {
    #[default]
    Soft,
    Tanh,
    Hard,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drive {
    pub drive_db: f32,
    pub curve: DriveCurve,
    pub mix: f32,
    pub compensate: bool,
    pub output_db: f32,
}

impl Default for Drive {
    fn default() -> Self {
        Self {
            drive_db: 0.0,
            curve: DriveCurve::Soft,
            mix: 1.0,
            compensate: true,
            output_db: 0.0,
        }
    }
}

impl Drive {
    pub(crate) fn sanitized(self) -> Self {
        Self {
            drive_db: clamp_db(self.drive_db, DRIVE_MIN_DB, DRIVE_MAX_DB),
            curve: self.curve,
            mix: if self.mix.is_nan() {
                1.0
            } else {
                self.mix.clamp(0.0, 1.0)
            },
            compensate: self.compensate,
            output_db: clamp_db(self.output_db, OUTPUT_MIN_DB, OUTPUT_MAX_DB),
        }
    }
}

fn clamp_db(db: f32, low: f32, high: f32) -> f32 {
    if db.is_nan() {
        0.0
    } else {
        db.clamp(low, high)
    }
}

fn amplitude(db: f32) -> f32 {
    pfx_core::sim::powf(10.0, db / 20.0)
}

pub(crate) fn shape(curve: DriveCurve, x: f32) -> f32 {
    match curve {
        DriveCurve::Soft => x / (1.0 + x.abs()),
        DriveCurve::Tanh => {
            if x >= 3.0 {
                1.0
            } else if x <= -3.0 {
                -1.0
            } else {
                let squared = x * x;
                x * (27.0 + squared) / (27.0 + 9.0 * squared)
            }
        }
        DriveCurve::Hard => x.clamp(-1.0, 1.0),
    }
}

pub(crate) struct DriveStage {
    on: bool,
    closing: bool,
    config: Option<Drive>,
    curve: DriveCurve,
    compensate: bool,
    db: Ramp,
    mix: Ramp,
    applied_db: f32,
    applied_mix: f32,
    gain: f32,
    wet: f32,
    dry: f32,
    output: f32,
}

impl DriveStage {
    pub(crate) fn new() -> Self {
        Self {
            on: false,
            closing: false,
            config: None,
            curve: DriveCurve::Soft,
            compensate: true,
            db: Ramp::new(0.0),
            mix: Ramp::new(0.0),
            applied_db: f32::NAN,
            applied_mix: f32::NAN,
            gain: 1.0,
            wet: 0.0,
            dry: 1.0,
            output: 1.0,
        }
    }

    pub(crate) fn target(&self) -> Option<Drive> {
        self.config
    }

    pub(crate) fn set(&mut self, drive: Option<Drive>, frames: u32) {
        match drive {
            Some(drive) => {
                let drive = drive.sanitized();
                if !self.on {
                    self.on = true;
                    self.db.snap(drive.drive_db);
                    self.mix.snap(0.0);
                    self.applied_db = f32::NAN;
                    self.applied_mix = f32::NAN;
                }
                self.closing = false;
                self.curve = drive.curve;
                self.compensate = drive.compensate;
                self.output = amplitude(drive.output_db);
                self.db.to(drive.drive_db, frames);
                self.mix.to(drive.mix, frames);
                self.config = Some(drive);
            }
            None => {
                self.config = None;
                if self.on {
                    self.closing = true;
                    self.mix.to(0.0, frames);
                }
            }
        }
    }

    pub(crate) fn glide(&mut self, drive_db: f32, frames: u32) {
        if let Some(config) = self.config.as_mut() {
            let drive_db = clamp_db(drive_db, DRIVE_MIN_DB, DRIVE_MAX_DB);
            config.drive_db = drive_db;
            self.db.to(drive_db, frames);
        }
    }

    pub(crate) fn idle(&mut self, frames: usize) {
        if self.on {
            self.db.skip(frames);
            self.mix.skip(frames);
            if self.closing && self.mix.is_settled() {
                self.on = false;
                self.closing = false;
            }
        }
    }

    pub(crate) fn process(&mut self, buf: &mut [f32], frames: usize, channels: usize) {
        if !self.on {
            return;
        }
        let (curve, compensate) = (self.curve, self.compensate);
        for frame in 0..frames {
            let db = self.db.next();
            if db != self.applied_db {
                self.applied_db = db;
                self.gain = amplitude(db);
            }
            let mix = self.mix.next();
            if mix != self.applied_mix {
                self.applied_mix = mix;
                self.wet = mix.sqrt();
                self.dry = (1.0 - mix).sqrt();
            }
            let gain = self.gain;
            let level = if compensate {
                self.output / gain
            } else {
                self.output
            };
            let base = frame * channels;
            for sample in &mut buf[base..base + channels] {
                let x = *sample;
                *sample = shape(curve, x * gain) * level * self.wet + x * self.dry;
            }
            if self.closing && self.mix.is_settled() {
                self.on = false;
                self.closing = false;
                return;
            }
        }
    }
}
