use crate::bloom::{self, Bloom};
use crate::comic::{self, ModernComic};
use crate::fx::{self, NoiseKind};
use crate::grade::{self, Contrast, WarmthKind};
use crate::image::Image;
use crate::tape::{self, Tape};
use crate::tone::Tone;
use crate::wgsl;
use std::borrow::Cow;
use std::fmt::Write;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Buffers {
    pub depth: bool,
    pub normal: bool,
}

impl Buffers {
    pub const NONE: Self = Self {
        depth: false,
        normal: false,
    };

    pub const GEOMETRY: Self = Self {
        depth: true,
        normal: true,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Aux {
    None,
    Depth,
    Normal,
    DepthNormal,
    Bloom,
    BloomB,
    BloomLinear,
    LinearSampler,
    Lut,
    Blue,
    Grade,
    GradeBloom,
    GradeBloomB,
    GradeBloomLinear,
}

#[derive(Clone, Debug)]
pub struct GpuDispatch {
    pub shader: Cow<'static, str>,
    pub params: [f32; 16],
    pub aux: Aux,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Uniforms {
    pub size: [u32; 2],
    pub frame: u32,
    pub seed: u32,
    pub params: [f32; 16],
    pub region: [u32; 4],
    pub extent: [u32; 4],
}

impl Uniforms {
    pub fn new(width: u32, height: u32, frame: u32, seed: u32, params: [f32; 16]) -> Self {
        Self {
            size: [width, height],
            frame,
            seed,
            params,
            region: [0; 4],
            extent: [width, height, width, height],
        }
    }

    pub fn region(mut self, region: [u32; 4]) -> Self {
        self.region = region;
        self
    }

    pub fn extent(mut self, source: (u32, u32), aux: (u32, u32)) -> Self {
        self.extent = [source.0, source.1, aux.0, aux.1];
        self
    }

    pub fn bytes(&self) -> [u8; 112] {
        let mut out = [0u8; 112];
        out[0..4].copy_from_slice(&self.size[0].to_ne_bytes());
        out[4..8].copy_from_slice(&self.size[1].to_ne_bytes());
        out[8..12].copy_from_slice(&self.frame.to_ne_bytes());
        out[12..16].copy_from_slice(&self.seed.to_ne_bytes());
        for (i, value) in self.params.iter().enumerate() {
            let at = 16 + i * 4;
            out[at..at + 4].copy_from_slice(&value.to_ne_bytes());
        }
        for (i, value) in self.region.iter().enumerate() {
            let at = 80 + i * 4;
            out[at..at + 4].copy_from_slice(&value.to_ne_bytes());
        }
        for (i, value) in self.extent.iter().enumerate() {
            let at = 96 + i * 4;
            out[at..at + 4].copy_from_slice(&value.to_ne_bytes());
        }
        out
    }
}

pub const BIND_UNIFORM: u32 = 0;
pub const BIND_SRC: u32 = 1;
pub const BIND_DST: u32 = 2;
pub const BIND_AUX: u32 = 3;
pub const BIND_NORMAL: u32 = 4;

#[derive(Clone, Debug)]
pub enum Pass {
    Exposure(f32),
    Black(f32),
    Bloom(Bloom),
    Saturation(grade::Saturation),
    Contrast(Contrast),
    Warmth(grade::Warmth),
    Lut(grade::Lut),
    Tone(Tone),
    Vignette(grade::Vignette),
    Grain(grade::Grain),
    Dither(grade::Dither),
    Encode,
    Cel(fx::Cel),
    Outline(fx::Outline),
    Cavity(fx::Cavity),
    Rim(fx::Rim),
    Bw(fx::Bw),
    Noir(fx::Noir),
    Sepia(f32),
    Posterize(f32),
    Neon(fx::Neon),
    Flicker(f32),
    Weave(f32),
    Dust(f32),
    Scratches(fx::Scratches),
    Halation(fx::Halation),
    Aberration(f32),
    Distortion(f32),
    Curvature(f32),
    Scanlines(fx::Scanlines),
    Aperture(f32),
    OneBit(fx::OneBit),
    Halftone(fx::Halftone),
    Comic(fx::Comic),
    RubberHose,
    ModernComic(ModernComic),
    Watercolor(fx::Watercolor),
    PaperGrain(fx::PaperGrain),
    Pixel(fx::Pixelate),
    Duotone(fx::Duotone),
    GradientMap(fx::GradientMap),
    Kuwahara(u32),
    TiltShift(fx::TiltShift),
    Tape(Tape),
    Grade(Vec<Pass>),
    FusedCavityRim(Vec<Pass>),
}

#[derive(Clone, Debug)]
pub struct Chain {
    pub passes: Vec<Pass>,
    pub seed: u32,
    pub frame: u32,
}

impl Chain {
    pub fn new() -> Self {
        Self {
            passes: Vec::new(),
            seed: 0,
            frame: 0,
        }
    }

    pub fn is_local(&self) -> bool {
        self.passes.iter().all(Pass::is_local)
    }

    pub fn apply(
        &self,
        image: &Image,
        depth: Option<&[f32]>,
        normal: Option<&[[f32; 3]]>,
    ) -> Image {
        let depth = depth.filter(|d| d.len() == image.pixels.len());
        let normal = normal.filter(|n| n.len() == image.pixels.len());
        let mut current = image.clone();
        for pass in &self.passes {
            current = pass.run(&current, depth, normal, self.frame, self.seed);
        }
        current
    }

    pub fn split_after_bloom(&self) -> (Self, Self) {
        let boundary = self
            .passes
            .iter()
            .position(|pass| matches!(pass, Pass::Bloom(_)))
            .map_or(0, |index| index + 1);
        (
            Self {
                passes: self.passes[..boundary].to_vec(),
                seed: self.seed,
                frame: self.frame,
            },
            Self {
                passes: self.passes[boundary..].to_vec(),
                seed: self.seed,
                frame: self.frame,
            },
        )
    }
}

impl Default for Chain {
    fn default() -> Self {
        Self::new()
    }
}

impl Pass {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Exposure(_) => "exposure",
            Self::Black(_) => "black",
            Self::Bloom(_) => "bloom",
            Self::Saturation(_) => "saturation",
            Self::Contrast(_) => "contrast",
            Self::Warmth(_) => "warmth",
            Self::Lut(_) => "lut",
            Self::Tone(_) => "tone",
            Self::Vignette(_) => "vignette",
            Self::Grain(_) => "grain",
            Self::Dither(_) => "dither",
            Self::Encode => "encode",
            Self::Cel(_) => "cel",
            Self::Outline(_) => "outline",
            Self::Cavity(_) => "cavity",
            Self::Rim(_) => "rim",
            Self::Bw(_) => "bw",
            Self::Noir(_) => "noir",
            Self::Sepia(_) => "sepia",
            Self::Posterize(_) => "posterize",
            Self::Neon(_) => "neon",
            Self::Flicker(_) => "flicker",
            Self::Weave(_) => "weave",
            Self::Dust(_) => "dust",
            Self::Scratches(_) => "scratches",
            Self::Halation(_) => "halation",
            Self::Aberration(_) => "aberration",
            Self::Distortion(_) => "distortion",
            Self::Curvature(_) => "curvature",
            Self::Scanlines(_) => "scanlines",
            Self::Aperture(_) => "aperture",
            Self::OneBit(_) => "one_bit",
            Self::Halftone(_) => "halftone",
            Self::Comic(_) => "comic",
            Self::RubberHose => "rubber_hose",
            Self::ModernComic(_) => "modern_comic",
            Self::Watercolor(_) => "watercolor",
            Self::PaperGrain(_) => "paper_grain",
            Self::Pixel(_) => "pixel",
            Self::Duotone(_) => "duotone",
            Self::GradientMap(_) => "gradient_map",
            Self::Kuwahara(_) => "kuwahara",
            Self::TiltShift(_) => "tilt_shift",
            Self::Tape(_) => "tape",
            Self::Grade(_) => "grade",
            Self::FusedCavityRim(_) => "fused_cavity_rim",
        }
    }

    pub fn is_identity(&self) -> bool {
        match self {
            Self::Exposure(v) => (*v - 1.0).abs() < 1e-8,
            Self::Bloom(v) => v.strength.abs() < 1e-8,
            Self::Vignette(v) => v.strength.abs() < 1e-8,
            Self::Saturation(v) => (v.amount - 1.0).abs() < 1e-8,
            Self::Warmth(v) => v.amount.abs() < 1e-8,
            Self::Halation(v) => v.strength.abs() < 1e-8,
            _ => false,
        }
    }

    pub fn run(
        &self,
        image: &Image,
        depth: Option<&[f32]>,
        normal: Option<&[[f32; 3]]>,
        frame: u32,
        seed: u32,
    ) -> Image {
        match self {
            Self::Exposure(v) => grade::exposure(image, *v),
            Self::Black(v) => grade::black(image, *v),
            Self::Bloom(v) => bloom::apply(image, v),
            Self::Saturation(v) => grade::saturation(image, v),
            Self::Contrast(v) => grade::contrast(image, v),
            Self::Warmth(v) => grade::warmth(image, v),
            Self::Lut(v) => grade::lut(image, v),
            Self::Tone(v) => grade::tone(image, v),
            Self::Vignette(v) => grade::vignette(image, v),
            Self::Grain(v) => grade::grain(image, v, frame, seed),
            Self::Dither(v) => grade::dither(image, v, frame, seed),
            Self::Encode => grade::encode(image),
            Self::Cel(v) => fx::cel(image, v, normal),
            Self::Outline(v) => fx::outline(image, v, depth, normal),
            Self::Cavity(v) => fx::cavity(image, v, normal),
            Self::Rim(v) => fx::rim(image, v, normal),
            Self::Bw(v) => fx::bw(image, v),
            Self::Noir(v) => fx::noir(image, v),
            Self::Sepia(v) => fx::sepia(image, *v),
            Self::Posterize(v) => fx::posterize(image, *v),
            Self::Neon(v) => fx::neon(image, v),
            Self::Flicker(v) => fx::flicker(image, *v, frame, seed),
            Self::Weave(v) => fx::weave(image, *v, frame, seed),
            Self::Dust(v) => fx::dust(image, *v, frame, seed),
            Self::Scratches(v) => fx::scratches(image, v, frame, seed),
            Self::Halation(v) => fx::halation(image, v),
            Self::Aberration(v) => fx::aberration(image, *v),
            Self::Distortion(v) | Self::Curvature(v) => fx::distort(image, *v),
            Self::Scanlines(v) => fx::scanlines(image, v),
            Self::Aperture(v) => fx::aperture(image, *v),
            Self::OneBit(v) => fx::one_bit(image, v),
            Self::Halftone(v) => fx::halftone(image, v),
            Self::Comic(v) => fx::comic(image, v, depth, normal),
            Self::RubberHose => fx::rubber_hose(image, depth, normal, frame, seed),
            Self::ModernComic(v) => comic::apply(image, v, depth, normal),
            Self::Watercolor(v) => fx::watercolor(image, v, frame, seed),
            Self::PaperGrain(v) => fx::paper_grain(image, v, frame, seed),
            Self::Pixel(v) => fx::pixelate(image, v),
            Self::Duotone(v) => fx::duotone(image, v),
            Self::GradientMap(v) => fx::gradient_map(image, v),
            Self::Kuwahara(v) => fx::kuwahara(image, *v),
            Self::TiltShift(v) => fx::tilt_shift(image, v, depth),
            Self::Tape(v) => tape::apply(image, v, frame, seed),
            Self::Grade(passes) | Self::FusedCavityRim(passes) => {
                passes.iter().fold(image.clone(), |current, pass| {
                    pass.run(&current, depth, normal, frame, seed)
                })
            }
        }
    }

    pub fn region_reach(&self) -> Option<u32> {
        match self {
            Self::Bloom(_)
            | Self::Neon(_)
            | Self::Halation(_)
            | Self::Outline(_)
            | Self::Comic(_)
            | Self::RubberHose
            | Self::ModernComic(_)
            | Self::TiltShift(_)
            | Self::Tape(_)
            | Self::Distortion(_)
            | Self::Curvature(_)
            | Self::Pixel(_) => None,
            Self::Grade(passes) => passes
                .iter()
                .try_fold(0u32, |reach, pass| Some(reach + pass.region_reach()?)),
            Self::Kuwahara(radius) => Some((*radius).min(8)),
            Self::Watercolor(_) => Some(1),
            Self::Weave(amount) => Some((amount.abs() * 2.0).ceil() as u32 + 1),
            Self::Aberration(amount) => Some(amount.abs().ceil() as u32 + 1),
            _ => Some(0),
        }
    }

    pub fn is_local(&self) -> bool {
        match self {
            Self::Exposure(_)
            | Self::Black(_)
            | Self::Saturation(_)
            | Self::Contrast(_)
            | Self::Warmth(_)
            | Self::Lut(_)
            | Self::Tone(_)
            | Self::Vignette(_)
            | Self::Grain(_)
            | Self::Dither(_)
            | Self::Encode
            | Self::Bw(_)
            | Self::Noir(_)
            | Self::Sepia(_)
            | Self::Posterize(_)
            | Self::Duotone(_) => true,
            Self::Bloom(bloom) => bloom.strength <= 0.0,
            Self::Grade(passes) => passes.iter().all(Self::is_local),
            _ => false,
        }
    }

    pub fn is_per_pixel(&self) -> bool {
        matches!(
            self,
            Self::Exposure(_)
                | Self::Black(_)
                | Self::Saturation(_)
                | Self::Contrast(_)
                | Self::Warmth(_)
                | Self::Tone(_)
                | Self::Vignette(_)
                | Self::Grain(_)
                | Self::Dither(_)
                | Self::Encode
                | Self::Bw(_)
                | Self::Noir(_)
                | Self::Sepia(_)
                | Self::Posterize(_)
                | Self::Flicker(_)
                | Self::Scanlines(_)
                | Self::Aperture(_)
                | Self::Duotone(_)
                | Self::GradientMap(_)
        )
    }

    pub fn params(&self, buffers: Buffers) -> Vec<[f32; 16]> {
        match self {
            Self::Grade(passes) => grade_params(passes),
            other => other
                .dispatches(buffers)
                .into_iter()
                .map(|dispatch| dispatch.params)
                .collect(),
        }
    }

    pub fn dispatches(&self, buffers: Buffers) -> Vec<GpuDispatch> {
        match self {
            Self::Bloom(bloom) => bloom_dispatches(bloom, 1.0),
            Self::Grade(passes) => grade_dispatches(passes),
            Self::Halation(pass) => halation_dispatches(pass),
            Self::Neon(pass) if pass.radius > 0 && pass.strength.abs() >= 1e-8 => vec![
                GpuDispatch {
                    shader: Cow::Borrowed(wgsl::NEON_EDGE),
                    params: fill(&[(pass.saturation, 0)]),
                    aux: Aux::None,
                },
                GpuDispatch {
                    shader: Cow::Borrowed(wgsl::BLOOM_BLUR),
                    params: fill(&[
                        (1.0, 0),
                        (0.0, 1),
                        (pass.radius.min(8).div_ceil(2) as f32, 2),
                        (pass.radius.max(1) as f32 * 1.5, 3),
                    ]),
                    aux: Aux::None,
                },
                GpuDispatch {
                    shader: Cow::Borrowed(wgsl::BLOOM_BLUR),
                    params: fill(&[
                        (0.0, 0),
                        (1.0, 1),
                        (pass.radius.min(8).div_ceil(2) as f32, 2),
                        (pass.radius.max(1) as f32 * 1.5, 3),
                    ]),
                    aux: Aux::None,
                },
                GpuDispatch {
                    shader: Cow::Borrowed(wgsl::NEON_ADD),
                    params: fill(&[(pass.saturation, 0), (pass.strength, 1)]),
                    aux: Aux::Bloom,
                },
            ],
            other => vec![GpuDispatch {
                shader: Cow::Borrowed(other.shader()),
                params: other.pack(buffers),
                aux: other.aux(),
            }],
        }
    }

    fn shader(&self) -> &'static str {
        match self {
            Self::Exposure(_) => wgsl::EXPOSURE,
            Self::Black(_) => wgsl::BLACK,
            Self::Bloom(_) => wgsl::BLOOM_ADD,
            Self::Saturation(_) => wgsl::SATURATION,
            Self::Contrast(_) => wgsl::CONTRAST,
            Self::Warmth(_) => wgsl::WARMTH,
            Self::Lut(_) => wgsl::LUT,
            Self::Tone(_) => wgsl::TONE,
            Self::Vignette(_) => wgsl::VIGNETTE,
            Self::Grain(_) => wgsl::GRAIN,
            Self::Dither(_) => wgsl::DITHER,
            Self::Encode => wgsl::ENCODE,
            Self::Cel(_) => wgsl::CEL,
            Self::Outline(_) => wgsl::OUTLINE,
            Self::Cavity(_) => wgsl::CAVITY,
            Self::Rim(_) => wgsl::RIM,
            Self::Bw(_) => wgsl::BW,
            Self::Noir(_) => wgsl::NOIR,
            Self::Sepia(_) => wgsl::SEPIA,
            Self::Posterize(_) => wgsl::POSTERIZE,
            Self::Neon(_) => wgsl::NEON,
            Self::Flicker(_) => wgsl::FLICKER,
            Self::Weave(_) => wgsl::WEAVE,
            Self::Dust(_) => wgsl::DUST,
            Self::Scratches(_) => wgsl::SCRATCHES,
            Self::Halation(_) => wgsl::HALATION_ADD,
            Self::Aberration(_) => wgsl::ABERRATION,
            Self::Distortion(_) | Self::Curvature(_) => wgsl::DISTORTION,
            Self::Scanlines(_) => wgsl::SCANLINES,
            Self::Aperture(_) => wgsl::APERTURE,
            Self::OneBit(pass) => match pass.kind {
                NoiseKind::Ordered => wgsl::ONE_BIT,
                NoiseKind::Blue => wgsl::ONE_BIT_BLUE,
            },
            Self::Halftone(_) => wgsl::HALFTONE,
            Self::Comic(_) => wgsl::COMIC,
            Self::RubberHose => wgsl::RUBBER_HOSE,
            Self::ModernComic(_) => wgsl::MODERN_COMIC,
            Self::Watercolor(_) => wgsl::WATERCOLOR,
            Self::PaperGrain(_) => wgsl::PAPER_GRAIN,
            Self::Pixel(_) => wgsl::PIXEL,
            Self::Duotone(_) => wgsl::DUOTONE,
            Self::GradientMap(_) => wgsl::GRADIENT,
            Self::Kuwahara(_) => wgsl::KUWAHARA,
            Self::TiltShift(_) => wgsl::TILT_SHIFT,
            Self::Tape(_) => tape::WGSL,
            Self::Grade(_) => wgsl::EXPOSURE,
            Self::FusedCavityRim(_) => wgsl::FUSED_CAVITY_RIM,
        }
    }

    fn aux(&self) -> Aux {
        match self {
            Self::Lut(_) => Aux::Lut,
            Self::Outline(_) | Self::ModernComic(_) | Self::Comic(_) | Self::RubberHose => {
                Aux::DepthNormal
            }
            Self::Cel(_) | Self::Cavity(_) | Self::Rim(_) | Self::FusedCavityRim(_) => Aux::Normal,
            Self::TiltShift(_) => Aux::Depth,
            Self::Tape(_) => Aux::LinearSampler,
            Self::OneBit(pass) if pass.kind == NoiseKind::Blue => Aux::Blue,
            _ => Aux::None,
        }
    }

    fn pack(&self, buffers: Buffers) -> [f32; 16] {
        match self {
            Self::Exposure(v) => fill(&[(*v, 0)]),
            Self::Black(v) => fill(&[(*v, 0)]),
            Self::Bloom(v) => bloom_add_params(v),
            Self::Saturation(v) => fill(&[
                (v.amount, 0),
                (v.weights[0], 1),
                (v.weights[1], 2),
                (v.weights[2], 3),
            ]),
            Self::Contrast(Contrast::Pivot { amount, pivot }) => {
                fill(&[(*amount, 0), (*pivot, 1), (0.0, 2)])
            }
            Self::Contrast(Contrast::Power { power, lift }) => {
                fill(&[(*power, 0), (*lift, 1), (1.0, 2)])
            }
            Self::Warmth(v) => fill(&[
                (v.amount, 0),
                (
                    if v.kind == WarmthKind::LumaCurve {
                        1.0
                    } else {
                        0.0
                    },
                    1,
                ),
                (v.shadow, 2),
                (v.highlight, 3),
                (v.low[0], 4),
                (v.low[1], 5),
                (v.high[0], 6),
                (v.high[1], 7),
            ]),
            Self::Lut(v) => fill(&[(v.size as f32, 0)]),
            Self::Tone(Tone::Aces { gain }) => fill(&[(0.0, 0), (*gain, 1)]),
            Self::Tone(Tone::Neutral {
                start,
                toe,
                desaturation,
                clamp,
            }) => fill(&[
                (1.0, 0),
                (*start, 1),
                (*toe, 2),
                (*desaturation, 3),
                (if *clamp { 1.0 } else { 0.0 }, 4),
            ]),
            Self::Tone(Tone::Agx) => fill(&[(2.0, 0)]),
            Self::Vignette(v) => fill(&[
                (v.strength, 0),
                (
                    if v.kind == grade::VignetteKind::Smoothstep {
                        1.0
                    } else {
                        0.0
                    },
                    1,
                ),
                (v.power, 2),
                (v.aspect, 3),
                (v.scale, 4),
                (v.inner, 5),
                (v.outer, 6),
            ]),
            Self::Grain(v) => fill(&[
                (v.strength, 0),
                (v.response, 1),
                (if v.clamp { 1.0 } else { 0.0 }, 2),
            ]),
            Self::Dither(v) => fill(&[(v.amplitude, 0)]),
            Self::Encode => [0.0; 16],
            Self::Cel(v) => fill(&[
                (v.bands as f32, 0),
                (v.shadow, 1),
                (v.threshold, 2),
                (v.softness, 3),
                (v.spec, 4),
                (v.spec_roughness, 5),
                (v.spec_threshold, 6),
                (if buffers.normal { 1.0 } else { 0.0 }, 7),
                (v.light[0], 8),
                (v.light[1], 9),
                (v.light[2], 10),
            ]),
            Self::Outline(v) => fill(&[
                (v.thickness, 0),
                (v.alpha, 1),
                (v.depth_gap, 2),
                (v.crease_angle.to_radians().cos(), 3),
                (v.color[0], 4),
                (v.color[1], 5),
                (v.color[2], 6),
                (if v.enabled { 1.0 } else { 0.0 }, 7),
                (if buffers.depth { 1.0 } else { 0.0 }, 8),
                (if buffers.normal { 1.0 } else { 0.0 }, 9),
                (if v.local_color { 1.0 } else { 0.0 }, 10),
            ]),
            Self::Cavity(v) => fill(&[
                (v.strength, 0),
                (v.distance, 1),
                (if buffers.normal { 1.0 } else { 0.0 }, 2),
            ]),
            Self::Rim(v) => fill(&[
                (v.strength, 0),
                (v.width, 1),
                (v.color[0], 2),
                (v.color[1], 3),
                (v.color[2], 4),
                (if buffers.normal { 1.0 } else { 0.0 }, 5),
            ]),
            Self::Bw(v) => fill(&[(v.weights[0], 0), (v.weights[1], 1), (v.weights[2], 2)]),
            Self::Noir(v) => {
                let keep = v.keep.unwrap_or([0.0; 3]);
                fill(&[
                    (v.contrast, 0),
                    (v.crush, 1),
                    (if v.keep.is_some() { 1.0 } else { 0.0 }, 2),
                    (v.keep_range, 3),
                    (keep[0], 4),
                    (keep[1], 5),
                    (keep[2], 6),
                ])
            }
            Self::Sepia(v) => fill(&[(*v, 0)]),
            Self::Posterize(v) => fill(&[(*v, 0)]),
            Self::Neon(v) => fill(&[(v.saturation, 0), (v.strength, 1), (v.radius as f32, 2)]),
            Self::Flicker(v)
            | Self::Weave(v)
            | Self::Dust(v)
            | Self::Aberration(v)
            | Self::Distortion(v)
            | Self::Curvature(v)
            | Self::Aperture(v) => fill(&[(*v, 0)]),
            Self::Scratches(v) => fill(&[(v.count as f32, 0), (v.strength, 1)]),
            Self::Halation(v) => fill(&[(v.strength, 0)]),
            Self::Scanlines(v) => fill(&[(v.strength, 0), (v.period as f32, 1)]),
            Self::OneBit(_) => [0.0; 16],
            Self::Halftone(v) => fill(&[
                (v.cell, 0),
                (v.angle, 1),
                (v.ink[0], 2),
                (v.ink[1], 3),
                (v.ink[2], 4),
            ]),
            Self::Comic(v) => fill(&[
                (v.levels as f32, 0),
                (v.ink, 1),
                (if buffers.depth { 1.0 } else { 0.0 }, 2),
                (if buffers.normal { 1.0 } else { 0.0 }, 3),
            ]),
            Self::RubberHose => fill(&[
                (if buffers.depth { 1.0 } else { 0.0 }, 0),
                (if buffers.normal { 1.0 } else { 0.0 }, 1),
            ]),
            Self::ModernComic(v) => fill(&[
                (v.tone_steps as f32, 0),
                (v.black_threshold, 1),
                (v.line_weight, 2),
                (v.interior_line_strength, 3),
                (v.saturation_lift, 4),
                (v.highlight_threshold, 5),
                (if buffers.depth { 1.0 } else { 0.0 }, 6),
                (if buffers.normal { 1.0 } else { 0.0 }, 7),
            ]),
            Self::Watercolor(v) => fill(&[(v.darkening, 0), (v.grain, 1), (v.scale, 2)]),
            Self::PaperGrain(v) => fill(&[(v.strength, 0), (v.scale, 1)]),
            Self::Pixel(v) => fill(&[
                (v.size as f32, 0),
                (v.levels as f32, 1),
                (if v.palette { 1.0 } else { 0.0 }, 2),
            ]),
            Self::Duotone(v) => fill(&[
                (v.shadow[0], 0),
                (v.shadow[1], 1),
                (v.shadow[2], 2),
                (v.highlight[0], 4),
                (v.highlight[1], 5),
                (v.highlight[2], 6),
            ]),
            Self::GradientMap(v) => gradient_params(v),
            Self::Kuwahara(v) => fill(&[(*v as f32, 0)]),
            Self::TiltShift(v) => fill(&[
                (v.focus, 0),
                (v.range, 1),
                (v.radius, 2),
                (if buffers.depth { 1.0 } else { 0.0 }, 3),
            ]),
            Self::Tape(v) => v.params(),
            Self::Grade(passes) => grade_params(passes).pop().unwrap_or([0.0; 16]),
            Self::FusedCavityRim(passes) => {
                let [Pass::Cavity(cavity), Pass::Rim(rim)] = passes.as_slice() else {
                    unreachable!()
                };
                fill(&[
                    (cavity.strength, 0),
                    (cavity.distance, 1),
                    (rim.strength, 2),
                    (rim.width, 3),
                    (rim.color[0], 4),
                    (rim.color[1], 5),
                    (rim.color[2], 6),
                    (if buffers.normal { 1.0 } else { 0.0 }, 7),
                ])
            }
        }
    }
}

fn fill(pairs: &[(f32, usize)]) -> [f32; 16] {
    let mut out = [0.0; 16];
    for (value, index) in pairs {
        out[*index] = *value;
    }
    out
}

fn bloom_add_params(bloom: &Bloom) -> [f32; 16] {
    fill(&[(bloom.strength, 0)])
}

fn bloom_dispatches(bloom: &Bloom, scale: f32) -> Vec<GpuDispatch> {
    match bloom.kind {
        bloom::BloomKind::Rings => vec![
            GpuDispatch {
                shader: Cow::Borrowed(wgsl::BLOOM_RING_DOWN),
                params: fill(&[
                    (bloom.down.max(1) as f32, 0),
                    (bloom.threshold, 1),
                    (bloom.clamp, 2),
                    (scale, 3),
                ]),
                aux: Aux::None,
            },
            GpuDispatch {
                shader: Cow::Borrowed(wgsl::BLOOM_RINGS),
                params: fill(&[
                    (bloom.unit / bloom.down.max(1) as f32, 3),
                    (bloom.radii[0], 4),
                    (bloom.radii[1], 5),
                    (bloom.radii[2], 6),
                    (bloom.radii[3], 7),
                    (bloom.gains[0], 8),
                    (bloom.gains[1], 9),
                    (bloom.gains[2], 10),
                    (bloom.gains[3], 11),
                ]),
                aux: Aux::None,
            },
            GpuDispatch {
                shader: Cow::Borrowed(wgsl::BLOOM_ADD),
                params: bloom_add_params(bloom),
                aux: Aux::BloomB,
            },
        ],
        bloom::BloomKind::Box | bloom::BloomKind::Tent => {
            let tent = if bloom.kind == bloom::BloomKind::Tent {
                1.0
            } else {
                0.0
            };
            let box2 = bloom.kind == bloom::BloomKind::Box && bloom.down == 2;
            let linear = bloom.kind == bloom::BloomKind::Box;
            let sampled = linear && bloom.linear_taps;
            let blur = if sampled {
                Cow::Owned(wgsl::bloom_blur_linear(bloom.radius, bloom.spread))
            } else {
                Cow::Borrowed(wgsl::BLOOM_BLUR)
            };
            let blur_aux = if sampled {
                Aux::LinearSampler
            } else {
                Aux::None
            };
            vec![
                GpuDispatch {
                    shader: Cow::Borrowed(if box2 {
                        wgsl::BLOOM_DOWN_BOX2
                    } else {
                        wgsl::BLOOM_DOWN
                    }),
                    params: fill(&[
                        (bloom.down.max(1) as f32, 0),
                        (bloom.threshold, 1),
                        (bloom.knee, 2),
                        (bloom.knee_width, 3),
                        (bloom.soft, 4),
                        (tent, 5),
                        (scale, 6),
                    ]),
                    aux: Aux::None,
                },
                GpuDispatch {
                    shader: blur.clone(),
                    params: fill(&[
                        (1.0, 0),
                        (0.0, 1),
                        (bloom.radius as f32, 2),
                        (bloom.spread, 3),
                    ]),
                    aux: blur_aux,
                },
                GpuDispatch {
                    shader: blur,
                    params: fill(&[
                        (0.0, 0),
                        (1.0, 1),
                        (bloom.radius as f32, 2),
                        (bloom.spread, 3),
                    ]),
                    aux: blur_aux,
                },
                GpuDispatch {
                    shader: Cow::Borrowed(if linear {
                        wgsl::BLOOM_ADD_LINEAR
                    } else {
                        wgsl::BLOOM_ADD
                    }),
                    params: bloom_add_params(bloom),
                    aux: if linear { Aux::BloomLinear } else { Aux::Bloom },
                },
            ]
        }
    }
}

fn grade_parts(passes: &[Pass]) -> (f32, Option<&Bloom>) {
    let lead = passes
        .iter()
        .take_while(|pass| matches!(pass, Pass::Exposure(_)))
        .count();
    let scale = passes[..lead]
        .iter()
        .map(|pass| match pass {
            Pass::Exposure(value) => *value,
            _ => 1.0,
        })
        .product();
    match passes.get(lead) {
        Some(Pass::Bloom(bloom)) => (scale, Some(bloom)),
        _ => (scale, None),
    }
}

fn grade_aux(bloom: Option<&Bloom>) -> Aux {
    match bloom.map(|bloom| bloom.kind) {
        None => Aux::Grade,
        Some(bloom::BloomKind::Box) => Aux::GradeBloomLinear,
        Some(bloom::BloomKind::Tent) => Aux::GradeBloom,
        Some(bloom::BloomKind::Rings) => Aux::GradeBloomB,
    }
}

fn grade_params(passes: &[Pass]) -> Vec<[f32; 16]> {
    let (scale, bloom) = grade_parts(passes);
    let mut out: Vec<[f32; 16]> = bloom.map_or_else(Vec::new, |bloom| {
        let mut stages = bloom_dispatches(bloom, scale);
        stages.pop();
        stages.into_iter().map(|stage| stage.params).collect()
    });
    out.push(fill(&[
        (1.0, 0),
        (bloom.map_or(0.0, |bloom| bloom.strength), 1),
    ]));
    out
}

fn grade_dispatches(passes: &[Pass]) -> Vec<GpuDispatch> {
    let (scale, bloom) = grade_parts(passes);
    let aux = grade_aux(bloom);
    let mut out = bloom.map_or_else(Vec::new, |bloom| {
        let mut stages = bloom_dispatches(bloom, scale);
        stages.pop();
        stages
    });
    out.push(GpuDispatch {
        shader: Cow::Owned(grade_shader(passes)),
        params: grade_params(passes).pop().unwrap_or([0.0; 16]),
        aux,
    });
    out
}

pub fn grade_shader(passes: &[Pass]) -> String {
    let aux = grade_aux(grade_parts(passes).1);
    let mut out = String::from(wgsl::HEAD);
    match aux {
        Aux::GradeBloom | Aux::GradeBloomB => out.push_str(wgsl::BLOOM_TEX),
        Aux::GradeBloomLinear => out.push_str(wgsl::BLOOM_LINEAR),
        _ => {}
    }
    out.push_str("\n@group(0) @binding(5) var<storage, read> grade: array<vec4<f32>>;\n");
    out.push_str(wgsl::GRADE_OPS);
    out.push_str(wgsl::COMMON);
    out.push_str(wgsl::MAIN);
    out.push_str("    var c = src;\n");
    let mut member = 0;
    for pass in passes {
        if matches!(pass, Pass::Bloom(_)) {
            out.push_str(match aux {
                Aux::GradeBloomLinear => {
                    "    let uv = bloom_uv(p);\n    c = as_half(vec4<f32>(c.rgb + textureSampleLevel(bloom_tex, bloom_sampler, uv, 0.0).rgb * post.p0.y, c.a));\n"
                }
                _ => {
                    "    let full = max(vec2<f32>(src_dim()), vec2<f32>(1.0));\n    let small = vec2<f32>(bloom_dim());\n    let s = sample_bloom((f32(p.x) + 0.5) / full.x * small.x - 0.5, (f32(p.y) + 0.5) / full.y * small.y - 0.5);\n    c = as_half(vec4<f32>(c.rgb + s.rgb * post.p0.y, c.a));\n"
                }
            });
            continue;
        }
        let at = member * 4;
        let first = if member == 0 && matches!(pass, Pass::Exposure(_)) {
            format!("grade[{at}u] * vec4<f32>(post.p0.x, 1.0, 1.0, 1.0)")
        } else {
            format!("grade[{at}u]")
        };
        let _ = writeln!(
            out,
            "    c = as_half(grade_{}(c, p, {first}, grade[{}u], grade[{}u], grade[{}u]));",
            pass.name(),
            at + 1,
            at + 2,
            at + 3
        );
        member += 1;
    }
    out.push_str("    store(p, c);\n}\n");
    out
}

pub fn grade_data(passes: &[Pass]) -> Vec<f32> {
    let mut out: Vec<f32> = passes
        .iter()
        .filter(|pass| !matches!(pass, Pass::Bloom(_)))
        .flat_map(|member| member.pack(Buffers::NONE))
        .collect();
    out.resize(out.len().max(16), 0.0);
    out
}

fn halation_dispatches(pass: &fx::Halation) -> Vec<GpuDispatch> {
    let spread = pass.radius.max(1) as f32 * 0.25;
    vec![
        GpuDispatch {
            shader: Cow::Borrowed(wgsl::HALATION_HOT),
            params: fill(&[(pass.threshold, 0), (4.0, 1)]),
            aux: Aux::None,
        },
        GpuDispatch {
            shader: Cow::Borrowed(wgsl::BLOOM_BLUR),
            params: fill(&[
                (1.0, 0),
                (0.0, 1),
                (pass.radius.div_ceil(4) as f32, 2),
                (spread, 3),
            ]),
            aux: Aux::None,
        },
        GpuDispatch {
            shader: Cow::Borrowed(wgsl::BLOOM_BLUR),
            params: fill(&[
                (0.0, 0),
                (1.0, 1),
                (pass.radius.div_ceil(4) as f32, 2),
                (spread, 3),
            ]),
            aux: Aux::None,
        },
        GpuDispatch {
            shader: Cow::Borrowed(wgsl::HALATION_ADD),
            params: fill(&[(pass.strength, 0)]),
            aux: Aux::Bloom,
        },
    ]
}

fn gradient_params(pass: &fx::GradientMap) -> [f32; 16] {
    let count = pass.count.clamp(2, 4) as usize;
    let mut stops = [0.0; 16];
    for i in 0..4 {
        let stop = pass.stops[i.min(count - 1)];
        stops[i * 4] = stop[0];
        stops[i * 4 + 1] = stop[1];
        stops[i * 4 + 2] = stop[2];
        stops[i * 4 + 3] = stop[3];
    }
    stops
}
