use std::collections::BTreeMap;

use pfx_geom::icon::{Icon, Part, PartShape};
use pfx_geom::mark::{Each, Form};
use pfx_materials::Material;
use pfx_post::{Lines, View};
use pfx_trace::rig::Rig;
use serde_json::{Map, Value};

pub const PRESET_KEYS: &[&str] = &[
    "colors",
    "material",
    "materials",
    "form",
    "rig",
    "ground",
    "style",
    "outline",
    "camera",
    "view",
    "look",
    "exposure",
    "denoise",
];

pub const MATERIAL_KEYS: &[&str] = &[
    "base",
    "metallic",
    "roughness",
    "coat",
    "coat_roughness",
    "transmission",
    "ior",
    "subsurface",
    "subsurface_scale",
    "grain",
    "grain_scale",
    "film",
    "film_ior",
    "wear",
    "wear_scale",
];

const SECTIONS: &[(&str, &[&str])] = &[
    ("material", MATERIAL_KEYS),
    (
        "form",
        &[
            "depth",
            "bevel",
            "segments",
            "voxel",
            "smooth",
            "layer",
            "inset",
            "element_depth",
            "element_z",
            "element_material",
            "merge",
        ],
    ),
    (
        "rig",
        &[
            "world",
            "strength",
            "turn",
            "key",
            "key_from",
            "key_color",
            "fill",
            "rim",
            "rim_color",
        ],
    ),
    (
        "ground",
        &[
            "color",
            "radius",
            "mark",
            "roughness",
            "coat",
            "coat_roughness",
            "thickness",
            "lift",
        ],
    ),
    (
        "style",
        &[
            "shading",
            "cel_bands",
            "cel_shadow",
            "cel_threshold",
            "cel_softness",
            "cel_spec",
            "cel_spec_roughness",
            "cel_spec_threshold",
            "cavity",
            "cavity_distance",
            "round_edges",
            "rim",
            "rim_width",
            "rim_color",
        ],
    ),
    (
        "outline",
        &[
            "enabled",
            "thickness",
            "color",
            "alpha",
            "crease_angle",
            "silhouette",
            "border",
            "crease",
            "contour",
            "external_contour",
        ],
    ),
    (
        "camera",
        &["lens", "tilt", "turn", "roll", "ortho", "margin"],
    ),
];

const ICON_KEYS: &[&str] = &["name", "parts", "fuse", "voxel"];

const PART_KEYS: &[(&str, &[&str])] = &[
    ("ball", &["kind", "at", "z", "r", "material"]),
    (
        "drop",
        &["kind", "at", "z", "r", "squash", "stretch", "material"],
    ),
    ("capsule", &["kind", "from", "to", "z", "r", "material"]),
    ("tube", &["kind", "points", "z", "r", "material"]),
    ("torus", &["kind", "at", "z", "major", "minor", "material"]),
    (
        "box",
        &["kind", "at", "z", "half", "round", "angle", "material"],
    ),
];

pub const VIEWS: &[&str] = &["Standard", "AgX"];

fn strict(where_: &str, map: &Map<String, Value>, allowed: &[&str]) -> Result<(), String> {
    let mut unknown: Vec<&str> = map
        .keys()
        .map(String::as_str)
        .filter(|k| !allowed.contains(k))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    unknown.sort_unstable();
    let mut known = allowed.to_vec();
    known.sort_unstable();
    Err(format!(
        "{where_}: unknown key(s) {}; allowed: {}",
        unknown.join(", "),
        known.join(", ")
    ))
}

pub fn check_preset(preset: &Value) -> Result<(), String> {
    let map = preset.as_object().ok_or("preset is not a table")?;
    strict("preset", map, PRESET_KEYS)?;
    for (section, allowed) in SECTIONS {
        if let Some(Value::Object(table)) = map.get(*section) {
            strict(&format!("preset [{section}]"), table, allowed)?;
        }
    }
    if let Some(Value::Object(looks)) = map.get("materials") {
        for (name, look) in looks {
            let look = look
                .as_object()
                .ok_or_else(|| format!("preset [materials.{name}] is a table"))?;
            strict(&format!("preset [materials.{name}]"), look, MATERIAL_KEYS)?;
        }
    }
    Ok(())
}

pub fn check_icon(icon: &Value) -> Result<(), String> {
    let map = icon.as_object().ok_or("an icon is a table")?;
    let name = map.get("name").and_then(Value::as_str).unwrap_or("?");
    strict(&format!("icon {name}"), map, ICON_KEYS)?;
    for part in map
        .get("parts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let part = part
            .as_object()
            .ok_or_else(|| format!("icon {name}: a part is a table"))?;
        let kind = part.get("kind").and_then(Value::as_str).unwrap_or("");
        let allowed = PART_KEYS
            .iter()
            .find(|(k, _)| *k == kind)
            .map(|(_, keys)| *keys)
            .ok_or_else(|| {
                let kinds: Vec<&str> = PART_KEYS.iter().map(|(k, _)| *k).collect();
                format!(
                    "icon {name}: unknown part kind {kind}; allowed: {}",
                    kinds.join(", ")
                )
            })?;
        strict(&format!("icon {name} {kind}"), part, allowed)?;
    }
    Ok(())
}

pub fn hex_srgb(text: &str) -> Result<[f32; 3], String> {
    let digits = text.trim().trim_start_matches('#');
    let full: String = if digits.len() == 3 {
        digits.chars().flat_map(|c| [c, c]).collect()
    } else {
        digits.to_string()
    };
    if full.len() != 6 || !full.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("{text} is not a #rrggbb colour"));
    }
    let byte = |i: usize| u8::from_str_radix(&full[i..i + 2], 16).unwrap_or(0) as f32 / 255.0;
    Ok([byte(0), byte(2), byte(4)])
}

pub fn hex_linear(text: &str) -> Result<[f32; 3], String> {
    Ok(hex_srgb(text)?.map(pfx_post::view::srgb_decode))
}

pub fn bytes_linear(rgb: [u8; 3]) -> [f32; 3] {
    rgb.map(|v| pfx_post::view::srgb_decode(f32::from(v) / 255.0))
}

fn number(map: &Map<String, Value>, key: &str, default: f64) -> Result<f32, String> {
    match map.get(key) {
        None => Ok(default as f32),
        Some(v) => v
            .as_f64()
            .filter(|v| v.is_finite())
            .map(|v| v as f32)
            .ok_or_else(|| format!("{key} is a number")),
    }
}

fn flag(map: &Map<String, Value>, key: &str, default: bool) -> Result<bool, String> {
    match map.get(key) {
        None => Ok(default),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(format!("{key} is true or false")),
    }
}

fn text<'a>(map: &'a Map<String, Value>, key: &str, default: &'a str) -> Result<&'a str, String> {
    match map.get(key) {
        None => Ok(default),
        Some(Value::String(s)) => Ok(s),
        Some(_) => Err(format!("{key} is text")),
    }
}

fn section<'a>(preset: &'a Value, name: &str) -> Result<Option<&'a Map<String, Value>>, String> {
    match preset.get(name) {
        None => Ok(None),
        Some(Value::Object(map)) => Ok(Some(map)),
        Some(_) => Err(format!("preset [{name}] is a table")),
    }
}

fn table(preset: &Value, name: &str) -> Result<Map<String, Value>, String> {
    Ok(section(preset, name)?.cloned().unwrap_or_default())
}

pub fn look_of(preset: &Value, name: Option<&str>) -> Result<Map<String, Value>, String> {
    let own = match name {
        Some(name) => section(preset, "materials")?
            .and_then(|looks| looks.get(name))
            .and_then(Value::as_object),
        None => None,
    };
    match own {
        Some(look) => Ok(look.clone()),
        None => table(preset, "material"),
    }
}

pub fn material(
    look: &Map<String, Value>,
    tint: Option<[f32; 3]>,
    ignored: &mut Vec<String>,
) -> Result<Material, String> {
    let base = match tint {
        Some(rgb) => rgb,
        None => hex_linear(text(look, "base", "#8c9096")?)?,
    };
    let wear = number(look, "wear", 0.0)?.max(0.0);
    let mut out = Material {
        base,
        metalness: number(look, "metallic", 1.0)?.clamp(0.0, 1.0),
        roughness: (number(look, "roughness", 0.25)? + 0.15 * wear).clamp(0.0, 1.0),
        clearcoat: number(look, "coat", 0.0)?.max(0.0),
        clearcoat_roughness: number(look, "coat_roughness", 0.05)?.clamp(0.0, 1.0),
        ..Material::default()
    };
    let transmission = number(look, "transmission", 0.0)?;
    if transmission > 0.0 {
        out.transmission = transmission.min(1.0);
        out.ior = number(look, "ior", 1.45)?;
    }
    let subsurface = number(look, "subsurface", 0.0)?;
    if subsurface > 0.0 {
        out.subsurface = subsurface.min(1.0);
        if look.contains_key("subsurface_scale") {
            note(ignored, "material.subsurface_scale");
        }
    }
    let film = number(look, "film", 0.0)?;
    if film > 0.0 {
        out.thin_film = film;
        out.thin_film_ior = number(look, "film_ior", 1.4)?;
        out.thin_film_amount = 1.0;
    }
    if number(look, "grain", 0.0)? > 0.0 {
        note(ignored, "material.grain");
    }
    if wear > 0.0 {
        note(ignored, "material.wear (colour)");
    }
    Ok(out)
}

pub fn note(ignored: &mut Vec<String>, what: &str) {
    if !ignored.iter().any(|known| known == what) {
        ignored.push(what.to_string());
    }
}

pub fn style_notes(preset: &Value, ignored: &mut Vec<String>) -> Result<(), String> {
    let Some(style) = section(preset, "style")? else {
        return Ok(());
    };
    if text(style, "shading", "pbr")? != "pbr" {
        note(ignored, "style.shading");
    }
    for key in ["cavity", "round_edges", "rim"] {
        if number(style, key, 0.0)? > 0.0 {
            note(ignored, &format!("style.{key}"));
        }
    }
    Ok(())
}

pub struct Ground {
    pub color: Option<[f32; 3]>,
    pub radius: f32,
    pub mark: f32,
    pub thickness: f32,
    pub lift: f32,
    pub roughness: f32,
    pub coat: f32,
    pub coat_roughness: f32,
}

pub fn ground(preset: &Value) -> Result<Ground, String> {
    let g = &table(preset, "ground")?;
    Ok(Ground {
        color: g
            .get("color")
            .and_then(Value::as_str)
            .map(hex_linear)
            .transpose()?,
        radius: number(g, "radius", 0.22)?,
        mark: number(g, "mark", 0.62)?,
        thickness: number(g, "thickness", 0.14)?,
        lift: number(g, "lift", 0.0)?,
        roughness: number(g, "roughness", 0.45)?,
        coat: number(g, "coat", 0.3)?,
        coat_roughness: number(g, "coat_roughness", 0.15)?,
    })
}

impl Ground {
    pub fn material(&self, base: [f32; 3]) -> Material {
        Material {
            base,
            metalness: 0.0,
            roughness: self.roughness.clamp(0.0, 1.0),
            clearcoat: self.coat.max(0.0),
            clearcoat_roughness: self.coat_roughness.clamp(0.0, 1.0),
            ..Material::default()
        }
    }
}

fn triple(value: &Value, key: &str) -> Result<[f32; 3], String> {
    let list: Vec<f32> = value
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(Value::as_f64)
                .map(|v| v as f32)
                .collect()
        })
        .unwrap_or_default();
    match list[..] {
        [x, y, z] => Ok([x, y, z]),
        _ => Err(format!("{key} is [x, y, z]")),
    }
}

pub fn rig(preset: &Value) -> Result<Rig, String> {
    let r = &table(preset, "rig")?;
    let base = Rig::default();
    let rim_color = match r.get("rim_color") {
        None => Vec::new(),
        Some(Value::String(one)) => vec![hex_linear(one)?],
        Some(Value::Array(list)) => list
            .iter()
            .map(|c| {
                c.as_str()
                    .ok_or("rig.rim_color lists colours".to_string())
                    .and_then(hex_linear)
            })
            .collect::<Result<_, _>>()?,
        Some(_) => return Err("rig.rim_color is a colour or a list of them".into()),
    };
    Ok(Rig {
        key: number(r, "key", 1.0)?,
        fill: number(r, "fill", 0.35)?,
        rim: number(r, "rim", 0.8)?,
        key_from: match r.get("key_from") {
            Some(v) => triple(v, "rig.key_from")?,
            None => base.key_from,
        },
        key_color: r
            .get("key_color")
            .and_then(Value::as_str)
            .map(hex_linear)
            .transpose()?,
        rim_color,
        strength: number(r, "strength", 0.6)?,
        turn: number(r, "turn", 25.0)?,
    })
}

pub struct Finish {
    pub view: View,
    pub view_name: String,
    pub exposure: f32,
    pub lines: Lines,
}

pub fn finish(preset: &Value, ignored: &mut Vec<String>) -> Result<Finish, String> {
    let map = preset.as_object().ok_or("preset is not a table")?;
    let name = text(map, "view", "Standard")?;
    let view = View::parse(name).ok_or_else(|| {
        format!(
            "preset view {name}: the tracer finishes in {}",
            VIEWS.join(" or ")
        )
    })?;
    let look = text(map, "look", "None")?;
    if look != "None" {
        note(ignored, "look");
    }
    if map.contains_key("denoise") {
        note(ignored, "denoise");
    }
    let mut lines = Lines::default();
    if let Some(o) = section(preset, "outline")? {
        lines.enabled = flag(o, "enabled", false)?;
        lines.thickness = number(o, "thickness", 2.0)?;
        if let Some(color) = o.get("color").and_then(Value::as_str) {
            lines.color = hex_linear(color)?;
        }
        lines.alpha = number(o, "alpha", 1.0)?;
        lines.crease_angle = number(o, "crease_angle", 134.0)?;
        lines.silhouette = flag(o, "silhouette", true)?;
        lines.border = flag(o, "border", true)?;
        lines.crease = flag(o, "crease", true)?;
        lines.contour = flag(o, "contour", false)?;
        lines.external_contour = flag(o, "external_contour", true)?;
    }
    Ok(Finish {
        view,
        view_name: if view == View::Agx { "AgX" } else { "Standard" }.into(),
        exposure: number(map, "exposure", 0.0)?,
        lines,
    })
}

fn each(value: Option<&Value>, all: f32, scalar: bool, key: &str) -> Result<Each, String> {
    let mut out = Each::new(all);
    match value {
        None => {}
        Some(Value::Number(n)) if scalar => out.all = n.as_f64().unwrap_or(0.0) as f32,
        Some(Value::Number(_)) => {}
        Some(Value::Array(list)) => {
            for (i, v) in list.iter().enumerate() {
                let v = v
                    .as_f64()
                    .ok_or_else(|| format!("form.{key} lists numbers"))?;
                out.by_element.insert(i, v as f32);
            }
        }
        Some(Value::Object(map)) => {
            for (k, v) in map {
                let i: usize = k
                    .parse()
                    .map_err(|_| format!("form.{key}: {k} is not an element number"))?;
                let v = v
                    .as_f64()
                    .ok_or_else(|| format!("form.{key} maps to numbers"))?;
                out.by_element.insert(i, v as f32);
            }
        }
        Some(_) => return Err(format!("form.{key} is a number, a list or a map")),
    }
    Ok(out)
}

pub fn form(preset: &Value) -> Result<Form, String> {
    let f = &table(preset, "form")?;
    let mut form = Form {
        depth: number(f, "depth", 0.12)?,
        bevel: number(f, "bevel", 0.05)?,
        segments: number(f, "segments", 8.0)?.max(1.0) as u32,
        layer: number(f, "layer", 0.25)?,
        inset: each(f.get("inset"), 0.0, true, "inset")?,
        element_depth: each(f.get("element_depth"), 1.0, false, "element_depth")?,
        element_z: each(f.get("element_z"), 0.0, false, "element_z")?,
        merge: flag(f, "merge", true)?,
        ..Form::default()
    };
    let mut named = BTreeMap::new();
    match f.get("element_material") {
        None | Some(Value::String(_)) => {}
        Some(Value::Array(list)) => {
            for (i, v) in list.iter().enumerate() {
                if let Some(name) = v.as_str() {
                    named.insert(i, name.to_string());
                }
            }
        }
        Some(Value::Object(map)) => {
            for (k, v) in map {
                let i: usize = k
                    .parse()
                    .map_err(|_| format!("form.element_material: {k} is not an element number"))?;
                if let Some(name) = v.as_str() {
                    named.insert(i, name.to_string());
                }
            }
        }
        Some(_) => return Err("form.element_material is a list or a map of names".into()),
    }
    form.element_material = named;
    Ok(form)
}

fn pair(map: &Map<String, Value>, key: &str, default: [f32; 2]) -> Result<[f32; 2], String> {
    match map.get(key) {
        None => Ok(default),
        Some(v) => {
            let list: Vec<f32> = v
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_f64)
                        .map(|v| v as f32)
                        .collect()
                })
                .unwrap_or_default();
            match list[..] {
                [x, y] => Ok([x, y]),
                _ => Err(format!("{key} is [x, y]")),
            }
        }
    }
}

pub fn icon(spec: &Value) -> Result<Icon, String> {
    check_icon(spec)?;
    let map = spec.as_object().ok_or("an icon is a table")?;
    let mut parts = Vec::new();
    for item in map
        .get("parts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let p = item.as_object().ok_or("a part is a table")?;
        let z = number(p, "z", 0.0)?;
        let shape = match p.get("kind").and_then(Value::as_str).unwrap_or("") {
            "ball" => PartShape::Ball {
                at: pair(p, "at", [0.0, 0.0])?,
                z,
                r: number(p, "r", 0.3)?,
            },
            "drop" => PartShape::Drop {
                at: pair(p, "at", [0.0, 0.0])?,
                z,
                r: number(p, "r", 1.0)?,
                squash: number(p, "squash", 0.45)?,
                stretch: number(p, "stretch", 1.0)?,
            },
            "capsule" => PartShape::Capsule {
                from: pair(p, "from", [0.0, 0.0])?,
                to: pair(p, "to", [0.0, 0.0])?,
                z,
                r: number(p, "r", 0.08)?,
            },
            "tube" => PartShape::Tube {
                points: p
                    .get("points")
                    .and_then(Value::as_array)
                    .ok_or("a tube lists points")?
                    .iter()
                    .map(|pt| {
                        let xy: Vec<f32> = pt
                            .as_array()
                            .map(|a| {
                                a.iter()
                                    .filter_map(Value::as_f64)
                                    .map(|v| v as f32)
                                    .collect()
                            })
                            .unwrap_or_default();
                        match xy[..] {
                            [x, y] => Ok([x, y]),
                            _ => Err("a tube's points are [x, y]".to_string()),
                        }
                    })
                    .collect::<Result<_, _>>()?,
                z,
                r: number(p, "r", 0.08)?,
            },
            "torus" => PartShape::Torus {
                at: pair(p, "at", [0.0, 0.0])?,
                z,
                major: number(p, "major", 0.6)?,
                minor: number(p, "minor", 0.1)?,
            },
            "box" => PartShape::Box {
                at: pair(p, "at", [0.0, 0.0])?,
                z,
                half: match p.get("half") {
                    Some(v) => triple(v, "half")?,
                    None => [0.3, 0.3, 0.1],
                },
                round: number(p, "round", 0.05)?,
                angle: number(p, "angle", 0.0)?,
            },
            other => return Err(format!("unknown part kind {other}")),
        };
        let mut part = Part::new(shape);
        if let Some(name) = p.get("material").and_then(Value::as_str)
            && !name.is_empty()
        {
            part = part.with_material(name);
        }
        parts.push(part);
    }
    let mut icon = Icon::new(parts);
    icon.fuse = flag(map, "fuse", true)?;
    icon.voxel = number(map, "voxel", f64::from(icon.voxel))?;
    Ok(icon)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn presets_refuse_unknown_keys_by_section() {
        assert!(check_preset(&json!({"material": {"base": "#808080"}})).is_ok());
        let error = check_preset(&json!({"material": {"shine": 1}})).unwrap_err();
        assert!(
            error.contains("preset [material]") && error.contains("shine"),
            "{error}"
        );
        assert!(check_preset(&json!({"lights": {}})).is_err());
        assert!(check_preset(&json!({"materials": {"gold": {"glow": 1}}})).is_err());
        assert!(check_icon(&json!({"name": "x", "parts": [{"kind": "ball", "r": 1}]})).is_ok());
        assert!(check_icon(&json!({"name": "x", "parts": [{"kind": "cone"}]})).is_err());
        assert!(check_icon(&json!({"name": "x", "parts": [{"kind": "ball", "half": 1}]})).is_err());
    }

    #[test]
    fn material_keys_map_onto_the_engine_material() {
        let mut ignored = Vec::new();
        let look = json!({
            "base": "#808080", "metallic": 0.0, "roughness": 0.5, "coat": 0.4,
            "coat_roughness": 0.1, "transmission": 1.0, "ior": 1.33, "film": 300.0
        });
        let m = material(look.as_object().unwrap(), None, &mut ignored).unwrap();
        assert!((m.base[0] - 0.2158605).abs() < 1e-5, "{:?}", m.base);
        assert_eq!((m.metalness, m.roughness, m.clearcoat), (0.0, 0.5, 0.4));
        assert_eq!(
            (m.clearcoat_roughness, m.transmission, m.ior),
            (0.1, 1.0, 1.33)
        );
        assert_eq!(
            (m.thin_film, m.thin_film_ior, m.thin_film_amount),
            (300.0, 1.4, 1.0)
        );
        assert!(ignored.is_empty());
        let plain = material(&Map::new(), Some([0.1, 0.2, 0.3]), &mut ignored).unwrap();
        assert_eq!(plain.base, [0.1, 0.2, 0.3]);
        assert_eq!(
            (plain.metalness, plain.roughness, plain.ior),
            (1.0, 0.25, 1.5)
        );
        let worn = json!({"wear": 0.4, "grain": 0.2});
        let w = material(worn.as_object().unwrap(), None, &mut ignored).unwrap();
        assert!((w.roughness - 0.31).abs() < 1e-6);
        assert_eq!(ignored, ["material.grain", "material.wear (colour)"]);
    }

    #[test]
    fn a_named_look_replaces_the_preset_material() {
        let preset =
            json!({"material": {"roughness": 0.3}, "materials": {"gold": {"roughness": 0.1}}});
        assert_eq!(
            look_of(&preset, Some("gold")).unwrap()["roughness"],
            json!(0.1)
        );
        assert_eq!(
            look_of(&preset, Some("none")).unwrap()["roughness"],
            json!(0.3)
        );
        assert_eq!(look_of(&preset, None).unwrap()["roughness"], json!(0.3));
        assert!(look_of(&json!({}), None).unwrap().is_empty());
    }

    #[test]
    fn the_rig_view_and_outline_read_their_keys() {
        let mut ignored = Vec::new();
        let preset = json!({
            "rig": {"key": 2.0, "key_from": [1, 2, 3], "rim_color": ["#ff0000", "#0000ff"], "turn": 10},
            "view": "AgX", "exposure": 0.5, "look": "Punchy",
            "outline": {"enabled": true, "thickness": 3, "color": "#000000", "contour": true}
        });
        let r = rig(&preset).unwrap();
        assert_eq!(
            (r.key, r.key_from, r.turn, r.strength),
            (2.0, [1.0, 2.0, 3.0], 10.0, 0.6)
        );
        assert_eq!(r.rim_color, [[1.0, 0.0, 0.0], [0.0, 0.0, 1.0]]);
        assert_eq!(r.boxes().len(), 4);
        let f = finish(&preset, &mut ignored).unwrap();
        assert_eq!((f.view, f.exposure), (View::Agx, 0.5));
        assert!(f.lines.enabled && f.lines.contour && f.lines.thickness == 3.0);
        assert_eq!(f.lines.color, [0.0; 3]);
        assert_eq!(ignored, ["look"]);
        assert!(finish(&json!({"view": "Filmic"}), &mut ignored).is_err());
        assert_eq!(
            finish(&json!({}), &mut ignored).unwrap().view,
            View::Standard
        );
    }

    #[test]
    fn form_keys_reach_every_element_or_one() {
        let f = form(&json!({"form": {
            "depth": 0.1, "bevel": 0.03, "inset": 0.5, "element_depth": [1.0, 2.0],
            "element_z": {"1": 0.5}, "element_material": ["", "gold"], "merge": false
        }}))
        .unwrap();
        assert_eq!((f.depth, f.bevel, f.merge), (0.1, 0.03, false));
        assert_eq!(
            (f.inset.at(0), f.element_depth.at(1), f.element_depth.at(5)),
            (0.5, 2.0, 1.0)
        );
        assert_eq!((f.element_z.at(0), f.element_z.at(1)), (0.0, 0.5));
        assert_eq!(f.element_material.get(&1).map(String::as_str), Some("gold"));
        let scalar = form(&json!({"form": {"element_depth": 3.0}})).unwrap();
        assert_eq!(scalar.element_depth.at(0), 1.0);
        assert_eq!(form(&json!({})).unwrap(), Form::default());
    }

    #[test]
    fn icon_parts_take_render_defaults() {
        let i = icon(
            &json!({"name": "x", "voxel": 0.02, "fuse": false, "parts": [
                {"kind": "ball", "at": [0.1, 0.2]},
                {"kind": "torus", "major": 0.5, "minor": 0.1, "material": "gold"},
                {"kind": "box"}
            ]}),
        )
        .unwrap();
        assert_eq!((i.voxel, i.fuse), (0.02, false));
        assert_eq!(
            i.parts[0].shape,
            PartShape::Ball {
                at: [0.1, 0.2],
                z: 0.0,
                r: 0.3
            }
        );
        assert_eq!(i.parts[1].material.as_deref(), Some("gold"));
        assert_eq!(i.parts[2].shape, PartShape::cube());
    }

    #[test]
    fn colours_read_three_and_six_digits() {
        assert_eq!(hex_srgb("#fff").unwrap(), [1.0; 3]);
        assert_eq!(hex_srgb("000000").unwrap(), [0.0; 3]);
        assert!(hex_srgb("#12345").is_err());
        assert_eq!(bytes_linear([255, 0, 0]), [1.0, 0.0, 0.0]);
    }
}
