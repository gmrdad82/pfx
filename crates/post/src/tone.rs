#[derive(Clone, Copy, Debug)]
pub enum Tone {
    Aces {
        gain: f32,
    },
    Neutral {
        start: f32,
        toe: f32,
        desaturation: f32,
        clamp: bool,
    },
    Agx,
}

impl Tone {
    pub fn aces() -> Self {
        Self::Aces { gain: 1.0 }
    }

    pub fn neutral(start: f32, clamp: bool) -> Self {
        Self::Neutral {
            start,
            toe: 0.04,
            desaturation: 0.15,
            clamp,
        }
    }

    pub fn map(&self, c: [f32; 3]) -> [f32; 3] {
        match *self {
            Self::Aces { gain } => aces(c, gain),
            Self::Neutral {
                start,
                toe,
                desaturation,
                clamp,
            } => neutral(c, start, toe, desaturation, clamp),
            Self::Agx => agx(c),
        }
    }
}

pub fn aces(c: [f32; 3], gain: f32) -> [f32; 3] {
    c.map(|v| aces_channel(v * gain))
}

fn aces_channel(x: f32) -> f32 {
    let a = x * (x * 2.51 + 0.03);
    let b = x * (x * 2.43 + 0.59) + 0.14;
    if b.abs() < 1e-8 {
        0.0
    } else {
        (a / b).clamp(0.0, 1.0)
    }
}

pub fn neutral(c: [f32; 3], start: f32, toe: f32, desaturation: f32, clamp_hi: bool) -> [f32; 3] {
    let low = c[0].min(c[1]).min(c[2]);
    let offset = if low < 0.08 {
        low - 6.25 * low * low
    } else {
        toe
    };
    let mut v = [c[0] - offset, c[1] - offset, c[2] - offset];
    let peak = v[0].max(v[1]).max(v[2]);
    if peak >= start && peak > 1e-8 {
        let d = 1.0 - start;
        let denom = peak + d - start;
        let mapped = if denom.abs() < 1e-8 {
            start
        } else {
            1.0 - d * d / denom
        };
        let scale = mapped / peak;
        v[0] *= scale;
        v[1] *= scale;
        v[2] *= scale;
        let g = 1.0 - 1.0 / (desaturation * (peak - mapped) + 1.0);
        v[0] = crate::color::lerp(v[0], mapped, g);
        v[1] = crate::color::lerp(v[1], mapped, g);
        v[2] = crate::color::lerp(v[2], mapped, g);
    }
    if clamp_hi {
        v.map(|x| x.clamp(0.0, 1.0))
    } else {
        v
    }
}

#[allow(clippy::excessive_precision)]
const AGX_MAT: [[f32; 3]; 3] = [
    [0.842479062253094, 0.0784335999999992, 0.0792237451477643],
    [0.0423282422610123, 0.878468636469772, 0.0791661274605434],
    [0.0423756549057051, 0.0784336, 0.879142973793104],
];

const AGX_MIN_EV: f32 = -12.47393;
const AGX_MAX_EV: f32 = 4.026069;

fn mul_row(m: [f32; 3], v: [f32; 3]) -> f32 {
    m[0] * v[0] + m[1] * v[1] + m[2] * v[2]
}

fn agx_contrast(x: f32) -> f32 {
    let x2 = x * x;
    let x4 = x2 * x2;
    15.5 * x4 * x2 - 40.14 * x4 * x + 31.96 * x4 - 6.868 * x2 * x + 0.4298 * x2 + 0.1191 * x
        - 0.00232
}

pub fn agx(c: [f32; 3]) -> [f32; 3] {
    let v = [
        mul_row(AGX_MAT[0], c),
        mul_row(AGX_MAT[1], c),
        mul_row(AGX_MAT[2], c),
    ];
    let span = AGX_MAX_EV - AGX_MIN_EV;
    v.map(|channel| {
        let x = channel.max(1e-10).log2();
        let t = ((x - AGX_MIN_EV) / span).clamp(0.0, 1.0);
        agx_contrast(t).clamp(0.0, 1.0)
    })
}
