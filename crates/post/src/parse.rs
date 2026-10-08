use crate::bloom::{Bloom, NEUTRAL_GAINS, NEUTRAL_RADII};
use crate::chain::{Chain, Pass};
use crate::color;
use crate::comic::ModernComic;
use crate::fx::{self, NoiseKind};
use crate::grade::{Contrast, Dither, Grain, Saturation, Vignette, Warmth};
use crate::preset::{self, Style};
use crate::tone::Tone;
use toml::{Table, Value};

pub const KEYS: &[&str] = &[
    "aberration",
    "angle",
    "black",
    "black_threshold",
    "bloom",
    "cavity",
    "cavity_distance",
    "cel_bands",
    "cel_shadow",
    "cel_softness",
    "cel_spec",
    "cel_spec_roughness",
    "cel_spec_threshold",
    "cel_threshold",
    "contrast",
    "crease_angle",
    "crush",
    "curvature",
    "darkening",
    "dither",
    "dither_kind",
    "distortion",
    "dot",
    "dust",
    "encode",
    "exposure",
    "flicker",
    "focus",
    "frame",
    "gain",
    "grain",
    "halation",
    "highlight",
    "highlight_threshold",
    "ink",
    "interior_line_strength",
    "keep",
    "keep_range",
    "levels",
    "line_weight",
    "lut",
    "mask",
    "outline",
    "outline_alpha",
    "outline_color",
    "outline_thickness",
    "palette",
    "paper_grain",
    "pixel",
    "posterize",
    "radius",
    "rim",
    "rim_color",
    "rim_width",
    "saturation",
    "saturation_lift",
    "scale",
    "scanlines",
    "scratches",
    "seed",
    "sepia",
    "shadow",
    "style",
    "tape",
    "threshold",
    "tilt_range",
    "tone",
    "tone_steps",
    "vignette",
    "warmth",
    "weave",
    "weights",
];

const BLOOM_KEYS: &[&str] = &[
    "clamp",
    "down",
    "gains",
    "kind",
    "knee",
    "knee_width",
    "linear_taps",
    "radii",
    "radius",
    "sigma",
    "soft",
    "strength",
    "threshold",
    "unit",
];

const VIGNETTE_KEYS: &[&str] = &[
    "aspect", "inner", "kind", "outer", "power", "scale", "strength",
];

const GRAIN_KEYS: &[&str] = &["clamp", "response", "strength"];

const LUT_KEYS: &[&str] = &["size", "values"];

const OUTLINE_KEYS: &[&str] = &[
    "alpha",
    "color",
    "crease_angle",
    "enabled",
    "local_color",
    "thickness",
];

const SCRATCH_KEYS: &[&str] = &["count", "strength"];

const KINDS: &[&str] = &["box", "rings", "tent"];

const VIGNETTE_KINDS: &[&str] = &["power", "smoothstep"];

const WARMTH_KEYS: &[&str] = &["amount", "high", "highlight", "kind", "low", "shadow"];

const TONE_KEYS: &[&str] = &["clamp", "kind", "start"];

const TONES: &[&str] = &["aces", "agx", "neutral"];

const DITHER_KINDS: &[&str] = &["blue", "ordered"];

#[derive(Clone, Debug)]
pub struct ParseError {
    message: String,
}

impl ParseError {
    fn msg(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ParseError {}

pub fn parse_str(text: &str) -> Result<Chain, ParseError> {
    let table: Table = text
        .parse()
        .map_err(|err| ParseError::msg(format!("toml: {err}")))?;
    parse(&table)
}

pub fn parse(table: &Table) -> Result<Chain, ParseError> {
    if let Some(bad) = table.keys().find(|key| !KEYS.contains(&key.as_str())) {
        return Err(ParseError::msg(format!(
            "unknown key {bad}; allowed: {}",
            KEYS.join(", ")
        )));
    }
    let mut chain = match table.get("style") {
        None => Chain::new(),
        Some(value) => {
            let name = value
                .as_str()
                .ok_or_else(|| ParseError::msg("style is a string"))?;
            Style::parse(name)
                .ok_or_else(|| {
                    ParseError::msg(format!(
                        "unknown style {name}; allowed: {}",
                        preset::names().join(", ")
                    ))
                })?
                .chain()
        }
    };
    if let Some(value) = table.get("seed") {
        chain.seed = integer(value, "seed")?;
    }
    if let Some(value) = table.get("frame") {
        chain.frame = integer(value, "frame")?;
    }
    for (key, value) in table {
        match key.as_str() {
            "style" | "seed" | "frame" => {}
            "exposure" => set_exposure(&mut chain, number(value, key)?),
            "black" => set_black(&mut chain, number(value, key)?),
            "black_threshold" => {
                let n = number(value, key)?;
                set_modern_comic(&mut chain, |p| p.black_threshold = n);
            }
            "saturation" => set_saturation(&mut chain, number(value, key)?),
            "saturation_lift" => {
                let n = number(value, key)?;
                set_modern_comic(&mut chain, |p| p.saturation_lift = n);
            }
            "contrast" => set_contrast(&mut chain, number(value, key)?),
            "warmth" => set_warmth(&mut chain, value)?,
            "gain" => set_gain(&mut chain, number(value, key)?),
            "tone" => set_tone(&mut chain, value)?,
            "tone_steps" => {
                let n = number_u(value, key)?;
                if !(3..=5).contains(&n) {
                    return Err(ParseError::msg("tone_steps is 3 to 5"));
                }
                set_modern_comic(&mut chain, |p| p.tone_steps = n);
            }
            "highlight_threshold" => {
                let n = number(value, key)?;
                set_modern_comic(&mut chain, |p| p.highlight_threshold = n);
            }
            "line_weight" => {
                let n = number(value, key)?;
                set_modern_comic(&mut chain, |p| p.line_weight = n);
            }
            "interior_line_strength" => {
                let n = number(value, key)?;
                set_modern_comic(&mut chain, |p| p.interior_line_strength = n);
            }
            "threshold" => set_threshold(&mut chain, number(value, key)?),
            "bloom" => set_bloom(&mut chain, value)?,
            "vignette" => set_vignette(&mut chain, value)?,
            "grain" => set_grain(&mut chain, value)?,
            "dither" => set_dither(&mut chain, value)?,
            "dither_kind" => set_dither_kind(&mut chain, value)?,
            "encode" => {
                if boolean(value, key)? {
                    push_missing(
                        &mut chain,
                        |pass| matches!(pass, Pass::Encode),
                        Pass::Encode,
                    );
                }
            }
            "lut" => set_lut(&mut chain, value)?,
            "weights" => set_weights(&mut chain, value)?,
            "cel_bands" => {
                let n = number_u(value, key)?;
                set_cel(&mut chain, |cel| cel.bands = n);
            }
            "cel_shadow" => {
                let n = number(value, key)?;
                set_cel(&mut chain, |cel| cel.shadow = n);
            }
            "cel_threshold" => {
                let n = number(value, key)?;
                set_cel(&mut chain, |cel| cel.threshold = n);
            }
            "cel_softness" => {
                let n = number(value, key)?;
                set_cel(&mut chain, |cel| cel.softness = n);
            }
            "cel_spec" => {
                let n = number(value, key)?;
                set_cel(&mut chain, |cel| cel.spec = n);
            }
            "cel_spec_roughness" => {
                let n = number(value, key)?;
                set_cel(&mut chain, |cel| cel.spec_roughness = n);
            }
            "cel_spec_threshold" => {
                let n = number(value, key)?;
                set_cel(&mut chain, |cel| cel.spec_threshold = n);
            }
            "cavity" => {
                let n = number(value, key)?;
                set_cavity(&mut chain, |c| c.strength = n);
            }
            "cavity_distance" => {
                let n = number(value, key)?;
                set_cavity(&mut chain, |c| c.distance = n);
            }
            "rim" => {
                let n = number(value, key)?;
                set_rim(&mut chain, |r| r.strength = n);
            }
            "rim_width" => {
                let n = number(value, key)?;
                set_rim(&mut chain, |r| r.width = n);
            }
            "rim_color" => {
                let c = color(value, key)?;
                set_rim(&mut chain, |r| r.color = c);
            }
            "outline" => set_outline(&mut chain, value)?,
            "outline_thickness" => {
                let n = number(value, key)?;
                set_outline_field(&mut chain, |o| o.thickness = n);
            }
            "outline_alpha" => {
                let n = number(value, key)?;
                set_outline_field(&mut chain, |o| o.alpha = n);
            }
            "outline_color" => {
                let c = color(value, key)?;
                set_outline_field(&mut chain, |o| o.color = c);
            }
            "crease_angle" => {
                let n = number(value, key)?;
                set_outline_field(&mut chain, |o| o.crease_angle = n);
            }
            "keep" => {
                let c = color(value, key)?;
                set_noir(&mut chain, |n| n.keep = Some(c));
            }
            "keep_range" => {
                let n = number(value, key)?;
                set_noir(&mut chain, |noir| noir.keep_range = n);
            }
            "crush" => {
                let n = number(value, key)?;
                set_noir(&mut chain, |noir| noir.crush = n);
            }
            "sepia" => set_sepia(&mut chain, number(value, key)?),
            "posterize" | "levels" => set_levels(&mut chain, number(value, key)?),
            "flicker" => set_f32(
                &mut chain,
                number(value, key)?,
                |p| matches!(p, Pass::Flicker(_)),
                Pass::Flicker,
            ),
            "weave" => set_f32(
                &mut chain,
                number(value, key)?,
                |p| matches!(p, Pass::Weave(_)),
                Pass::Weave,
            ),
            "dust" => set_f32(
                &mut chain,
                number(value, key)?,
                |p| matches!(p, Pass::Dust(_)),
                Pass::Dust,
            ),
            "scratches" => set_scratches(&mut chain, value)?,
            "halation" => set_halation(&mut chain, number(value, key)?),
            "aberration" => set_f32(
                &mut chain,
                number(value, key)?,
                |p| matches!(p, Pass::Aberration(_)),
                Pass::Aberration,
            ),
            "distortion" => set_f32(
                &mut chain,
                number(value, key)?,
                |p| matches!(p, Pass::Distortion(_)),
                Pass::Distortion,
            ),
            "curvature" => set_f32(
                &mut chain,
                number(value, key)?,
                |p| matches!(p, Pass::Curvature(_)),
                Pass::Curvature,
            ),
            "scanlines" => set_scanlines(&mut chain, number(value, key)?),
            "mask" => set_f32(
                &mut chain,
                number(value, key)?,
                |p| matches!(p, Pass::Aperture(_)),
                Pass::Aperture,
            ),
            "dot" => {
                let n = number(value, key)?;
                set_halftone(&mut chain, |h| h.cell = n);
            }
            "angle" => {
                let n = number(value, key)?;
                set_halftone(&mut chain, |h| h.angle = n);
            }
            "ink" => set_ink(&mut chain, value)?,
            "darkening" => {
                let n = number(value, key)?;
                set_water(&mut chain, |w| w.darkening = n);
            }
            "paper_grain" => set_paper_grain(&mut chain, number(value, key)?),
            "scale" => set_scale(&mut chain, number(value, key)?),
            "pixel" => set_pixel_size(&mut chain, integer(value, key)?),
            "palette" => set_palette(&mut chain, boolean(value, key)?),
            "shadow" => {
                let c = color(value, key)?;
                set_duotone(&mut chain, |d| d.shadow = c);
            }
            "highlight" => {
                let c = color(value, key)?;
                set_duotone(&mut chain, |d| d.highlight = c);
            }
            "radius" => set_radius(&mut chain, number(value, key)?),
            "tape" => {
                let tape = crate::tape::read(value, chain.tape()).map_err(ParseError::msg)?;
                chain.set_tape(tape);
            }
            "focus" => {
                let n = number(value, key)?;
                set_tilt(&mut chain, |t| t.focus = n);
            }
            "tilt_range" => {
                let n = number(value, key)?;
                set_tilt(&mut chain, |t| t.range = n);
            }
            other => {
                return Err(ParseError::msg(format!(
                    "unknown key {other}; allowed: {}",
                    KEYS.join(", ")
                )));
            }
        }
    }
    Ok(chain)
}

fn number_u(value: &Value, key: &str) -> Result<u32, ParseError> {
    let n = number(value, key)?;
    if n < 0.0 {
        return Err(ParseError::msg(format!("{key} is a positive number")));
    }
    Ok(n.round() as u32)
}

fn number(value: &Value, key: &str) -> Result<f32, ParseError> {
    let n = if let Some(n) = value.as_float() {
        n
    } else if let Some(n) = value.as_integer() {
        n as f64
    } else {
        return Err(ParseError::msg(format!("{key} is a number")));
    };
    if !n.is_finite() {
        return Err(ParseError::msg(format!("{key} is a number")));
    }
    Ok(n as f32)
}

fn integer(value: &Value, key: &str) -> Result<u32, ParseError> {
    let Some(n) = value.as_integer() else {
        return Err(ParseError::msg(format!("{key} is an integer")));
    };
    u32::try_from(n).map_err(|_| ParseError::msg(format!("{key} is an integer")))
}

fn boolean(value: &Value, key: &str) -> Result<bool, ParseError> {
    value
        .as_bool()
        .ok_or_else(|| ParseError::msg(format!("{key} is a bool")))
}

fn color(value: &Value, key: &str) -> Result<[f32; 3], ParseError> {
    if let Some(text) = value.as_str() {
        return color::parse_hex(text)
            .ok_or_else(|| ParseError::msg(format!("{key} is #rrggbb or three numbers")));
    }
    let Some(items) = value.as_array() else {
        return Err(ParseError::msg(format!(
            "{key} is #rrggbb or three numbers"
        )));
    };
    if items.len() != 3 {
        return Err(ParseError::msg(format!(
            "{key} is #rrggbb or three numbers"
        )));
    }
    Ok([
        number(&items[0], key)?,
        number(&items[1], key)?,
        number(&items[2], key)?,
    ])
}

fn floats(value: &Value, key: &str, len: usize) -> Result<Vec<f32>, ParseError> {
    let Some(items) = value.as_array() else {
        return Err(ParseError::msg(format!("{key} is {len} numbers")));
    };
    if items.len() != len {
        return Err(ParseError::msg(format!("{key} is {len} numbers")));
    }
    items.iter().map(|item| number(item, key)).collect()
}

fn unknown(prefix: &str, key: &str, allowed: &[&str]) -> ParseError {
    ParseError::msg(format!(
        "unknown key {prefix}.{key}; allowed: {}",
        allowed.join(", ")
    ))
}

fn push_missing(chain: &mut Chain, found: impl Fn(&Pass) -> bool, pass: Pass) {
    if !chain.passes.iter().any(found) {
        chain.passes.push(pass);
    }
}

fn set_exposure(chain: &mut Chain, value: f32) {
    for pass in &mut chain.passes {
        if let Pass::Exposure(slot) = pass {
            *slot = value;
            return;
        }
    }
    chain.passes.insert(0, Pass::Exposure(value));
}

fn set_black(chain: &mut Chain, value: f32) {
    for pass in &mut chain.passes {
        if let Pass::Black(slot) = pass {
            *slot = value;
            return;
        }
    }
    chain.passes.push(Pass::Black(value));
}

fn set_saturation(chain: &mut Chain, value: f32) {
    for pass in &mut chain.passes {
        if let Pass::Saturation(slot) = pass {
            slot.amount = value;
            return;
        }
    }
    chain
        .passes
        .push(Pass::Saturation(Saturation::rec709(value)));
}

fn set_contrast(chain: &mut Chain, value: f32) {
    for pass in &mut chain.passes {
        match pass {
            Pass::Contrast(Contrast::Power { power, .. }) => {
                *power = value;
                return;
            }
            Pass::Contrast(Contrast::Pivot { amount, .. }) => {
                *amount = value;
                return;
            }
            Pass::Noir(noir) => {
                noir.contrast = value;
                return;
            }
            _ => {}
        }
    }
    chain.passes.push(Pass::Contrast(Contrast::Pivot {
        amount: value,
        pivot: 0.5,
    }));
}

fn set_warmth(chain: &mut Chain, value: &Value) -> Result<(), ParseError> {
    if let Some(table) = value.as_table() {
        if let Some(bad) = table
            .keys()
            .find(|key| !WARMTH_KEYS.contains(&key.as_str()))
        {
            return Err(unknown("warmth", bad, WARMTH_KEYS));
        }
        if let Some(kind) = table.get("kind") {
            let name = kind
                .as_str()
                .ok_or_else(|| ParseError::msg("warmth.kind is a string"))?;
            let amount = warmth_amount(chain);
            let next = match name {
                "linear" => Warmth::linear(amount),
                "luma_curve" => Warmth::luma_curve(amount, [1.0; 2], [1.0; 2], 0.0, 1.0),
                other => {
                    return Err(ParseError::msg(format!(
                        "unknown warmth.kind {other}; allowed: linear, luma_curve"
                    )));
                }
            };
            replace_warmth(chain, next);
        }
        if let Some(v) = table.get("amount") {
            warmth_mut(chain).amount = number(v, "warmth.amount")?;
        }
        if let Some(v) = table.get("low") {
            let items = floats(v, "warmth.low", 2)?;
            warmth_mut(chain).low = [items[0], items[1]];
        }
        if let Some(v) = table.get("high") {
            let items = floats(v, "warmth.high", 2)?;
            warmth_mut(chain).high = [items[0], items[1]];
        }
        if let Some(v) = table.get("shadow") {
            warmth_mut(chain).shadow = number(v, "warmth.shadow")?;
        }
        if let Some(v) = table.get("highlight") {
            warmth_mut(chain).highlight = number(v, "warmth.highlight")?;
        }
        if !chain
            .passes
            .iter()
            .any(|pass| matches!(pass, Pass::Warmth(_)))
        {
            chain.passes.push(Pass::Warmth(Warmth::linear(0.0)));
        }
        return Ok(());
    }
    let n = number(value, "warmth")?;
    if chain
        .passes
        .iter()
        .any(|pass| matches!(pass, Pass::Warmth(_)))
    {
        warmth_mut(chain).amount = n;
    } else {
        chain.passes.push(Pass::Warmth(Warmth::linear(n)));
    }
    Ok(())
}

fn warmth_amount(chain: &Chain) -> f32 {
    chain
        .passes
        .iter()
        .find_map(|pass| match pass {
            Pass::Warmth(warmth) => Some(warmth.amount),
            _ => None,
        })
        .unwrap_or(0.0)
}

fn replace_warmth(chain: &mut Chain, warmth: Warmth) {
    for pass in &mut chain.passes {
        if let Pass::Warmth(slot) = pass {
            *slot = warmth;
            return;
        }
    }
    chain.passes.push(Pass::Warmth(warmth));
}

fn warmth_mut(chain: &mut Chain) -> &mut Warmth {
    if !chain
        .passes
        .iter()
        .any(|pass| matches!(pass, Pass::Warmth(_)))
    {
        chain.passes.push(Pass::Warmth(Warmth::linear(0.0)));
    }
    let Some(Pass::Warmth(slot)) = chain
        .passes
        .iter_mut()
        .find(|pass| matches!(pass, Pass::Warmth(_)))
    else {
        unreachable!("warmth was inserted");
    };
    slot
}

fn set_gain(chain: &mut Chain, value: f32) {
    for pass in &mut chain.passes {
        if let Pass::Tone(Tone::Aces { gain }) = pass {
            *gain = value;
            return;
        }
    }
    chain.passes.push(Pass::Tone(Tone::Aces { gain: value }));
}

fn set_tone(chain: &mut Chain, value: &Value) -> Result<(), ParseError> {
    let tone = if let Some(name) = value.as_str() {
        match name {
            "aces" => Tone::aces(),
            "agx" => Tone::Agx,
            "neutral" => existing_neutral(chain).unwrap_or_else(|| Tone::neutral(1.0, false)),
            other => {
                return Err(ParseError::msg(format!(
                    "unknown tone {other}; allowed: {}",
                    TONES.join(", ")
                )));
            }
        }
    } else if let Some(table) = value.as_table() {
        tone_from_table(table)?
    } else {
        return Err(ParseError::msg("tone is aces, agx, neutral, or a table"));
    };
    for pass in &mut chain.passes {
        if let Pass::Tone(slot) = pass {
            if matches!(
                (&*slot, &tone),
                (Tone::Neutral { .. }, Tone::Neutral { .. })
            ) {
                return Ok(());
            }
            *slot = tone;
            return Ok(());
        }
    }
    chain.passes.push(Pass::Tone(tone));
    Ok(())
}

fn tone_from_table(table: &Table) -> Result<Tone, ParseError> {
    if let Some(bad) = table.keys().find(|key| !TONE_KEYS.contains(&key.as_str())) {
        return Err(unknown("tone", bad, TONE_KEYS));
    }
    let kind = table
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| ParseError::msg("tone.kind is a string"))?;
    match kind {
        "aces" | "agx" => {
            if table.contains_key("start") || table.contains_key("clamp") {
                return Err(ParseError::msg("tone.start and tone.clamp are for neutral"));
            }
            Ok(if kind == "aces" {
                Tone::aces()
            } else {
                Tone::Agx
            })
        }
        "neutral" => {
            let start = match table.get("start") {
                Some(value) => number(value, "tone.start")?,
                None => 1.0,
            };
            let clamp = match table.get("clamp") {
                Some(value) => boolean(value, "tone.clamp")?,
                None => false,
            };
            Ok(Tone::neutral(start, clamp))
        }
        other => Err(ParseError::msg(format!(
            "unknown tone.kind {other}; allowed: {}",
            TONES.join(", ")
        ))),
    }
}

fn existing_neutral(chain: &Chain) -> Option<Tone> {
    chain.passes.iter().find_map(|pass| match pass {
        Pass::Tone(tone @ Tone::Neutral { .. }) => Some(*tone),
        _ => None,
    })
}

fn ensure_bloom(chain: &mut Chain) -> &mut Bloom {
    if !chain
        .passes
        .iter()
        .any(|pass| matches!(pass, Pass::Bloom(_)))
    {
        let index = chain
            .passes
            .iter()
            .position(|pass| matches!(pass, Pass::Exposure(_)))
            .map(|i| i + 1)
            .unwrap_or(0);
        chain.passes.insert(index, Pass::Bloom(Bloom::neutral(0.0)));
    }
    let Some(Pass::Bloom(bloom)) = chain
        .passes
        .iter_mut()
        .find(|pass| matches!(pass, Pass::Bloom(_)))
    else {
        unreachable!("bloom was inserted");
    };
    bloom
}

fn set_threshold(chain: &mut Chain, value: f32) {
    ensure_bloom(chain).threshold = value;
}

fn set_bloom(chain: &mut Chain, value: &Value) -> Result<(), ParseError> {
    if let Some(table) = value.as_table() {
        if let Some(bad) = table.keys().find(|key| !BLOOM_KEYS.contains(&key.as_str())) {
            return Err(unknown("bloom", bad, BLOOM_KEYS));
        }
        let bloom = ensure_bloom(chain);
        if let Some(kind) = table.get("kind") {
            let name = kind
                .as_str()
                .ok_or_else(|| ParseError::msg("bloom.kind is a string"))?;
            *bloom = match name {
                "box" => Bloom::neutral(bloom.strength),
                "tent" => Bloom::tent(bloom.strength, 1.0, 4, 16.0, 2),
                "rings" => Bloom::rings(bloom.strength, 1.0, 4, NEUTRAL_RADII, NEUTRAL_GAINS),
                other => {
                    return Err(ParseError::msg(format!(
                        "unknown bloom.kind {other}; allowed: {}",
                        KINDS.join(", ")
                    )));
                }
            };
        }
        let bloom = ensure_bloom(chain);
        if let Some(v) = table.get("strength") {
            bloom.strength = number(v, "bloom.strength")?;
        }
        if let Some(v) = table.get("threshold") {
            bloom.threshold = number(v, "bloom.threshold")?;
        }
        if let Some(v) = table.get("knee") {
            bloom.knee = number(v, "bloom.knee")?;
        }
        if let Some(v) = table.get("knee_width") {
            bloom.knee_width = number(v, "bloom.knee_width")?;
        }
        if let Some(v) = table.get("soft") {
            bloom.soft = number(v, "bloom.soft")?;
        }
        if let Some(v) = table.get("radius") {
            bloom.radius = number_u(v, "bloom.radius")?;
        }
        if let Some(v) = table.get("sigma") {
            bloom.spread = number(v, "bloom.sigma")?;
        }
        if let Some(v) = table.get("down") {
            bloom.down = number_u(v, "bloom.down")?.max(1);
        }
        if let Some(v) = table.get("clamp") {
            bloom.clamp = number(v, "bloom.clamp")?;
        }
        if let Some(v) = table.get("unit") {
            bloom.unit = number(v, "bloom.unit")?;
        }
        if let Some(v) = table.get("linear_taps") {
            bloom.linear_taps = boolean(v, "bloom.linear_taps")?;
        }
        if let Some(v) = table.get("radii") {
            let items = floats(v, "bloom.radii", 4)?;
            bloom.radii = [items[0], items[1], items[2], items[3]];
        }
        if let Some(v) = table.get("gains") {
            let items = floats(v, "bloom.gains", 4)?;
            bloom.gains = [items[0], items[1], items[2], items[3]];
        }
        return Ok(());
    }
    ensure_bloom(chain).strength = number(value, "bloom")?;
    Ok(())
}

fn set_vignette(chain: &mut Chain, value: &Value) -> Result<(), ParseError> {
    if let Some(table) = value.as_table() {
        if let Some(bad) = table
            .keys()
            .find(|key| !VIGNETTE_KEYS.contains(&key.as_str()))
        {
            return Err(unknown("vignette", bad, VIGNETTE_KEYS));
        }
        ensure_vignette(chain);
        if let Some(kind) = table.get("kind") {
            let name = kind
                .as_str()
                .ok_or_else(|| ParseError::msg("vignette.kind is a string"))?;
            let strength = vignette_strength(chain);
            let next = match name {
                "power" => Vignette::power(strength, 2.0, 1.0, 1.0),
                "smoothstep" => Vignette::smoothstep(strength, 1.0, 0.0, 1.0),
                other => {
                    return Err(ParseError::msg(format!(
                        "unknown vignette.kind {other}; allowed: {}",
                        VIGNETTE_KINDS.join(", ")
                    )));
                }
            };
            replace_vignette(chain, next);
        }
        if let Some(v) = table.get("strength") {
            let n = number(v, "vignette.strength")?;
            vignette_mut(chain).strength = n;
        }
        if let Some(v) = table.get("power") {
            let n = number(v, "vignette.power")?;
            vignette_mut(chain).power = n;
        }
        if let Some(v) = table.get("aspect") {
            let n = number(v, "vignette.aspect")?;
            vignette_mut(chain).aspect = n;
        }
        if let Some(v) = table.get("scale") {
            let n = number(v, "vignette.scale")?;
            vignette_mut(chain).scale = n;
        }
        if let Some(v) = table.get("inner") {
            let n = number(v, "vignette.inner")?;
            vignette_mut(chain).inner = n;
        }
        if let Some(v) = table.get("outer") {
            let n = number(v, "vignette.outer")?;
            vignette_mut(chain).outer = n;
        }
        return Ok(());
    }
    let n = number(value, "vignette")?;
    if chain
        .passes
        .iter()
        .any(|pass| matches!(pass, Pass::Vignette(_)))
    {
        vignette_mut(chain).strength = n;
    } else {
        chain
            .passes
            .push(Pass::Vignette(Vignette::power(n, 2.0, 1.0, 1.0)));
    }
    Ok(())
}

fn ensure_vignette(chain: &mut Chain) {
    if !chain
        .passes
        .iter()
        .any(|pass| matches!(pass, Pass::Vignette(_)))
    {
        chain
            .passes
            .push(Pass::Vignette(Vignette::power(0.0, 2.0, 1.0, 1.0)));
    }
}

fn vignette_strength(chain: &Chain) -> f32 {
    chain
        .passes
        .iter()
        .find_map(|pass| match pass {
            Pass::Vignette(v) => Some(v.strength),
            _ => None,
        })
        .unwrap_or(0.0)
}

fn replace_vignette(chain: &mut Chain, vignette: Vignette) {
    for pass in &mut chain.passes {
        if let Pass::Vignette(slot) = pass {
            *slot = vignette;
            return;
        }
    }
}

fn vignette_mut(chain: &mut Chain) -> &mut Vignette {
    ensure_vignette(chain);
    let Some(Pass::Vignette(slot)) = chain
        .passes
        .iter_mut()
        .find(|pass| matches!(pass, Pass::Vignette(_)))
    else {
        unreachable!("vignette was inserted");
    };
    slot
}

fn set_grain(chain: &mut Chain, value: &Value) -> Result<(), ParseError> {
    if let Some(table) = value.as_table() {
        if let Some(bad) = table.keys().find(|key| !GRAIN_KEYS.contains(&key.as_str())) {
            return Err(unknown("grain", bad, GRAIN_KEYS));
        }
        ensure_grain(chain);
        if let Some(v) = table.get("strength") {
            let n = number(v, "grain.strength")?;
            grain_mut(chain).strength = n;
        }
        if let Some(v) = table.get("response") {
            let n = number(v, "grain.response")?;
            grain_mut(chain).response = n;
        }
        if let Some(v) = table.get("clamp") {
            let n = boolean(v, "grain.clamp")?;
            grain_mut(chain).clamp = n;
        }
        return Ok(());
    }
    let n = number(value, "grain")?;
    if chain
        .passes
        .iter()
        .any(|pass| matches!(pass, Pass::Grain(_)))
    {
        grain_mut(chain).strength = n;
    } else {
        chain.passes.push(Pass::Grain(Grain {
            strength: n,
            response: 0.0,
            clamp: false,
        }));
    }
    Ok(())
}

fn ensure_grain(chain: &mut Chain) {
    if !chain
        .passes
        .iter()
        .any(|pass| matches!(pass, Pass::Grain(_)))
    {
        chain.passes.push(Pass::Grain(Grain {
            strength: 0.0,
            response: 0.0,
            clamp: false,
        }));
    }
}

fn grain_mut(chain: &mut Chain) -> &mut Grain {
    ensure_grain(chain);
    let Some(Pass::Grain(slot)) = chain
        .passes
        .iter_mut()
        .find(|pass| matches!(pass, Pass::Grain(_)))
    else {
        unreachable!("grain was inserted");
    };
    slot
}

fn set_dither(chain: &mut Chain, value: &Value) -> Result<(), ParseError> {
    let amplitude = if let Some(on) = value.as_bool() {
        if on { 1.0 } else { 0.0 }
    } else {
        number(value, "dither")?
    };
    for pass in &mut chain.passes {
        if let Pass::Dither(slot) = pass {
            slot.amplitude = amplitude;
            return Ok(());
        }
    }
    chain.passes.push(Pass::Dither(Dither { amplitude }));
    Ok(())
}

fn set_dither_kind(chain: &mut Chain, value: &Value) -> Result<(), ParseError> {
    let name = value
        .as_str()
        .ok_or_else(|| ParseError::msg("dither_kind is a string"))?;
    let kind = match name {
        "ordered" => NoiseKind::Ordered,
        "blue" => NoiseKind::Blue,
        other => {
            return Err(ParseError::msg(format!(
                "unknown dither_kind {other}; allowed: {}",
                DITHER_KINDS.join(", ")
            )));
        }
    };
    for pass in &mut chain.passes {
        if let Pass::OneBit(slot) = pass {
            slot.kind = kind;
            return Ok(());
        }
    }
    chain.passes.push(Pass::OneBit(fx::OneBit { kind }));
    Ok(())
}

fn set_lut(chain: &mut Chain, value: &Value) -> Result<(), ParseError> {
    let Some(table) = value.as_table() else {
        return Err(ParseError::msg("lut is a table"));
    };
    if let Some(bad) = table.keys().find(|key| !LUT_KEYS.contains(&key.as_str())) {
        return Err(unknown("lut", bad, LUT_KEYS));
    }
    let size = table
        .get("size")
        .ok_or_else(|| ParseError::msg("lut.size is an integer"))
        .and_then(|v| integer(v, "lut.size"))?;
    if !(2..=32).contains(&size) {
        return Err(ParseError::msg("lut.size is from 2 to 32"));
    }
    let values = table
        .get("values")
        .ok_or_else(|| ParseError::msg("lut.values is a list"))?;
    let Some(items) = values.as_array() else {
        return Err(ParseError::msg("lut.values is a list"));
    };
    let need = (size as usize) * (size as usize) * (size as usize) * 3;
    if items.len() != need {
        return Err(ParseError::msg(format!("lut.values has {need} numbers")));
    }
    let mut rgb = Vec::with_capacity(need / 3);
    for chunk in items.chunks(3) {
        rgb.push([
            number(&chunk[0], "lut.values")?,
            number(&chunk[1], "lut.values")?,
            number(&chunk[2], "lut.values")?,
        ]);
    }
    let lut = crate::grade::Lut { size, values: rgb };
    for pass in &mut chain.passes {
        if let Pass::Lut(slot) = pass {
            *slot = lut;
            return Ok(());
        }
    }
    let index = chain
        .passes
        .iter()
        .position(|pass| matches!(pass, Pass::Tone(_)))
        .unwrap_or(chain.passes.len());
    chain.passes.insert(index, Pass::Lut(lut));
    Ok(())
}

fn set_weights(chain: &mut Chain, value: &Value) -> Result<(), ParseError> {
    let items = floats(value, "weights", 3)?;
    let weights = [items[0], items[1], items[2]];
    for pass in &mut chain.passes {
        match pass {
            Pass::Bw(slot) => {
                slot.weights = weights;
                return Ok(());
            }
            Pass::Saturation(slot) => {
                slot.weights = weights;
                return Ok(());
            }
            _ => {}
        }
    }
    chain.passes.push(Pass::Bw(fx::Bw { weights }));
    Ok(())
}

fn default_cel() -> fx::Cel {
    fx::Cel {
        bands: 3,
        shadow: 0.45,
        threshold: 0.45,
        softness: 0.08,
        spec: 0.0,
        spec_roughness: 0.25,
        spec_threshold: 0.72,
        light: [0.3, 0.75, 0.6],
    }
}

fn set_cel(chain: &mut Chain, edit: impl FnOnce(&mut fx::Cel)) {
    if !chain.passes.iter().any(|pass| matches!(pass, Pass::Cel(_))) {
        chain.passes.insert(0, Pass::Cel(default_cel()));
    }
    if let Some(Pass::Cel(slot)) = chain
        .passes
        .iter_mut()
        .find(|pass| matches!(pass, Pass::Cel(_)))
    {
        edit(slot);
    }
}

fn set_cavity(chain: &mut Chain, edit: impl FnOnce(&mut fx::Cavity)) {
    if !chain
        .passes
        .iter()
        .any(|pass| matches!(pass, Pass::Cavity(_)))
    {
        chain.passes.push(Pass::Cavity(fx::Cavity {
            strength: 0.35,
            distance: 2.0,
        }));
    }
    if let Some(Pass::Cavity(slot)) = chain
        .passes
        .iter_mut()
        .find(|pass| matches!(pass, Pass::Cavity(_)))
    {
        edit(slot);
    }
}

fn set_rim(chain: &mut Chain, edit: impl FnOnce(&mut fx::Rim)) {
    if !chain.passes.iter().any(|pass| matches!(pass, Pass::Rim(_))) {
        chain.passes.push(Pass::Rim(fx::Rim {
            strength: 0.22,
            width: 0.35,
            color: [1.0, 0.95, 0.85],
        }));
    }
    if let Some(Pass::Rim(slot)) = chain
        .passes
        .iter_mut()
        .find(|pass| matches!(pass, Pass::Rim(_)))
    {
        edit(slot);
    }
}

fn default_outline() -> fx::Outline {
    fx::Outline {
        enabled: true,
        thickness: 1.25,
        color: [0.02, 0.015, 0.02],
        local_color: false,
        alpha: 0.92,
        crease_angle: 35.0,
        depth_gap: 0.04,
    }
}

fn set_outline(chain: &mut Chain, value: &Value) -> Result<(), ParseError> {
    if let Some(on) = value.as_bool() {
        set_outline_field(chain, |outline| outline.enabled = on);
        return Ok(());
    }
    let Some(table) = value.as_table() else {
        return Err(ParseError::msg("outline is a bool or a table"));
    };
    if let Some(bad) = table
        .keys()
        .find(|key| !OUTLINE_KEYS.contains(&key.as_str()))
    {
        return Err(unknown("outline", bad, OUTLINE_KEYS));
    }
    if let Some(v) = table.get("enabled") {
        let on = boolean(v, "outline.enabled")?;
        set_outline_field(chain, |o| o.enabled = on);
    } else {
        set_outline_field(chain, |_| {});
    }
    if let Some(v) = table.get("thickness") {
        let n = number(v, "outline.thickness")?;
        set_outline_field(chain, |o| o.thickness = n);
    }
    if let Some(v) = table.get("alpha") {
        let n = number(v, "outline.alpha")?;
        set_outline_field(chain, |o| o.alpha = n);
    }
    if let Some(v) = table.get("crease_angle") {
        let n = number(v, "outline.crease_angle")?;
        set_outline_field(chain, |o| o.crease_angle = n);
    }
    if let Some(v) = table.get("color") {
        let c = color(v, "outline.color")?;
        set_outline_field(chain, |o| o.color = c);
    }
    if let Some(v) = table.get("local_color") {
        let on = boolean(v, "outline.local_color")?;
        set_outline_field(chain, |o| o.local_color = on);
    }
    Ok(())
}

fn set_outline_field(chain: &mut Chain, edit: impl FnOnce(&mut fx::Outline)) {
    if !chain
        .passes
        .iter()
        .any(|pass| matches!(pass, Pass::Outline(_)))
    {
        chain.passes.push(Pass::Outline(default_outline()));
    }
    if let Some(Pass::Outline(slot)) = chain
        .passes
        .iter_mut()
        .find(|pass| matches!(pass, Pass::Outline(_)))
    {
        edit(slot);
    }
}

fn set_noir(chain: &mut Chain, edit: impl FnOnce(&mut fx::Noir)) {
    if !chain
        .passes
        .iter()
        .any(|pass| matches!(pass, Pass::Noir(_)))
    {
        chain.passes.insert(
            0,
            Pass::Noir(fx::Noir {
                contrast: 1.9,
                crush: 0.18,
                keep: None,
                keep_range: 0.45,
            }),
        );
    }
    if let Some(Pass::Noir(slot)) = chain
        .passes
        .iter_mut()
        .find(|pass| matches!(pass, Pass::Noir(_)))
    {
        edit(slot);
    }
}

fn set_sepia(chain: &mut Chain, value: f32) {
    for pass in &mut chain.passes {
        if let Pass::Sepia(slot) = pass {
            *slot = value;
            return;
        }
    }
    chain.passes.push(Pass::Sepia(value));
}

fn set_levels(chain: &mut Chain, value: f32) {
    let mut hit = false;
    for pass in &mut chain.passes {
        match pass {
            Pass::Posterize(slot) => {
                *slot = value;
                hit = true;
            }
            Pass::Comic(slot) => {
                slot.levels = value.max(2.0).round() as u32;
                hit = true;
            }
            _ => {}
        }
    }
    if !hit {
        chain.passes.push(Pass::Posterize(value));
    }
}

fn set_modern_comic(chain: &mut Chain, edit: impl FnOnce(&mut ModernComic)) {
    if let Some(Pass::ModernComic(pass)) = chain
        .passes
        .iter_mut()
        .find(|pass| matches!(pass, Pass::ModernComic(_)))
    {
        edit(pass);
    } else {
        let mut pass = ModernComic::default();
        edit(&mut pass);
        chain.passes.push(Pass::ModernComic(pass));
    }
}

fn set_f32(
    chain: &mut Chain,
    value: f32,
    found: impl Fn(&Pass) -> bool,
    make: impl Fn(f32) -> Pass,
) {
    for pass in &mut chain.passes {
        if found(pass) {
            match pass {
                Pass::Flicker(slot)
                | Pass::Weave(slot)
                | Pass::Dust(slot)
                | Pass::Aberration(slot)
                | Pass::Distortion(slot)
                | Pass::Curvature(slot)
                | Pass::Aperture(slot) => *slot = value,
                _ => {}
            }
            return;
        }
    }
    chain.passes.push(make(value));
}

fn set_scratches(chain: &mut Chain, value: &Value) -> Result<(), ParseError> {
    if let Some(table) = value.as_table() {
        if let Some(bad) = table
            .keys()
            .find(|key| !SCRATCH_KEYS.contains(&key.as_str()))
        {
            return Err(unknown("scratches", bad, SCRATCH_KEYS));
        }
        ensure_scratches(chain);
        if let Some(v) = table.get("count") {
            let n = integer(v, "scratches.count")?;
            scratches_mut(chain).count = n;
        }
        if let Some(v) = table.get("strength") {
            let n = number(v, "scratches.strength")?;
            scratches_mut(chain).strength = n;
        }
        return Ok(());
    }
    let n = number(value, "scratches")?;
    if chain
        .passes
        .iter()
        .any(|pass| matches!(pass, Pass::Scratches(_)))
    {
        scratches_mut(chain).strength = n;
    } else {
        chain.passes.push(Pass::Scratches(fx::Scratches {
            count: 3,
            strength: n,
        }));
    }
    Ok(())
}

fn ensure_scratches(chain: &mut Chain) {
    if !chain
        .passes
        .iter()
        .any(|pass| matches!(pass, Pass::Scratches(_)))
    {
        chain.passes.push(Pass::Scratches(fx::Scratches {
            count: 3,
            strength: 0.35,
        }));
    }
}

fn scratches_mut(chain: &mut Chain) -> &mut fx::Scratches {
    ensure_scratches(chain);
    let Some(Pass::Scratches(slot)) = chain
        .passes
        .iter_mut()
        .find(|pass| matches!(pass, Pass::Scratches(_)))
    else {
        unreachable!("scratches were inserted");
    };
    slot
}

fn set_halation(chain: &mut Chain, value: f32) {
    for pass in &mut chain.passes {
        if let Pass::Halation(slot) = pass {
            slot.strength = value;
            return;
        }
    }
    chain.passes.push(Pass::Halation(fx::Halation {
        strength: value,
        threshold: 0.65,
        radius: 6,
    }));
}

fn set_scanlines(chain: &mut Chain, value: f32) {
    for pass in &mut chain.passes {
        if let Pass::Scanlines(slot) = pass {
            slot.strength = value;
            return;
        }
    }
    chain.passes.push(Pass::Scanlines(fx::Scanlines {
        strength: value,
        period: 2,
    }));
}

fn set_halftone(chain: &mut Chain, edit: impl FnOnce(&mut fx::Halftone)) {
    if !chain
        .passes
        .iter()
        .any(|pass| matches!(pass, Pass::Halftone(_)))
    {
        chain.passes.push(Pass::Halftone(fx::Halftone {
            cell: 6.0,
            angle: 15.0,
            ink: [0.02, 0.02, 0.03],
        }));
    }
    if let Some(Pass::Halftone(slot)) = chain
        .passes
        .iter_mut()
        .find(|pass| matches!(pass, Pass::Halftone(_)))
    {
        edit(slot);
    }
}

fn set_ink(chain: &mut Chain, value: &Value) -> Result<(), ParseError> {
    if value.as_array().is_some() || value.as_str().is_some() {
        let c = color(value, "ink")?;
        set_halftone(chain, |h| h.ink = c);
        return Ok(());
    }
    let n = number(value, "ink")?;
    for pass in &mut chain.passes {
        if let Pass::Comic(slot) = pass {
            slot.ink = n;
            return Ok(());
        }
    }
    chain
        .passes
        .push(Pass::Comic(fx::Comic { levels: 5, ink: n }));
    Ok(())
}

fn set_water(chain: &mut Chain, edit: impl FnOnce(&mut fx::Watercolor)) {
    if !chain
        .passes
        .iter()
        .any(|pass| matches!(pass, Pass::Watercolor(_)))
    {
        chain.passes.push(Pass::Watercolor(fx::Watercolor {
            darkening: 0.65,
            grain: 0.22,
            scale: 6.0,
        }));
    }
    if let Some(Pass::Watercolor(slot)) = chain
        .passes
        .iter_mut()
        .find(|pass| matches!(pass, Pass::Watercolor(_)))
    {
        edit(slot);
    }
}

fn set_paper_grain(chain: &mut Chain, value: f32) {
    let mut hit = false;
    for pass in &mut chain.passes {
        match pass {
            Pass::PaperGrain(slot) => {
                slot.strength = value;
                hit = true;
            }
            Pass::Watercolor(slot) => {
                slot.grain = value;
                hit = true;
            }
            _ => {}
        }
    }
    if !hit {
        chain.passes.push(Pass::PaperGrain(fx::PaperGrain {
            strength: value,
            scale: 5.0,
        }));
    }
}

fn set_scale(chain: &mut Chain, value: f32) {
    let mut hit = false;
    for pass in &mut chain.passes {
        match pass {
            Pass::PaperGrain(slot) => {
                slot.scale = value;
                hit = true;
            }
            Pass::Watercolor(slot) => {
                slot.scale = value;
                hit = true;
            }
            _ => {}
        }
    }
    if !hit {
        chain.passes.push(Pass::PaperGrain(fx::PaperGrain {
            strength: 0.28,
            scale: value,
        }));
    }
}

fn set_pixel_size(chain: &mut Chain, value: u32) {
    for pass in &mut chain.passes {
        if let Pass::Pixel(slot) = pass {
            slot.size = value.max(1);
            return;
        }
    }
    chain.passes.push(Pass::Pixel(fx::Pixelate {
        size: value.max(1),
        levels: 0,
        palette: true,
    }));
}

fn set_palette(chain: &mut Chain, value: bool) {
    for pass in &mut chain.passes {
        if let Pass::Pixel(slot) = pass {
            slot.palette = value;
            return;
        }
    }
    chain.passes.push(Pass::Pixel(fx::Pixelate {
        size: 4,
        levels: 0,
        palette: value,
    }));
}

fn set_duotone(chain: &mut Chain, edit: impl FnOnce(&mut fx::Duotone)) {
    if !chain
        .passes
        .iter()
        .any(|pass| matches!(pass, Pass::Duotone(_)))
    {
        chain.passes.push(Pass::Duotone(fx::Duotone {
            shadow: [0.05, 0.08, 0.18],
            highlight: [0.96, 0.82, 0.55],
        }));
    }
    if let Some(Pass::Duotone(slot)) = chain
        .passes
        .iter_mut()
        .find(|pass| matches!(pass, Pass::Duotone(_)))
    {
        edit(slot);
    }
}

fn set_radius(chain: &mut Chain, value: f32) {
    let mut hit = false;
    for pass in &mut chain.passes {
        match pass {
            Pass::Bloom(slot) => {
                slot.radius = value.max(0.0).round() as u32;
                hit = true;
            }
            Pass::Kuwahara(slot) => {
                *slot = value.max(0.0).round() as u32;
                hit = true;
            }
            Pass::Halation(slot) => {
                slot.radius = value.max(0.0).round() as u32;
                hit = true;
            }
            Pass::Neon(slot) => {
                slot.radius = value.max(0.0).round() as u32;
                hit = true;
            }
            Pass::TiltShift(slot) => {
                slot.radius = value.max(0.0);
                hit = true;
            }
            _ => {}
        }
    }
    if !hit {
        chain
            .passes
            .push(Pass::Kuwahara(value.max(0.0).round() as u32));
    }
}

fn set_tilt(chain: &mut Chain, edit: impl FnOnce(&mut fx::TiltShift)) {
    if !chain
        .passes
        .iter()
        .any(|pass| matches!(pass, Pass::TiltShift(_)))
    {
        chain.passes.push(Pass::TiltShift(fx::TiltShift {
            focus: 0.5,
            range: 0.22,
            radius: 5.0,
        }));
    }
    if let Some(Pass::TiltShift(slot)) = chain
        .passes
        .iter_mut()
        .find(|pass| matches!(pass, Pass::TiltShift(_)))
    {
        edit(slot);
    }
}
