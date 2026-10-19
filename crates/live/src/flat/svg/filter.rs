use super::blur;
use super::{Tag, colour, fraction, number, numbers, styled};
use crate::flat::Srgba;

const LINEAR: [u8; 256] = [
    0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3,
    4, 4, 4, 4, 4, 5, 5, 5, 5, 6, 6, 6, 6, 7, 7, 7, 8, 8, 8, 8, 9, 9, 9, 10, 10, 10, 11, 11, 12,
    12, 12, 13, 13, 13, 14, 14, 15, 15, 16, 16, 17, 17, 17, 18, 18, 19, 19, 20, 20, 21, 22, 22, 23,
    23, 24, 24, 25, 25, 26, 27, 27, 28, 29, 29, 30, 30, 31, 32, 32, 33, 34, 35, 35, 36, 37, 37, 38,
    39, 40, 41, 41, 42, 43, 44, 45, 45, 46, 47, 48, 49, 50, 51, 51, 52, 53, 54, 55, 56, 57, 58, 59,
    60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 76, 77, 78, 79, 80, 81, 82, 84, 85,
    86, 87, 88, 90, 91, 92, 93, 95, 96, 97, 99, 100, 101, 103, 104, 105, 107, 108, 109, 111, 112,
    114, 115, 116, 118, 119, 121, 122, 124, 125, 127, 128, 130, 131, 133, 134, 136, 138, 139, 141,
    142, 144, 146, 147, 149, 151, 152, 154, 156, 157, 159, 161, 163, 164, 166, 168, 170, 171, 173,
    175, 177, 179, 181, 183, 184, 186, 188, 190, 192, 194, 196, 198, 200, 202, 204, 206, 208, 210,
    212, 214, 216, 218, 220, 222, 224, 226, 229, 231, 233, 235, 237, 239, 242, 244, 246, 248, 250,
    253, 255,
];

const GAMMA: [u8; 256] = [
    0, 13, 22, 28, 34, 38, 42, 46, 50, 53, 56, 59, 61, 64, 66, 69, 71, 73, 75, 77, 79, 81, 83, 85,
    86, 88, 90, 92, 93, 95, 96, 98, 99, 101, 102, 104, 105, 106, 108, 109, 110, 112, 113, 114, 115,
    117, 118, 119, 120, 121, 122, 124, 125, 126, 127, 128, 129, 130, 131, 132, 133, 134, 135, 136,
    137, 138, 139, 140, 141, 142, 143, 144, 145, 146, 147, 148, 148, 149, 150, 151, 152, 153, 154,
    155, 155, 156, 157, 158, 159, 159, 160, 161, 162, 163, 163, 164, 165, 166, 167, 167, 168, 169,
    170, 170, 171, 172, 173, 173, 174, 175, 175, 176, 177, 178, 178, 179, 180, 180, 181, 182, 182,
    183, 184, 185, 185, 186, 187, 187, 188, 189, 189, 190, 190, 191, 192, 192, 193, 194, 194, 195,
    196, 196, 197, 197, 198, 199, 199, 200, 200, 201, 202, 202, 203, 203, 204, 205, 205, 206, 206,
    207, 208, 208, 209, 209, 210, 210, 211, 212, 212, 213, 213, 214, 214, 215, 215, 216, 216, 217,
    218, 218, 219, 219, 220, 220, 221, 221, 222, 222, 223, 223, 224, 224, 225, 226, 226, 227, 227,
    228, 228, 229, 229, 230, 230, 231, 231, 232, 232, 233, 233, 234, 234, 235, 235, 236, 236, 237,
    237, 238, 238, 238, 239, 239, 240, 240, 241, 241, 242, 242, 243, 243, 244, 244, 245, 245, 246,
    246, 246, 247, 247, 248, 248, 249, 249, 250, 250, 251, 251, 251, 252, 252, 253, 253, 254, 254,
    255, 255,
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Input {
    Source,
    Alpha,
    Result(usize),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Operator {
    Over,
    In,
    Out,
    Atop,
    Xor,
    Arithmetic([f32; 4]),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Blur {
        input: Input,
        deviation: [f32; 2],
    },
    Offset {
        input: Input,
        shift: [f32; 2],
    },
    Shadow {
        input: Input,
        deviation: [f32; 2],
        shift: [f32; 2],
        colour: Srgba,
    },
    Merge(Vec<Input>),
    Flood(Srgba),
    Composite {
        input: Input,
        under: Input,
        operator: Operator,
    },
    Matrix {
        input: Input,
        values: [f32; 20],
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Primitive {
    pub kind: Kind,
    area: [Option<String>; 4],
    linear: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Filter {
    user: bool,
    area: [Option<String>; 4],
    primitive_user: bool,
    pub primitives: Vec<Primitive>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    kind: Kind,
    area: [i64; 4],
    linear: bool,
}

const AREA: [&str; 4] = ["x", "y", "width", "height"];
const REGION: [f32; 4] = [-0.1, -0.1, 1.2, 1.2];

fn area_of(tag: &Tag) -> [Option<String>; 4] {
    AREA.map(|k| tag.get(k).map(str::to_string))
}

fn linear_of(tag: &Tag, inherited: bool) -> bool {
    match styled(tag, "color-interpolation-filters") {
        Some("sRGB") => false,
        Some("linearRGB") => true,
        _ => inherited,
    }
}

pub(super) struct Builder {
    filter: Filter,
    linear: bool,
    names: Vec<Option<String>>,
    merging: bool,
    refused: Option<String>,
}

impl Builder {
    pub(super) fn new(tag: &Tag) -> Builder {
        Builder {
            filter: Filter {
                user: tag.get("filterUnits") == Some("userSpaceOnUse"),
                area: area_of(tag),
                primitive_user: tag.get("primitiveUnits") != Some("objectBoundingBox"),
                primitives: Vec::new(),
            },
            linear: linear_of(tag, true),
            names: Vec::new(),
            merging: false,
            refused: None,
        }
    }

    fn input(&self, value: Option<&str>) -> Input {
        let last = || {
            self.filter
                .primitives
                .len()
                .checked_sub(1)
                .map_or(Input::Source, Input::Result)
        };
        match value.map(str::trim) {
            None => last(),
            Some("SourceGraphic") => Input::Source,
            Some("SourceAlpha") => Input::Alpha,
            Some(name) => self
                .names
                .iter()
                .rposition(|n| n.as_deref() == Some(name))
                .map_or_else(last, Input::Result),
        }
    }

    pub(super) fn add(&mut self, tag: &Tag) {
        if self.refused.is_some() {
            return;
        }
        if let Err(e) = self.add_in(tag) {
            self.refused = Some(e);
        }
    }

    fn add_in(&mut self, tag: &Tag) -> Result<(), String> {
        if tag.name == "feMerge" && tag.closing {
            self.merging = false;
            return Ok(());
        }
        if tag.closing {
            return Ok(());
        }
        if tag.name == "feMergeNode" {
            let input = self.input(tag.get("in"));
            let Some(Primitive {
                kind: Kind::Merge(inputs),
                ..
            }) = self.filter.primitives.last_mut().filter(|_| self.merging)
            else {
                return Err("<feMergeNode> is outside an <feMerge>".into());
            };
            inputs.push(input);
            return Ok(());
        }
        let input = self.input(tag.get("in"));
        let under = self.input(tag.get("in2"));
        let pair = |key: &str, default: [f32; 2]| -> Result<[f32; 2], String> {
            match tag.get(key).map(numbers).transpose()?.as_deref() {
                None => Ok(default),
                Some([a]) => Ok([*a, *a]),
                Some([a, b]) => Ok([*a, *b]),
                Some(_) => Ok([0.0, 0.0]),
            }
        };
        let flood = || -> Result<Srgba, String> {
            let c = styled(tag, "flood-color").map_or(Ok(Srgba::hex(0)), colour)?;
            let opacity = styled(tag, "flood-opacity").map_or(Ok(1.0), fraction)?;
            Ok(c.alpha(c.0[3] * opacity.clamp(0.0, 1.0)))
        };
        let shift = |default: f32| -> Result<[f32; 2], String> {
            Ok([
                tag.get("dx").map_or(Ok(default), number)?,
                tag.get("dy").map_or(Ok(default), number)?,
            ])
        };
        let kind = match tag.name.as_str() {
            "feGaussianBlur" => Kind::Blur {
                input,
                deviation: pair("stdDeviation", [0.0, 0.0])?,
            },
            "feOffset" => Kind::Offset {
                input,
                shift: shift(0.0)?,
            },
            "feDropShadow" => Kind::Shadow {
                input,
                deviation: pair("stdDeviation", [2.0, 2.0])?,
                shift: shift(2.0)?,
                colour: flood()?,
            },
            "feMerge" => {
                self.merging = true;
                Kind::Merge(Vec::new())
            }
            "feFlood" => Kind::Flood(flood()?),
            "feComposite" => Kind::Composite {
                input,
                under,
                operator: match tag.get("operator").unwrap_or("over") {
                    "in" => Operator::In,
                    "out" => Operator::Out,
                    "atop" => Operator::Atop,
                    "xor" => Operator::Xor,
                    "arithmetic" => Operator::Arithmetic(
                        ["k1", "k2", "k3", "k4"]
                            .map(|k| tag.get(k).and_then(|v| number(v).ok()).unwrap_or(0.0)),
                    ),
                    _ => Operator::Over,
                },
            },
            "feBlend" => match tag.get("mode").unwrap_or("normal") {
                "normal" => Kind::Composite {
                    input,
                    under,
                    operator: Operator::Over,
                },
                other => return Err(format!("<feBlend mode=\"{other}\"> is unsupported")),
            },
            "feColorMatrix" => Kind::Matrix {
                input,
                values: matrix(tag)?,
            },
            other => return Err(format!("the filter primitive <{other}> is unsupported")),
        };
        self.filter.primitives.push(Primitive {
            kind,
            area: area_of(tag),
            linear: linear_of(tag, self.linear),
        });
        self.names.push(tag.get("result").map(str::to_string));
        Ok(())
    }

    pub(super) fn finish(self) -> Result<Filter, String> {
        if let Some(refused) = self.refused {
            return Err(refused);
        }
        if self.filter.primitives.is_empty() {
            return Err("a filter has no primitives".into());
        }
        Ok(self.filter)
    }
}

const IDENTITY_MATRIX: [f32; 20] = [
    1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    0.0,
];

fn matrix(tag: &Tag) -> Result<[f32; 20], String> {
    let values = tag.get("values").map(numbers).transpose()?;
    match tag.get("type").unwrap_or("matrix") {
        "saturate" => {
            let v = values
                .as_deref()
                .and_then(|v| v.first().copied())
                .unwrap_or(1.0)
                .clamp(0.0, 1.0);
            Ok([
                0.213 + 0.787 * v,
                0.715 - 0.715 * v,
                0.072 - 0.072 * v,
                0.0,
                0.0,
                0.213 - 0.213 * v,
                0.715 + 0.285 * v,
                0.072 - 0.072 * v,
                0.0,
                0.0,
                0.213 - 0.213 * v,
                0.715 - 0.715 * v,
                0.072 + 0.928 * v,
                0.0,
                0.0,
                0.0,
                0.0,
                0.0,
                1.0,
                0.0,
            ])
        }
        "hueRotate" => {
            let (a2, a1) = values
                .as_deref()
                .and_then(|v| v.first().copied())
                .unwrap_or(0.0)
                .to_radians()
                .sin_cos();
            Ok([
                0.213 + 0.787 * a1 - 0.213 * a2,
                0.715 - 0.715 * a1 - 0.715 * a2,
                0.072 - 0.072 * a1 + 0.928 * a2,
                0.0,
                0.0,
                0.213 - 0.213 * a1 + 0.143 * a2,
                0.715 + 0.285 * a1 + 0.140 * a2,
                0.072 - 0.072 * a1 - 0.283 * a2,
                0.0,
                0.0,
                0.213 - 0.213 * a1 - 0.787 * a2,
                0.715 - 0.715 * a1 + 0.715 * a2,
                0.072 + 0.928 * a1 + 0.072 * a2,
                0.0,
                0.0,
                0.0,
                0.0,
                0.0,
                1.0,
                0.0,
            ])
        }
        "luminanceToAlpha" => {
            let mut m = [0.0; 20];
            m[15] = 0.2125;
            m[16] = 0.7154;
            m[17] = 0.0721;
            Ok(m)
        }
        _ => match values {
            Some(v) if v.len() == 20 => Ok(std::array::from_fn(|i| v[i])),
            _ => Ok(IDENTITY_MATRIX),
        },
    }
}

fn length(value: &str, user: bool, side: f32, origin: f32) -> Result<f32, String> {
    let v = value.trim();
    Ok(if user {
        match v.strip_suffix('%') {
            Some(p) => number(p)? / 100.0 * side,
            None => number(v)?,
        }
    } else {
        origin + fraction(v)? * side
    })
}

impl Filter {
    pub fn region(&self, bounds: [f32; 4], view: [f32; 2]) -> Result<[f32; 4], String> {
        let size = [bounds[2] - bounds[0], bounds[3] - bounds[1]];
        if !self.user && !(size[0] > 0.0 && size[1] > 0.0) {
            return Err("a filter on a shape with no area".into());
        }
        let mut out = [0.0; 4];
        for (i, v) in out.iter_mut().enumerate() {
            let axis = i % 2;
            let raw = self.area[i]
                .clone()
                .unwrap_or_else(|| format!("{}%", REGION[i] * 100.0));
            let side = if self.user { view[axis] } else { size[axis] };
            let origin = if i < 2 && !self.user {
                bounds[axis]
            } else {
                0.0
            };
            *v = length(&raw, self.user, side, origin)?;
        }
        if !(out[2] > 0.0 && out[3] > 0.0) {
            return Err("a filter region has no area".into());
        }
        Ok([out[0], out[1], out[0] + out[2], out[1] + out[3]])
    }

    pub fn plan(
        &self,
        bounds: [f32; 4],
        region: [f32; 4],
        view: [f32; 2],
        pixels: [u32; 2],
    ) -> Result<Vec<Step>, String> {
        let k = [
            pixels[0] as f32 / (region[2] - region[0]),
            pixels[1] as f32 / (region[3] - region[1]),
        ];
        let size = [bounds[2] - bounds[0], bounds[3] - bounds[1]];
        let unit = if self.primitive_user {
            [1.0, 1.0]
        } else {
            size
        };
        let scale = |v: [f32; 2]| [v[0] * unit[0] * k[0], v[1] * unit[1] * k[1]];
        let mut steps = Vec::with_capacity(self.primitives.len());
        for p in &self.primitives {
            let mut sub = [
                region[0],
                region[1],
                region[2] - region[0],
                region[3] - region[1],
            ];
            for (i, (edge, given)) in sub.iter_mut().zip(&p.area).enumerate() {
                let axis = i % 2;
                let side = if self.primitive_user {
                    view[axis]
                } else {
                    size[axis]
                };
                let origin = if i < 2 && !self.primitive_user {
                    bounds[axis]
                } else {
                    0.0
                };
                if let Some(v) = given {
                    *edge = length(v, self.primitive_user, side, origin)?;
                }
            }
            let x0 = ((sub[0] - region[0]) * k[0]).floor() as i64;
            let y0 = ((sub[1] - region[1]) * k[1]).floor() as i64;
            let x1 = ((sub[0] + sub[2] - region[0]) * k[0]).ceil() as i64;
            let y1 = ((sub[1] + sub[3] - region[1]) * k[1]).ceil() as i64;
            let kind = match &p.kind {
                Kind::Blur { input, deviation } => Kind::Blur {
                    input: *input,
                    deviation: scale(*deviation),
                },
                Kind::Offset { input, shift } => Kind::Offset {
                    input: *input,
                    shift: scale(*shift),
                },
                Kind::Shadow {
                    input,
                    deviation,
                    shift,
                    colour,
                } => Kind::Shadow {
                    input: *input,
                    deviation: scale(*deviation),
                    shift: scale(*shift),
                    colour: *colour,
                },
                other => other.clone(),
            };
            steps.push(Step {
                kind,
                area: [x0, y0, x1, y1],
                linear: p.linear,
            });
        }
        Ok(steps)
    }
}

#[derive(Clone)]
struct Image {
    pixels: Vec<[u8; 4]>,
    linear: bool,
}

fn demultiply(p: &mut [u8; 4]) {
    let a = p[3] as f32 / 255.0;
    for c in &mut p[..3] {
        *c = (*c as f32 / a + 0.5) as u8;
    }
}

fn premultiply(p: &mut [u8; 4]) {
    let a = p[3] as f32 / 255.0;
    for c in &mut p[..3] {
        *c = (*c as f32 * a + 0.5) as u8;
    }
}

impl Image {
    fn into_space(mut self, linear: bool) -> Image {
        if self.linear == linear {
            return self;
        }
        let table = if linear { &LINEAR } else { &GAMMA };
        for p in &mut self.pixels {
            match p[3] {
                0 => {}
                255 => {
                    for c in &mut p[..3] {
                        *c = table[*c as usize];
                    }
                }
                _ => {
                    demultiply(p);
                    for c in &mut p[..3] {
                        *c = table[*c as usize];
                    }
                    premultiply(p);
                }
            }
        }
        self.linear = linear;
        self
    }
}

fn over(dst: &mut [u8; 4], src: [u8; 4]) {
    let keep = 255 - src[3] as u32;
    for c in 0..4 {
        dst[c] = (src[c] as u32 + div255(dst[c] as u32 * keep)).min(255) as u8;
    }
}

fn div255(v: u32) -> u32 {
    (v + 128 + ((v + 128) >> 8)) >> 8
}

fn composite(a: [u8; 4], b: [u8; 4], operator: Operator) -> [u8; 4] {
    let (aa, ab) = (a[3] as u32, b[3] as u32);
    let (fa, fb) = match operator {
        Operator::Over => (255, 255 - aa),
        Operator::In => (ab, 0),
        Operator::Out => (255 - ab, 0),
        Operator::Atop => (ab, 255 - aa),
        Operator::Xor => (255 - ab, 255 - aa),
        Operator::Arithmetic([k1, k2, k3, k4]) => {
            let calc = |i1: u8, i2: u8, most: f32| {
                let (i1, i2) = (i1 as f32 / 255.0, i2 as f32 / 255.0);
                (k1 * i1 * i2 + k2 * i1 + k3 * i2 + k4).clamp(0.0, most)
            };
            let alpha = calc(a[3], b[3], 1.0);
            if alpha <= f32::EPSILON {
                return [0; 4];
            }
            return [
                (calc(a[0], b[0], alpha) * 255.0) as u8,
                (calc(a[1], b[1], alpha) * 255.0) as u8,
                (calc(a[2], b[2], alpha) * 255.0) as u8,
                (alpha * 255.0) as u8,
            ];
        }
    };
    std::array::from_fn(|c| (div255(a[c] as u32 * fa) + div255(b[c] as u32 * fb)).min(255) as u8)
}

fn shifted(image: &Image, size: [usize; 2], shift: [f32; 2]) -> Image {
    let (dx, dy) = (shift[0].round() as i64, shift[1].round() as i64);
    let mut pixels = vec![[0u8; 4]; image.pixels.len()];
    for y in 0..size[1] as i64 {
        let sy = y - dy;
        if sy < 0 || sy >= size[1] as i64 {
            continue;
        }
        for x in 0..size[0] as i64 {
            let sx = x - dx;
            if sx < 0 || sx >= size[0] as i64 {
                continue;
            }
            pixels[(y as usize) * size[0] + x as usize] =
                image.pixels[(sy as usize) * size[0] + sx as usize];
        }
    }
    Image {
        pixels,
        linear: image.linear,
    }
}

fn flood(colour: Srgba, count: usize) -> Image {
    let mut p = colour.0.map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
    premultiply(&mut p);
    Image {
        pixels: vec![p; count],
        linear: false,
    }
}

fn recoloured(values: &[f32; 20], mut image: Image) -> Image {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0) as u8;
    for p in &mut image.pixels {
        demultiply(p);
        let [r, g, b, a] = p.map(|v| v as f32 / 255.0);
        let m = values;
        *p = [
            byte(r * m[0] + g * m[1] + b * m[2] + a * m[3] + m[4]),
            byte(r * m[5] + g * m[6] + b * m[7] + a * m[8] + m[9]),
            byte(r * m[10] + g * m[11] + b * m[12] + a * m[13] + m[14]),
            byte(r * m[15] + g * m[16] + b * m[17] + a * m[18] + m[19]),
        ];
        premultiply(p);
    }
    image
}

pub fn run(steps: &[Step], source: Vec<[u8; 4]>, size: [u32; 2]) -> Vec<[u8; 4]> {
    let size = [size[0] as usize, size[1] as usize];
    let count = size[0] * size[1];
    let source = Image {
        pixels: source,
        linear: false,
    };
    let mut results: Vec<Image> = Vec::with_capacity(steps.len());
    let get = |input: Input, results: &[Image]| -> Image {
        match input {
            Input::Source => source.clone(),
            Input::Alpha => Image {
                pixels: source.pixels.iter().map(|p| [0, 0, 0, p[3]]).collect(),
                linear: false,
            },
            Input::Result(i) => results.get(i).cloned().unwrap_or_else(|| source.clone()),
        }
    };
    for step in steps {
        let space = step.linear;
        let mut out = match &step.kind {
            Kind::Blur { input, deviation } => {
                let mut image = get(*input, &results).into_space(space);
                blur::gaussian(&mut image.pixels, size, *deviation);
                image
            }
            Kind::Offset { input, shift } => shifted(&get(*input, &results), size, *shift),
            Kind::Shadow {
                input,
                deviation,
                shift,
                colour,
            } => {
                let image = get(*input, &results).into_space(space);
                let mut shade = image.clone();
                blur::gaussian(&mut shade.pixels, size, *deviation);
                let mut c = colour.0.map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
                let tone = c[3] as u32;
                for p in &mut shade.pixels {
                    c[3] = div255(tone * p[3] as u32) as u8;
                    let mut q = c;
                    premultiply(&mut q);
                    *p = q;
                }
                shade.linear = false;
                let mut shade = shifted(&shade.into_space(space), size, *shift);
                for (d, s) in shade.pixels.iter_mut().zip(&image.pixels) {
                    over(d, *s);
                }
                shade
            }
            Kind::Merge(inputs) => {
                let mut pixels = vec![[0u8; 4]; count];
                for input in inputs {
                    let layer = get(*input, &results).into_space(space);
                    for (d, s) in pixels.iter_mut().zip(&layer.pixels) {
                        over(d, *s);
                    }
                }
                Image {
                    pixels,
                    linear: space,
                }
            }
            Kind::Flood(colour) => flood(*colour, count),
            Kind::Composite {
                input,
                under,
                operator,
            } => {
                let a = get(*input, &results).into_space(space);
                let b = get(*under, &results).into_space(space);
                Image {
                    pixels: a
                        .pixels
                        .iter()
                        .zip(&b.pixels)
                        .map(|(a, b)| composite(*a, *b, *operator))
                        .collect(),
                    linear: space,
                }
            }
            Kind::Matrix { input, values } => {
                recoloured(values, get(*input, &results).into_space(space))
            }
        };
        let [x0, y0, x1, y1] = step.area;
        if x0 > 0 || y0 > 0 || x1 < size[0] as i64 || y1 < size[1] as i64 {
            for y in 0..size[1] {
                for x in 0..size[0] {
                    let (xi, yi) = (x as i64, y as i64);
                    if xi < x0 || xi >= x1 || yi < y0 || yi >= y1 {
                        out.pixels[y * size[0] + x] = [0; 4];
                    }
                }
            }
        }
        results.push(out);
    }
    results.pop().map_or(source.pixels.clone(), |image| {
        image.into_space(false).pixels
    })
}
