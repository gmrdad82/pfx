use std::fmt;
use std::str::FromStr;

pub const NTSC: [(&str, u32); 5] = [
    ("23.976", 24000),
    ("29.97", 30000),
    ("47.952", 48000),
    ("59.94", 60000),
    ("119.88", 120000),
];

const NTSC_DEN: u32 = 1001;

const NTSC_SHORT: [(&str, u32); 2] = [("23.98", 24000), ("47.95", 48000)];
const ON_FRAME: f64 = 1e-12;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rate {
    num: u32,
    den: u32,
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

impl Rate {
    pub const fn whole(fps: u32) -> Self {
        Self {
            num: if fps == 0 { 1 } else { fps },
            den: 1,
        }
    }

    pub fn new(num: u64, den: u64) -> Result<Self, String> {
        if num == 0 || den == 0 {
            return Err("is not a frame rate above 0".into());
        }
        let g = gcd(num, den);
        let (num, den) = (num / g, den / g);
        match (u32::try_from(num), u32::try_from(den)) {
            (Ok(num), Ok(den)) => Ok(Self { num, den }),
            _ => Err("has a numerator or denominator too large".into()),
        }
    }

    pub fn num(&self) -> u32 {
        self.num
    }

    pub fn den(&self) -> u32 {
        self.den
    }

    pub fn whole_fps(&self) -> Option<u32> {
        (self.den == 1).then_some(self.num)
    }

    pub fn fps(&self) -> f64 {
        f64::from(self.num) / f64::from(self.den)
    }

    pub fn within(&self, least: u32, most: u32) -> bool {
        let scaled = |fps: u32| u64::from(fps) * u64::from(self.den);
        (scaled(least)..=scaled(most)).contains(&u64::from(self.num))
    }

    pub fn time(&self, frame: u64) -> f64 {
        (u128::from(frame) * u128::from(self.den)) as f64 / f64::from(self.num)
    }

    pub fn step(&self) -> f64 {
        f64::from(self.den) / f64::from(self.num)
    }

    pub fn frames(&self, seconds: f64) -> f64 {
        seconds * f64::from(self.num) / f64::from(self.den)
    }

    pub fn frame_at(&self, time: f64) -> Option<u64> {
        let at = self.frames(time);
        let near = at.round();
        (near >= 0.0 && (at - near).abs() <= ON_FRAME * near.max(1.0)).then_some(near as u64)
    }

    pub fn snap(&self, time: f64) -> f64 {
        self.time(self.frame_count(time))
    }

    pub fn before(&self, time: f64) -> f64 {
        match self.frame_at(time) {
            Some(frame) if frame > 0 => self.time(frame - 1),
            _ => time - self.step(),
        }
    }

    pub fn frame_count(&self, seconds: f64) -> u64 {
        self.frames(seconds).round().max(0.0) as u64
    }

    pub fn samples(&self, frame: u64, hz: u32) -> u64 {
        let scaled = u128::from(frame) * u128::from(self.den) * u128::from(hz);
        (scaled / u128::from(self.num)) as u64
    }

    pub fn samples_nearest(&self, frame: u64, hz: u32) -> u64 {
        let scaled = u128::from(frame) * u128::from(self.den) * u128::from(hz);
        let num = u128::from(self.num);
        ((scaled + num / 2) / num) as u64
    }

    pub fn label(&self) -> String {
        NTSC.iter()
            .find(|(_, num)| self.num == *num && self.den == NTSC_DEN)
            .map_or_else(|| self.to_string(), |(text, _)| (*text).to_string())
    }

    pub fn parse_within(text: &str, least: u32, most: u32) -> Result<Self, String> {
        let rate = text.parse::<Self>()?;
        if !rate.within(least, most) {
            return Err(format!("is not a frame rate from {least} to {most}"));
        }
        Ok(rate)
    }

    pub fn from_f64(value: f64) -> Result<Self, String> {
        if !(value.is_finite() && value > 0.0) {
            return Err("is not a frame rate above 0".into());
        }
        if value.fract() == 0.0 && value <= f64::from(u32::MAX) {
            return Ok(Self::whole(value as u32));
        }
        let ntsc = NTSC.iter().map(|(_, num)| Self {
            num: *num,
            den: NTSC_DEN,
        });
        if let Some(rate) = ntsc.clone().find(|rate| rate.fps() == value) {
            return Ok(rate);
        }
        decimal(&format!("{value}"))
    }
}

fn digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
}

fn decimal(text: &str) -> Result<Rate, String> {
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    if !digits(whole) || !(fraction.is_empty() || digits(fraction)) {
        return Err(SHAPE.into());
    }
    let fraction = fraction.trim_end_matches('0');
    let trimmed = whole.trim_start_matches('0');
    if fraction.is_empty() {
        let fps = whole.parse::<u64>().map_err(|_| SHAPE.to_string())?;
        return Rate::new(fps, 1);
    }
    let normal = if trimmed.is_empty() {
        format!("0.{fraction}")
    } else {
        format!("{trimmed}.{fraction}")
    };
    if let Some((_, num)) = NTSC
        .iter()
        .chain(NTSC_SHORT.iter())
        .find(|(known, _)| *known == normal)
    {
        return Rate::new(u64::from(*num), u64::from(NTSC_DEN));
    }
    let value: f64 = normal.parse().map_err(|_| SHAPE.to_string())?;
    let exact = || {
        let places = u32::try_from(fraction.len()).ok().filter(|p| *p <= 9)?;
        let scale = 10u64.pow(places);
        let top = format!("{trimmed}{fraction}").parse::<u64>().ok()?;
        Rate::new(top, scale).ok()
    };
    let near = NTSC
        .iter()
        .map(|(_, num)| Rate {
            num: *num,
            den: NTSC_DEN,
        })
        .find(|rate| (rate.fps() - value).abs() < 0.01);
    let suggestion = near.or_else(exact).map_or_else(
        || "a whole number or num/den".to_string(),
        |rate| rate.to_string(),
    );
    Err(format!(
        "is not an exact frame rate; use {suggestion} (a whole number, num/den, or one of 23.976 (23.98), 29.97, 47.952 (47.95), 59.94, 119.88)"
    ))
}

const SHAPE: &str = "expected a frame rate: a whole number (24), num/den (24000/1001) or 23.976 (23.98), 29.97, 47.952 (47.95), 59.94, 119.88";

impl FromStr for Rate {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        let text = text.trim();
        if let Some((num, den)) = text.split_once('/') {
            let (num, den) = (num.trim(), den.trim());
            if !digits(num) || !digits(den) {
                return Err(SHAPE.into());
            }
            let parse = |part: &str| part.parse::<u64>().map_err(|_| SHAPE.to_string());
            return Self::new(parse(num)?, parse(den)?);
        }
        if digits(text) {
            let fps = text.parse::<u64>().map_err(|_| SHAPE.to_string())?;
            return Self::new(fps, 1);
        }
        decimal(text)
    }
}

impl fmt::Display for Rate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.den == 1 {
            write!(f, "{}", self.num)
        } else {
            write!(f, "{}/{}", self.num, self.den)
        }
    }
}

impl From<u32> for Rate {
    fn from(fps: u32) -> Self {
        Self::whole(fps)
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for Rate {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.whole_fps() {
            Some(fps) => serializer.serialize_u32(fps),
            None => serializer.collect_str(self),
        }
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Rate {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visit;

        impl serde::de::Visitor<'_> for Visit {
            type Value = Rate;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(SHAPE)
            }

            fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Rate, E> {
                Rate::new(value, 1).map_err(|e| E::custom(format!("{value} {e}")))
            }

            fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Rate, E> {
                let whole = u64::try_from(value)
                    .map_err(|_| E::custom(format!("{value} is not a frame rate above 0")))?;
                self.visit_u64(whole)
            }

            fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<Rate, E> {
                Rate::from_f64(value).map_err(|e| E::custom(format!("{value} {e}")))
            }

            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Rate, E> {
                value
                    .parse()
                    .map_err(|e| E::custom(format!("'{value}' {e}")))
            }
        }

        deserializer.deserialize_any(Visit)
    }
}

#[cfg(test)]
mod tests;
