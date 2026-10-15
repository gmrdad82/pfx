use crate::colour::colour;
use serde::Deserialize;

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Style {
    pub size: Option<f32>,
    pub weight: Option<u16>,
    pub tracking: Option<f32>,
    pub spread: Option<f32>,
    pub leading: Option<f32>,
    pub colour: Option<String>,
    pub align: Option<String>,
    pub caps: Option<bool>,
    pub place: Option<[f32; 2]>,
    pub fade: Option<f64>,
    pub out: Option<f64>,
    pub grow: Option<f32>,
    pub smash: Option<f32>,
    pub slide: Option<[f32; 2]>,
    pub outline: Option<f32>,
    pub edge: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Text {
    pub text: String,
    pub style: Option<String>,
    pub at: f64,
    pub until: Option<f64>,
    pub size: Option<f32>,
    pub weight: Option<u16>,
    pub tracking: Option<f32>,
    pub spread: Option<f32>,
    pub leading: Option<f32>,
    pub colour: Option<String>,
    pub align: Option<String>,
    pub caps: Option<bool>,
    pub place: Option<[f32; 2]>,
    pub fade: Option<f64>,
    pub out: Option<f64>,
    pub grow: Option<f32>,
    pub smash: Option<f32>,
    pub slide: Option<[f32; 2]>,
    pub outline: Option<f32>,
    pub edge: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sound {
    pub at: f64,
    pub sound: String,
    #[serde(default = "full")]
    pub gain: f32,
    #[serde(default)]
    pub pitch: f32,
    pub seconds: Option<f32>,
    pub every: Option<f64>,
    pub until: Option<f64>,
    #[serde(default = "same")]
    pub accel: f64,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Flash {
    pub at: f64,
    #[serde(default = "blink")]
    pub seconds: f64,
    pub colour: Option<String>,
    #[serde(default = "full")]
    pub amount: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Shake {
    pub at: f64,
    pub amount: f32,
}

fn full() -> f32 {
    1.0
}

fn same() -> f64 {
    1.0
}

fn blink() -> f64 {
    0.2
}

const ALIGNS: [&str; 3] = ["start", "middle", "end"];
const MOST_REPEATS: f64 = 400.0;

pub fn span(at: f64, until: Option<f64>, seconds: f64) -> Result<(), String> {
    if !(at.is_finite() && (0.0..seconds).contains(&at)) {
        return Err(format!("at is in [0, {seconds})"));
    }
    if let Some(u) = until
        && !(u.is_finite() && u > at)
    {
        return Err("until comes after at".into());
    }
    Ok(())
}

pub fn times(fade: Option<f64>, out: Option<f64>) -> Result<(), String> {
    for t in [fade, out].into_iter().flatten() {
        if !(t.is_finite() && (0.0..=30.0).contains(&t)) {
            return Err("fade and out are seconds in [0, 30]".into());
        }
    }
    Ok(())
}

pub fn tone(c: &Option<String>) -> Result<(), String> {
    match c {
        Some(c) => colour(c).map(|_| ()),
        None => Ok(()),
    }
}

impl Style {
    pub fn check(&self) -> Result<(), String> {
        tone(&self.colour)?;
        tone(&self.edge)?;
        times(self.fade, self.out)?;
        if self
            .outline
            .is_some_and(|o| !(o.is_finite() && (0.0..=40.0).contains(&o)))
        {
            return Err("outline is in [0, 40]".into());
        }
        if let Some(a) = &self.align
            && !ALIGNS.contains(&a.as_str())
        {
            return Err(format!("align is one of {}", ALIGNS.join(", ")));
        }
        if self
            .size
            .is_some_and(|s| !(s.is_finite() && s > 0.0 && s <= 1200.0))
        {
            return Err("size is in (0, 1200]".into());
        }
        Ok(())
    }
}

impl Text {
    pub fn style(&self) -> Style {
        Style {
            size: self.size,
            weight: self.weight,
            tracking: self.tracking,
            spread: self.spread,
            leading: self.leading,
            colour: self.colour.clone(),
            align: self.align.clone(),
            caps: self.caps,
            place: self.place,
            fade: self.fade,
            out: self.out,
            grow: self.grow,
            smash: self.smash,
            slide: self.slide,
            outline: self.outline,
            edge: self.edge.clone(),
        }
    }

    pub fn check(&self, seconds: f64) -> Result<(), String> {
        span(self.at, self.until, seconds)?;
        self.style().check()
    }
}

impl Sound {
    pub fn check(&self, seconds: f64, known: &dyn Fn(&str) -> bool) -> Result<(), String> {
        span(self.at, self.until, seconds)?;
        if !known(&self.sound) {
            return Err(format!("the score has no sound '{}'", self.sound));
        }
        if !(self.gain.is_finite() && (0.0..=4.0).contains(&self.gain)) {
            return Err("a sound's gain is in [0, 4]".into());
        }
        if !(self.pitch.is_finite() && self.pitch.abs() <= 24.0) {
            return Err("a sound's pitch is within 24 semitones".into());
        }
        if self
            .seconds
            .is_some_and(|s| !(s.is_finite() && s > 0.0 && s <= 60.0))
        {
            return Err("a sound's seconds are in (0, 60]".into());
        }
        if let Some(every) = self.every {
            let end = self.until.unwrap_or(seconds) - self.at;
            if !(every.is_finite() && every >= 0.02 && end / every <= MOST_REPEATS) {
                return Err("every is at least 0.02 s, with at most 400 repeats".into());
            }
            if !(self.accel.is_finite() && (0.2..=2.0).contains(&self.accel)) {
                return Err("accel is in [0.2, 2]".into());
            }
        }
        Ok(())
    }

    pub fn times(&self, seconds: f64) -> Vec<f64> {
        let Some(mut every) = self.every else {
            return vec![self.at];
        };
        let end = self.until.unwrap_or(seconds);
        let mut out = Vec::new();
        let mut t = self.at;
        while t < end && out.len() < MOST_REPEATS as usize {
            out.push(t);
            t += every;
            every = (every * self.accel).max(0.02);
        }
        out
    }
}

impl Flash {
    pub fn check(&self, seconds: f64) -> Result<(), String> {
        span(self.at, None, seconds)?;
        tone(&self.colour)?;
        if !(self.seconds.is_finite() && self.seconds > 0.0 && self.seconds <= 10.0) {
            return Err("a flash lasts (0, 10] s".into());
        }
        if !(self.amount.is_finite() && (0.0..=1.0).contains(&self.amount)) {
            return Err("a flash's amount is in [0, 1]".into());
        }
        Ok(())
    }
}

impl Shake {
    pub fn check(&self, seconds: f64) -> Result<(), String> {
        span(self.at, None, seconds)?;
        if !(self.amount.is_finite() && (0.0..=1.0).contains(&self.amount)) {
            return Err("a shake's amount is in [0, 1]".into());
        }
        Ok(())
    }
}
