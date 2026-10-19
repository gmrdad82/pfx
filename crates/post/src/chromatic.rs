use crate::color;
use crate::image::Image;

pub const AMOUNT: f32 = 0.02;
pub const MAX_SAMPLES: u32 = 8;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Chromatic {
    pub amount: f32,
    pub falloff: f32,
}

impl Default for Chromatic {
    fn default() -> Self {
        Self {
            amount: AMOUNT,
            falloff: 0.0,
        }
    }
}

impl Chromatic {
    pub const MOST: f32 = 0.25;
    pub const FALLOFF_MOST: f32 = 8.0;

    pub fn is_off(&self) -> bool {
        self.amount <= 0.0
    }

    pub fn reach(&self, pixel: [f32; 2], full: [u32; 2]) -> [f32; 2] {
        let centre = full.map(|side| side.saturating_sub(1) as f32 * 0.5);
        let corner = (centre[0] * centre[0] + centre[1] * centre[1])
            .sqrt()
            .max(1e-4);
        let to = [centre[0] - pixel[0], centre[1] - pixel[1]];
        let r = (to[0] * to[0] + to[1] * to[1]).sqrt() / corner;
        let k = self.amount * color::safe_pow(r, self.falloff);
        [to[0] * k, to[1] * k]
    }
}

pub fn spectrum(t: f32) -> [f32; 3] {
    let s = 2.0 * t;
    [
        (1.0 - s).max(0.0),
        (1.0 - (s - 1.0).abs()).max(0.0),
        (s - 1.0).max(0.0),
    ]
}

pub fn apply(image: &Image, pass: &Chromatic) -> Image {
    apply_at(image, pass, [0, 0], [image.width, image.height])
}

pub fn apply_at(image: &Image, pass: &Chromatic, origin: [u32; 2], full: [u32; 2]) -> Image {
    if pass.is_off() || image.width == 0 || image.height == 0 {
        return image.clone();
    }
    Image::from_fn_banded(image.width, image.height, |x, y| {
        pixel(image, pass, x, y, origin, full)
    })
}

fn pixel(
    image: &Image,
    pass: &Chromatic,
    x: u32,
    y: u32,
    origin: [u32; 2],
    full: [u32; 2],
) -> [f32; 4] {
    let src = image.at(x, y);
    let end = pass.reach([(origin[0] + x) as f32, (origin[1] + y) as f32], full);
    let count = ((end[0] * end[0] + end[1] * end[1]).sqrt().ceil() as u32).min(MAX_SAMPLES);
    if count <= 1 {
        return src;
    }
    let mut sum = [0.0f32; 3];
    let mut weight = [0.0f32; 3];
    for i in 0..count {
        let t = (i as f32 + 0.5) / count as f32;
        let m = spectrum(t);
        let s = image.sample(x as f32 + end[0] * t, y as f32 + end[1] * t);
        for c in 0..3 {
            sum[c] += s[c] * m[c];
            weight[c] += m[c];
        }
    }
    [
        sum[0] / weight[0],
        sum[1] / weight[1],
        sum[2] / weight[2],
        src[3],
    ]
}
