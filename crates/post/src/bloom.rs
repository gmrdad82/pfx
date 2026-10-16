use crate::grade::{self, Downsample, keep_rgb};
use crate::image::Image;

pub const RING_SAMPLES: u32 = 20;
pub const RING_TURN: f32 = 0.37;

pub const NEUTRAL_RADII: [f32; 4] = [1.0, 2.0, 4.0, 8.0];
pub const NEUTRAL_GAINS: [f32; 4] = [1.0, 0.5, 0.25, 0.125];

pub const ROUND_MIP_HEIGHT: u32 = 512;
pub const ROUND_MIPS: usize = 8;
pub const ROUND_LEAD: usize = 2;
pub const ROUND_LEVELS: usize = ROUND_LEAD + ROUND_MIPS;
pub const LOW_FREQUENCY_BOOST: f32 = 0.7;
pub const LOW_FREQUENCY_BOOST_CURVATURE: f32 = 0.95;
pub const HIGH_PASS_FREQUENCY: f32 = 1.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BloomKind {
    Box,
    Tent,
    Rings,
    Round,
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

    pub fn round(strength: f32, threshold: f32) -> Self {
        Self {
            kind: BloomKind::Round,
            strength,
            threshold,
            knee: 0.0,
            knee_width: 0.0,
            soft: 0.0,
            radius: 0,
            spread: 1.0,
            down: 1,
            clamp: 0.0,
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
        BloomKind::Round => round(image, bloom),
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

pub fn intensity(bloom: &Bloom) -> f32 {
    bloom.strength.clamp(0.0, 1.0)
}

pub fn blend_factor(intensity: f32, mip: usize) -> f32 {
    let max_mip = (ROUND_MIPS - 1) as f32;
    let t = mip as f32 / max_mip;
    let lf_boost = (1.0 - (1.0 - t).powf(1.0 / (1.0 - LOW_FREQUENCY_BOOST_CURVATURE)))
        * LOW_FREQUENCY_BOOST
        * (1.0 - intensity);
    let high_pass = 1.0 - ((t - HIGH_PASS_FREQUENCY) / HIGH_PASS_FREQUENCY).clamp(0.0, 1.0);
    (intensity + lf_boost) * high_pass
}

pub fn round_levels(width: u32, height: u32) -> [(u32, u32); ROUND_LEVELS] {
    let (width, height) = (width.max(1), height.max(1));
    let ratio = ROUND_MIP_HEIGHT as f32 / height as f32;
    let mip = (
        ((width as f32 * ratio).round() as u32).max(1),
        ROUND_MIP_HEIGHT,
    );
    let lead = |times: u32| {
        if mip.1 * times >= height {
            (width, height)
        } else {
            (mip.0 * times, mip.1 * times)
        }
    };
    let mut out = [(1, 1); ROUND_LEVELS];
    out[0] = lead(4);
    out[1] = lead(2);
    for (i, slot) in out[ROUND_LEAD..].iter_mut().enumerate() {
        *slot = ((mip.0 >> i).max(1), (mip.1 >> i).max(1));
    }
    out
}

pub fn scaled_level(level: (u32, u32), capacity: (u32, u32), viewport: (u32, u32)) -> (u32, u32) {
    let scale = |n: u32, from: u32, to: u32| {
        ((u64::from(n) * u64::from(to)).div_ceil(u64::from(from.max(1))) as u32).clamp(1, n)
    };
    (
        scale(level.0, capacity.0, viewport.0),
        scale(level.1, capacity.1, viewport.1),
    )
}

const TAPS_13: [(f32, f32); 13] = [
    (-2.0, 2.0),
    (0.0, 2.0),
    (2.0, 2.0),
    (-2.0, 0.0),
    (0.0, 0.0),
    (2.0, 0.0),
    (-2.0, -2.0),
    (0.0, -2.0),
    (2.0, -2.0),
    (-1.0, 1.0),
    (1.0, 1.0),
    (-1.0, -1.0),
    (1.0, -1.0),
];

fn tap(image: &Image, x: f32, y: f32) -> [f32; 3] {
    let s = image.sample(x - 0.5, y - 0.5);
    [s[0], s[1], s[2]]
}

fn karis_weight(c: [f32; 3]) -> f32 {
    let g = c.map(|v| v.max(0.0).powf(1.0 / 2.2));
    let luma = (0.2126 * g[0] + 0.7152 * g[1] + 0.0722 * g[2]) / 4.0;
    1.0 / (1.0 + luma)
}

fn scale3(c: [f32; 3], k: f32) -> [f32; 3] {
    [c[0] * k, c[1] * k, c[2] * k]
}

fn sum3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sample_13_tap(image: &Image, x: f32, y: f32, karis: bool) -> [f32; 3] {
    let s: [[f32; 3]; 13] = std::array::from_fn(|i| tap(image, x + TAPS_13[i].0, y + TAPS_13[i].1));
    let [a, b, c, d, e, f, g, h, i, j, k, l, m] = s;
    if karis {
        let groups = [
            (scale3(sum3(sum3(a, b), sum3(d, e)), 0.25), 0.125),
            (scale3(sum3(sum3(b, c), sum3(e, f)), 0.25), 0.125),
            (scale3(sum3(sum3(d, e), sum3(g, h)), 0.25), 0.125),
            (scale3(sum3(sum3(e, f), sum3(h, i)), 0.25), 0.125),
            (scale3(sum3(sum3(j, k), sum3(l, m)), 0.25), 0.5),
        ];
        let mut acc = [0.0; 3];
        let mut total = 0.0;
        for (group, weight) in groups {
            let w = weight * karis_weight(group);
            acc = sum3(acc, scale3(group, w));
            total += w;
        }
        return scale3(acc, 1.0 / total);
    }
    let corners = scale3(sum3(sum3(a, c), sum3(g, i)), 0.03125);
    let edges = scale3(sum3(sum3(b, d), sum3(f, h)), 0.0625);
    let centre = scale3(sum3(sum3(sum3(e, j), sum3(k, l)), m), 0.125);
    sum3(sum3(corners, edges), centre)
}

fn sample_tent(image: &Image, x: f32, y: f32) -> [f32; 3] {
    let mut acc = [0.0; 3];
    for (dx, dy, w) in [
        (0.0, 0.0, 0.25),
        (0.0, 1.0, 0.125),
        (-1.0, 0.0, 0.125),
        (1.0, 0.0, 0.125),
        (0.0, -1.0, 0.125),
        (-1.0, 1.0, 0.0625),
        (1.0, 1.0, 0.0625),
        (-1.0, -1.0, 0.0625),
        (1.0, -1.0, 0.0625),
    ] {
        acc = sum3(acc, scale3(tap(image, x + dx, y + dy), w));
    }
    acc
}

fn centre_in(x: u32, y: u32, target: (u32, u32), source: &Image) -> (f32, f32) {
    (
        (x as f32 + 0.5) / target.0 as f32 * source.width as f32,
        (y as f32 + 0.5) / target.1 as f32 * source.height as f32,
    )
}

fn round_down(source: &Image, size: (u32, u32), first: Option<&Bloom>) -> Image {
    Image::from_fn(size.0, size.1, |x, y| {
        let (cx, cy) = centre_in(x, y, size, source);
        let c = match first {
            Some(bloom) => {
                let c = sample_13_tap(source, cx, cy, true).map(|v| v.clamp(1e-4, 3.402_823_5e37));
                keep_rgb(c, bloom.threshold, bloom.knee, bloom.knee_width, bloom.soft)
            }
            None => sample_13_tap(source, cx, cy, false),
        };
        [c[0], c[1], c[2], 1.0]
    })
}

fn round_up(coarse: &Image, fine: &Image, blend: f32) -> Image {
    let size = (fine.width, fine.height);
    Image::from_fn(size.0, size.1, |x, y| {
        let (cx, cy) = centre_in(x, y, size, coarse);
        let t = sample_tent(coarse, cx, cy);
        let d = fine.at(x, y);
        let mut out = [0.0, 0.0, 0.0, 1.0];
        for ch in 0..3 {
            out[ch] = d[ch] * (1.0 - blend) + t[ch] * blend;
        }
        out
    })
}

pub fn round_glow(image: &Image, bloom: &Bloom) -> Image {
    let levels = round_levels(image.width, image.height);
    let mut downs: Vec<Image> = Vec::with_capacity(ROUND_LEVELS);
    downs.push(round_down(image, levels[0], Some(bloom)));
    for size in &levels[1..] {
        let next = round_down(downs.last().unwrap(), *size, None);
        downs.push(next);
    }
    let strength = intensity(bloom);
    let mut up = downs[ROUND_LEVELS - 1].clone();
    for mip in (1..ROUND_MIPS).rev() {
        up = round_up(
            &up,
            &downs[ROUND_LEAD + mip - 1],
            blend_factor(strength, mip),
        );
    }
    up
}

fn round(image: &Image, bloom: &Bloom) -> Image {
    let glow = round_glow(image, bloom);
    let c0 = blend_factor(intensity(bloom), 0);
    let size = (image.width, image.height);
    let mut out = image.clone();
    for y in 0..image.height {
        for x in 0..image.width {
            let (cx, cy) = centre_in(x, y, size, &glow);
            let t = sample_tent(&glow, cx, cy);
            let i = image.index(x, y);
            let p = image.pixels[i];
            let kept = keep_rgb(
                [p[0], p[1], p[2]],
                bloom.threshold,
                bloom.knee,
                bloom.knee_width,
                bloom.soft,
            );
            out.pixels[i] = [
                p[0] + (t[0] - kept[0]) * c0,
                p[1] + (t[1] - kept[1]) * c0,
                p[2] + (t[2] - kept[2]) * c0,
                p[3],
            ];
        }
    }
    out
}
