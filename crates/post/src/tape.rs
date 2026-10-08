use crate::chain::{Chain, Pass};
use crate::hash::{grain_hash, mixed};
use crate::image::Image;

pub const WGSL: &str = include_str!("tape.wgsl");

pub const SPLIT: [f32; 2] = [0.0034, 0.0039];
pub const MIX: [[f32; 3]; 3] = [[0.41, 0.59, 0.0], [0.92, 0.06, 0.02], [0.45, 0.08, 0.47]];
pub const ECHOES: [(f32, f32); 2] = [(0.007, 0.25), (0.016, 0.10)];
pub const SOFT: f32 = 0.5;
pub const BLEED: f32 = 0.75 / 1920.0;
pub const LIFT: f32 = 0.07;
pub const DESATURATE: f32 = 0.22;
pub const GRAIN: f32 = 0.008;
pub const SCANLINES: f32 = 0.04;
pub const SHOVE: f32 = 0.0025;
pub const JITTER: f32 = 0.0003;
pub const BANDS: [Band; 3] = [
    Band {
        turn: 0.618_034,
        start: 0.0,
        speed: 0.21,
        width: 0.05,
        gain: 0.9,
    },
    Band {
        turn: 0.414_213_56,
        start: 0.37,
        speed: 0.34,
        width: 0.022,
        gain: 1.0,
    },
    Band {
        turn: 0.732_050_8,
        start: 0.71,
        speed: 0.13,
        width: 0.09,
        gain: 0.6,
    },
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Band {
    pub turn: f32,
    pub start: f32,
    pub speed: f32,
    pub width: f32,
    pub gain: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Forward,
    Rewind,
}

impl Direction {
    pub fn name(self) -> &'static str {
        match self {
            Self::Forward => "forward",
            Self::Rewind => "rewind",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        [Self::Forward, Self::Rewind]
            .into_iter()
            .find(|direction| direction.name() == name)
    }

    pub fn lead(self) -> f32 {
        match self {
            Self::Forward => -1.0,
            Self::Rewind => 1.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tape {
    pub strength: f32,
    pub direction: Direction,
    pub split: Option<f32>,
    pub echo: Option<f32>,
    pub wash: Option<f32>,
    pub smear: Option<f32>,
    pub grain: Option<f32>,
    pub bands: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Amounts {
    pub split: f32,
    pub echo: f32,
    pub wash: f32,
    pub smear: f32,
    pub grain: f32,
    pub bands: f32,
}

impl Default for Tape {
    fn default() -> Self {
        Self::OFF
    }
}

impl Tape {
    pub const OFF: Self = Self::new(0.0, Direction::Forward);

    pub const fn new(strength: f32, direction: Direction) -> Self {
        Self {
            strength,
            direction,
            split: None,
            echo: None,
            wash: None,
            smear: None,
            grain: None,
            bands: None,
        }
    }

    pub const fn forward(strength: f32) -> Self {
        Self::new(strength, Direction::Forward)
    }

    pub const fn rewind(strength: f32) -> Self {
        Self::new(strength, Direction::Rewind)
    }

    pub fn strength(&self) -> f32 {
        if self.strength.is_finite() {
            self.strength.clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    pub fn is_off(&self) -> bool {
        self.strength() <= 0.0
    }

    pub fn amounts(&self) -> Amounts {
        let strength = self.strength();
        let amount = |value: Option<f32>, default: f32| {
            let value = value.unwrap_or(default);
            if value.is_finite() {
                strength * value.max(0.0)
            } else {
                0.0
            }
        };
        Amounts {
            split: amount(self.split, 1.0),
            echo: amount(self.echo, 0.0).min(2.0),
            wash: amount(self.wash, 1.0).min(1.0),
            smear: amount(self.smear, 0.0).min(1.0),
            grain: amount(self.grain, 1.0),
            bands: amount(self.bands, 1.0),
        }
    }

    pub fn params(&self) -> [f32; 16] {
        let a = self.amounts();
        [
            SPLIT[0] * a.split,
            SPLIT[1] * a.split,
            0.0,
            self.direction.lead(),
            ECHOES[0].1 * a.echo,
            ECHOES[1].1 * a.echo,
            SOFT * a.smear,
            BLEED * a.smear,
            LIFT * a.wash,
            1.0 - DESATURATE * a.smear,
            GRAIN * a.grain,
            SCANLINES * a.grain,
            a.bands,
            SHOVE,
            0.0,
            self.strength(),
        ]
    }

    pub fn split_pixels(&self, width: u32) -> [f32; 2] {
        let p = self.params();
        [p[0] * width as f32, p[1] * width as f32]
    }

    pub fn echo_pixels(&self, width: u32) -> [f32; 2] {
        ECHOES.map(|(offset, _)| offset * width as f32)
    }

    pub fn band_centres(&self, time: f32, seed: u32) -> [f32; 3] {
        let roll = -self.direction.lead();
        BANDS.map(|band| centre(roll, time, &band, seed))
    }

    pub fn envelope(&self, y: f32, time: f32, seed: u32) -> f32 {
        envelope(-self.direction.lead(), y, time, seed)
    }
}

fn centre(roll: f32, time: f32, band: &Band, seed: u32) -> f32 {
    let spun = (seed & 0xffff) as f32 * band.turn + band.start;
    let travel = spun - spun.floor() + roll * band.speed * time;
    (travel - travel.floor()) * (1.0 + 2.0 * band.width) - band.width
}

fn envelope(roll: f32, y: f32, time: f32, seed: u32) -> f32 {
    BANDS
        .iter()
        .map(|band| {
            let c = centre(roll, time, band, seed);
            let d = (1.0 - (y - c).abs() / band.width).max(0.0);
            d * d * band.gain
        })
        .fold(0.0, f32::max)
}

fn hash(x: u32, y: u32, seed: u32) -> u32 {
    let mut h =
        x.wrapping_mul(0x8da6_b343) ^ y.wrapping_mul(0xd816_3841) ^ seed.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h
}

impl Chain {
    pub fn tape(&self) -> Option<Tape> {
        self.passes.iter().find_map(|pass| match pass {
            Pass::Tape(tape) => Some(*tape),
            _ => None,
        })
    }

    pub fn set_tape(&mut self, tape: Tape) {
        if let Some(Pass::Tape(slot)) = self
            .passes
            .iter_mut()
            .find(|pass| matches!(pass, Pass::Tape(_)))
        {
            *slot = tape;
            return;
        }
        let at = self
            .passes
            .iter()
            .rposition(|pass| !matches!(pass, Pass::Encode))
            .map_or(0, |index| index + 1);
        self.passes.insert(at, Pass::Tape(tape));
    }
}

const TO_I: [f32; 3] = [0.596, -0.274, -0.322];
const TO_Q: [f32; 3] = [0.211, -0.523, 0.312];

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn root(c: [f32; 3]) -> [f32; 3] {
    c.map(|v| v.max(0.0).sqrt())
}

pub fn apply(image: &Image, tape: &Tape, frame: u32, seed: u32) -> Image {
    if tape.is_off() || image.width == 0 || image.height == 0 {
        return image.clone();
    }
    let p = tape.params();
    let fw = image.width as f32;
    let lead = p[3];
    let sr = lead * p[0] * fw;
    let sb = lead * p[1] * fw;
    let mix = mixed(seed, frame);
    let t = frame as f32 / 60.0;
    let roll = -lead;
    let bands = p[12];
    let d = p[7] * fw;
    let (lift, saturation, grain, scan_depth) = (p[8], p[9], p[10], p[11]);
    let period = (image.height as f32 / 540.0).round_ties_even().max(2.0);
    let speck_scale = (image.width / 960).max(1);
    let mut out = image.clone();
    for row in 0..image.height {
        let v = (row as f32 + 0.5) / image.height as f32;
        let env = envelope(roll, v, t, seed);
        let rh = hash(row, 17, mix);
        let shove =
            lead * bands * fw * (p[13] * env + JITTER * env * ((rh >> 16) as f32 / 32768.0 - 1.0));
        let scan =
            1.0 - scan_depth * (0.5 + 0.5 * (std::f32::consts::TAU * row as f32 / period).cos());
        let at = |x: f32| image.sample(x - 0.5, row as f32);
        for col in 0..image.width {
            let x = col as f32 + 0.5 + shove + p[6];
            let here = at(x);
            let (left, right) = if d > 0.0 {
                let pair = |p: f32| {
                    let a = at(p - d);
                    let b = at(p + d);
                    std::array::from_fn::<f32, 4, _>(|c| 0.5 * (a[c] + b[c]))
                };
                (pair(x + sr), pair(x - sb))
            } else {
                (at(x + sr), at(x - sb))
            };
            let mut split: [f32; 3] = std::array::from_fn(|c| {
                MIX[c][0] * here[c] + MIX[c][1] * left[c] + MIX[c][2] * right[c]
            });
            if p[4] > 0.0 || p[5] > 0.0 {
                let e1 = at(x + lead * ECHOES[0].0 * fw);
                let e2 = at(x + lead * ECHOES[1].0 * fw);
                let keep = 1.0 - p[4] - p[5];
                split = std::array::from_fn(|c| split[c] * keep + p[4] * e1[c] + p[5] * e2[c]);
            }
            let split = root(split);
            let mut luma = lift + (1.0 - lift) * dot(split, [0.299, 0.587, 0.114]);
            let mut chroma = [dot(split, TO_I) * saturation, dot(split, TO_Q) * saturation];
            if env > 0.0 {
                let n = grain_hash(col / speck_scale, row, mix ^ 0x51);
                let lo = 1.0 - 0.12 * env;
                let u = ((n - lo) / (1.0 - lo)).clamp(0.0, 1.0);
                luma += bands * env * 0.1 * u * u * (3.0 - 2.0 * u);
            }
            let h = hash(col, row, mix ^ 7);
            let n = (h & 0x3ff) as f32 / 1024.0 + ((h >> 10) & 0x3ff) as f32 / 1024.0 - 1.0;
            luma = (luma + n * grain) * scan;
            chroma[0] += (((h >> 20) & 0x3f) as f32 / 64.0 - 0.5) * grain * 0.6;
            chroma[1] += ((h >> 26) as f32 / 64.0 - 0.5) * grain * 0.6;
            let g = [
                luma + 0.956_170_7 * chroma[0] + 0.621_432_6 * chroma[1],
                luma - 0.272_688_6 * chroma[0] - 0.646_813_2 * chroma[1],
                luma - 1.103_744_1 * chroma[0] + 1.700_623_1 * chroma[1],
            ]
            .map(|v| v.clamp(0.0, 1.0));
            let i = image.index(col, row);
            out.pixels[i] = [g[0] * g[0], g[1] * g[1], g[2] * g[2], here[3]];
        }
    }
    out
}

pub const KEYS: &[&str] = &[
    "bands",
    "direction",
    "echo",
    "grain",
    "smear",
    "split",
    "strength",
    "wash",
];

pub fn read(value: &toml::Value, current: Option<Tape>) -> Result<Tape, String> {
    let mut tape = current.unwrap_or(Tape::forward(1.0));
    let number = |value: &toml::Value, key: &str| {
        value
            .as_float()
            .or_else(|| value.as_integer().map(|n| n as f64))
            .map(|n| n as f32)
            .filter(|n| n.is_finite() && *n >= 0.0)
            .ok_or_else(|| format!("{key} is a number of at least 0"))
    };
    let direction = |value: &toml::Value| {
        value
            .as_str()
            .and_then(Direction::parse)
            .ok_or_else(|| "tape.direction is \"forward\" or \"rewind\"".to_string())
    };
    match value {
        toml::Value::String(_) => tape.direction = direction(value)?,
        toml::Value::Table(table) => {
            if let Some(bad) = table.keys().find(|key| !KEYS.contains(&key.as_str())) {
                return Err(format!(
                    "unknown tape key {bad}; allowed: {}",
                    KEYS.join(", ")
                ));
            }
            for (key, value) in table {
                let name = format!("tape.{key}");
                match key.as_str() {
                    "strength" => tape.strength = number(value, &name)?.min(1.0),
                    "direction" => tape.direction = direction(value)?,
                    "split" => tape.split = Some(number(value, &name)?),
                    "echo" => tape.echo = Some(number(value, &name)?),
                    "wash" => tape.wash = Some(number(value, &name)?),
                    "smear" => tape.smear = Some(number(value, &name)?),
                    "grain" => tape.grain = Some(number(value, &name)?),
                    _ => tape.bands = Some(number(value, &name)?),
                }
            }
        }
        _ => tape.strength = number(value, "tape")?.min(1.0),
    }
    Ok(tape)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu::GpuChain;
    use crate::gpu::tests::half;
    use crate::preset::Style;
    use pfx_gpu::{Gpu, OffscreenTarget};

    const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

    fn float(value: u16) -> f32 {
        let sign = if value & 0x8000 != 0 { -1.0 } else { 1.0 };
        let exponent = ((value >> 10) & 31) as i32;
        let mantissa = (value & 1023) as f32;
        if exponent == 0 {
            return sign * mantissa * 2.0f32.powi(-24);
        }
        sign * (1.0 + mantissa / 1024.0) * 2.0f32.powi(exponent - 15)
    }

    fn scene(width: u32, height: u32) -> Image {
        Image::from_fn(width, height, |x, y| {
            let u = x as f32 / width as f32;
            let v = y as f32 / height as f32;
            let mut c = [0.004, 0.004, 0.004, 1.0];
            if (0.1..0.12).contains(&u) && (0.1..0.8).contains(&v) {
                c = [1.0, 1.0, 1.0, 1.0];
            }
            if (0.3..0.5).contains(&u) && (0.55..0.85).contains(&v) {
                c = [0.02, 0.3, 0.55, 1.0];
            }
            if (0.6..0.9).contains(&u) && (0.2..0.24).contains(&v) {
                c = [0.9, 0.12, 0.05, 1.0];
            }
            if (0.15..0.95).contains(&v) && x % 23 == 4 {
                c = [0.05, 0.35, 0.4, 1.0];
            }
            c.map(|value| float(half(value)))
        })
    }

    const LINE: u32 = 5000;

    fn line() -> Image {
        Image::from_fn(2 * LINE, 2, |x, _| {
            if x == LINE {
                [1.0, 1.0, 1.0, 1.0]
            } else {
                [0.0, 0.0, 0.0, 1.0]
            }
        })
    }

    fn check_split(image: &Image, out: &[[f32; 4]], direction: Direction, tolerance: f32) {
        let tape = only_split(direction);
        let [sr, sb] = tape.split_pixels(image.width);
        assert_eq!((sr, sb), (34.0, 39.0));
        let lead = direction.lead() as i32;
        let at = |x: i32, c: usize| out[(image.width as i32 + x) as usize][c];
        let line = LINE as i32;
        let red = line - lead * 34;
        let blue = line + lead * 39;
        for (c, weights) in MIX.iter().enumerate() {
            for (x, want) in [(line, weights[0]), (red, weights[1]), (blue, weights[2])] {
                let got = at(x, c);
                assert!(
                    (got - want).abs() < tolerance,
                    "{direction:?} channel {c} at {x}: {got} vs {want}"
                );
            }
            for x in [line - 1, line + 1, red - 1, red + 1, blue - 1, blue + 1] {
                assert!(
                    at(x, c) < tolerance,
                    "{direction:?} channel {c} spreads to {x}"
                );
            }
        }
    }

    fn only_split(direction: Direction) -> Tape {
        Tape {
            echo: Some(0.0),
            wash: Some(0.0),
            grain: Some(0.0),
            bands: Some(0.0),
            ..Tape::new(1.0, direction)
        }
    }

    fn output_target(gpu: &Gpu, width: u32, height: u32) -> OffscreenTarget {
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("tape test output"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        OffscreenTarget {
            texture,
            view,
            format: FORMAT,
            width,
            height,
        }
    }

    struct Bench {
        gpu: Gpu,
        input: OffscreenTarget,
        output: OffscreenTarget,
    }

    impl Bench {
        fn new(image: &Image) -> Self {
            let gpu = pollster::block_on(Gpu::headless()).unwrap();
            let input = gpu.offscreen(image.width, image.height, FORMAT).unwrap();
            let output = output_target(&gpu, image.width, image.height);
            let pixels: Vec<u16> = image
                .pixels
                .iter()
                .flat_map(|p| p.iter().map(|v| half(*v)))
                .collect();
            gpu.upload_rgba16(&input, &pixels).unwrap();
            Self { gpu, input, output }
        }

        fn chain(&self, chain: Chain) -> GpuChain {
            GpuChain::new(chain, &self.gpu.device, self.input.width, self.input.height)
        }

        fn run(&self, post: &GpuChain, frame: u32, seed: u32) -> Vec<u16> {
            let mut encoder =
                self.gpu
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("tape test"),
                    });
            post.run(
                &mut encoder,
                &self.input.view,
                None,
                None,
                &self.output.view,
                frame,
                seed,
            );
            self.gpu.queue.submit(Some(encoder.finish()));
            self.gpu.readback_rgba16(&self.output).unwrap()
        }
    }

    fn rows(pixels: &[[f32; 4]], clean: &Image) -> Vec<f32> {
        (0..clean.height)
            .map(|y| {
                (0..clean.width)
                    .map(|x| {
                        let i = clean.index(x, y);
                        (pixels[i][1] - clean.pixels[i][1]).abs()
                    })
                    .sum::<f32>()
                    / clean.width as f32
            })
            .collect()
    }

    fn shift(a: &[f32], b: &[f32]) -> i32 {
        let n = a.len() as i32;
        let mut best = (f32::MIN, 0);
        for d in -n / 4..=n / 4 {
            let mut score = 0.0;
            for y in 0..n {
                let z = y - d;
                if (0..n).contains(&z) {
                    score += b[y as usize] * a[z as usize];
                }
            }
            if score > best.0 {
                best = (score, d);
            }
        }
        best.1
    }

    fn pixels(bytes: &[u16]) -> Vec<[f32; 4]> {
        bytes
            .chunks_exact(4)
            .map(|c| [float(c[0]), float(c[1]), float(c[2]), float(c[3])])
            .collect()
    }

    #[test]
    fn wgsl_validates() {
        let module = naga::front::wgsl::parse_str(WGSL).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .unwrap();
    }

    #[test]
    fn strength_zero_is_the_chain_without_it() {
        let image = scene(96, 48);
        let mut plain = Style::Crt.chain();
        plain.passes.push(Pass::Encode);
        plain.frame = 9;
        plain.seed = 4;
        let mut taped = plain.clone();
        taped.set_tape(Tape::rewind(0.0));
        assert!(matches!(
            taped.passes[taped.passes.len() - 2..],
            [Pass::Tape(_), Pass::Encode]
        ));
        assert_eq!(
            plain.apply(&image, None, None),
            taped.apply(&image, None, None)
        );
        assert_eq!(apply(&image, &Tape::OFF, 3, 5), image);
        assert_eq!(Tape::forward(f32::NAN).params()[15], 0.0);
    }

    #[test]
    fn same_time_and_seed_give_the_same_image() {
        let image = scene(96, 48);
        let tape = Tape::rewind(1.0);
        assert_eq!(apply(&image, &tape, 12, 7), apply(&image, &tape, 12, 7));
        assert_ne!(apply(&image, &tape, 12, 7), apply(&image, &tape, 13, 7));
        assert_ne!(apply(&image, &tape, 12, 7), apply(&image, &tape, 12, 8));
    }

    #[test]
    fn the_split_moves_red_and_blue_the_stated_pixels() {
        for direction in [Direction::Forward, Direction::Rewind] {
            check_split(
                &line(),
                &apply(&line(), &only_split(direction), 0, 0).pixels,
                direction,
                1e-4,
            );
        }
    }

    #[test]
    fn offsets_scale_with_width() {
        let tape = Tape::rewind(1.0);
        let hd = tape.split_pixels(1920);
        let uhd = tape.split_pixels(3840);
        assert!((uhd[0] - 2.0 * hd[0]).abs() < 1e-4 && (uhd[1] - 2.0 * hd[1]).abs() < 1e-4);
        assert!((hd[0] - 6.528).abs() < 1e-3 && (hd[1] - 7.488).abs() < 1e-3);
        let [a, b] = tape.echo_pixels(3840);
        assert!((a - 2.0 * tape.echo_pixels(1920)[0]).abs() < 1e-3 && b > a);
        assert!((Tape::rewind(0.5).split_pixels(1920)[0] - hd[0] * 0.5).abs() < 1e-4);
        let custom = Tape {
            split: Some(2.0),
            ..Tape::rewind(0.5)
        };
        assert!((custom.split_pixels(1920)[1] - hd[1]).abs() < 1e-4);
        let a = Tape::rewind(1.0).amounts();
        assert_eq!((a.echo, a.smear, a.wash), (0.0, 0.0, 1.0));
    }

    #[test]
    fn bands_roll_down_forward_and_up_in_rewind() {
        for (tape, sign) in [(Tape::forward(1.0), 1.0), (Tape::rewind(1.0), -1.0)] {
            let a = tape.band_centres(0.5, 3);
            let b = tape.band_centres(0.6, 3);
            for (k, band) in BANDS.iter().enumerate() {
                let moved = b[k] - a[k];
                let expected = sign * band.speed * 0.1 * (1.0 + 2.0 * band.width);
                if moved.abs() < 0.5 {
                    assert!((moved - expected).abs() < 1e-4, "{tape:?} band {k}");
                }
            }
        }
    }

    #[test]
    fn a_chain_reads_tape_from_toml() {
        let chain = crate::parse::parse_str(
            "style = \"noir\"\nencode = true\ntape = { strength = 0.8, direction = \"rewind\" }\n",
        )
        .unwrap();
        let tape = chain.tape().unwrap();
        assert_eq!(tape, Tape::rewind(0.8));
        assert!(matches!(
            chain.passes[chain.passes.len() - 2..],
            [Pass::Tape(_), Pass::Encode]
        ));
        let plain = crate::parse::parse_str("tape = 0.5\n").unwrap();
        assert_eq!(plain.tape(), Some(Tape::forward(0.5)));
        let named = crate::parse::parse_str("tape = \"rewind\"\n").unwrap();
        assert_eq!(named.tape(), Some(Tape::rewind(1.0)));
        let tuned = crate::parse::parse_str(
            "tape = { strength = 1, split = 0.5, echo = 1, wash = 2, smear = 0.5, grain = 0.25, bands = 1.5 }\n",
        )
        .unwrap()
        .tape()
        .unwrap();
        assert_eq!(
            tuned.amounts(),
            Amounts {
                split: 0.5,
                echo: 1.0,
                wash: 1.0,
                smear: 0.5,
                grain: 0.25,
                bands: 1.5
            }
        );
        assert!(crate::parse::parse_str("tape = { speed = 1 }\n").is_err());
        assert!(crate::parse::parse_str("tape = \"sideways\"\n").is_err());
        assert!(crate::parse::parse_str("tape = -1\n").is_err());
    }

    #[test]
    fn set_tape_replaces_the_slot() {
        let mut chain = Chain::new();
        chain.set_tape(Tape::forward(0.3));
        chain.passes.push(Pass::Encode);
        chain.set_tape(Tape::rewind(0.9));
        assert_eq!(chain.passes.len(), 2);
        assert_eq!(chain.tape(), Some(Tape::rewind(0.9)));
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn gpu_strength_zero_is_byte_identical() {
        let image = scene(200, 72);
        let bench = Bench::new(&image);
        for style in [Style::Noir, Style::Film, Style::Neon, Style::Crt] {
            let mut plain = style.chain();
            plain.passes.push(Pass::Encode);
            let want = bench.run(&bench.chain(plain.clone()), 7, 23);
            let mut slot = plain.clone();
            slot.set_tape(Tape::OFF);
            assert_eq!(bench.run(&bench.chain(slot), 7, 23), want, "{style:?}");
            let mut ramp = bench.chain(plain.clone());
            ramp.set_tape(Tape::OFF);
            assert_eq!(bench.run(&ramp, 7, 23), want, "{style:?} unset");
            ramp.set_tape(Tape::rewind(1.0));
            assert_ne!(bench.run(&ramp, 7, 23), want, "{style:?} on");
            ramp.set_tape(Tape::rewind(0.0));
            assert_eq!(bench.run(&ramp, 7, 23), want, "{style:?} ramped out");
        }
        let mut alone = Chain::new();
        alone.set_tape(Tape::OFF);
        let got = bench.run(&bench.chain(alone), 7, 23);
        let want: Vec<u16> = image
            .pixels
            .iter()
            .flat_map(|p| p.iter().map(|v| half(*v)))
            .collect();
        assert_eq!(got, want);
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn gpu_same_time_and_seed_give_the_same_bytes() {
        let image = scene(1100, 64);
        let bench = Bench::new(&image);
        let mut chain = Chain::new();
        chain.set_tape(Tape::rewind(1.0));
        let post = bench.chain(chain);
        let a = bench.run(&post, 30, 11);
        let b = bench.run(&post, 30, 11);
        assert_eq!(a, b);
        assert_ne!(a, bench.run(&post, 31, 11));
        assert_ne!(a, bench.run(&post, 30, 12));
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn gpu_split_moves_red_and_blue_the_stated_pixels() {
        let image = line();
        let bench = Bench::new(&image);
        for direction in [Direction::Forward, Direction::Rewind] {
            let mut chain = Chain::new();
            chain.set_tape(only_split(direction));
            let out = pixels(&bench.run(&bench.chain(chain), 0, 0));
            check_split(&image, &out, direction, 2e-3);
        }
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn gpu_bands_move_opposite_ways() {
        let image = Image::from_fn(1280, 360, |x, _| {
            let v = if x % 16 < 8 { 0.75 } else { 0.0625 };
            [v, v, v, 1.0]
        });
        let bench = Bench::new(&image);
        let mut shifts = Vec::new();
        for direction in [Direction::Forward, Direction::Rewind] {
            let tape = Tape {
                split: Some(0.0),
                echo: Some(0.0),
                wash: Some(0.0),
                grain: Some(0.0),
                ..Tape::new(1.0, direction)
            };
            let mut chain = Chain::new();
            chain.set_tape(tape);
            let post = bench.chain(chain);
            let a = rows(&pixels(&bench.run(&post, 60, 5)), &image);
            let b = rows(&pixels(&bench.run(&post, 66, 5)), &image);
            assert!(a.iter().any(|v| *v > 0.01), "{direction:?} shows no band");
            let moved = shift(&a, &b);
            let centres = tape.band_centres(1.0, 5);
            let later = tape.band_centres(1.1, 5);
            println!("{direction:?}: rows moved {moved}, centres {centres:?} -> {later:?}");
            shifts.push(moved);
        }
        assert!(shifts[0] > 0, "forward rolls down: {shifts:?}");
        assert!(shifts[1] < 0, "rewind rolls up: {shifts:?}");
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn gpu_matches_cpu() {
        let image = scene(1300, 90);
        let bench = Bench::new(&image);
        for (tape, frame) in [
            (Tape::rewind(1.0), 40),
            (Tape::forward(0.6), 3),
            (
                Tape {
                    echo: Some(1.0),
                    smear: Some(1.0),
                    ..Tape::forward(1.0)
                },
                9,
            ),
            (
                Tape {
                    split: Some(3.0),
                    wash: Some(0.3),
                    bands: Some(2.0),
                    ..Tape::rewind(0.9)
                },
                77,
            ),
        ] {
            let mut chain = Chain::new();
            chain.set_tape(tape);
            chain.frame = frame;
            chain.seed = 13;
            let want = chain.apply(&image, None, None);
            let got = pixels(&bench.run(&bench.chain(chain), frame, 13));
            let mut worst = 0.0f32;
            for (g, w) in got.iter().zip(&want.pixels) {
                for c in 0..4 {
                    worst = worst.max((g[c] - w[c]).abs());
                }
            }
            assert!(worst <= 0.006, "{tape:?}: {worst}");
        }
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn gpu_folds_the_encode_into_the_tape() {
        let image = scene(640, 48);
        let bench = Bench::new(&image);
        for style in [Style::Film, Style::Crt, Style::Noir] {
            let mut chain = style.chain();
            chain.passes.insert(0, Pass::Exposure(1.0));
            chain.passes.push(Pass::Encode);
            chain.set_tape(Tape::rewind(0.8));
            chain.frame = 21;
            chain.seed = 3;
            let want = chain.apply(&image, None, None);
            let got = pixels(&bench.run(&bench.chain(chain), 21, 3));
            let mut worst = 0.0f32;
            for (g, w) in got.iter().zip(&want.pixels) {
                for c in 0..3 {
                    worst = worst.max((g[c] - w[c]).abs());
                }
            }
            assert!(worst <= 0.03, "{style:?}: {worst}");
        }
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn benchmark_4k() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let input = gpu.offscreen(3840, 2160, FORMAT).unwrap();
        let output = output_target(&gpu, 3840, 2160);
        let mut timer = pfx_gpu::GpuProfiler::new(&gpu.device, &gpu.queue);
        let mut turns = pfx_gpu::pace::Turns::default();
        let mut chain = Chain::new();
        chain.set_tape(Tape::rewind(1.0));
        let post = turns.cpu(|| GpuChain::new(chain, &gpu.device, 3840, 2160));
        let mut samples = Vec::new();
        for attempt in 0..24u32 {
            let mut encoder = gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("tape benchmark"),
                });
            post.run_timed(
                &mut encoder,
                &input.view,
                None,
                None,
                &output.view,
                attempt,
                23,
                Some(&mut timer),
            );
            let slot = timer.finish(&mut encoder);
            gpu.queue.submit(Some(encoder.finish()));
            if let Some(slot) = slot {
                timer.submitted(slot);
            }
            gpu.device
                .poll(wgpu::PollType::wait_indefinitely())
                .unwrap();
            let total: f64 = timer
                .collect(&gpu.device)
                .iter()
                .flatten()
                .map(|pass| pass.milliseconds)
                .sum();
            turns.add(total);
            if attempt > 3 {
                samples.push(total);
            }
        }
        samples.sort_by(f64::total_cmp);
        let median = samples[samples.len() / 2];
        println!(
            "tape at 3840x2160: median {median:.3} ms, max {:.3} ms",
            samples[samples.len() - 1]
        );
        for tape in [None, Some(Tape::rewind(1.0))] {
            let mut finish = Style::Noir.chain();
            finish.passes.insert(0, Pass::Exposure(1.0));
            finish.passes.push(Pass::Encode);
            if let Some(tape) = tape {
                finish.set_tape(tape);
            }
            let post = turns.cpu(|| GpuChain::new(finish, &gpu.device, 3840, 2160));
            let mut totals = Vec::new();
            for attempt in 0..24u32 {
                let mut encoder =
                    gpu.device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("tape benchmark finish"),
                        });
                post.run_timed(
                    &mut encoder,
                    &input.view,
                    None,
                    None,
                    &output.view,
                    attempt,
                    23,
                    Some(&mut timer),
                );
                let slot = timer.finish(&mut encoder);
                gpu.queue.submit(Some(encoder.finish()));
                if let Some(slot) = slot {
                    timer.submitted(slot);
                }
                gpu.device
                    .poll(wgpu::PollType::wait_indefinitely())
                    .unwrap();
                let passes: Vec<_> = timer.collect(&gpu.device).into_iter().flatten().collect();
                let total: f64 = passes.iter().map(|pass| pass.milliseconds).sum();
                turns.add(total);
                if attempt == 23 {
                    for pass in &passes {
                        println!("  {} {:.3} ms", pass.label, pass.milliseconds);
                    }
                }
                if attempt > 3 {
                    totals.push(total);
                }
            }
            totals.sort_by(f64::total_cmp);
            println!(
                "noir finish with tape {:?}: median {:.3} ms",
                tape.map(|t| t.strength),
                totals[totals.len() / 2]
            );
        }
        assert!(median < 0.5, "{median:.3} ms");
    }
}
