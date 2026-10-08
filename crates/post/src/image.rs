#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<[f32; 4]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageError;

impl std::fmt::Display for ImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("pixel count does not match the image size")
    }
}

impl std::error::Error for ImageError {}

impl Image {
    pub fn new(width: u32, height: u32) -> Self {
        Self::solid(width, height, [0.0, 0.0, 0.0, 1.0])
    }

    pub fn solid(width: u32, height: u32, pixel: [f32; 4]) -> Self {
        let n = (width as usize).saturating_mul(height as usize);
        Self {
            width,
            height,
            pixels: vec![pixel; n],
        }
    }

    pub fn from_pixels(width: u32, height: u32, pixels: Vec<[f32; 4]>) -> Result<Self, ImageError> {
        if pixels.len() != (width as usize).saturating_mul(height as usize) {
            return Err(ImageError);
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    pub fn from_fn(width: u32, height: u32, mut f: impl FnMut(u32, u32) -> [f32; 4]) -> Self {
        let mut pixels = Vec::with_capacity((width as usize).saturating_mul(height as usize));
        for y in 0..height {
            for x in 0..width {
                pixels.push(f(x, y));
            }
        }
        Self {
            width,
            height,
            pixels,
        }
    }

    pub fn index(&self, x: u32, y: u32) -> usize {
        (y * self.width + x) as usize
    }

    pub fn at(&self, x: u32, y: u32) -> [f32; 4] {
        self.pixels[self.index(x, y)]
    }

    pub fn get(&self, x: i32, y: i32) -> [f32; 4] {
        if self.width == 0 || self.height == 0 {
            return [0.0; 4];
        }
        let x = x.clamp(0, self.width as i32 - 1) as u32;
        let y = y.clamp(0, self.height as i32 - 1) as u32;
        self.at(x, y)
    }

    pub fn sample(&self, x: f32, y: f32) -> [f32; 4] {
        if self.width == 0 || self.height == 0 {
            return [0.0; 4];
        }
        let max_x = self.width as f32 - 1.0;
        let max_y = self.height as f32 - 1.0;
        let x = x.clamp(0.0, max_x);
        let y = y.clamp(0.0, max_y);
        let x0 = x.floor() as i32;
        let y0 = y.floor() as i32;
        let tx = x - x0 as f32;
        let ty = y - y0 as f32;
        let a = self.get(x0, y0);
        let b = self.get(x0 + 1, y0);
        let c = self.get(x0, y0 + 1);
        let d = self.get(x0 + 1, y0 + 1);
        let mut out = [0.0; 4];
        for i in 0..4 {
            let ab = a[i] + (b[i] - a[i]) * tx;
            let cd = c[i] + (d[i] - c[i]) * tx;
            out[i] = ab + (cd - ab) * ty;
        }
        out
    }

    pub fn map_rgb(&self, mut f: impl FnMut(u32, u32, [f32; 3]) -> [f32; 3]) -> Self {
        let mut out = self.clone();
        for y in 0..self.height {
            for x in 0..self.width {
                let i = self.index(x, y);
                let p = self.pixels[i];
                let c = f(x, y, [p[0], p[1], p[2]]);
                out.pixels[i] = [c[0], c[1], c[2], p[3]];
            }
        }
        out
    }
}

pub fn rgb(p: [f32; 4]) -> [f32; 3] {
    [p[0], p[1], p[2]]
}

pub fn px(c: [f32; 3], a: f32) -> [f32; 4] {
    [c[0], c[1], c[2], a]
}
