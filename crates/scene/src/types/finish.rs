use serde::{Deserialize, Serialize};

use super::either::{Color, either};
use super::keys::{Key, KeyShape, Keys};

pub const STYLES: &[&str] = &[
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
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum WarmthKind {
    Linear,
    LumaCurve,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct WarmthTable {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub high: Option<[f32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub highlight: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<WarmthKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub low: Option<[f32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Warmth {
    Amount(f32),
    Table(WarmthTable),
}

either!(Warmth, "a number or a table", number => Amount, map => Table(WarmthTable),);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ToneKind {
    Aces,
    Agx,
    Neutral,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ToneTable {
    pub kind: ToneKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clamp: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Tone {
    Kind(ToneKind),
    Table(ToneTable),
}

either!(Tone, "aces, agx, neutral or a table", text => Kind(ToneKind), map => Table(ToneTable),);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum BloomKind {
    Box,
    Rings,
    Tent,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct BloomTable {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clamp: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub down: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gains: Option<[f32; 4]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<BloomKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knee: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knee_width: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linear_taps: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radii: Option<[f32; 4]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sigma: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub soft: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strength: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Bloom {
    Strength(f32),
    Table(BloomTable),
}

either!(Bloom, "a number or a table", number => Strength, map => Table(BloomTable),);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum VignetteKind {
    Power,
    Smoothstep,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct VignetteTable {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aspect: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inner: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<VignetteKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outer: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub power: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strength: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Vignette {
    Strength(f32),
    Table(VignetteTable),
}

either!(Vignette, "a number or a table", number => Strength, map => Table(VignetteTable),);

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct GrainTable {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clamp: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strength: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Grain {
    Strength(f32),
    Table(GrainTable),
}

either!(Grain, "a number or a table", number => Strength, map => Table(GrainTable),);

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ChromaticTable {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub falloff: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Chromatic {
    Amount(f32),
    Table(ChromaticTable),
}

either!(Chromatic, "a number or a table", number => Amount, map => Table(ChromaticTable),);

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Dither {
    On(bool),
    Amplitude(f32),
}

either!(Dither, "a bool or a number", bool => On, number => Amplitude,);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum DitherKind {
    Blue,
    Ordered,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Lut {
    pub size: u32,
    pub values: Vec<f32>,
}

impl Lut {
    pub const SIZES: std::ops::RangeInclusive<u32> = 2..=64;

    pub fn needs(&self) -> usize {
        (self.size as usize).pow(3) * 3
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum LutKey {
    Path(String),
    Table(Lut),
}

either!(LutKey, "a path to a .cube file or a table", text => Path(String), map => Table(Lut),);

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct OutlineTable {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alpha: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<Color>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crease_angle: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_color: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thickness: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Outline {
    On(bool),
    Table(OutlineTable),
}

either!(Outline, "a bool or a table", bool => On, map => Table(OutlineTable),);

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ScratchesTable {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strength: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Scratches {
    Strength(f32),
    Table(ScratchesTable),
}

either!(Scratches, "a number or a table", number => Strength, map => Table(ScratchesTable),);

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Ink {
    Amount(f32),
    Color(Color),
}

either!(Ink, "a number, \"#rrggbb\" or three numbers", number => Amount, text => Color(Color), seq => Color(Color),);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum TapeDirection {
    Forward,
    Rewind,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct TapeTable {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bands: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<TapeDirection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub echo: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grain: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smear: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub split: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strength: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wash: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Tape {
    Direction(TapeDirection),
    Strength(f32),
    Table(TapeTable),
}

either!(Tape, "forward, rewind, a number or a table", number => Strength, text => Direction(TapeDirection), map => Table(TapeTable),);

macro_rules! finish {
    ($($field:ident: $kind:ty,)*) => {
        #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
        #[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
        #[serde(deny_unknown_fields)]
        pub struct FinishKeys {
            $(
                #[serde(default, skip_serializing_if = "Option::is_none")]
                pub $field: Option<$kind>,
            )*
        }

        #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
        #[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
        #[serde(deny_unknown_fields)]
        pub struct Finish {
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub file: Option<String>,
            $(
                #[serde(default, skip_serializing_if = "Option::is_none")]
                pub $field: Option<$kind>,
            )*
            #[serde(default, skip_serializing_if = "Vec::is_empty")]
            pub pass: Vec<FinishKeys>,
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub keys: Option<Keys>,
        }

        impl Finish {
            pub fn keys(&self) -> FinishKeys {
                FinishKeys {
                    $($field: self.$field.clone(),)*
                }
            }
        }

        impl FinishKeys {
            pub const KEYS: &[&str] = &[$(stringify!($field),)*];
        }
    };
}

finish! {
    aberration: f32,
    angle: f32,
    black: f32,
    black_threshold: f32,
    bloom: Bloom,
    cavity: f32,
    cavity_distance: f32,
    cel_bands: f32,
    cel_shadow: f32,
    cel_softness: f32,
    cel_spec: f32,
    cel_spec_roughness: f32,
    cel_spec_threshold: f32,
    cel_threshold: f32,
    chromatic: Chromatic,
    contrast: f32,
    crease_angle: f32,
    crush: f32,
    curvature: f32,
    darkening: f32,
    dither: Dither,
    dither_kind: DitherKind,
    distortion: f32,
    dot: f32,
    dust: f32,
    encode: bool,
    exposure: f32,
    flicker: f32,
    focus: f32,
    frame: u32,
    gain: f32,
    grain: Grain,
    halation: f32,
    highlight: Color,
    highlight_threshold: f32,
    ink: Ink,
    interior_line_strength: f32,
    keep: Color,
    keep_range: f32,
    levels: f32,
    line_weight: f32,
    lut: LutKey,
    lut_amount: f32,
    mask: f32,
    outline: Outline,
    outline_alpha: f32,
    outline_color: Color,
    outline_thickness: f32,
    palette: bool,
    paper_grain: f32,
    pixel: u32,
    posterize: f32,
    radius: f32,
    rim: f32,
    rim_color: Color,
    rim_width: f32,
    saturation: f32,
    saturation_lift: f32,
    scale: f32,
    scanlines: f32,
    scratches: Scratches,
    seed: u32,
    sepia: f32,
    shadow: Color,
    style: Style,
    tape: Tape,
    threshold: f32,
    tilt_range: f32,
    tone: Tone,
    tone_steps: f32,
    vignette: Vignette,
    warmth: Warmth,
    weave: f32,
    weights: [f32; 3],
}

impl Finish {
    pub const KEYED: &[(&str, KeyShape)] = &[
        ("exposure", KeyShape::Number),
        ("lut_amount", KeyShape::Number),
    ];

    pub fn track(&self, name: &str) -> Option<Vec<Key>> {
        self.keys.as_ref()?.get(name).map(<[Key]>::to_vec)
    }
}

impl FinishKeys {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    pub fn to_table(&self) -> toml::Table {
        toml::Table::try_from(self).unwrap_or_default()
    }
}
