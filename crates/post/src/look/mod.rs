pub mod gpu;
pub mod shader;
#[cfg(test)]
mod tests;

pub use gpu::LookGpu;

pub const GRADIENT_STOPS: usize = 8;
pub const PALETTE_MIN: usize = 2;
pub const PALETTE_MAX: usize = 256;

fn encode(c: f32) -> f32 {
    pfx_materials::encode_channel(c.clamp(0.0, 1.0))
}

pub fn oklab(encoded: [f32; 3]) -> [f32; 3] {
    let [r, g, b] = encoded.map(pfx_materials::linear_channel);
    let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

pub fn srgb(lab: [f32; 3]) -> [f32; 3] {
    let [l, a, b] = lab;
    let l_ = l + 0.396_337_78 * a + 0.215_803_76 * b;
    let m_ = l - 0.105_561_346 * a - 0.063_854_17 * b;
    let s_ = l - 0.089_484_18 * a - 1.291_485_5 * b;
    let (l, m, s) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
    [
        4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s,
        -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s,
        -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s,
    ]
    .map(encode)
}

pub fn mix_oklab(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    let la = oklab([a[0], a[1], a[2]]);
    let lb = oklab([b[0], b[1], b[2]]);
    let [r, g, bl] = srgb(std::array::from_fn(|i| la[i] + (lb[i] - la[i]) * t));
    [r, g, bl, a[3] + (b[3] - a[3]) * t]
}

fn premultiply(c: [f32; 4]) -> [f32; 4] {
    [c[0] * c[3], c[1] * c[3], c[2] * c[3], c[3]]
}

fn over(top: [f32; 4], under: [f32; 4]) -> [f32; 4] {
    std::array::from_fn(|i| top[i] + under[i] * (1.0 - top[3]))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    pub layout: [f32; 2],
    pub scale: f32,
    pub offset: [f32; 2],
}

impl Place {
    pub fn area(&self, size: [u32; 2]) -> [f32; 4] {
        let [x0, y0] = self.to_layout([0.0, 0.0]);
        let [x1, y1] = self.to_layout([size[0] as f32, size[1] as f32]);
        [x0, y0, x1, y1]
    }

    pub fn to_layout(&self, pixel: [f32; 2]) -> [f32; 2] {
        [
            (pixel[0] - self.offset[0]) / self.scale,
            (pixel[1] - self.offset[1]) / self.scale,
        ]
    }

    pub fn to_target(&self, point: [f32; 2]) -> [f32; 2] {
        [
            point[0] * self.scale + self.offset[0],
            point[1] * self.scale + self.offset[1],
        ]
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Palette {
    colours: Vec<[f32; 3]>,
    lab: Vec<[f32; 3]>,
}

impl Palette {
    pub fn new(colours: Vec<[f32; 3]>) -> Result<Self, String> {
        if !(PALETTE_MIN..=PALETTE_MAX).contains(&colours.len()) {
            return Err(format!(
                "a palette holds {PALETTE_MIN} to {PALETTE_MAX} colours, not {}",
                colours.len()
            ));
        }
        if colours
            .iter()
            .flatten()
            .any(|value| !(0.0..=1.0).contains(value))
        {
            return Err("palette colours are sRGB-encoded values in 0..=1".into());
        }
        let lab = colours.iter().map(|c| oklab(*c)).collect();
        Ok(Self { colours, lab })
    }

    pub fn hex(colours: &[u32]) -> Result<Self, String> {
        Self::new(
            colours
                .iter()
                .map(|rgb| {
                    [
                        ((rgb >> 16) & 255) as f32 / 255.0,
                        ((rgb >> 8) & 255) as f32 / 255.0,
                        (rgb & 255) as f32 / 255.0,
                    ]
                })
                .collect(),
        )
    }

    pub fn cube(levels: u32) -> Result<Self, String> {
        if !(2..=6).contains(&levels) {
            return Err("a colour cube takes 2 to 6 levels a channel".into());
        }
        let step = |i: u32| i as f32 / (levels - 1) as f32;
        let mut colours = Vec::new();
        for r in 0..levels {
            for g in 0..levels {
                for b in 0..levels {
                    colours.push([step(r), step(g), step(b)]);
                }
            }
        }
        Self::new(colours)
    }

    pub fn colours(&self) -> &[[f32; 3]] {
        &self.colours
    }

    pub fn len(&self) -> usize {
        self.colours.len()
    }

    pub fn is_empty(&self) -> bool {
        self.colours.is_empty()
    }

    pub fn nearest_lab(&self, lab: [f32; 3]) -> usize {
        let mut best = 0;
        let mut best_distance = f32::MAX;
        for (index, entry) in self.lab.iter().enumerate() {
            let d = [entry[0] - lab[0], entry[1] - lab[1], entry[2] - lab[2]];
            let distance = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
            if distance < best_distance {
                best_distance = distance;
                best = index;
            }
        }
        best
    }

    pub fn nearest(&self, rgb: [f32; 3]) -> usize {
        self.nearest_lab(oklab(rgb))
    }

    pub fn packed(&self) -> Vec<[f32; 4]> {
        let mut order: Vec<usize> = (0..self.colours.len()).collect();
        order.sort_by(|&a, &b| self.lab[a][0].total_cmp(&self.lab[b][0]).then(a.cmp(&b)));
        let mut out = vec![[0.0; 4]; 2 * PALETTE_MAX];
        for (at, &index) in order.iter().enumerate() {
            let lab = self.lab[index];
            let rgb = self.colours[index];
            out[at] = [lab[0], lab[1], lab[2], index as f32];
            out[PALETTE_MAX + at] = [rgb[0], rgb[1], rgb[2], 1.0];
        }
        out
    }
}

pub fn bayer_value(order: u32, x: u32, y: u32) -> u32 {
    let bits = order.trailing_zeros();
    let mut value = 0;
    for i in 0..bits {
        let xb = (x >> i) & 1;
        let yb = (y >> i) & 1;
        value += ((xb ^ yb) * 2 + yb) << (2 * (bits - 1 - i));
    }
    value
}

pub fn bayer_matrix(order: u32) -> Option<Vec<u32>> {
    matches!(order, 2 | 4 | 8).then(|| {
        (0..order * order)
            .map(|i| bayer_value(order, i % order, i / order))
            .collect()
    })
}

pub fn threshold(order: u32, x: u32, y: u32) -> f32 {
    (bayer_value(order, x % order, y % order) as f32 + 0.5) / (order * order) as f32
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Dither {
    #[default]
    None,
    Bayer {
        order: u32,
        spread: f32,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Quantise {
    pub palette: Palette,
    pub dither: Dither,
}

impl Quantise {
    pub fn new(palette: Palette, dither: Dither) -> Result<Self, String> {
        if let Dither::Bayer { order, spread } = dither {
            if !matches!(order, 2 | 4 | 8) {
                return Err("a Bayer dither has order 2, 4 or 8".into());
            }
            if !(spread.is_finite() && spread >= 0.0) {
                return Err("a dither's spread is finite and nonnegative".into());
            }
        }
        Ok(Self { palette, dither })
    }

    pub fn pixel(&self, premultiplied: [f32; 4], at: [u32; 2]) -> [f32; 4] {
        let a = premultiplied[3];
        if a <= 0.0 {
            return [0.0; 4];
        }
        let mut lab = oklab([
            premultiplied[0] / a,
            premultiplied[1] / a,
            premultiplied[2] / a,
        ]);
        if let Dither::Bayer { order, spread } = self.dither {
            lab[0] += (threshold(order, at[0], at[1]) - 0.5) * spread;
        }
        let [r, g, b] = self.palette.colours[self.palette.nearest_lab(lab)];
        [r * a, g * a, b * a, a]
    }

    pub fn image(&self, pixels: &[[f32; 4]], size: [u32; 2]) -> Vec<[f32; 4]> {
        pixels
            .iter()
            .enumerate()
            .map(|(i, p)| self.pixel(*p, [i as u32 % size[0], i as u32 / size[0]]))
            .collect()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Upscale {
    #[default]
    Integer,
    Fit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UpscaleMap {
    pub source: [u32; 2],
    pub target: [u32; 2],
    pub offset: [u32; 2],
    pub extent: [u32; 2],
}

impl UpscaleMap {
    pub fn new(source: [u32; 2], target: [u32; 2], mode: Upscale) -> Result<Self, String> {
        if source.contains(&0) || target.contains(&0) {
            return Err("an upscale needs a nonempty source and target".into());
        }
        let whole = (target[0] / source[0]).min(target[1] / source[1]);
        let extent = if mode == Upscale::Integer && whole >= 1 {
            [source[0] * whole, source[1] * whole]
        } else if u64::from(target[0]) * u64::from(source[1])
            <= u64::from(target[1]) * u64::from(source[0])
        {
            let h = (2 * u64::from(source[1]) * u64::from(target[0]) + u64::from(source[0]))
                / (2 * u64::from(source[0]));
            [target[0], (h as u32).clamp(1, target[1])]
        } else {
            let w = (2 * u64::from(source[0]) * u64::from(target[1]) + u64::from(source[1]))
                / (2 * u64::from(source[1]));
            [(w as u32).clamp(1, target[0]), target[1]]
        };
        Ok(Self {
            source,
            target,
            offset: [(target[0] - extent[0]) / 2, (target[1] - extent[1]) / 2],
            extent,
        })
    }

    pub fn source_pixel(&self, pixel: [u32; 2]) -> Option<[u32; 2]> {
        let inside = |axis: usize| {
            pixel[axis] >= self.offset[axis] && pixel[axis] < self.offset[axis] + self.extent[axis]
        };
        if !(inside(0) && inside(1)) {
            return None;
        }
        Some(std::array::from_fn(|axis| {
            (pixel[axis] - self.offset[axis]) * self.source[axis] / self.extent[axis]
        }))
    }

    pub fn scale(&self) -> [f32; 2] {
        [
            self.extent[0] as f32 / self.source[0] as f32,
            self.extent[1] as f32 / self.source[1] as f32,
        ]
    }

    pub fn image(&self, pixels: &[[f32; 4]], clear: [f32; 4]) -> Vec<[f32; 4]> {
        let mut out = Vec::with_capacity((self.target[0] * self.target[1]) as usize);
        for y in 0..self.target[1] {
            for x in 0..self.target[0] {
                out.push(self.source_pixel([x, y]).map_or(clear, |[sx, sy]| {
                    pixels[(sy * self.source[0] + sx) as usize]
                }));
            }
        }
        out
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GradientShape {
    Linear { from: [f32; 2], to: [f32; 2] },
    Radial { centre: [f32; 2], radius: [f32; 2] },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Interpolation {
    #[default]
    Srgb,
    Oklab,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gradient {
    pub shape: GradientShape,
    pub space: Interpolation,
    count: u8,
    offsets: [f32; GRADIENT_STOPS],
    colours: [[f32; 4]; GRADIENT_STOPS],
}

impl Gradient {
    pub fn new(shape: GradientShape, stops: &[(f32, [f32; 4])]) -> Result<Self, String> {
        if stops.is_empty() || stops.len() > GRADIENT_STOPS {
            return Err(format!(
                "a gradient takes 1 to {GRADIENT_STOPS} stops, not {}",
                stops.len()
            ));
        }
        let finite = |v: &[f32]| v.iter().all(|value| value.is_finite());
        let geometry_ok = match shape {
            GradientShape::Linear { from, to } => finite(&from) && finite(&to),
            GradientShape::Radial { centre, radius } => {
                finite(&centre) && finite(&radius) && radius[0] > 0.0 && radius[1] > 0.0
            }
        };
        if !geometry_ok {
            return Err("a gradient's geometry must be finite, with positive radii".into());
        }
        let mut offsets = [1.0; GRADIENT_STOPS];
        let mut colours = [[0.0; 4]; GRADIENT_STOPS];
        let mut previous = 0.0f32;
        for (index, (offset, colour)) in stops.iter().enumerate() {
            if !offset.is_finite() || !finite(colour) {
                return Err("gradient stops must be finite".into());
            }
            previous = offset.clamp(0.0, 1.0).max(previous);
            offsets[index] = previous;
            colours[index] = colour.map(|value| value.clamp(0.0, 1.0));
        }
        Ok(Self {
            shape,
            space: Interpolation::Srgb,
            count: stops.len() as u8,
            offsets,
            colours,
        })
    }

    pub fn linear(from: [f32; 2], to: [f32; 2], stops: &[(f32, [f32; 4])]) -> Result<Self, String> {
        Self::new(GradientShape::Linear { from, to }, stops)
    }

    pub fn radial(
        centre: [f32; 2],
        radius: [f32; 2],
        stops: &[(f32, [f32; 4])],
    ) -> Result<Self, String> {
        Self::new(GradientShape::Radial { centre, radius }, stops)
    }

    pub fn space(mut self, space: Interpolation) -> Self {
        self.space = space;
        self
    }

    pub fn stops(&self) -> impl Iterator<Item = (f32, [f32; 4])> + '_ {
        (0..usize::from(self.count)).map(|i| (self.offsets[i], self.colours[i]))
    }

    pub fn max_alpha(&self) -> f32 {
        self.stops().map(|(_, c)| c[3]).fold(0.0, f32::max)
    }

    pub fn geometry(&self) -> [f32; 4] {
        match self.shape {
            GradientShape::Linear { from, to } => [from[0], from[1], to[0], to[1]],
            GradientShape::Radial { centre, radius } => {
                [centre[0], centre[1], radius[0], radius[1]]
            }
        }
    }

    pub fn header(&self) -> [f32; 4] {
        [
            f32::from(self.count),
            match self.shape {
                GradientShape::Linear { .. } => 0.0,
                GradientShape::Radial { .. } => 1.0,
            },
            match self.space {
                Interpolation::Srgb => 0.0,
                Interpolation::Oklab => 1.0,
            },
            self.max_alpha(),
        ]
    }

    pub fn packed(&self) -> [[f32; 4]; 12] {
        let mut out = [[0.0; 4]; 12];
        out[..GRADIENT_STOPS].copy_from_slice(&self.colours);
        out[8] = [
            self.offsets[0],
            self.offsets[1],
            self.offsets[2],
            self.offsets[3],
        ];
        out[9] = [
            self.offsets[4],
            self.offsets[5],
            self.offsets[6],
            self.offsets[7],
        ];
        out[10] = self.geometry();
        out[11] = self.header();
        out
    }

    pub fn position(&self, p: [f32; 2]) -> f32 {
        let t = match self.shape {
            GradientShape::Linear { from, to } => {
                let d = [to[0] - from[0], to[1] - from[1]];
                ((p[0] - from[0]) * d[0] + (p[1] - from[1]) * d[1])
                    / (d[0] * d[0] + d[1] * d[1]).max(1e-12)
            }
            GradientShape::Radial { centre, radius } => {
                let q = [
                    (p[0] - centre[0]) / radius[0],
                    (p[1] - centre[1]) / radius[1],
                ];
                (q[0] * q[0] + q[1] * q[1]).sqrt()
            }
        };
        t.clamp(0.0, 1.0)
    }

    pub fn colour(&self, t: f32) -> [f32; 4] {
        let n = usize::from(self.count);
        if t <= self.offsets[0] {
            return self.colours[0];
        }
        for i in 0..n - 1 {
            if t < self.offsets[i + 1] {
                let f = (t - self.offsets[i]) / (self.offsets[i + 1] - self.offsets[i]);
                let (a, b) = (self.colours[i], self.colours[i + 1]);
                return match self.space {
                    Interpolation::Srgb => std::array::from_fn(|c| a[c] + (b[c] - a[c]) * f),
                    Interpolation::Oklab => mix_oklab(a, b, f),
                };
            }
        }
        self.colours[n - 1]
    }

    pub fn paint(&self, p: [f32; 2]) -> [f32; 4] {
        premultiply(self.colour(self.position(p)))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Major {
    pub every: u32,
    pub width: f32,
    pub colour: [f32; 4],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grid {
    pub spacing: [f32; 2],
    pub origin: [f32; 2],
    pub width: f32,
    pub colour: [f32; 4],
    pub major: Option<Major>,
}

impl Grid {
    pub fn new(spacing: f32, width: f32, colour: [f32; 4]) -> Self {
        Self {
            spacing: [spacing; 2],
            origin: [0.0; 2],
            width,
            colour,
            major: None,
        }
    }

    pub fn major(mut self, every: u32, width: f32, colour: [f32; 4]) -> Self {
        self.major = Some(Major {
            every,
            width,
            colour,
        });
        self
    }

    fn cover(distance: f32, width: f32, scale: f32) -> f32 {
        (width * scale * 0.5 - distance * scale + 0.5).clamp(0.0, 1.0)
    }

    pub fn paint(&self, p: [f32; 2], scale: f32) -> [f32; 4] {
        let mut minor = 0.0f32;
        let mut major = 0.0f32;
        for ((&at, &spacing), &origin) in p.iter().zip(&self.spacing).zip(&self.origin) {
            if spacing.is_nan() || spacing <= 0.0 {
                continue;
            }
            let u = (at - origin) / spacing;
            let d = (u - (u + 0.5).floor()).abs() * spacing;
            minor = minor.max(Self::cover(d, self.width, scale));
            if let Some(m) = self.major.filter(|m| m.every > 0) {
                let period = spacing * m.every as f32;
                let v = (at - origin) / period;
                let d = (v - (v + 0.5).floor()).abs() * period;
                major = major.max(Self::cover(d, m.width, scale));
            }
        }
        let paint = premultiply(self.colour).map(|value| value * minor);
        match self.major {
            Some(m) => over(premultiply(m.colour).map(|value| value * major), paint),
            None => paint,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Backdrop {
    pub colour: Option<[f32; 4]>,
    pub gradient: Option<Gradient>,
    pub grid: Option<Grid>,
}

impl Backdrop {
    pub fn paint(&self, p: [f32; 2], scale: f32) -> [f32; 4] {
        let mut base = self.colour.map_or([0.0; 4], premultiply);
        if let Some(gradient) = &self.gradient {
            base = over(gradient.paint(p), base);
        }
        if let Some(grid) = &self.grid {
            base = over(grid.paint(p, scale), base);
        }
        base
    }

    pub fn under(&self, pixels: &[[f32; 4]], size: [u32; 2], place: &Place) -> Vec<[f32; 4]> {
        pixels
            .iter()
            .enumerate()
            .map(|(i, top)| {
                let pixel = [
                    (i as u32 % size[0]) as f32 + 0.5,
                    (i as u32 / size[0]) as f32 + 0.5,
                ];
                over(*top, self.paint(place.to_layout(pixel), place.scale))
            })
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vignette {
    pub centre: [f32; 2],
    pub radius: [f32; 2],
    pub falloff: f32,
    pub floor: f32,
    pub colour: [f32; 4],
}

impl Vignette {
    pub fn light(&self, p: [f32; 2]) -> f32 {
        let q = [
            (p[0] - self.centre[0]) / self.radius[0].max(1e-6),
            (p[1] - self.centre[1]) / self.radius[1].max(1e-6),
        ];
        let e = (q[0] * q[0] + q[1] * q[1]).sqrt();
        let t = ((e - 1.0) / self.falloff.max(1e-6)).clamp(0.0, 1.0);
        let s = t * t * (3.0 - 2.0 * t);
        let floor = self.floor.clamp(0.0, 1.0);
        1.0 - (1.0 - floor) * s * self.colour[3].clamp(0.0, 1.0)
    }

    pub fn pixel(&self, c: [f32; 4], p: [f32; 2]) -> [f32; 4] {
        let light = self.light(p);
        [
            c[0] * light + self.colour[0] * c[3] * (1.0 - light),
            c[1] * light + self.colour[1] * c[3] * (1.0 - light),
            c[2] * light + self.colour[2] * c[3] * (1.0 - light),
            c[3],
        ]
    }

    pub fn image(&self, pixels: &[[f32; 4]], size: [u32; 2], place: &Place) -> Vec<[f32; 4]> {
        pixels
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let pixel = [
                    (i as u32 % size[0]) as f32 + 0.5,
                    (i as u32 / size[0]) as f32 + 0.5,
                ];
                self.pixel(*c, place.to_layout(pixel))
            })
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scanlines {
    pub period: f32,
    pub strength: f32,
}

impl Scanlines {
    pub fn gain(&self, y: f32) -> f32 {
        let phase = std::f32::consts::TAU * y / self.period.max(1e-6);
        1.0 - self.strength.clamp(0.0, 1.0) * (0.5 + 0.5 * phase.cos())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glow {
    pub threshold: f32,
    pub radius: f32,
    pub strength: f32,
    pub tint: [f32; 4],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Curvature {
    pub amount: f32,
    pub corner: f32,
    pub bezel: [f32; 4],
}

impl Curvature {
    pub fn source(&self, pixel: [f32; 2], size: [u32; 2]) -> Option<[f32; 2]> {
        let size = [size[0] as f32, size[1] as f32];
        let uv = [
            pixel[0] / size[0] * 2.0 - 1.0,
            pixel[1] / size[1] * 2.0 - 1.0,
        ];
        let w = [
            uv[0] * (1.0 + self.amount * uv[1] * uv[1]),
            uv[1] * (1.0 + self.amount * uv[0] * uv[0]),
        ];
        (w[0].abs() <= 1.0 && w[1].abs() <= 1.0)
            .then(|| [(w[0] * 0.5 + 0.5) * size[0], (w[1] * 0.5 + 0.5) * size[1]])
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aberration {
    pub shift: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Crt {
    pub scanlines: Option<Scanlines>,
    pub glow: Option<Glow>,
    pub curvature: Option<Curvature>,
    pub aberration: Option<Aberration>,
}

impl Crt {
    pub fn is_empty(&self) -> bool {
        self.scanlines.is_none()
            && self.glow.is_none()
            && self.curvature.is_none()
            && self.aberration.is_none()
    }

    pub fn scanlines_image(
        &self,
        pixels: &[[f32; 4]],
        size: [u32; 2],
        place: &Place,
    ) -> Vec<[f32; 4]> {
        let Some(lines) = self.scanlines else {
            return pixels.to_vec();
        };
        pixels
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let y = (i as u32 / size[0]) as f32 + 0.5;
                let gain = lines.gain(place.to_layout([0.0, y])[1]);
                [c[0] * gain, c[1] * gain, c[2] * gain, c[3]]
            })
            .collect()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Blend {
    #[default]
    Crossfade,
    Wipe {
        angle: f32,
        softness: f32,
    },
}

impl Blend {
    pub fn weight(&self, progress: f32, p: [f32; 2], area: [f32; 4]) -> f32 {
        let progress = progress.clamp(0.0, 1.0);
        match *self {
            Blend::Crossfade => progress,
            Blend::Wipe { angle, softness } => {
                let (front, along) = wipe_front(angle, softness, progress, area);
                let s = p[0] * along[0] + p[1] * along[1];
                let soft = softness.max(1e-4);
                ((front - s) / soft + 0.5).clamp(0.0, 1.0)
            }
        }
    }
}

pub fn wipe_front(angle: f32, softness: f32, progress: f32, area: [f32; 4]) -> (f32, [f32; 2]) {
    let along = [angle.cos(), angle.sin()];
    let corners = [
        [area[0], area[1]],
        [area[2], area[1]],
        [area[0], area[3]],
        [area[2], area[3]],
    ]
    .map(|c| c[0] * along[0] + c[1] * along[1]);
    let low = corners.iter().copied().fold(f32::MAX, f32::min);
    let high = corners.iter().copied().fold(f32::MIN, f32::max);
    let soft = softness.max(1e-4);
    (low - soft * 0.5 + (high - low + soft) * progress, along)
}
