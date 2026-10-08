use crate::bloom::Bloom;
use crate::chain::{Chain, Pass};
use crate::color::REC709;
use crate::comic::ModernComic;
use crate::fx::{
    self, Bw, Cel, Comic, Duotone, GradientMap, Halation, Halftone, Neon, Noir, OneBit, Outline,
    PaperGrain, Pixelate, Scanlines, TiltShift, Watercolor,
};
use crate::grade::{Grain, Vignette};
use crate::tone::Tone;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    Cel,
    Bw,
    Noir,
    Vignette,
    Neon,
    RubberHose,
    Sepia,
    Film,
    Crt,
    OneBit,
    Halftone,
    Comic,
    ModernComic,
    Watercolor,
    PaperGrain,
    Pixel,
    Duotone,
    GradientMap,
    Posterize,
    Kuwahara,
    TiltShift,
}

impl Style {
    pub const ALL: [Self; 21] = [
        Self::Cel,
        Self::Bw,
        Self::Noir,
        Self::Vignette,
        Self::Neon,
        Self::RubberHose,
        Self::Sepia,
        Self::Film,
        Self::Crt,
        Self::OneBit,
        Self::Halftone,
        Self::Comic,
        Self::ModernComic,
        Self::Watercolor,
        Self::PaperGrain,
        Self::Pixel,
        Self::Duotone,
        Self::GradientMap,
        Self::Posterize,
        Self::Kuwahara,
        Self::TiltShift,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Cel => "cel",
            Self::Bw => "bw",
            Self::Noir => "noir",
            Self::Vignette => "vignette",
            Self::Neon => "neon",
            Self::RubberHose => "rubber_hose",
            Self::Sepia => "sepia",
            Self::Film => "film",
            Self::Crt => "crt",
            Self::OneBit => "one_bit",
            Self::Halftone => "halftone",
            Self::Comic => "comic",
            Self::ModernComic => "modern_comic",
            Self::Watercolor => "watercolor",
            Self::PaperGrain => "paper_grain",
            Self::Pixel => "pixel",
            Self::Duotone => "duotone",
            Self::GradientMap => "gradient_map",
            Self::Posterize => "posterize",
            Self::Kuwahara => "kuwahara",
            Self::TiltShift => "tilt_shift",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|style| style.name() == name)
    }

    pub fn chain(self) -> Chain {
        Chain {
            passes: self.passes(),
            seed: 0,
            frame: 0,
        }
    }

    fn passes(self) -> Vec<Pass> {
        match self {
            Self::Cel => vec![
                Pass::Exposure(1.5),
                Pass::Cel(Cel {
                    bands: 3,
                    shadow: 0.62,
                    threshold: 0.45,
                    softness: 0.045,
                    spec: 0.35,
                    spec_roughness: 0.25,
                    spec_threshold: 0.72,
                    light: [0.3, 0.75, 0.6],
                }),
                Pass::Cavity(fx::Cavity {
                    strength: 0.35,
                    distance: 2.0,
                }),
                Pass::Rim(fx::Rim {
                    strength: 0.3,
                    width: 0.18,
                    color: [0.62, 0.82, 1.0],
                }),
                Pass::Outline(Outline {
                    enabled: true,
                    thickness: 1.25,
                    color: [0.02, 0.015, 0.02],
                    local_color: true,
                    alpha: 0.92,
                    crease_angle: 35.0,
                    depth_gap: 0.003,
                }),
            ],
            Self::Bw => vec![Pass::Bw(Bw { weights: REC709 })],
            Self::Noir => vec![
                Pass::Noir(Noir {
                    contrast: 1.9,
                    crush: 0.18,
                    keep: None,
                    keep_range: 0.45,
                }),
                Pass::Grain(Grain {
                    strength: 0.07,
                    response: 0.45,
                    clamp: false,
                }),
                Pass::Vignette(Vignette::power(0.62, 2.4, 0.8, 1.28)),
            ],
            Self::Vignette => vec![Pass::Vignette(Vignette::power(0.55, 2.4, 0.8, 1.28))],
            Self::Neon => vec![
                Pass::Neon(Neon {
                    saturation: 1.8,
                    strength: 1.35,
                    radius: 3,
                }),
                Pass::Bloom(Bloom {
                    linear_taps: false,
                    ..Bloom::pyramid(0.85, 0.55, 0.4, 0.3, 0.2, 5, 10.0, 4)
                }),
            ],
            Self::RubberHose => vec![Pass::RubberHose],
            Self::Sepia => vec![Pass::Sepia(1.0)],
            Self::Film => vec![
                Pass::Distortion(0.045),
                Pass::Aberration(0.85),
                Pass::Halation(Halation {
                    strength: 0.4,
                    threshold: 0.65,
                    radius: 6,
                }),
                Pass::Tone(Tone::aces()),
                Pass::Sepia(0.18),
                Pass::Grain(Grain {
                    strength: 0.03,
                    response: 0.5,
                    clamp: true,
                }),
                Pass::Vignette(Vignette::smoothstep(0.3, 0.72, 0.28, 0.8)),
            ],
            Self::Crt => vec![
                Pass::Curvature(0.12),
                Pass::Aberration(0.65),
                Pass::Bloom(Bloom::pyramid(0.2, 0.9, 0.7, 0.6, 0.15, 12, 50.0, 2)),
                Pass::Scanlines(Scanlines {
                    strength: 0.38,
                    period: 2,
                }),
                Pass::Aperture(0.45),
                Pass::Vignette(Vignette::power(0.4, 2.4, 0.8, 1.28)),
            ],
            Self::OneBit => vec![Pass::OneBit(OneBit {
                kind: fx::NoiseKind::Blue,
            })],
            Self::Halftone => vec![Pass::Halftone(Halftone {
                cell: 6.0,
                angle: 15.0,
                ink: [0.02, 0.02, 0.03],
            })],
            Self::Comic => vec![Pass::Comic(Comic {
                levels: 5,
                ink: 0.9,
            })],
            Self::ModernComic => vec![Pass::ModernComic(ModernComic::default())],
            Self::Watercolor => vec![Pass::Watercolor(Watercolor {
                darkening: 0.65,
                grain: 0.22,
                scale: 6.0,
            })],
            Self::PaperGrain => vec![Pass::PaperGrain(PaperGrain {
                strength: 0.28,
                scale: 5.0,
            })],
            Self::Pixel => vec![Pass::Pixel(Pixelate {
                size: 4,
                levels: 0,
                palette: true,
            })],
            Self::Duotone => vec![Pass::Duotone(Duotone {
                shadow: [0.05, 0.08, 0.18],
                highlight: [0.96, 0.82, 0.55],
            })],
            Self::GradientMap => vec![Pass::GradientMap(GradientMap {
                stops: [
                    [0.04, 0.02, 0.05, 0.0],
                    [0.42, 0.16, 0.10, 0.35],
                    [0.86, 0.52, 0.28, 0.7],
                    [0.96, 0.90, 0.78, 1.0],
                ],
                count: 4,
            })],
            Self::Posterize => vec![Pass::Posterize(5.0)],
            Self::Kuwahara => vec![Pass::Kuwahara(2)],
            Self::TiltShift => vec![Pass::TiltShift(TiltShift {
                focus: 0.5,
                range: 0.22,
                radius: 5.0,
            })],
        }
    }
}

pub fn names() -> &'static [&'static str] {
    &[
        "cel",
        "bw",
        "noir",
        "vignette",
        "neon",
        "rubber_hose",
        "sepia",
        "film",
        "crt",
        "one_bit",
        "halftone",
        "comic",
        "modern_comic",
        "watercolor",
        "paper_grain",
        "pixel",
        "duotone",
        "gradient_map",
        "posterize",
        "kuwahara",
        "tilt_shift",
    ]
}
