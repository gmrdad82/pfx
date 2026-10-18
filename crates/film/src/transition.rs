use pfx_core::ease::Ease;
use serde::Deserialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Dissolve,
    Push,
    Whip,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Towards {
    #[default]
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transition {
    pub kind: Kind,
    pub seconds: f64,
    #[serde(default)]
    pub towards: Towards,
    #[serde(default)]
    pub ease: Option<String>,
}

impl Transition {
    pub fn check(&self, scene: f64) -> Result<(), String> {
        if !(self.seconds.is_finite() && self.seconds > 0.0 && self.seconds <= scene) {
            return Err("[scene.transition] seconds is in (0, the scene's length]".into());
        }
        if let Some(name) = &self.ease
            && Ease::from_name(name).is_none()
        {
            return Err(format!(
                "[scene.transition] no ease '{name}' (linear, sine_in_out, cubic_in_out, expo_in_out, …)"
            ));
        }
        Ok(())
    }

    pub fn name(&self) -> &'static str {
        match self.kind {
            Kind::Dissolve => "dissolve",
            Kind::Push => "push",
            Kind::Whip => "whip",
        }
    }

    pub fn eased(&self, w: f32) -> f32 {
        let w = w.clamp(0.0, 1.0);
        let fallback = match self.kind {
            Kind::Dissolve => Ease::Linear,
            Kind::Push => Ease::SineInOut,
            Kind::Whip => Ease::ExpoInOut,
        };
        self.ease
            .as_deref()
            .and_then(Ease::from_name)
            .unwrap_or(fallback)
            .at(w)
    }

    pub fn mix(
        &self,
        out: &[u8],
        into: &[u8],
        size: [u32; 2],
        w: f32,
        before: f32,
    ) -> Result<Vec<u8>, String> {
        let [width, height] = size.map(|v| v as usize);
        if out.len() != width * height * 4 || into.len() != out.len() {
            return Err(format!(
                "a {} needs RGBA8 frames of {}x{} from the show",
                self.name(),
                size[0],
                size[1]
            ));
        }
        let e = self.eased(w);
        Ok(match self.kind {
            Kind::Dissolve => dissolve(out, into, e),
            Kind::Push => push(out, into, [width, height], self.towards, e),
            Kind::Whip => {
                let mut moved = push(out, into, [width, height], self.towards, e);
                let span = match self.towards {
                    Towards::Left | Towards::Right => width,
                    Towards::Up | Towards::Down => height,
                };
                let streak = ((e - self.eased(before)).abs() * span as f32).round() as usize;
                smear(&mut moved, [width, height], self.towards, streak);
                moved
            }
        })
    }
}

pub fn dissolve(out: &[u8], into: &[u8], w: f32) -> Vec<u8> {
    out.iter()
        .zip(into)
        .map(|(a, b)| {
            let (a, b) = (f32::from(*a), f32::from(*b));
            (a + (b - a) * w).round().clamp(0.0, 255.0) as u8
        })
        .collect()
}

pub fn push(out: &[u8], into: &[u8], size: [usize; 2], towards: Towards, w: f32) -> Vec<u8> {
    let [width, height] = size;
    let mut frame = vec![0u8; out.len()];
    let horizontal = matches!(towards, Towards::Left | Towards::Right);
    let span = if horizontal { width } else { height };
    let moved = ((w * span as f32).round() as usize).min(span);
    let forward = matches!(towards, Towards::Left | Towards::Up);
    for y in 0..height {
        for x in 0..width {
            let along = if horizontal { x } else { y };
            let (from, at) = if forward {
                if along + moved < span {
                    (out, along + moved)
                } else {
                    (into, along + moved - span)
                }
            } else if along >= moved {
                (out, along - moved)
            } else {
                (into, along + span - moved)
            };
            let (sx, sy) = if horizontal { (at, y) } else { (x, at) };
            let s = (sy * width + sx) * 4;
            let d = (y * width + x) * 4;
            frame[d..d + 4].copy_from_slice(&from[s..s + 4]);
        }
    }
    frame
}

pub fn smear(frame: &mut [u8], size: [usize; 2], towards: Towards, streak: usize) {
    let [width, height] = size;
    if streak < 2 {
        return;
    }
    let horizontal = matches!(towards, Towards::Left | Towards::Right);
    let (lines, span) = if horizontal {
        (height, width)
    } else {
        (width, height)
    };
    let at = |line: usize, i: usize| {
        if horizontal {
            (line * width + i) * 4
        } else {
            (i * width + line) * 4
        }
    };
    let half = streak / 2;
    let mut line_px = vec![[0u32; 4]; span];
    for line in 0..lines {
        for (i, px) in line_px.iter_mut().enumerate() {
            let p = at(line, i);
            *px = std::array::from_fn(|c| u32::from(frame[p + c]));
        }
        let mut sum = [0u32; 4];
        let mut count = 0u32;
        for px in line_px.iter().take(half.min(span)) {
            for c in 0..4 {
                sum[c] += px[c];
            }
            count += 1;
        }
        for i in 0..span {
            let enter = i + half;
            if enter < span {
                for c in 0..4 {
                    sum[c] += line_px[enter][c];
                }
                count += 1;
            }
            if i > half {
                let leave = i - half - 1;
                for c in 0..4 {
                    sum[c] -= line_px[leave][c];
                }
                count -= 1;
            }
            let p = at(line, i);
            for c in 0..4 {
                frame[p + c] = ((sum[c] + count / 2) / count) as u8;
            }
        }
    }
}
