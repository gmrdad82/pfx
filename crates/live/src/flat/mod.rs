#[cfg(test)]
mod corner_tests;
pub mod cpu;
pub mod cpu_text;
#[cfg(test)]
mod cpu_text_tests;
pub mod effects;
pub mod grid;
pub mod icons;
pub mod look;
mod pass;
mod roles;
pub mod sdf;
pub mod shader;
pub mod shadow;
#[cfg(test)]
mod sprite_tests;
pub mod sprites;
pub mod style;
#[cfg(test)]
mod style_tests;
pub mod svg;
#[cfg(test)]
mod tests;
pub mod viewport;

use crate::text::{GpuAtlas, RichQuad};
use bytemuck::{Pod, Zeroable};
pub use icons::{Icon, IconKind, IconShape, Icons};
pub use pass::{
    Environment, FlatIcons, FlatPass, FlatTarget, finish_entries, layer_entries, text_entries,
    view_entries,
};
use pfx_gpu::window::{Letterbox, Size, letterbox_nearest};
pub use pfx_post::look::{Gradient, GradientShape, Interpolation};
use pfx_text::Paragraph;
pub use roles::{Roles, TokenKey};
pub use shadow::{CurvePoint, ShadowCurve};
pub use sprites::{FlatSprites, SpriteFilter, SpriteImage};
pub use style::{Bevel, Outline, Style};

pub type Matrix = [[f32; 4]; 4];

pub const IDENTITY: Matrix = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Srgba(pub [f32; 4]);

impl Srgba {
    pub const TRANSPARENT: Self = Self([0.0; 4]);
    pub const WHITE: Self = Self([1.0; 4]);

    pub const fn hex(rgb: u32) -> Self {
        Self([
            ((rgb >> 16) & 255) as f32 / 255.0,
            ((rgb >> 8) & 255) as f32 / 255.0,
            (rgb & 255) as f32 / 255.0,
            1.0,
        ])
    }

    pub const fn alpha(self, alpha: f32) -> Self {
        Self([self.0[0], self.0[1], self.0[2], alpha])
    }

    pub fn premultiplied(self) -> [f32; 4] {
        let [r, g, b, a] = self.0;
        [r * a, g * a, b * a, a]
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fit {
    pub layout: [f32; 2],
    pub target: [u32; 2],
    pub scale: f32,
    pub offset: [f32; 2],
    pub letterbox: Letterbox,
}

impl Fit {
    pub fn new(layout: [f32; 2], target: [u32; 2]) -> Result<Self, String> {
        if !(layout[0] > 0.0 && layout[1] > 0.0 && layout[0].is_finite() && layout[1].is_finite()) {
            return Err("the layout size must be finite and positive".into());
        }
        let letterbox = letterbox_nearest(
            layout[0],
            layout[1],
            Size {
                width: target[0],
                height: target[1],
            },
        )
        .ok_or("the target is empty")?;
        Ok(Self {
            layout,
            target,
            scale: letterbox.matrix[0],
            offset: [letterbox.matrix[12], letterbox.matrix[13]],
            letterbox,
        })
    }

    pub fn to_target(&self, point: [f32; 2]) -> [f32; 2] {
        self.letterbox.map(point[0], point[1])
    }

    pub fn to_layout(&self, pixel: [f32; 2]) -> [f32; 2] {
        self.letterbox.unmap(pixel[0], pixel[1])
    }

    pub fn viewport(&self) -> [f32; 4] {
        [
            self.offset[0],
            self.offset[1],
            self.offset[0] + self.layout[0] * self.scale,
            self.offset[1] + self.layout[1] * self.scale,
        ]
    }

    pub fn clip_from_layout(&self) -> Matrix {
        let sx = 2.0 * self.scale / self.target[0] as f32;
        let sy = -2.0 * self.scale / self.target[1] as f32;
        [
            [sx, 0.0, 0.0, 0.0],
            [0.0, sy, 0.0, 0.0],
            [0.0, 0.0, 0.0, 0.0],
            [
                2.0 * self.offset[0] / self.target[0] as f32 - 1.0,
                1.0 - 2.0 * self.offset[1] / self.target[1] as f32,
                0.5,
                1.0,
            ],
        ]
    }
}

pub fn multiply(a: Matrix, b: Matrix) -> Matrix {
    crate::frame::multiply(a, b)
}

pub fn translation(x: f32, y: f32) -> Matrix {
    let mut m = IDENTITY;
    m[3][0] = x;
    m[3][1] = y;
    m
}

pub fn rotation(radians: f32) -> Matrix {
    let (s, c) = radians.sin_cos();
    let mut m = IDENTITY;
    m[0][0] = c;
    m[0][1] = s;
    m[1][0] = -s;
    m[1][1] = c;
    m
}

pub fn scaling(x: f32, y: f32) -> Matrix {
    let mut m = IDENTITY;
    m[0][0] = x;
    m[1][1] = y;
    m[2][2] = x.abs().max(y.abs());
    m
}

pub fn tip_x(radians: f32) -> Matrix {
    let (s, c) = radians.sin_cos();
    let mut m = IDENTITY;
    m[1][1] = c;
    m[1][2] = s;
    m[2][1] = -s;
    m[2][2] = c;
    m
}

pub fn tip_y(radians: f32) -> Matrix {
    let (s, c) = radians.sin_cos();
    let mut m = IDENTITY;
    m[0][0] = c;
    m[0][2] = -s;
    m[2][0] = s;
    m[2][2] = c;
    m
}

pub fn place(centre: [f32; 2], radians: f32) -> Matrix {
    multiply(translation(centre[0], centre[1]), rotation(radians))
}

pub fn clamp_radii(half: [f32; 2], radii: [f32; 4]) -> [f32; 4] {
    let limit = half[0].min(half[1]);
    radii.map(|radius| radius.min(limit).max(0.0))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    Rect {
        half: [f32; 2],
        radii: [f32; 4],
    },
    Circle {
        radius: f32,
    },
    Ring {
        radius: f32,
        width: f32,
    },
    Arc {
        radius: f32,
        width: f32,
        start: f32,
        sweep: f32,
    },
    Icon(Icon),
    Text(usize),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fill {
    None,
    Solid(Srgba),
    Vertical(Srgba, Srgba),
    Gradient(Gradient),
    Image(SpriteFill),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpriteFill {
    pub frame: [f32; 4],
    pub tint: Srgba,
}

impl SpriteFill {
    pub fn new(frame: [f32; 4]) -> Self {
        Self {
            frame,
            tint: Srgba::WHITE,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dash {
    pub on: f32,
    pub off: f32,
    pub phase: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stroke {
    pub colour: Srgba,
    pub width: f32,
    pub dash: Option<Dash>,
}

impl Stroke {
    pub fn solid(colour: Srgba, width: f32) -> Self {
        Self {
            colour,
            width,
            dash: None,
        }
    }

    pub fn dashed(colour: Srgba, width: f32, on: f32, off: f32) -> Self {
        Self {
            colour,
            width,
            dash: Some(Dash {
                on,
                off,
                phase: 0.0,
            }),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glow {
    pub colour: Srgba,
    pub sigma: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Shadow {
    None,
    #[default]
    Cast,
    Turned,
    Glow(Glow),
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Material {
    pub thickness: f32,
    pub bevel: f32,
    pub gloss: f32,
    pub roughness: f32,
    pub reflection: f32,
}

impl Material {
    pub fn is_flat(&self) -> bool {
        self.thickness <= 0.0 && self.bevel <= 0.0 && self.gloss <= 0.0 && self.reflection <= 0.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Draw {
    pub transform: Matrix,
    pub elevation: f32,
    pub shape: Shape,
    pub fill: Fill,
    pub stroke: Option<Stroke>,
    pub opacity: f32,
    pub shadow: Shadow,
    pub material: Material,
    pub pattern: Option<Pattern>,
    pub id: u32,
    pub group: Option<u16>,
    pub style: Option<Style>,
    pub fx: Fx,
    pub roles: Roles,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Fx {
    pub blur: [f32; 2],
    pub additive: bool,
    pub unpicked: bool,
}

impl Fx {
    pub fn is_none(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PatternKind {
    Stripes,
    Dots,
    CrossHatch,
    Chevrons,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pattern {
    pub kind: PatternKind,
    pub scale: f32,
    pub angle: f32,
    pub width: f32,
    pub colour: Srgba,
}

impl Pattern {
    pub fn new(kind: PatternKind, scale: f32, angle: f32, width: f32, colour: Srgba) -> Self {
        Self {
            kind,
            scale,
            angle,
            width,
            colour,
        }
    }

    fn bits(&self) -> u32 {
        let code = match self.kind {
            PatternKind::Stripes => 1,
            PatternKind::Dots => 2,
            PatternKind::CrossHatch => 3,
            PatternKind::Chevrons => 4,
        };
        code << PATTERN_SHIFT
    }

    fn packed_colour(&self) -> u32 {
        let byte = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u32;
        let [r, g, b, a] = self.colour.0;
        byte(r) | byte(g) << 8 | byte(b) << 16 | byte(a) << 24
    }
}

impl Shape {
    pub fn rounded(half: [f32; 2], radius: f32) -> Self {
        Self::Rect {
            half,
            radii: [radius; 4],
        }
    }

    pub fn corners(half: [f32; 2], radii: [f32; 4]) -> Self {
        Self::Rect { half, radii }
    }
}

impl Draw {
    pub fn new(shape: Shape) -> Self {
        Self {
            transform: IDENTITY,
            elevation: 0.0,
            shape,
            fill: Fill::None,
            stroke: None,
            opacity: 1.0,
            shadow: Shadow::Cast,
            material: Material::default(),
            pattern: None,
            id: 0,
            group: None,
            style: None,
            fx: Fx::default(),
            roles: Roles::default(),
        }
    }

    pub fn rect(centre: [f32; 2], size: [f32; 2], radius: f32) -> Self {
        Self::rect_corners(centre, size, [radius; 4])
    }

    pub fn rect_corners(centre: [f32; 2], size: [f32; 2], radii: [f32; 4]) -> Self {
        Self::new(Shape::Rect {
            half: [size[0] * 0.5, size[1] * 0.5],
            radii,
        })
        .at(centre)
    }

    pub fn sprite(centre: [f32; 2], size: [f32; 2], frame: [f32; 4]) -> Self {
        let mut draw = Self::rect(centre, size, 0.0);
        draw.fill = Fill::Image(SpriteFill::new(frame));
        draw
    }

    pub fn circle(centre: [f32; 2], radius: f32) -> Self {
        Self::new(Shape::Circle { radius }).at(centre)
    }

    pub fn at(mut self, centre: [f32; 2]) -> Self {
        self.transform = translation(centre[0], centre[1]);
        self
    }

    pub fn transform(mut self, transform: Matrix) -> Self {
        self.transform = transform;
        self
    }

    pub fn elevation(mut self, elevation: f32) -> Self {
        self.elevation = elevation;
        self
    }

    pub fn fill(mut self, colour: Srgba) -> Self {
        self.fill = Fill::Solid(colour);
        self
    }

    pub fn vertical(mut self, top: Srgba, bottom: Srgba) -> Self {
        self.fill = Fill::Vertical(top, bottom);
        self
    }

    pub fn stroke(mut self, stroke: Stroke) -> Self {
        self.stroke = Some(stroke);
        self
    }

    pub fn opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity;
        self
    }

    pub fn shadow(mut self, shadow: Shadow) -> Self {
        self.shadow = shadow;
        self
    }

    pub fn material(mut self, material: Material) -> Self {
        self.material = material;
        self
    }

    pub fn pattern(mut self, pattern: Pattern) -> Self {
        self.pattern = Some(pattern);
        self
    }

    pub fn id(mut self, id: u32) -> Self {
        self.id = id;
        self
    }

    pub fn group(mut self, group: u16) -> Self {
        self.group = Some(group);
        self
    }

    pub fn style(mut self, style: Style) -> Self {
        self.style = Some(style);
        self
    }

    pub fn fill_token(mut self, key: impl Into<TokenKey>) -> Self {
        self.roles.fill = Some(key.into());
        self
    }

    pub fn fill_end_token(mut self, key: impl Into<TokenKey>) -> Self {
        self.roles.fill_end = Some(key.into());
        self
    }

    pub fn stroke_token(mut self, key: impl Into<TokenKey>) -> Self {
        self.roles.stroke = Some(key.into());
        self
    }

    pub fn pattern_token(mut self, key: impl Into<TokenKey>) -> Self {
        self.roles.pattern = Some(key.into());
        self
    }

    pub fn glow_token(mut self, key: impl Into<TokenKey>) -> Self {
        self.roles.glow = Some(key.into());
        self
    }

    pub fn blur(mut self, displacement: [f32; 2]) -> Self {
        self.fx.blur = displacement;
        self
    }

    pub fn additive(mut self) -> Self {
        self.fx.additive = true;
        self
    }

    pub fn unpicked(mut self) -> Self {
        self.fx.unpicked = true;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Group {
    pub opacity: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Light {
    pub shadow: [f32; 2],
    pub key: [f32; 3],
    pub ambient: f32,
}

impl Default for Light {
    fn default() -> Self {
        Self {
            shadow: [0.0, 1.0],
            key: normalize3([0.0, -0.5, 1.0]),
            ambient: 0.55,
        }
    }
}

#[derive(Clone, Copy)]
pub enum Glyphs<'a> {
    Paragraph(&'a Paragraph),
    Quads(&'a [RichQuad]),
}

#[derive(Clone, Copy)]
pub struct FlatText<'a> {
    pub atlas: &'a GpuAtlas,
    pub glyphs: Glyphs<'a>,
    pub colour: Srgba,
}

#[derive(Clone, Copy)]
pub struct FlatScene<'a> {
    pub layout: [f32; 2],
    pub clear: Option<Srgba>,
    pub curve: &'a ShadowCurve,
    pub light: Light,
    pub draws: &'a [Draw],
    pub groups: &'a [Group],
    pub text: &'a [FlatText<'a>],
    pub icons: Option<&'a FlatIcons>,
    pub sprites: Option<&'a FlatSprites>,
    pub environment: Option<&'a Environment>,
    pub post: bool,
    pub frame: u32,
    pub seed: u32,
}

pub(crate) fn normalize3(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l > 0.0 {
        [v[0] / l, v[1] / l, v[2] / l]
    } else {
        [0.0, 0.0, 1.0]
    }
}

pub const KIND_RECT: u32 = 0;
pub const KIND_CIRCLE: u32 = 1;
pub const KIND_RING: u32 = 2;
pub const KIND_ARC: u32 = 3;
pub const KIND_ICON_FILL: u32 = 4;
pub const KIND_ICON_LINE: u32 = 5;
pub const KIND_SHADOW: u32 = 6;
pub const FILL_SOLID: u32 = 1 << 4;
pub const FILL_VERTICAL: u32 = 2 << 4;
pub const FILL_GRADIENT: u32 = 3 << 4;
pub const FILL_IMAGE: u32 = 4 << 4;
pub const FILL_MASK: u32 = 15 << 4;
pub const DASHED: u32 = 1 << 8;
pub const SLAB: u32 = 1 << 9;
pub const LIT: u32 = 1 << 10;
pub const BLUR: u32 = 1 << 11;
pub const PATTERN_SHIFT: u32 = 12;
pub const PATTERN_MASK: u32 = 7 << PATTERN_SHIFT;
pub const ADDITIVE: u32 = 1 << 15;
pub const UNPICKED: u32 = 1 << 16;
pub const FX: u32 = BLUR | ADDITIVE | UNPICKED;
pub const BLUR_MIN_PIXELS: f32 = 0.25;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct Instance {
    pub axes: [f32; 4],
    pub origin: [f32; 4],
    pub bounds: [f32; 4],
    pub shape: [f32; 4],
    pub stroke: [f32; 4],
    pub fill_top: [f32; 4],
    pub fill_bottom: [f32; 4],
    pub stroke_colour: [f32; 4],
    pub icon: [f32; 4],
    pub info: [u32; 4],
    pub material: [f32; 4],
    pub frame: [[f32; 4]; 3],
    pub corners: [f32; 4],
}

pub const INSTANCE_ATTRIBUTES: u32 = 15;

impl Instance {
    pub fn kind(&self) -> u32 {
        self.info[0] & 15
    }

    pub fn opacity(&self) -> f32 {
        f32::from_bits(self.info[3])
    }

    pub fn inverse(&self) -> [[f32; 2]; 2] {
        let [a, b, c, d] = self.axes;
        let det = a * d - c * b;
        [[d / det, -b / det], [-c / det, a / det]]
    }

    pub fn margin(&self) -> [f32; 2] {
        let inverse = self.inverse();
        [
            1.5 * (inverse[0][0] * inverse[0][0] + inverse[1][0] * inverse[1][0]).sqrt(),
            1.5 * (inverse[0][1] * inverse[0][1] + inverse[1][1] * inverse[1][1]).sqrt(),
        ]
    }

    pub fn pixel_bounds(&self) -> [f32; 4] {
        let margin = self.margin();
        let [cx, cy, hx, hy] = self.bounds;
        let mut out = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for corner in [[-1.0, -1.0], [1.0, -1.0], [-1.0, 1.0], [1.0, 1.0]] {
            let local = [
                cx + corner[0] * (hx + margin[0]),
                cy + corner[1] * (hy + margin[1]),
            ];
            let pixel = [
                self.axes[0] * local[0] + self.axes[2] * local[1] + self.origin[0],
                self.axes[1] * local[0] + self.axes[3] * local[1] + self.origin[1],
            ];
            out = [
                out[0].min(pixel[0]),
                out[1].min(pixel[1]),
                out[2].max(pixel[0]),
                out[3].max(pixel[1]),
            ];
        }
        out
    }

    pub fn to_local(&self, pixel: [f32; 2]) -> [f32; 2] {
        let inverse = self.inverse();
        let d = [pixel[0] - self.origin[0], pixel[1] - self.origin[1]];
        [
            inverse[0][0] * d[0] + inverse[1][0] * d[1],
            inverse[0][1] * d[0] + inverse[1][1] * d[1],
        ]
    }
}

pub(crate) fn positive(value: f32) -> bool {
    value > 0.0
}

fn length2(v: [f32; 2]) -> f32 {
    (v[0] * v[0] + v[1] * v[1]).sqrt()
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Placed {
    pub axes: [f32; 4],
    pub origin: [f32; 2],
    pub layout_axes: [f32; 4],
}

pub(crate) fn placed(transform: &Matrix, fit: &Fit) -> Option<Placed> {
    let layout_axes = [
        transform[0][0],
        transform[0][1],
        transform[1][0],
        transform[1][1],
    ];
    let det = layout_axes[0] * layout_axes[3] - layout_axes[2] * layout_axes[1];
    if !det.is_finite() || det.abs() < 1e-8 {
        return None;
    }
    if transform.iter().flatten().any(|value| !value.is_finite()) {
        return None;
    }
    Some(Placed {
        axes: layout_axes.map(|value| value * fit.scale),
        origin: fit.to_target([transform[3][0], transform[3][1]]),
        layout_axes,
    })
}

fn rotation_rows(transform: &Matrix) -> [[f32; 4]; 3] {
    let columns =
        [0, 1, 2].map(|c| normalize3([transform[c][0], transform[c][1], transform[c][2]]));
    [0, 1, 2].map(|r| [columns[0][r], columns[1][r], columns[2][r], 0.0])
}

pub(crate) fn caster(draw: &Draw) -> Option<([f32; 2], [f32; 4])> {
    let grow = draw
        .stroke
        .map_or(0.0, |stroke| stroke.width.max(0.0) * 0.5);
    match draw.shape {
        Shape::Rect { half, radii } => Some((
            [half[0] + grow, half[1] + grow],
            clamp_radii(half, radii).map(|radius| radius + grow),
        )),
        Shape::Circle { radius } => Some(([radius + grow; 2], [radius + grow; 4])),
        Shape::Ring { radius, width } | Shape::Arc { radius, width, .. } => {
            let outer = radius + width * 0.5 + grow;
            Some(([outer; 2], [outer; 4]))
        }
        Shape::Icon(_) | Shape::Text(_) => None,
    }
}

fn paint_alpha(draw: &Draw) -> f32 {
    let fill = match draw.fill {
        Fill::None => 0.0,
        Fill::Solid(colour) => colour.0[3],
        Fill::Vertical(top, bottom) => top.0[3].max(bottom.0[3]),
        Fill::Gradient(gradient) => gradient.max_alpha(),
        Fill::Image(image) => image.tint.0[3],
    };
    let stroke = draw
        .stroke
        .filter(|stroke| stroke.width > 0.0)
        .map_or(0.0, |stroke| stroke.colour.0[3]);
    fill.max(stroke).clamp(0.0, 1.0)
}

pub fn slab_size(draw: &Draw, icons: Option<&Icons>) -> Option<(f32, f32)> {
    let material = draw.material;
    if !(positive(material.thickness) || positive(material.bevel)) {
        return None;
    }
    let m = draw.transform;
    let scale = (m[0][0] * m[0][0] + m[0][1] * m[0][1] + m[0][2] * m[0][2]).sqrt();
    if !positive(scale) || !scale.is_finite() {
        return None;
    }
    let mut thickness = material.thickness.max(material.bevel).max(0.0);
    let bevel_limit = match draw.shape {
        Shape::Rect { half, .. } => half[0].min(half[1]) * scale,
        Shape::Circle { radius } => radius * scale,
        Shape::Icon(handle) => {
            let entry = icons?.entry(handle)?;
            let side = (entry.bounds[2] - entry.bounds[0]).min(entry.bounds[3] - entry.bounds[1]);
            thickness = thickness.min(side * scale / 8.0);
            match entry.kind {
                IconKind::Line => draw.stroke.map_or(0.0, |stroke| stroke.width * scale * 0.5),
                IconKind::Fill => f32::MAX,
            }
        }
        _ => return None,
    };
    let bevel = material
        .bevel
        .clamp(0.0, (thickness * 0.5).min(bevel_limit));
    Some((thickness / scale, bevel / scale))
}

pub fn pack_shape(draw: &Draw, fit: &Fit, icons: Option<&Icons>) -> Option<Instance> {
    match (draw.style, draw.fill) {
        (None, _) => pack_plain(draw, fit, icons),
        (Some(style), Fill::Image(image)) => {
            let tinted = Draw {
                fill: Fill::Solid(image.tint),
                ..*draw
            };
            style::pack(&tinted, style, fit, icons)
        }
        (Some(style), _) => style::pack(draw, style, fit, icons),
    }
}

pub(crate) fn pack_plain(draw: &Draw, fit: &Fit, icons: Option<&Icons>) -> Option<Instance> {
    let placed = placed(&draw.transform, fit)?;
    if !positive(draw.opacity)
        || (draw.fill == Fill::None && draw.stroke.is_none() && draw.pattern.is_none())
    {
        return None;
    }
    let stroke = draw
        .stroke
        .filter(|stroke| stroke.width > 0.0 && stroke.width.is_finite());
    let grow = stroke.map_or(0.0, |stroke| stroke.width * 0.5);
    let mut kind;
    let mut shape = [0.0; 4];
    let mut corners = [0.0; 4];
    let mut icon = [0.0; 4];
    let bounds;
    let gradient;
    match draw.shape {
        Shape::Rect { half, radii } => {
            kind = KIND_RECT;
            corners = clamp_radii(half, radii);
            shape = [half[0], half[1], 0.0, 0.0];
            bounds = [0.0, 0.0, half[0] + grow, half[1] + grow];
            gradient = [-half[1], half[1]];
        }
        Shape::Circle { radius } => {
            kind = KIND_CIRCLE;
            shape[0] = radius;
            bounds = [0.0, 0.0, radius + grow, radius + grow];
            gradient = [-radius, radius];
        }
        Shape::Ring { radius, width } => {
            kind = KIND_RING;
            shape = [radius, width, 0.0, 0.0];
            let outer = radius + width * 0.5 + grow;
            bounds = [0.0, 0.0, outer, outer];
            gradient = [-outer, outer];
        }
        Shape::Arc {
            radius,
            width,
            start,
            sweep,
        } => {
            kind = KIND_ARC;
            shape = [radius, width, start, sweep];
            let outer = radius + width * 0.5 + grow;
            bounds = [0.0, 0.0, outer, outer];
            gradient = [-outer, outer];
        }
        Shape::Icon(handle) => {
            let entry = icons?.entry(handle)?;
            kind = match entry.kind {
                IconKind::Fill => KIND_ICON_FILL,
                IconKind::Line => KIND_ICON_LINE,
            };
            let [x0, y0, x1, y1] = entry.bounds;
            bounds = [
                (x0 + x1) * 0.5,
                (y0 + y1) * 0.5,
                (x1 - x0) * 0.5,
                (y1 - y0) * 0.5,
            ];
            icon = entry.uv;
            gradient = [y0, y1];
        }
        Shape::Text(_) => return None,
    }
    let mut flags = 0;
    let (top, bottom) = match draw.fill {
        Fill::None => (Srgba::TRANSPARENT, Srgba::TRANSPARENT),
        Fill::Solid(colour) => {
            flags |= FILL_SOLID;
            (colour, colour)
        }
        Fill::Vertical(top, bottom) => {
            flags |= FILL_VERTICAL;
            (top, bottom)
        }
        Fill::Image(image) => {
            flags |= FILL_IMAGE;
            let reach = match draw.shape {
                Shape::Rect { half, .. } => Some(half),
                Shape::Circle { radius } => Some([radius; 2]),
                Shape::Ring { radius, width } | Shape::Arc { radius, width, .. } => {
                    Some([radius + width * 0.5; 2])
                }
                Shape::Icon(_) | Shape::Text(_) => None,
            };
            match reach.filter(|half| positive(half[0]) && positive(half[1])) {
                Some(half) => {
                    let size = [
                        image.frame[2] - image.frame[0],
                        image.frame[3] - image.frame[1],
                    ];
                    icon = [
                        image.frame[0] + size[0] * 0.5,
                        image.frame[1] + size[1] * 0.5,
                        size[0] / (2.0 * half[0]),
                        size[1] / (2.0 * half[1]),
                    ];
                }
                None => flags = flags & !FILL_IMAGE | FILL_SOLID,
            }
            (image.tint, image.tint)
        }
        Fill::Gradient(gradient) => {
            flags |= FILL_GRADIENT;
            let header = gradient.header();
            (
                Srgba(gradient.geometry()),
                Srgba([0.0, header[0], header[1] + 2.0 * header[2], header[3]]),
            )
        }
    };
    if kind == KIND_ICON_LINE {
        flags &= !FILL_MASK;
    }
    let mut pattern_colour = 0;
    if let Some(pattern) = draw.pattern
        && matches!(kind, KIND_RECT | KIND_CIRCLE | KIND_RING | KIND_ARC)
        && flags & FILL_MASK != FILL_IMAGE
        && positive(pattern.scale)
        && pattern.scale.is_finite()
        && pattern.angle.is_finite()
        && positive(pattern.colour.0[3])
    {
        flags |= pattern.bits();
        if flags & (FILL_SOLID | FILL_VERTICAL) == 0 {
            flags |= FILL_SOLID;
        }
        icon = [
            pattern.scale,
            pattern.angle,
            pattern.width.clamp(0.0, 1.0),
            0.0,
        ];
        pattern_colour = pattern.packed_colour();
    }
    let mut stroke_params = [0.0; 4];
    let mut stroke_colour = [0.0; 4];
    if let Some(stroke) = stroke {
        stroke_params[0] = stroke.width;
        stroke_colour = stroke.colour.0;
        if let Some(dash) = stroke.dash
            && dash.on > 0.0
            && dash.off > 0.0
            && matches!(kind, KIND_RECT | KIND_CIRCLE)
        {
            flags |= DASHED;
            stroke_params[1] = dash.on;
            stroke_params[2] = dash.off;
            stroke_params[3] = dash.phase;
        }
    }
    let material = draw.material;
    let mut material_params = [0.0; 4];
    let mut frame = [[0.0; 4]; 3];
    let flat_bounds = bounds;
    let mut bounds = bounds;
    if !material.is_flat() {
        flags |= LIT;
        frame = rotation_rows(&draw.transform);
        frame[0][3] = material.reflection.max(0.0);
        frame[1][3] = material.roughness.clamp(0.02, 1.0);
        material_params[2] = material.gloss.max(0.0);
        if let Some((thickness, bevel)) = slab_size(draw, icons) {
            flags |= SLAB;
            material_params[0] = thickness;
            material_params[1] = bevel;
            let down = [frame[2][0], frame[2][1], frame[2][2]];
            if down[2].abs() > 1e-3 {
                let reach = [
                    (thickness * down[0] / down[2]).abs(),
                    (thickness * down[1] / down[2]).abs(),
                ];
                bounds[2] += reach[0];
                bounds[3] += reach[1];
            }
        }
    }
    if draw.fx.additive {
        flags |= ADDITIVE;
    }
    if draw.fx.unpicked {
        flags |= UNPICKED;
    }
    if let Some(shift) = blur_shift(draw, &placed, fit) {
        if flags & FILL_MASK == FILL_IMAGE {
            flags = flags & !FILL_MASK | FILL_SOLID;
        }
        flags = (flags & !(LIT | SLAB)) | BLUR;
        material_params = [shift[0], shift[1], 0.0, 0.0];
        frame = [[0.0; 4]; 3];
        if matches!(kind, KIND_ICON_FILL | KIND_ICON_LINE) {
            shape = flat_bounds;
        }
        bounds = [
            flat_bounds[0],
            flat_bounds[1],
            flat_bounds[2] + shift[0].abs() * 0.5,
            flat_bounds[3] + shift[1].abs() * 0.5,
        ];
    }
    kind |= flags;
    let opacity = draw.opacity.min(1.0);
    let mut instance = Instance {
        axes: placed.axes,
        origin: [placed.origin[0], placed.origin[1], gradient[0], gradient[1]],
        bounds,
        shape,
        stroke: stroke_params,
        fill_top: top.0,
        fill_bottom: bottom.0,
        stroke_colour,
        icon,
        info: [kind, draw.id, pattern_colour, opacity.to_bits()],
        material: material_params,
        frame,
        corners,
    };
    if matches!(kind & 15, KIND_ICON_FILL | KIND_ICON_LINE) {
        let margin = instance.margin();
        instance.bounds[2] = (instance.bounds[2] - margin[0]).max(0.0);
        instance.bounds[3] = (instance.bounds[3] - margin[1]).max(0.0);
    }
    Some(instance)
}

fn blur_shift(draw: &Draw, placed: &Placed, fit: &Fit) -> Option<[f32; 2]> {
    let blur = draw.fx.blur;
    if !(blur[0].is_finite() && blur[1].is_finite()) {
        return None;
    }
    if length2(blur) * fit.scale < BLUR_MIN_PIXELS {
        return None;
    }
    let [a, b, c, d] = placed.layout_axes;
    let det = a * d - c * b;
    Some([
        (d * blur[0] - c * blur[1]) / det,
        (-b * blur[0] + a * blur[1]) / det,
    ])
}

pub fn pack_shadow(draw: &Draw, fit: &Fit, curve: &ShadowCurve, light: &Light) -> Option<Instance> {
    if draw.style.is_some() {
        return pack_shadow(&style::resolve(draw), fit, curve, light);
    }
    let (half, radii) = caster(draw)?;
    let placed = placed(&draw.transform, fit)?;
    let opacity = draw.opacity.min(1.0);
    let alpha = match draw.shadow {
        Shadow::Glow(_) => opacity,
        _ => paint_alpha(draw) * opacity,
    };
    if !positive(alpha) {
        return None;
    }
    let (colour, sigma, offset_local) = match draw.shadow {
        Shadow::None => return None,
        Shadow::Glow(glow) => (glow.colour, glow.sigma, [0.0, 0.0]),
        Shadow::Cast | Shadow::Turned => {
            let point = curve.at(draw.elevation)?;
            let direction = light.shadow;
            let length = length2(direction);
            let direction = if length > 0.0 {
                [direction[0] / length, direction[1] / length]
            } else {
                [0.0, 1.0]
            };
            let offset = [direction[0] * point.offset, direction[1] * point.offset];
            let local = if draw.shadow == Shadow::Turned {
                offset
            } else {
                let [a, b, c, d] = placed.layout_axes;
                let det = a * d - c * b;
                [
                    (d * offset[0] - c * offset[1]) / det,
                    (-b * offset[0] + a * offset[1]) / det,
                ]
            };
            (
                curve.colour.alpha(curve.colour.0[3] * point.alpha),
                point.sigma,
                local,
            )
        }
    };
    let alpha = alpha * colour.0[3];
    if !(sigma >= 0.0 && sigma.is_finite() && alpha > 0.0) {
        return None;
    }
    let [a, b, c, d] = placed.layout_axes;
    let unit = [length2([a, b]), length2([c, d])];
    let sigma_local = [(sigma / unit[0]).max(1e-4), (sigma / unit[1]).max(1e-4)];
    let reach = [4.0 * sigma_local[0], 4.0 * sigma_local[1]];
    Some(Instance {
        axes: placed.axes,
        origin: [placed.origin[0], placed.origin[1], 0.0, 0.0],
        bounds: [
            offset_local[0],
            offset_local[1],
            half[0] + reach[0],
            half[1] + reach[1],
        ],
        shape: [half[0], half[1], 0.0, 0.0],
        stroke: [0.0; 4],
        fill_top: colour.alpha(alpha).0,
        fill_bottom: [0.0; 4],
        stroke_colour: [0.0; 4],
        icon: [
            sigma_local[0],
            sigma_local[1],
            offset_local[0],
            offset_local[1],
        ],
        info: [KIND_SHADOW, 0, 0, 1.0f32.to_bits()],
        material: [0.0; 4],
        frame: [[0.0; 4]; 3],
        corners: radii,
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Step {
    Draw(usize),
    Layer(u16),
    EndLayer(u16),
}

#[derive(Clone, Debug, Default)]
pub struct Order {
    pub steps: Vec<Step>,
    keys: Vec<(f32, usize, f32, usize)>,
    bases: Vec<(f32, usize)>,
}

impl Order {
    pub fn build(&mut self, draws: &[Draw], groups: usize) -> Result<(), String> {
        self.steps.clear();
        self.keys.clear();
        self.bases.clear();
        self.bases.resize(groups, (f32::MAX, usize::MAX));
        for (index, draw) in draws.iter().enumerate() {
            if !draw.elevation.is_finite() {
                return Err(format!("draw {index} has a non-finite elevation"));
            }
            if let Some(group) = draw.group {
                let base = self
                    .bases
                    .get_mut(usize::from(group))
                    .ok_or_else(|| format!("draw {index} names group {group}, which is missing"))?;
                base.0 = base.0.min(draw.elevation);
                base.1 = base.1.min(index);
            }
        }
        for (index, draw) in draws.iter().enumerate() {
            self.keys.push(match draw.group {
                None => (draw.elevation, index, 0.0, index),
                Some(group) => {
                    let (base, first) = self.bases[usize::from(group)];
                    (base, first, draw.elevation, index)
                }
            });
        }
        self.keys.sort_unstable_by(|a, b| {
            a.0.total_cmp(&b.0)
                .then(a.1.cmp(&b.1))
                .then(a.2.total_cmp(&b.2))
                .then(a.3.cmp(&b.3))
        });
        let mut open: Option<u16> = None;
        for &(_, _, _, index) in &self.keys {
            let group = draws[index].group;
            if open != group {
                if let Some(previous) = open {
                    self.steps.push(Step::EndLayer(previous));
                }
                if let Some(next) = group {
                    self.steps.push(Step::Layer(next));
                }
                open = group;
            }
            self.steps.push(Step::Draw(index));
        }
        if let Some(previous) = open {
            self.steps.push(Step::EndLayer(previous));
        }
        Ok(())
    }
}
