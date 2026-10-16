use crate::grade::{Lut, LutShape, NEUTRAL_DOMAIN};

pub const MAX_CUBE: u32 = 64;
pub const MAX_CURVES: u32 = 65536;
pub const MAX_BYTES: usize = 16 << 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CubeError {
    pub line: Option<usize>,
    pub message: String,
}

impl CubeError {
    fn at(line: usize, message: impl Into<String>) -> Self {
        Self {
            line: Some(line),
            message: message.into(),
        }
    }

    fn whole(message: impl Into<String>) -> Self {
        Self {
            line: None,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for CubeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.line {
            Some(line) => write!(f, "line {line}: {}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

impl std::error::Error for CubeError {}

fn numbers<const N: usize>(words: &[&str], line: usize, key: &str) -> Result<[f32; N], CubeError> {
    if words.len() != N {
        return Err(CubeError::at(line, format!("{key} takes {N} numbers")));
    }
    let mut out = [0.0; N];
    for (slot, word) in out.iter_mut().zip(words) {
        let value: f32 = word
            .parse()
            .map_err(|_| CubeError::at(line, format!("{key}: {word} is not a number")))?;
        if !value.is_finite() {
            return Err(CubeError::at(line, format!("{key}: {word} is not finite")));
        }
        *slot = value;
    }
    Ok(out)
}

fn size(words: &[&str], line: usize, key: &str, max: u32) -> Result<u32, CubeError> {
    let [word] = words else {
        return Err(CubeError::at(line, format!("{key} takes one integer")));
    };
    let n: u32 = word
        .parse()
        .map_err(|_| CubeError::at(line, format!("{key}: {word} is not an integer")))?;
    if !(2..=max).contains(&n) {
        return Err(CubeError::at(line, format!("{key} is from 2 to {max}")));
    }
    Ok(n)
}

pub fn parse_bytes(bytes: &[u8]) -> Result<Lut, CubeError> {
    if bytes.len() > MAX_BYTES {
        return Err(CubeError::whole(format!(
            "a .cube file is at most {MAX_BYTES} bytes"
        )));
    }
    let text =
        std::str::from_utf8(bytes).map_err(|_| CubeError::whole("a .cube file is UTF-8 text"))?;
    parse(text)
}

pub fn parse(text: &str) -> Result<Lut, CubeError> {
    let mut shape: Option<(LutShape, u32)> = None;
    let mut domain = NEUTRAL_DOMAIN;
    let mut values: Vec<[f32; 3]> = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let content = raw.split('#').next().unwrap_or("").trim();
        if content.is_empty() {
            continue;
        }
        let words: Vec<&str> = content.split_whitespace().collect();
        let (key, rest) = (words[0], &words[1..]);
        let starts_number = key
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit() || matches!(c, '-' | '+' | '.'));
        if starts_number {
            let Some((_, n)) = shape else {
                return Err(CubeError::at(
                    line,
                    "data comes after LUT_3D_SIZE or LUT_1D_SIZE",
                ));
            };
            let need = match shape {
                Some((LutShape::Cube, _)) => (n as usize).pow(3),
                _ => n as usize,
            };
            if values.len() >= need {
                return Err(CubeError::at(line, format!("more than {need} entries")));
            }
            values.push(numbers::<3>(&words, line, "an entry")?);
            continue;
        }
        if !values.is_empty() {
            return Err(CubeError::at(line, format!("{key} after the data")));
        }
        match key {
            "TITLE" => {}
            "LUT_3D_SIZE" | "LUT_1D_SIZE" => {
                if shape.is_some() {
                    return Err(CubeError::at(line, "one LUT_3D_SIZE or LUT_1D_SIZE"));
                }
                shape = Some(if key == "LUT_3D_SIZE" {
                    (LutShape::Cube, size(rest, line, key, MAX_CUBE)?)
                } else {
                    (LutShape::Curves, size(rest, line, key, MAX_CURVES)?)
                });
            }
            "DOMAIN_MIN" => domain[0] = numbers::<3>(rest, line, key)?,
            "DOMAIN_MAX" => domain[1] = numbers::<3>(rest, line, key)?,
            "LUT_1D_INPUT_RANGE" | "LUT_3D_INPUT_RANGE" => {
                let [lo, hi] = numbers::<2>(rest, line, key)?;
                domain = [[lo; 3], [hi; 3]];
            }
            other
                if other
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') => {}
            other => {
                return Err(CubeError::at(
                    line,
                    format!("{other} is not a .cube keyword"),
                ));
            }
        }
    }
    let Some((shape, size)) = shape else {
        return Err(CubeError::whole("no LUT_3D_SIZE or LUT_1D_SIZE"));
    };
    if (0..3).any(|k| domain[0][k] >= domain[1][k]) {
        return Err(CubeError::whole(
            "DOMAIN_MIN lies below DOMAIN_MAX on every channel",
        ));
    }
    let lut = Lut {
        size,
        values,
        shape,
        amount: 1.0,
        domain,
    };
    if lut.values.len() != lut.needs() {
        return Err(CubeError::whole(format!(
            "{} entries, the size needs {}",
            lut.values.len(),
            lut.needs()
        )));
    }
    Ok(lut)
}

pub fn format(lut: &Lut, title: &str) -> String {
    let mut out = format!("TITLE \"{title}\"\n");
    out.push_str(match lut.shape {
        LutShape::Cube => "LUT_3D_SIZE ",
        LutShape::Curves => "LUT_1D_SIZE ",
    });
    out.push_str(&format!("{}\n", lut.size));
    if lut.domain != NEUTRAL_DOMAIN {
        let [lo, hi] = lut.domain;
        out.push_str(&format!("DOMAIN_MIN {} {} {}\n", lo[0], lo[1], lo[2]));
        out.push_str(&format!("DOMAIN_MAX {} {} {}\n", hi[0], hi[1], hi[2]));
    }
    for rgb in &lut.values {
        out.push_str(&format!("{:.6} {:.6} {:.6}\n", rgb[0], rgb[1], rgb[2]));
    }
    out
}

pub fn generate(size: u32, mut map: impl FnMut([f32; 3]) -> [f32; 3]) -> Lut {
    let n = size.clamp(2, MAX_CUBE);
    let scale = (n - 1) as f32;
    let mut values = Vec::with_capacity((n as usize).pow(3));
    for b in 0..n {
        for g in 0..n {
            for r in 0..n {
                values.push(map([r as f32 / scale, g as f32 / scale, b as f32 / scale]));
            }
        }
    }
    Lut::cube(n, values)
}
