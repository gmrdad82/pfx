use super::icons::Icons;
use super::sdf::{self, Field};
use super::{
    BLUR, DASHED, Dash, Draw, FILL_SOLID, FILL_VERTICAL, Fill, Fit, Instance, KIND_ARC,
    KIND_CIRCLE, KIND_RECT, KIND_RING, Material, Matrix, Shadow, Shape, Srgba, Stroke, pack_plain,
};

pub const BEVEL: u32 = 1 << 24;

pub const STYLE_CALL: &str =
    "        fill = flat_bevel_over(v, kind, p, field, inverse, fill, fill_cover);\n";

pub const STYLE_WGSL: &str = r#"
const FLAT_BEVEL: u32 = 16777216u;

fn flat_bevel_light(v: FlatVaryings, kind: u32, p: vec2f, inverse: mat2x2f) -> f32 {
    let toward = v.material.yz;
    if (kind == FLAT_RECT) {
        let s = vec2f(flat_sign(p.x), flat_sign(p.y));
        let q = abs(p) - max(v.shape.xy - vec2f(flat_corner(p, v.corners)), vec2f(0.0));
        if (q.x > 0.0 && q.y > 0.0) {
            return flat_coverage(vec3f(-dot(s * q, toward), -toward), inverse);
        }
        let x_lit = select(0.0, 1.0, s.x * toward.x > 0.0);
        let y_lit = select(0.0, 1.0, s.y * toward.y > 0.0);
        let x_side = flat_coverage(vec3f(q.y - q.x, -s.x, s.y), inverse);
        return mix(y_lit, x_lit, x_side);
    }
    var side = 1.0;
    if (kind == FLAT_RING || kind == FLAT_ARC) {
        side = flat_sign(length(p) - v.shape.x);
    }
    return flat_coverage(vec3f(-side * dot(p, toward), -side * toward), inverse);
}

fn flat_bevel_over(v: FlatVaryings, kind: u32, p: vec2f, field: vec3f, inverse: mat2x2f, fill: vec4f, fill_cover: f32) -> vec4f {
    if ((v.info.x & FLAT_BEVEL) == 0u) {
        return fill;
    }
    let inner = flat_coverage(vec3f(field.x + v.material.x, field.yz), inverse);
    let band = max(fill_cover - inner, 0.0);
    let colour = mix(v.frame_y, v.frame_x, flat_bevel_light(v, kind, p, inverse));
    let paint = vec4f(colour.rgb * colour.a, colour.a);
    let share = select(0.0, band / fill_cover, fill_cover > 0.0);
    return paint * band + fill * (1.0 - paint.a * share);
}
"#;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Style {
    Outline(Outline),
    Bevel(Bevel),
    Flat,
}

impl Style {
    pub fn outline(width: f32) -> Self {
        Self::Outline(Outline::solid(width))
    }

    pub fn ghost(width: f32, on: f32, off: f32, alpha: f32) -> Self {
        Self::Outline(Outline::ghost(width, on, off, alpha))
    }

    pub fn bevel(width: f32, light: Srgba, dark: Srgba) -> Self {
        Self::Bevel(Bevel::new(width, light, dark))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Outline {
    pub width: f32,
    pub dash: Option<Dash>,
    pub colour: Option<Srgba>,
    pub alpha: f32,
}

impl Outline {
    pub fn solid(width: f32) -> Self {
        Self {
            width,
            dash: None,
            colour: None,
            alpha: 1.0,
        }
    }

    pub fn dashed(width: f32, on: f32, off: f32) -> Self {
        Self {
            dash: Some(Dash {
                on,
                off,
                phase: 0.0,
            }),
            ..Self::solid(width)
        }
    }

    pub fn ghost(width: f32, on: f32, off: f32, alpha: f32) -> Self {
        Self {
            alpha,
            ..Self::dashed(width, on, off)
        }
    }

    pub fn colour(mut self, colour: Srgba) -> Self {
        self.colour = Some(colour);
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bevel {
    pub width: f32,
    pub light: Srgba,
    pub dark: Srgba,
    pub toward: [f32; 2],
}

impl Bevel {
    pub fn new(width: f32, light: Srgba, dark: Srgba) -> Self {
        Self {
            width,
            light,
            dark,
            toward: [-1.0, -1.0],
        }
    }

    pub fn toward(mut self, toward: [f32; 2]) -> Self {
        self.toward = toward;
        self
    }
}

pub fn restyle(draws: &mut [Draw], style: Option<Style>) {
    for draw in draws {
        draw.style = style;
    }
}

pub fn restyle_group(draws: &mut [Draw], group: u16, style: Option<Style>) {
    for draw in draws.iter_mut().filter(|draw| draw.group == Some(group)) {
        draw.style = style;
    }
}

fn axes(transform: &Matrix) -> [f32; 4] {
    [
        transform[0][0],
        transform[0][1],
        transform[1][0],
        transform[1][1],
    ]
}

fn mean_scale(transform: &Matrix) -> f32 {
    let [a, b, c, d] = axes(transform);
    (a * d - c * b).abs().sqrt()
}

fn paint_colour(draw: &Draw) -> Option<Srgba> {
    draw.stroke
        .filter(|stroke| stroke.width > 0.0)
        .map(|stroke| stroke.colour)
        .or(match draw.fill {
            Fill::None => None,
            Fill::Solid(colour) => Some(colour),
            Fill::Vertical(top, _) => Some(top),
            Fill::Gradient(gradient) => gradient.stops().next().map(|(_, colour)| Srgba(colour)),
            Fill::Image(image) => Some(image.tint),
        })
}

pub fn resolve(draw: &Draw) -> Draw {
    let mut out = *draw;
    out.style = None;
    let Some(style) = draw.style else {
        return out;
    };
    if matches!(draw.shape, Shape::Text(_)) {
        return out;
    }
    out.shadow = match draw.shadow {
        Shadow::Glow(glow) => Shadow::Glow(glow),
        _ => Shadow::None,
    };
    out.material = Material::default();
    if let Style::Outline(outline) = style {
        out.fill = Fill::None;
        out.pattern = None;
        let scale = mean_scale(&draw.transform);
        let colour = outline.colour.or_else(|| paint_colour(draw));
        out.stroke = match colour {
            Some(colour)
                if scale > 0.0
                    && scale.is_finite()
                    && outline.width > 0.0
                    && outline.width.is_finite() =>
            {
                Some(Stroke {
                    colour: colour.alpha(colour.0[3] * outline.alpha.clamp(0.0, 1.0)),
                    width: outline.width / scale,
                    dash: outline.dash.map(|dash| Dash {
                        on: dash.on / scale,
                        off: dash.off / scale,
                        phase: dash.phase / scale,
                    }),
                })
            }
            _ => None,
        };
    }
    out
}

pub(super) fn pack(
    draw: &Draw,
    style: Style,
    fit: &Fit,
    icons: Option<&Icons>,
) -> Option<Instance> {
    let resolved = resolve(draw);
    let mut instance = pack_plain(&resolved, fit, icons)?;
    let kind = instance.kind();
    match style {
        Style::Outline(_) => {
            if let Some(dash) = resolved.stroke.and_then(|stroke| stroke.dash)
                && matches!(kind, KIND_RING | KIND_ARC)
                && instance.stroke[0] > 0.0
                && dash.on > 0.0
                && dash.off > 0.0
            {
                instance.info[0] |= DASHED;
                instance.stroke[1] = dash.on;
                instance.stroke[2] = dash.off;
                instance.stroke[3] = dash.phase;
            }
        }
        Style::Bevel(bevel) => {
            if instance.info[0] & (FILL_SOLID | FILL_VERTICAL) != 0
                && instance.info[0] & BLUR == 0
                && matches!(kind, KIND_RECT | KIND_CIRCLE | KIND_RING | KIND_ARC)
                && bevel.width > 0.0
                && bevel.width.is_finite()
            {
                let [a, b, c, d] = axes(&draw.transform);
                let det = a * d - c * b;
                let unit = det.abs().sqrt() * fit.scale;
                let pixels = (bevel.width * fit.scale).round().max(1.0);
                let toward = unit_or(bevel.toward, [0.0, -1.0]);
                let local = unit_or(
                    [
                        (d * toward[0] - c * toward[1]) / det,
                        (-b * toward[0] + a * toward[1]) / det,
                    ],
                    [0.0, -1.0],
                );
                instance.material = [pixels / unit, local[0], local[1], 0.0];
                instance.frame[0] = bevel.light.0;
                instance.frame[1] = bevel.dark.0;
                instance.info[0] |= BEVEL;
            }
        }
        Style::Flat => {}
    }
    Some(instance)
}

fn unit_or(v: [f32; 2], fallback: [f32; 2]) -> [f32; 2] {
    let length = (v[0] * v[0] + v[1] * v[1]).sqrt();
    if length > 0.0 && length.is_finite() {
        [v[0] / length, v[1] / length]
    } else {
        fallback
    }
}

fn sign(value: f32) -> f32 {
    if value >= 0.0 { 1.0 } else { -1.0 }
}

fn bevel_light(instance: &Instance, p: [f32; 2], inverse: [[f32; 2]; 2]) -> f32 {
    let toward = [instance.material[1], instance.material[2]];
    let shape = instance.shape;
    match instance.kind() {
        KIND_RECT => {
            let s = [sign(p[0]), sign(p[1])];
            let radius = sdf::corner(p, instance.corners);
            let q = [
                p[0].abs() - (shape[0] - radius).max(0.0),
                p[1].abs() - (shape[1] - radius).max(0.0),
            ];
            if q[0] > 0.0 && q[1] > 0.0 {
                let lean = s[0] * q[0] * toward[0] + s[1] * q[1] * toward[1];
                return sdf::coverage([-lean, -toward[0], -toward[1]], inverse);
            }
            let x_lit = if s[0] * toward[0] > 0.0 { 1.0 } else { 0.0 };
            let y_lit = if s[1] * toward[1] > 0.0 { 1.0 } else { 0.0 };
            let x_side = sdf::coverage([q[1] - q[0], -s[0], s[1]], inverse);
            y_lit + (x_lit - y_lit) * x_side
        }
        kind => {
            let side = if matches!(kind, KIND_RING | KIND_ARC) {
                sign((p[0] * p[0] + p[1] * p[1]).sqrt() - shape[0])
            } else {
                1.0
            };
            let lean = side * (p[0] * toward[0] + p[1] * toward[1]);
            sdf::coverage([-lean, -side * toward[0], -side * toward[1]], inverse)
        }
    }
}

pub fn bevel_over(
    instance: &Instance,
    p: [f32; 2],
    field: Field,
    inverse: [[f32; 2]; 2],
    fill: [f32; 4],
    fill_cover: f32,
) -> [f32; 4] {
    if instance.info[0] & BEVEL == 0 {
        return fill;
    }
    let inner = sdf::coverage(
        [field[0] + instance.material[0], field[1], field[2]],
        inverse,
    );
    let band = (fill_cover - inner).max(0.0);
    let towards = bevel_light(instance, p, inverse);
    let dark = instance.frame[1];
    let light = instance.frame[0];
    let colour: [f32; 4] = std::array::from_fn(|i| dark[i] + (light[i] - dark[i]) * towards);
    let paint = [
        colour[0] * colour[3],
        colour[1] * colour[3],
        colour[2] * colour[3],
        colour[3],
    ];
    let share = if fill_cover > 0.0 {
        band / fill_cover
    } else {
        0.0
    };
    std::array::from_fn(|i| paint[i] * band + fill[i] * (1.0 - paint[3] * share))
}
