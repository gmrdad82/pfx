use crate::ease::Ease;

pub const CLOCK_VERSION: u32 = 1;
pub const CLOCK_BYTES: usize = 56;
pub const SPEED_MIN: f32 = 0.05;
pub const SPEED_MAX: f32 = 4.0;
const RECOVER_SLICES: u32 = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClockError {
    Length { found: usize },
    Version { found: u32 },
    Invalid(&'static str),
}

impl std::fmt::Display for ClockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Length { found } => {
                write!(f, "saved clock is {found} bytes, expected {CLOCK_BYTES}")
            }
            Self::Version { found } => write!(
                f,
                "saved clock is version {found}, this build reads {CLOCK_VERSION}"
            ),
            Self::Invalid(what) => write!(f, "saved clock has {what}"),
        }
    }
}

impl std::error::Error for ClockError {}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HitStop {
    pub floor: f32,
    pub hold: f32,
    pub recover: f32,
    pub ease: Ease,
}

impl HitStop {
    pub fn new(floor: f32, hold: f32, recover: f32) -> Self {
        Self {
            floor,
            hold,
            recover,
            ease: Ease::QuadOut,
        }
    }

    pub fn with_ease(mut self, ease: Ease) -> Self {
        self.ease = ease;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dip {
    pub floor: f32,
    pub hold: f32,
    pub recover: f32,
    pub ease: Ease,
    pub elapsed: f64,
}

impl Dip {
    pub fn length(&self) -> f64 {
        f64::from(self.hold) + f64::from(self.recover)
    }

    pub fn scale_at(&self, elapsed: f64) -> f64 {
        let hold = f64::from(self.hold);
        let floor = f64::from(self.floor);
        if elapsed < hold {
            return floor;
        }
        let recover = f64::from(self.recover);
        if recover <= 0.0 || elapsed >= hold + recover {
            return 1.0;
        }
        let u = ((elapsed - hold) / recover) as f32;
        floor + (1.0 - floor) * f64::from(self.ease.at(u))
    }

    fn integral(&self, from: f64, to: f64) -> f64 {
        let hold = f64::from(self.hold);
        let end = self.length();
        let mut total = 0.0;
        let held = (to.min(hold) - from).max(0.0);
        total += held * f64::from(self.floor);
        let a = from.max(hold);
        let b = to.min(end);
        if b > a {
            let slices = f64::from(RECOVER_SLICES);
            let width = (b - a) / slices;
            let mut sum = self.scale_at(a) + self.scale_at(b);
            for index in 1..RECOVER_SLICES {
                let weight = if index % 2 == 1 { 4.0 } else { 2.0 };
                sum += weight * self.scale_at(a + width * f64::from(index));
            }
            total += sum * width / 3.0;
        }
        total += (to - from.max(end)).max(0.0);
        total
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Tick {
    pub ticks: u32,
    pub dt: f32,
    pub paced: f32,
    pub real: f32,
    pub scale: f32,
    pub alpha: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clock {
    rate: f64,
    speed: f32,
    paused: bool,
    dip: Option<Dip>,
    time: f64,
    ticks: u64,
}

impl Clock {
    pub fn new(tick_rate: f64) -> Result<Self, ClockError> {
        if !(tick_rate.is_finite() && tick_rate > 0.0) {
            return Err(ClockError::Invalid(
                "a tick rate that is not finite and positive",
            ));
        }
        Ok(Self {
            rate: tick_rate,
            speed: 1.0,
            paused: false,
            dip: None,
            time: 0.0,
            ticks: 0,
        })
    }

    pub fn tick_rate(&self) -> f64 {
        self.rate
    }

    pub fn tick_dt(&self) -> f64 {
        1.0 / self.rate
    }

    pub fn speed(&self) -> f32 {
        self.speed
    }

    pub fn set_speed(&mut self, speed: f32) {
        if speed.is_finite() {
            self.speed = speed.clamp(SPEED_MIN, SPEED_MAX);
        }
    }

    pub fn paused(&self) -> bool {
        self.paused
    }

    pub fn pause(&mut self) {
        self.paused = true;
    }

    pub fn resume(&mut self) {
        self.paused = false;
    }

    pub fn hit_stop(&mut self, stop: HitStop) {
        let floor = if stop.floor.is_finite() {
            stop.floor.clamp(0.0, 1.0)
        } else {
            1.0
        };
        let hold = finite_time(stop.hold);
        let recover = finite_time(stop.recover);
        if floor >= 1.0 || hold + recover <= 0.0 {
            return;
        }
        self.dip = Some(Dip {
            floor,
            hold,
            recover,
            ease: stop.ease,
            elapsed: 0.0,
        });
    }

    pub fn dip(&self) -> Option<Dip> {
        self.dip
    }

    pub fn scale(&self) -> f32 {
        if self.paused {
            return 0.0;
        }
        let dip = self.dip.map_or(1.0, |dip| dip.scale_at(dip.elapsed));
        (f64::from(self.speed) * dip) as f32
    }

    pub fn time(&self) -> f64 {
        self.time
    }

    pub fn ticks(&self) -> u64 {
        self.ticks
    }

    pub fn alpha(&self) -> f32 {
        let due = self.time * self.rate - self.ticks as f64;
        due.clamp(0.0, 1.0) as f32
    }

    pub fn advance(&mut self, real: f32) -> Tick {
        let real = if real.is_finite() { real.max(0.0) } else { 0.0 };
        if self.paused || real <= 0.0 {
            return Tick {
                ticks: 0,
                dt: 0.0,
                paced: 0.0,
                real,
                scale: self.scale(),
                alpha: self.alpha(),
            };
        }
        let seconds = f64::from(real);
        let speed = f64::from(self.speed);
        let scaled = match self.dip.as_mut() {
            None => seconds,
            Some(dip) => {
                let from = dip.elapsed;
                dip.elapsed += seconds;
                dip.integral(from, dip.elapsed)
            }
        };
        if self.dip.is_some_and(|dip| dip.elapsed >= dip.length()) {
            self.dip = None;
        }
        let before = self.time;
        self.time += speed * scaled;
        let due = (self.time * self.rate).floor().max(0.0) as u64;
        let ticks = due.saturating_sub(self.ticks);
        self.ticks = self.ticks.max(due);
        Tick {
            ticks: u32::try_from(ticks).unwrap_or(u32::MAX),
            dt: (self.time - before) as f32,
            paced: (speed * seconds) as f32,
            real,
            scale: self.scale(),
            alpha: self.alpha(),
        }
    }

    pub fn sound_rate(&self, tie: f32) -> f32 {
        let tie = if tie.is_finite() {
            tie.clamp(0.0, 1.0)
        } else {
            0.0
        };
        (1.0 + tie * (self.scale() - 1.0)).clamp(1.0 / 16.0, 8.0)
    }

    pub fn save(&self) -> [u8; CLOCK_BYTES] {
        let mut out = [0u8; CLOCK_BYTES];
        let dip = self.dip.unwrap_or(Dip {
            floor: 1.0,
            hold: 0.0,
            recover: 0.0,
            ease: Ease::Linear,
            elapsed: 0.0,
        });
        out[0..4].copy_from_slice(&CLOCK_VERSION.to_le_bytes());
        out[4..12].copy_from_slice(&self.rate.to_le_bytes());
        out[12..16].copy_from_slice(&self.speed.to_le_bytes());
        out[16] = u8::from(self.paused);
        out[17] = u8::from(self.dip.is_some());
        out[18] = dip.ease as u8;
        out[20..28].copy_from_slice(&self.time.to_le_bytes());
        out[28..36].copy_from_slice(&self.ticks.to_le_bytes());
        out[36..40].copy_from_slice(&dip.floor.to_le_bytes());
        out[40..44].copy_from_slice(&dip.hold.to_le_bytes());
        out[44..48].copy_from_slice(&dip.recover.to_le_bytes());
        out[48..56].copy_from_slice(&dip.elapsed.to_le_bytes());
        out
    }

    pub fn restore(bytes: &[u8]) -> Result<Self, ClockError> {
        if bytes.len() != CLOCK_BYTES {
            return Err(ClockError::Length { found: bytes.len() });
        }
        let u32_at = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        let f32_at = |at: usize| f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        let f64_at = |at: usize| f64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
        let version = u32_at(0);
        if version != CLOCK_VERSION {
            return Err(ClockError::Version { found: version });
        }
        let rate = f64_at(4);
        if !(rate.is_finite() && rate > 0.0) {
            return Err(ClockError::Invalid(
                "a tick rate that is not finite and positive",
            ));
        }
        let speed = f32_at(12);
        if !(SPEED_MIN..=SPEED_MAX).contains(&speed) {
            return Err(ClockError::Invalid("a speed outside 0.05..=4"));
        }
        let flag = |at: usize, what| match bytes[at] {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(ClockError::Invalid(what)),
        };
        let paused = flag(16, "a pause flag that is neither 0 nor 1")?;
        let dipping = flag(17, "a dip flag that is neither 0 nor 1")?;
        let ease = *Ease::ALL
            .get(usize::from(bytes[18]))
            .ok_or(ClockError::Invalid("an unknown easing"))?;
        if bytes[19] != 0 {
            return Err(ClockError::Invalid("a nonzero padding byte"));
        }
        let time = f64_at(20);
        if !(time.is_finite() && time >= 0.0) {
            return Err(ClockError::Invalid(
                "a game time that is not finite and nonnegative",
            ));
        }
        let ticks = u64::from_le_bytes(bytes[28..36].try_into().unwrap());
        let dip = Dip {
            floor: f32_at(36),
            hold: f32_at(40),
            recover: f32_at(44),
            ease,
            elapsed: f64_at(48),
        };
        let dip = if dipping {
            let sane = (0.0..1.0).contains(&dip.floor)
                && dip.hold.is_finite()
                && dip.hold >= 0.0
                && dip.recover.is_finite()
                && dip.recover >= 0.0
                && dip.hold + dip.recover > 0.0
                && dip.elapsed.is_finite()
                && dip.elapsed >= 0.0
                && dip.elapsed < dip.length();
            if !sane {
                return Err(ClockError::Invalid("a dip out of range"));
            }
            Some(dip)
        } else {
            None
        };
        Ok(Self {
            rate,
            speed,
            paused,
            dip,
            time,
            ticks,
        })
    }
}

fn finite_time(seconds: f32) -> f32 {
    if seconds.is_finite() {
        seconds.max(0.0)
    } else {
        0.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Steps {
    rate: f64,
    done: u64,
}

impl Steps {
    pub fn new(rate: f64, clock: &Clock) -> Self {
        let rate = if rate.is_finite() && rate > 0.0 {
            rate
        } else {
            60.0
        };
        Self {
            rate,
            done: (clock.time() * rate).floor().max(0.0) as u64,
        }
    }

    pub fn rate(&self) -> f64 {
        self.rate
    }

    pub fn done(&self) -> u64 {
        self.done
    }

    pub fn due(&mut self, clock: &Clock) -> u32 {
        let due = (clock.time() * self.rate).floor().max(0.0) as u64;
        let n = due.saturating_sub(self.done);
        self.done = self.done.max(due);
        u32::try_from(n).unwrap_or(u32::MAX)
    }
}

#[cfg(test)]
mod tests;
