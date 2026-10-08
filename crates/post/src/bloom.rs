use crate::grade::{self, Downsample, keep_rgb};
use crate::image::Image;

pub const RING_SAMPLES: u32 = 20;
pub const RING_TURN: f32 = 0.37;

pub const NEUTRAL_RADII: [f32; 4] = [1.0, 2.0, 4.0, 8.0];
pub const NEUTRAL_GAINS: [f32; 4] = [1.0, 0.5, 0.25, 0.125];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BloomKind {
    Box,
    Tent,
    Rings,
}

#[derive(Clone, Copy, Debug)]
pub struct Bloom {
    pub kind: BloomKind,
    pub strength: f32,
    pub threshold: f32,
    pub knee: f32,
    pub knee_width: f32,
    pub soft: f32,
    pub radius: u32,
    pub spread: f32,
    pub down: u32,
    pub clamp: f32,
    pub unit: f32,
    pub radii: [f32; 4],
    pub gains: [f32; 4],
    pub linear_taps: bool,
}

impl Bloom {
    pub fn neutral(strength: f32) -> Self {
        Self::pyramid(strength, 1.0, 0.0, 0.0, 0.0, 4, 16.0, 2)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn pyramid(
        strength: f32,
        threshold: f32,
        knee: f32,
        knee_width: f32,
        soft: f32,
        radius: u32,
        spread: f32,
        down: u32,
    ) -> Self {
        Self {
            kind: BloomKind::Box,
            strength,
            threshold,
            knee,
            knee_width,
            soft,
            radius,
            spread,
            down,
            clamp: 4.0,
            unit: 1.0,
            radii: NEUTRAL_RADII,
            gains: NEUTRAL_GAINS,
            linear_taps: true,
        }
    }

    pub fn tent(strength: f32, threshold: f32, radius: u32, spread: f32, down: u32) -> Self {
        Self {
            kind: BloomKind::Tent,
            strength,
            threshold,
            knee: 0.0,
            knee_width: 0.0,
            soft: 0.0,
            radius,
            spread,
            down,
            clamp: 4.0,
            unit: 1.0,
            radii: NEUTRAL_RADII,
            gains: NEUTRAL_GAINS,
            linear_taps: false,
        }
    }

    pub fn rings(
        strength: f32,
        threshold: f32,
        down: u32,
        radii: [f32; 4],
        gains: [f32; 4],
    ) -> Self {
        Self {
            kind: BloomKind::Rings,
            strength,
            threshold,
            knee: 0.0,
            knee_width: 0.0,
            soft: 0.0,
            radius: 0,
            spread: 1.0,
            down,
            clamp: 4.0,
            unit: 1.0,
            radii,
            gains,
            linear_taps: false,
        }
    }
}

pub fn apply(image: &Image, bloom: &Bloom) -> Image {
    if bloom.strength.abs() < 1e-8 {
        return image.clone();
    }
    match bloom.kind {
        BloomKind::Rings => rings(image, bloom),
        BloomKind::Box | BloomKind::Tent => pyramid(image, bloom),
    }
}

fn pyramid(image: &Image, bloom: &Bloom) -> Image {
    let filter = if bloom.kind == BloomKind::Tent {
        Downsample::Tent
    } else {
        Downsample::Box
    };
    let small = grade::downsample(image, bloom.down.max(1), filter);
    let bright = small
        .map_rgb(|_, _, c| keep_rgb(c, bloom.threshold, bloom.knee, bloom.knee_width, bloom.soft));
    let blurred = grade::separable(&bright, bloom.radius, bloom.spread);
    add(image, &blurred, bloom.strength)
}

pub fn add(image: &Image, bloom: &Image, strength: f32) -> Image {
    let bw = bloom.width as f32;
    let bh = bloom.height as f32;
    let w = image.width.max(1) as f32;
    let h = image.height.max(1) as f32;
    let mut out = image.clone();
    for y in 0..image.height {
        for x in 0..image.width {
            let u = (x as f32 + 0.5) / w;
            let v = (y as f32 + 0.5) / h;
            let s = bloom.sample(u * bw - 0.5, v * bh - 0.5);
            let i = image.index(x, y);
            let p = out.pixels[i];
            out.pixels[i] = [
                p[0] + s[0] * strength,
                p[1] + s[1] * strength,
                p[2] + s[2] * strength,
                p[3],
            ];
        }
    }
    out
}

fn rings(image: &Image, bloom: &Bloom) -> Image {
    let factor = bloom.down.max(1);
    let small = grade::downsample(image, factor, Downsample::Tent).map_rgb(|_, _, c| {
        std::array::from_fn(|ch| {
            let value = (c[ch] - bloom.threshold).max(0.0);
            if bloom.clamp > 0.0 {
                value.min(bloom.clamp)
            } else {
                value
            }
        })
    });
    let mut blurred = small.clone();
    for y in 0..small.height {
        for x in 0..small.width {
            let mut acc = [0.0; 3];
            let mut weight = 0.0;
            for ring in 0..4 {
                let turn = ring as f32 * RING_TURN;
                let radius = bloom.radii[ring] * bloom.unit / factor as f32;
                let gain = bloom.gains[ring];
                for i in 0..RING_SAMPLES {
                    let a = i as f32 * std::f32::consts::TAU / RING_SAMPLES as f32 + turn;
                    let s = small.sample(x as f32 + a.cos() * radius, y as f32 + a.sin() * radius);
                    for ch in 0..3 {
                        acc[ch] += s[ch] * gain;
                    }
                }
                weight += RING_SAMPLES as f32 * gain;
            }
            if weight > 1e-8 {
                let i = small.index(x, y);
                blurred.pixels[i] = [acc[0] / weight, acc[1] / weight, acc[2] / weight, 1.0];
            }
        }
    }
    add(image, &blurred, bloom.strength)
}

pub fn linear_taps(radius: u32, spread: f32) -> (f64, Vec<(f64, f64)>) {
    let radius = radius.min(64);
    let spread = if spread <= 1e-4 {
        1.0
    } else {
        f64::from(spread)
    };
    let raw: Vec<f64> = (0..=radius)
        .map(|k| (-f64::from(k * k) / spread).exp())
        .collect();
    let total = raw[0] + 2.0 * raw[1..].iter().sum::<f64>();
    let mut taps = Vec::new();
    let mut k = 1;
    while k <= radius as usize {
        if k < radius as usize {
            let (left, right) = (raw[k], raw[k + 1]);
            let sum = left + right;
            taps.push((
                sum / total,
                (k as f64 * left + (k + 1) as f64 * right) / sum,
            ));
            k += 2;
        } else {
            taps.push((raw[k] / total, k as f64));
            k += 1;
        }
    }
    (raw[0] / total, taps)
}
