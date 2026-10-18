use super::super::{rgb, table, tagged, within};
use pfx_materials::{Crack, Grime, LAYER_CAP, NoiseKind, NoiseLayer, Track};
use pfx_recipe::Textures;
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub const FLAT: &str = "flat";
pub const DETAILED: &str = "asphalt-detailed";
pub const DEFAULT: &str = FLAT;
pub const LOOKS: [&str; 2] = [FLAT, DETAILED];

const SURFACE: &[&str] = &[
    "base",
    "roughness",
    "wet",
    "grain",
    "patches",
    "cracks",
    "tracks",
    "seed",
    "base_map",
    "normal_map",
    "roughness_map",
    "map_size",
];

const SEED_SPAN: f64 = (1u64 << 19) as f64;
const GAUGE: f64 = 0.8;
const BAND: f64 = 0.3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Asphalt,
    Paint,
    Concrete,
    Kerb,
}

impl Slot {
    pub const ALL: [Slot; 4] = [Slot::Asphalt, Slot::Paint, Slot::Concrete, Slot::Kerb];

    pub fn key(self) -> &'static str {
        match self {
            Slot::Asphalt => "asphalt",
            Slot::Paint => "paint",
            Slot::Concrete => "concrete",
            Slot::Kerb => "kerb",
        }
    }

    fn seed(self) -> u64 {
        match self {
            Slot::Asphalt => 1,
            Slot::Paint => 2,
            Slot::Concrete | Slot::Kerb => 3,
        }
    }

    fn keys(self) -> &'static [&'static str] {
        match self {
            Slot::Paint => &["base", "roughness", "wet"],
            Slot::Asphalt | Slot::Concrete | Slot::Kerb => SURFACE,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Detail {
    amount: f64,
    scale: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Wear {
    amount: f64,
    gauge: f64,
    band: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Layers {
    grain: Detail,
    patches: Detail,
    cracks: Detail,
    tracks: Wear,
}

fn preset(look: &str, slot: Slot) -> Layers {
    let on = |amount: f64| if look == DETAILED { amount } else { 0.0 };
    let detail = |amount: f64, scale: f64| Detail {
        amount: on(amount),
        scale,
    };
    let (grain, patches, cracks, tracks) = match slot {
        Slot::Asphalt => (
            detail(0.5, 0.02),
            detail(0.6, 7.0),
            detail(0.8, 1.6),
            on(0.7),
        ),
        Slot::Concrete => (detail(0.25, 0.012), detail(0.5, 4.0), detail(0.6, 1.2), 0.0),
        Slot::Kerb => (detail(0.3, 0.01), detail(0.7, 1.5), detail(0.0, 0.8), 0.0),
        Slot::Paint => (detail(0.0, 1.0), detail(0.0, 1.0), detail(0.0, 1.0), 0.0),
    };
    Layers {
        grain,
        patches,
        cracks,
        tracks: Wear {
            amount: tracks,
            gauge: GAUGE,
            band: BAND,
        },
    }
}

fn base(slot: Slot) -> Value {
    match slot {
        Slot::Asphalt => json!({ "base": [0.045, 0.045, 0.05], "roughness": 0.6 }),
        Slot::Paint => json!({
            "base": [0.82, 0.82, 0.8],
            "roughness": 0.55,
            "ageing": {
                "fade": 0.1, "yellow": 0.05, "ink": 0.0, "scratch": 0.3,
                "edge": 0.0, "dust": 0.25, "patina": 0.0, "seed": 0
            }
        }),
        Slot::Concrete | Slot::Kerb => json!({ "base": [0.5, 0.49, 0.46], "roughness": 0.85 }),
    }
}

pub struct Surface {
    pub material: Value,
    pub textures: Option<Textures>,
    pub map_size: f64,
}

struct Mix(u64);

impl Mix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn layer_seed(&mut self) -> u32 {
        ((self.next() >> 11) as f64 / (1u64 << 53) as f64 * SEED_SPAN) as u32
    }
}

fn detail(given: Option<&Value>, key: &str, default: Detail) -> Result<Detail, String> {
    let Some(value) = given else {
        return Ok(default);
    };
    let found = table(value, key, &["amount", "scale"])?;
    let mut out = default;
    if let Some(amount) = found.get("amount") {
        out.amount = within(amount, &format!("{key}.amount"), 0.0, 1.0)?;
    }
    if let Some(scale) = found.get("scale") {
        out.scale = positive(scale, &format!("{key}.scale"))?;
    }
    Ok(out)
}

fn wear(given: Option<&Value>, key: &str, default: Wear) -> Result<Wear, String> {
    let Some(value) = given else {
        return Ok(default);
    };
    let found = table(value, key, &["amount", "gauge", "band"])?;
    let mut out = default;
    if let Some(amount) = found.get("amount") {
        out.amount = within(amount, &format!("{key}.amount"), 0.0, 1.0)?;
    }
    if let Some(gauge) = found.get("gauge") {
        out.gauge = within(gauge, &format!("{key}.gauge"), 0.0, 10.0)?;
    }
    if let Some(band) = found.get("band") {
        out.band = positive(band, &format!("{key}.band"))?;
    }
    Ok(out)
}

fn positive(value: &Value, key: &str) -> Result<f64, String> {
    value
        .as_f64()
        .filter(|v| v.is_finite() && *v > 0.0)
        .ok_or_else(|| format!("params.{key}: expected a number above 0"))
}

fn path(value: &Value, key: &str) -> Result<String, String> {
    match value {
        Value::String(text) if !text.is_empty() => Ok(text.clone()),
        _ => Err(format!(
            "params.{key}: expected the path of a PNG, relative to the recipe"
        )),
    }
}

fn wet(value: &Value, key: &str, seed: Option<u64>) -> Result<Value, String> {
    let given = table(value, key, &["amount", "puddles", "puddle_scale", "seed"])?;
    let mut out = serde_json::Map::new();
    for (name, item) in given {
        let at = format!("{key}.{name}");
        let v = match name.as_str() {
            "amount" | "puddles" => json!(within(item, &at, 0.0, 1.0)?),
            "puddle_scale" => json!(positive(item, &at)?),
            _ => json!(
                item.as_u64()
                    .ok_or_else(|| format!("params.{at}: expected a whole number of 0 or more"))?
            ),
        };
        out.insert(name.clone(), v);
    }
    if let Some(seed) = seed {
        out.entry("seed").or_insert(json!(seed));
    }
    Ok(Value::Object(out))
}

fn short(value: f32) -> f64 {
    value.to_string().parse().unwrap_or(f64::from(value))
}

fn layer(layer: NoiseLayer) -> Value {
    let kind = serde_json::to_value(layer.kind).unwrap_or(Value::Null);
    json!({
        "kind": kind,
        "frequency": short(layer.frequency),
        "amplitude": short(layer.amplitude),
        "seed": layer.seed,
        "params": layer.params.map(short),
    })
}

fn noise(layers: Layers, slot: Slot, seed: u64, lane_width: f64) -> Vec<Value> {
    let mut mix = Mix(seed);
    let seeds = [
        mix.layer_seed(),
        mix.layer_seed(),
        mix.layer_seed(),
        mix.layer_seed(),
    ];
    let mut out = Vec::new();
    let Layers {
        grain,
        patches,
        cracks,
        tracks,
    } = layers;
    if grain.amount > 0.0 {
        out.push(layer(NoiseLayer::new(
            NoiseKind::Value,
            (1.0 / grain.scale) as f32,
            grain.amount as f32,
            seeds[0],
        )));
    }
    if patches.amount > 0.0 {
        let f = (1.0 / patches.scale) as f32;
        out.push(layer(
            Grime {
                frequencies: [f, f * 5.0],
                dirty: [0.62, 0.62, 0.65],
                clean: [1.4, 1.37, 1.3],
                roughness: [0.92, 1.08],
            }
            .layer(patches.amount as f32, seeds[1]),
        ));
    }
    if cracks.amount > 0.0 {
        let cell = cracks.scale as f32;
        let width = if slot == Slot::Asphalt { 0.006 } else { 0.004 };
        out.push(layer(
            Crack {
                cell,
                width,
                warp: cell * 0.11,
                warp_frequency: 0.9 / cell,
                density: 0.45,
                spread: 1.0 / (6.0 * cell),
                darkness: 0.8,
                roughness: 0.95,
                randomness: 1.0,
            }
            .layer(cracks.amount as f32, seeds[2]),
        ));
    }
    if tracks.amount > 0.0 && slot == Slot::Asphalt {
        let lw = lane_width as f32;
        out.push(layer(
            Track {
                period: lw,
                offset: lw * 0.5,
                gauge: tracks.gauge as f32,
                band: tracks.band as f32,
                tint: 1.3,
                polish: 0.8,
                ..Track::SAMPLE
            }
            .layer(tracks.amount as f32, seeds[3]),
        ));
    }
    out
}

pub fn look(params: &BTreeMap<String, Value>) -> Result<&'static str, String> {
    match params.get("look") {
        None => Ok(DEFAULT),
        Some(Value::String(name)) => LOOKS
            .into_iter()
            .find(|known| known == name)
            .ok_or_else(look_error),
        Some(_) => Err(look_error()),
    }
}

fn look_error() -> String {
    format!(
        "params.look: expected {}",
        LOOKS
            .iter()
            .map(|name| format!("\"{name}\""))
            .collect::<Vec<_>>()
            .join(" or ")
    )
}

pub fn surface(
    slot: Slot,
    look: &str,
    given: &[(&str, &serde_json::Map<String, Value>)],
    lane_width: f64,
) -> Result<Surface, String> {
    let mut material = base(slot);
    let at = |name: &str| {
        given
            .iter()
            .rev()
            .find_map(|(key, map)| map.get(name).map(|v| (*key, v)))
    };
    let keys = material.as_object_mut().expect("a material is a table");
    if let Some((key, value)) = at("base") {
        keys.insert(
            "base".to_string(),
            json!(rgb(value, &format!("{key}.base"), Some(1.0))?),
        );
    }
    if let Some((key, value)) = at("roughness") {
        keys.insert(
            "roughness".to_string(),
            json!(within(value, &format!("{key}.roughness"), 0.0, 1.0)?),
        );
    }
    let seed = match at("seed") {
        None => slot.seed(),
        Some((key, value)) => value
            .as_u64()
            .ok_or_else(|| format!("params.{key}.seed: expected a whole number of 0 or more"))?,
    };
    let mut textures = Textures::default();
    for (name, slot) in [
        ("base_map", &mut textures.base_map),
        ("normal_map", &mut textures.normal_map),
        ("roughness_map", &mut textures.roughness_map),
    ] {
        if let Some((key, value)) = at(name) {
            *slot = Some(path(value, &format!("{key}.{name}"))?);
        }
    }
    let mapped = textures != Textures::default();
    let map_size = match at("map_size") {
        None => 1.0,
        Some((key, value)) => positive(value, &format!("{key}.map_size"))?,
    };
    if let Some((key, value)) = at("wet") {
        if mapped {
            return Err(format!(
                "params.{key}.wet: a surface with maps draws them from the GLB, which carries no wet table; expected maps or wet, not both"
            ));
        }
        let seeded = (look == DETAILED).then_some(seed);
        keys.insert(
            "wet".to_string(),
            wet(value, &format!("{key}.wet"), seeded)?,
        );
    }
    if slot != Slot::Paint && !mapped {
        let defaults = preset(look, slot);
        let named = |name: &str| at(name).map(|(key, value)| (format!("{key}.{name}"), value));
        let pick = |name: &str, default: Detail| match named(name) {
            Some((key, value)) => detail(Some(value), &key, default),
            None => Ok(default),
        };
        let layers = Layers {
            grain: pick("grain", defaults.grain)?,
            patches: pick("patches", defaults.patches)?,
            cracks: pick("cracks", defaults.cracks)?,
            tracks: match named("tracks") {
                Some((key, value)) => wear(Some(value), &key, defaults.tracks)?,
                None => defaults.tracks,
            },
        };
        let mut list = noise(layers, slot, seed, lane_width);
        if !list.is_empty() {
            list.resize_with(LAYER_CAP, || layer(NoiseLayer::default()));
            keys.insert("layers".to_string(), Value::Array(list));
        }
    }
    Ok(Surface {
        material,
        textures: mapped.then_some(textures),
        map_size,
    })
}

pub fn check(slot: Slot, value: &Value) -> Result<&serde_json::Map<String, Value>, String> {
    let found = table(value, slot.key(), slot.keys())?;
    if slot != Slot::Asphalt && found.contains_key("tracks") {
        return Err(format!(
            "params.{}.tracks: tyre tracks wear the asphalt only",
            slot.key()
        ));
    }
    Ok(found)
}

pub fn named(
    default: &str,
    value: &Value,
    default_value: &Value,
    textures: Option<&Textures>,
) -> String {
    match textures {
        None if value == default_value => default.to_string(),
        None => tagged(default, value),
        Some(textures) => {
            let mut keyed = value.clone();
            if let Some(keys) = keyed.as_object_mut() {
                keys.insert(
                    "maps".to_string(),
                    json!([
                        textures.base_map,
                        textures.normal_map,
                        textures.roughness_map
                    ]),
                );
            }
            tagged(default, &keyed)
        }
    }
}
