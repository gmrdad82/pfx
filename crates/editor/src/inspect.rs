use std::collections::BTreeMap;
use std::path::PathBuf;

use pfx_load::scene::{
    BodyShape, EditError, PatchGroup, Projection, Scene, SceneEdit, Shadow, Target, Value,
};
use pfx_play::characters::{self, Setting};
use pfx_play::{Edit, Kind, TunableValue, Tunables, body_volume};

use crate::outline::Item;

pub type Vec3 = [f32; 3];

pub const SHADOWS: [&str; 3] = ["cast", "only", "none"];
pub const SUN_MODELS: [&str; 2] = ["daylight", "authored"];
pub const SKY_KINDS: [&str; 4] = ["analytic", "hdr", "room", "mix"];
pub const PROJECTIONS: [&str; 2] = ["perspective", "orthographic"];
pub const NONE: &str = "";
pub const TUNABLES: &str = "tunables";
pub const CHARACTER: &str = "character";
pub const PLANES: [&str; 2] = ["xy", "yz"];
pub const KINDS: [&str; 2] = ["3d", "2d"];

#[derive(Clone, Debug, PartialEq)]
pub enum Field {
    Number(f32),
    Pair([f32; 2]),
    Vector(Vec3),
    Color(Vec3),
    Toggle(bool),
    Choice { value: String, options: Vec<String> },
    Text(String),
    Planes(Vec<[f32; 4]>),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Range {
    pub min: f32,
    pub max: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub label: String,
    pub target: Target,
    pub path: Vec<String>,
    pub field: Field,
    pub unit: &'static str,
    pub speed: f32,
    pub range: Option<Range>,
    pub pending: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub target: Target,
    pub path: Vec<String>,
    pub value: Option<Value>,
}

impl Change {
    pub fn apply(&self, edit: &mut SceneEdit) -> Result<PatchGroup, EditError> {
        let path: Vec<&str> = self.path.iter().map(String::as_str).collect();
        match &self.value {
            Some(value) => edit.set(&self.target, &path, value.clone()),
            None => edit.unset(&self.target, &path),
        }
    }

    pub fn describe(&self) -> String {
        format!("{} {}", self.target, self.path.join("."))
    }

    pub fn tunable(&self) -> Option<&str> {
        match (&self.target, self.path.as_slice()) {
            (Target::Scene, [first, name]) if first == TUNABLES => Some(name),
            _ => None,
        }
    }

    pub fn filed(&self, scene: &Scene) -> Change {
        let body = match (&self.target, self.path.as_slice()) {
            (Target::Object(name), [first, key]) if first == "body" && key == "mass" => {
                scene.bodies.get(name)
            }
            _ => None,
        };
        let mass = self.value.as_ref().and_then(number_of);
        match (body, mass) {
            (Some(body), Some(mass)) => Change {
                target: self.target.clone(),
                path: vec!["body".to_string(), "density".to_string()],
                value: Some(Value::from(mass / body_volume(body.shape))),
            },
            _ => self.clone(),
        }
    }

    pub fn live(&self, tunables: &Tunables) -> Result<Edit, String> {
        let Some(value) = &self.value else {
            return Err(format!("{}: play mode cannot unset a key", self.describe()));
        };
        let Some(name) = self.tunable() else {
            let path: Vec<&str> = self.path.iter().map(String::as_str).collect();
            return Ok(Edit::scene(self.target.clone(), &path, value.clone()));
        };
        let tunable = tunables
            .get(name)
            .ok_or_else(|| format!("no tunable {name}"))?;
        let parsed = match (tunable.kind, value) {
            (Kind::Bool, Value::Bool(on)) => Some(TunableValue::Bool(*on)),
            (Kind::Int, value) => {
                number_of(value).map(|amount| TunableValue::Int(amount.round() as i64))
            }
            (Kind::Float, value) => number_of(value).map(TunableValue::Float),
            (Kind::Vector, value) => vector_of(value).map(TunableValue::Vector),
            _ => None,
        };
        parsed
            .map(|value| Edit::tunable(name, value))
            .ok_or_else(|| format!("tunable {name}: {value:?} is not a {}", tunable.kind.word()))
    }
}

fn number_of(value: &Value) -> Option<f32> {
    match value {
        Value::Float(value) => Some(*value),
        Value::Int(value) => Some(*value as f32),
        _ => None,
    }
}

fn vector_of(value: &Value) -> Option<Vec3> {
    let Value::Array(items) = value else {
        return None;
    };
    if items.len() != 3 {
        return None;
    }
    let mut out = [0.0; 3];
    for (slot, item) in out.iter_mut().zip(items) {
        *slot = number_of(item)?;
    }
    Some(out)
}

pub fn field_of(field: &Field, value: &Value) -> Option<Field> {
    Some(match (field, value) {
        (Field::Number(_), value) => Field::Number(number_of(value)?),
        (Field::Vector(_), value) => Field::Vector(vector_of(value)?),
        (Field::Color(_), value) => Field::Color(vector_of(value)?),
        (Field::Pair(_), Value::Array(items)) if items.len() == 2 => {
            Field::Pair([number_of(&items[0])?, number_of(&items[1])?])
        }
        (Field::Toggle(_), Value::Bool(on)) => Field::Toggle(*on),
        (Field::Choice { options, .. }, Value::Text(text)) => Field::Choice {
            value: text.clone(),
            options: options.clone(),
        },
        (Field::Text(_), Value::Text(text)) => Field::Text(text.clone()),
        _ => return None,
    })
}

pub fn overlay(rows: &mut [Row], live: &BTreeMap<String, Value>) {
    for row in rows {
        let key = format!("{} {}", row.target, row.path.join("."));
        row.pending = false;
        if let Some(value) = live.get(&key)
            && let Some(field) = field_of(&row.field, value)
        {
            row.field = field;
            row.pending = true;
        }
    }
}

fn value_of(field: &Field) -> Option<Value> {
    Some(match field {
        Field::Number(value) => Value::from(*value),
        Field::Pair(value) => Value::from(*value),
        Field::Vector(value) | Field::Color(value) => Value::from(*value),
        Field::Toggle(value) => Value::from(*value),
        Field::Choice { value, .. } if value.is_empty() => return None,
        Field::Choice { value, .. } | Field::Text(value) => Value::from(value.as_str()),
        Field::Planes(planes) => {
            Value::Array(planes.iter().map(|plane| Value::from(*plane)).collect())
        }
    })
}

impl Row {
    pub fn change(&self, field: Field) -> Option<Change> {
        if std::mem::discriminant(&field) != std::mem::discriminant(&self.field)
            || field == self.field
        {
            return None;
        }
        let field = match (field, self.range) {
            (Field::Number(value), Some(range)) => Field::Number(value.clamp(range.min, range.max)),
            (field, _) => field,
        };
        if field == self.field {
            return None;
        }
        Some(Change {
            target: self.target.clone(),
            path: self.path.clone(),
            value: value_of(&field),
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Raw {
    pub tables: BTreeMap<String, (PathBuf, toml::Table)>,
}

pub const SINGLETONS: [&str; 5] = ["sun", "sky", "haze", "camera", "finish"];

impl Raw {
    pub fn of(edit: &SceneEdit) -> Raw {
        let mut tables = BTreeMap::new();
        for file in edit.files() {
            let Some(text) = edit.text(file) else {
                continue;
            };
            let Ok(parsed) = toml::from_str::<toml::Table>(text) else {
                continue;
            };
            for key in SINGLETONS {
                if let Some(toml::Value::Table(table)) = parsed.get(key) {
                    tables
                        .entry(key.to_string())
                        .or_insert_with(|| (file.to_path_buf(), table.clone()));
                }
            }
        }
        Raw { tables }
    }

    pub fn table(&self, key: &str) -> Option<&toml::Table> {
        self.tables.get(key).map(|(_, table)| table)
    }

    pub fn file(&self, key: &str) -> Option<&PathBuf> {
        self.tables.get(key).map(|(file, _)| file)
    }
}

fn number(value: Option<&toml::Value>) -> Option<f32> {
    match value? {
        toml::Value::Float(value) => Some(*value as f32),
        toml::Value::Integer(value) => Some(*value as f32),
        _ => None,
    }
}

fn vector(value: Option<&toml::Value>) -> Option<Vec3> {
    let toml::Value::Array(items) = value? else {
        return None;
    };
    if items.len() != 3 {
        return None;
    }
    let mut out = [0.0; 3];
    for (slot, item) in out.iter_mut().zip(items) {
        *slot = number(Some(item))?;
    }
    Some(out)
}

fn text(value: Option<&toml::Value>) -> Option<String> {
    match value? {
        toml::Value::String(text) => Some(text.clone()),
        _ => None,
    }
}

struct Rows {
    target: Target,
    rows: Vec<Row>,
}

impl Rows {
    fn new(target: Target) -> Rows {
        Rows {
            target,
            rows: Vec::new(),
        }
    }

    fn push(&mut self, path: &[&str], field: Field) -> &mut Row {
        self.rows.push(Row {
            label: path.join("."),
            target: self.target.clone(),
            path: path.iter().map(|key| key.to_string()).collect(),
            field,
            unit: "",
            speed: 0.01,
            range: None,
            pending: false,
        });
        self.rows.last_mut().expect("a row was just pushed")
    }

    fn number(&mut self, path: &[&str], value: f32) -> &mut Row {
        self.push(path, Field::Number(value))
    }

    fn unit(&mut self, path: &[&str], value: f32) {
        self.number(path, value).range = Some(Range { min: 0.0, max: 1.0 });
    }

    fn positive(&mut self, path: &[&str], value: f32) {
        self.number(path, value).range = Some(Range {
            min: 0.0,
            max: f32::MAX,
        });
    }

    fn degrees(&mut self, path: &[&str], value: f32) {
        let row = self.number(path, value);
        row.unit = "°";
        row.speed = 0.5;
    }

    fn vector(&mut self, path: &[&str], value: Vec3) {
        self.push(path, Field::Vector(value));
    }

    fn color(&mut self, path: &[&str], value: Vec3) {
        self.push(path, Field::Color(value));
    }

    fn toggle(&mut self, path: &[&str], value: bool) {
        self.push(path, Field::Toggle(value));
    }

    fn choice<'a>(
        &mut self,
        path: &[&str],
        value: impl Into<String>,
        options: impl IntoIterator<Item = &'a str>,
    ) {
        self.push(
            path,
            Field::Choice {
                value: value.into(),
                options: options.into_iter().map(str::to_string).collect(),
            },
        );
    }

    fn text(&mut self, path: &[&str], value: impl Into<String>) {
        self.push(path, Field::Text(value.into()));
    }
}

fn shadow_name(shadow: Shadow) -> &'static str {
    match shadow {
        Shadow::Cast => "cast",
        Shadow::Only => "only",
        Shadow::None => "none",
    }
}

fn materials(scene: &Scene) -> Vec<&str> {
    std::iter::once(NONE).chain(scene.library.names()).collect()
}

fn object_rows(scene: &Scene, name: &str) -> Vec<Row> {
    let Some(object) = scene.object(name) else {
        return Vec::new();
    };
    let mut rows = Rows::new(Target::Object(name.to_string()));
    rows.vector(&["at"], object.at);
    rows.push(&["rotate"], Field::Vector(object.rotate)).unit = "°";
    rows.vector(&["scale"], object.scale);
    rows.choice(
        &["material"],
        object.material.clone().unwrap_or_default(),
        materials(scene),
    );
    rows.choice(&["shadow"], shadow_name(object.shadow), SHADOWS);
    rows.toggle(&["two_sided"], object.two_sided);
    rows.toggle(&["hidden"], object.hidden);
    rows.push(&["clip"], Field::Planes(object.clip.clone()));
    let parents = std::iter::once(NONE).chain(
        scene
            .objects
            .iter()
            .map(|other| other.name.as_str())
            .filter(|other| *other != name),
    );
    rows.choice(
        &["parent"],
        object.parent.clone().unwrap_or_default(),
        parents,
    );
    if let Some(body) = scene.bodies.get(name) {
        rows.positive(&["body", "mass"], body.density * body_volume(body.shape));
        rows.positive(&["body", "friction"], body.friction);
        rows.positive(&["body", "restitution"], body.restitution);
        rows.push(&["body", "damping"], Field::Pair(body.damping));
        rows.vector(&["body", "velocity"], body.velocity);
        match body.shape {
            BodyShape::Box { half } => rows.vector(&["body", "half"], half),
            BodyShape::Sphere { radius } => rows.positive(&["body", "radius"], radius),
            BodyShape::Capsule {
                half_height,
                radius,
            }
            | BodyShape::Cylinder {
                half_height,
                radius,
            } => {
                rows.positive(&["body", "half_height"], half_height);
                rows.positive(&["body", "radius"], radius);
            }
        }
    }
    if let Some(character) = scene.characters.get(name) {
        for (key, setting) in characters::settings(character) {
            let path = [CHARACTER, key];
            match setting {
                Setting::Number(value) => match key {
                    "max_climb" => rows.degrees(&path, value),
                    "coyote" | "jump_buffer" => {
                        let row = rows.number(&path, value);
                        row.speed = 0.1;
                        row.unit = "ticks";
                        row.range = Some(Range {
                            min: 0.0,
                            max: f32::MAX,
                        });
                    }
                    "variable_jump" => rows.unit(&path, value),
                    _ => rows.positive(&path, value),
                },
                Setting::Toggle(on) => rows.toggle(&path, on),
                Setting::Word(word) if key == "kind" => rows.choice(&path, word, KINDS),
                Setting::Pair(pair) => {
                    rows.push(&path, Field::Pair(pair));
                }
                Setting::Word(word) => rows.choice(&path, word, PLANES),
            }
        }
    }
    rows.rows
}

fn sound_rows(scene: &Scene, name: &str) -> Vec<Row> {
    let Some(sound) = scene.sounds.get(name) else {
        return Vec::new();
    };
    let mut rows = Rows::new(Target::Sound(name.to_string()));
    rows.positive(&["volume"], sound.volume);
    rows.number(&["pan"], sound.pan).range = Some(Range {
        min: -1.0,
        max: 1.0,
    });
    rows.toggle(&["loop"], sound.looping);
    rows.rows
}

fn tunable_rows(tunables: &Tunables, group: &str) -> Vec<Row> {
    let mut rows = Rows::new(Target::Scene);
    for tunable in tunables.iter().filter(|tunable| tunable.group == group) {
        let path = [TUNABLES, tunable.name.as_str()];
        let bound = |value: Option<TunableValue>, at: f32| match value {
            Some(TunableValue::Float(value)) => value,
            Some(TunableValue::Int(value)) => value as f32,
            _ => at,
        };
        let row = match tunable.default {
            TunableValue::Float(value) => rows.number(&path, value),
            TunableValue::Int(value) => {
                let row = rows.number(&path, value as f32);
                row.speed = 0.1;
                row
            }
            TunableValue::Bool(value) => rows.push(&path, Field::Toggle(value)),
            TunableValue::Vector(value) => rows.push(&path, Field::Vector(value)),
        };
        row.label = tunable.name.clone();
        if matches!(tunable.kind, Kind::Float | Kind::Int)
            && (tunable.min.is_some() || tunable.max.is_some())
        {
            row.range = Some(Range {
                min: bound(tunable.min, f32::MIN),
                max: bound(tunable.max, f32::MAX),
            });
        }
    }
    rows.rows
}

fn material_rows(scene: &Scene, name: &str) -> Vec<Row> {
    let Some(material) = scene.library.get(name) else {
        return Vec::new();
    };
    let family = serde_json::to_value(material.family)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default();
    let mut rows = Rows::new(Target::Material(name.to_string()));
    rows.text(&["family"], family);
    rows.color(&["base"], material.base);
    rows.unit(&["roughness"], material.roughness);
    rows.unit(&["metalness"], material.metalness);
    rows.unit(&["specular"], material.specular);
    rows.unit(&["clearcoat"], material.clearcoat);
    rows.unit(&["clearcoat_roughness"], material.clearcoat_roughness);
    rows.unit(&["sheen"], material.sheen);
    rows.unit(&["transmission"], material.transmission);
    rows.positive(&["ior"], material.ior);
    rows.positive(&["thickness"], material.thickness);
    rows.unit(&["subsurface"], material.subsurface);
    rows.positive(&["absorption"], material.absorption);
    rows.vector(&["emission"], material.emission);
    rows.rows
}

fn light_rows(scene: &Scene, name: &str) -> Vec<Row> {
    let Some(light) = scene.lights.get(name) else {
        return Vec::new();
    };
    let mut rows = Rows::new(Target::Light(name.to_string()));
    rows.vector(&["position"], light.position);
    rows.color(&["color"], light.color);
    rows.positive(&["intensity"], light.intensity);
    rows.positive(&["radius"], light.radius);
    rows.positive(&["range"], light.range);
    rows.toggle(&["shadow"], light.shadow);
    rows.rows
}

fn emitter_rows(scene: &Scene, name: &str) -> Vec<Row> {
    let Some(emitter) = scene.emitters.get(name) else {
        return Vec::new();
    };
    let mut rows = Rows::new(Target::Emitter(name.to_string()));
    rows.vector(&["position"], emitter.position);
    rows.positive(&["radius"], emitter.radius);
    rows.color(&["color"], emitter.color);
    rows.positive(&["intensity"], emitter.intensity);
    rows.rows
}

fn mesh_rows(scene: &Scene, name: &str) -> Vec<Row> {
    let Some(mesh) = scene.meshes.get(name) else {
        return Vec::new();
    };
    let mut rows = Rows::new(Target::Mesh(name.to_string()));
    rows.text(&["file"], crate::outline::relative(&scene.path, &mesh.file));
    rows.text(&["node"], mesh.node.clone().unwrap_or_default());
    rows.rows
}

fn node_rows(scene: &Scene, mesh: &str, node: &str) -> Vec<Row> {
    let Some(part) = scene
        .meshes
        .get(mesh)
        .and_then(|found| found.parts.iter().find(|part| part.node == node))
    else {
        return Vec::new();
    };
    let overrides = &part.overrides;
    let mut rows = Rows::new(Target::Node {
        mesh: mesh.to_string(),
        node: node.to_string(),
    });
    rows.choice(
        &["material"],
        overrides.material.clone().unwrap_or_default(),
        materials(scene),
    );
    rows.toggle(&["hidden"], overrides.hidden);
    rows.choice(
        &["shadow"],
        overrides.shadow.map(shadow_name).unwrap_or(NONE),
        std::iter::once(NONE).chain(SHADOWS),
    );
    rows.toggle(&["two_sided"], overrides.two_sided.unwrap_or(false));
    rows.rows
}

fn sun_rows(raw: &Raw) -> Vec<Row> {
    let empty = toml::Table::new();
    let table = raw.table("sun").unwrap_or(&empty);
    let mut rows = Rows::new(Target::Sun);
    let model = text(table.get("model")).unwrap_or_else(|| "daylight".to_string());
    rows.choice(&["model"], model.clone(), SUN_MODELS);
    rows.number(&["hour"], number(table.get("hour")).unwrap_or(12.0))
        .range = Some(Range {
        min: 0.0,
        max: 24.0,
    });
    rows.number(&["day"], number(table.get("day")).unwrap_or(172.0));
    rows.degrees(&["latitude"], number(table.get("latitude")).unwrap_or(45.0));
    rows.degrees(&["heading"], number(table.get("heading")).unwrap_or(180.0));
    if model == "authored" {
        rows.number(
            &["reference_hour"],
            number(table.get("reference_hour")).unwrap_or(12.0),
        );
        rows.vector(
            &["toward"],
            vector(table.get("toward")).unwrap_or([0.0, 1.0, 0.0]),
        );
        rows.color(&["color"], vector(table.get("color")).unwrap_or([1.0; 3]));
        rows.positive(
            &["irradiance"],
            number(table.get("irradiance")).unwrap_or(1.0),
        );
    }
    rows.number(&["radius"], number(table.get("radius")).unwrap_or(0.0))
        .range = Some(Range {
        min: 0.0,
        max: 10.0,
    });
    rows.rows
}

fn sky_rows(raw: &Raw) -> Vec<Row> {
    let empty = toml::Table::new();
    let table = raw.table("sky").unwrap_or(&empty);
    let mut rows = Rows::new(Target::Sky);
    let kind = text(table.get("kind")).unwrap_or_default();
    rows.choice(&["kind"], kind.clone(), SKY_KINDS);
    match kind.as_str() {
        "analytic" => {
            rows.number(
                &["turbidity"],
                number(table.get("turbidity")).unwrap_or(3.0),
            )
            .range = Some(Range {
                min: 1.0,
                max: 10.0,
            });
            rows.color(
                &["ground_albedo"],
                vector(table.get("ground_albedo")).unwrap_or([0.2; 3]),
            );
            rows.positive(&["ambient"], number(table.get("ambient")).unwrap_or(1.0));
        }
        "hdr" | "room" => {
            if kind == "hdr" {
                rows.text(&["path"], text(table.get("path")).unwrap_or_default());
            }
            rows.degrees(
                &["rotation_deg"],
                number(table.get("rotation_deg")).unwrap_or(0.0),
            );
            rows.positive(
                &["intensity"],
                number(table.get("intensity")).unwrap_or(1.0),
            );
            if kind == "room"
                && let Some(toml::Value::Table(room)) = table.get("room")
            {
                for key in ["floor", "wall", "ceiling"] {
                    if let Some(value) = vector(room.get(key)) {
                        rows.color(&["room", key], value);
                    }
                }
            }
        }
        _ => {}
    }
    rows.rows
}

const HAZE_VECTORS: [&str; 2] = ["lo", "hi"];
const HAZE_NUMBERS: [&str; 8] = [
    "amount", "fog", "smoke", "mist", "floor", "phase", "back", "reach",
];
const HAZE_COLORS: [&str; 2] = ["gold", "ambient"];

fn haze_rows(scene: &Scene, raw: &Raw) -> Vec<Row> {
    let mut rows = Rows::new(Target::Haze);
    let Some(haze) = &scene.haze else {
        return rows.rows;
    };
    let empty = toml::Table::new();
    let table = raw.table("haze").unwrap_or(&empty);
    rows.vector(&[HAZE_VECTORS[0]], haze.lo);
    rows.vector(&[HAZE_VECTORS[1]], haze.hi);
    for key in HAZE_NUMBERS {
        let value = match key {
            "amount" => number(table.get(key)).unwrap_or(0.0),
            "fog" => haze.fog,
            "smoke" => haze.smoke,
            "mist" => haze.mist,
            "floor" => haze.floor,
            "phase" => haze.phase,
            "back" => haze.back,
            _ => haze.reach,
        };
        if key == "amount" || key == "phase" {
            let (min, max) = if key == "amount" {
                (0.0, 1.0)
            } else {
                (-0.9, 0.9)
            };
            rows.number(&[key], value).range = Some(Range { min, max });
        } else {
            rows.positive(&[key], value);
        }
    }
    rows.color(&[HAZE_COLORS[0]], haze.gold);
    rows.color(&[HAZE_COLORS[1]], haze.ambient);
    rows.rows
}

fn camera_rows(scene: &Scene, raw: &Raw) -> Vec<Row> {
    let camera = scene.camera_or_default();
    let empty = toml::Table::new();
    let table = raw.table("camera").unwrap_or(&empty);
    let mut rows = Rows::new(Target::Camera);
    rows.vector(&["at"], camera.at);
    rows.vector(&["look_at"], camera.look_at);
    rows.vector(&["up"], camera.up);
    match camera.projection {
        Projection::Perspective { fov } => {
            rows.choice(&["projection"], PROJECTIONS[0], PROJECTIONS);
            if table.contains_key("focal") {
                rows.positive(&["focal"], number(table.get("focal")).unwrap_or(50.0));
                rows.positive(&["sensor"], number(table.get("sensor")).unwrap_or(24.0));
            } else {
                rows.degrees(&["fov"], fov);
            }
            if let Some(depth) = camera.depth_of_field {
                rows.positive(&["fstop"], depth.fstop);
                rows.positive(&["focus"], depth.distance);
            }
        }
        Projection::Orthographic { height } => {
            rows.choice(&["projection"], PROJECTIONS[1], PROJECTIONS);
            rows.positive(&["height"], height);
        }
    }
    rows.positive(&["near"], camera.near);
    rows.positive(&["far"], camera.far);
    let exposure = raw
        .table("finish")
        .and_then(|finish| number(finish.get("exposure")))
        .unwrap_or(1.0);
    rows.target = Target::Finish;
    rows.number(&["exposure"], exposure).range = Some(Range {
        min: 0.001,
        max: f32::MAX,
    });
    rows.rows
}

fn finish_rows(raw: &Raw) -> Vec<Row> {
    let mut rows = Rows::new(Target::Finish);
    let Some(table) = raw.table("finish") else {
        return rows.rows;
    };
    for (key, value) in table {
        match value {
            toml::Value::Float(_) | toml::Value::Integer(_) => {
                rows.number(&[key], number(Some(value)).unwrap_or(0.0));
            }
            toml::Value::Boolean(on) => rows.toggle(&[key], *on),
            toml::Value::String(text) => rows.text(&[key], text.clone()),
            toml::Value::Array(_) if vector(Some(value)).is_some() => {
                rows.vector(&[key], vector(Some(value)).unwrap_or([0.0; 3]));
            }
            _ => {}
        }
    }
    rows.rows
}

pub fn rows(scene: &Scene, raw: &Raw, tunables: &Tunables, item: &Item) -> Vec<Row> {
    match item {
        Item::Object(name) => object_rows(scene, name),
        Item::Sound(name) => sound_rows(scene, name),
        Item::Tunables(group) => tunable_rows(tunables, group),
        Item::Material(name) => material_rows(scene, name),
        Item::Light(name) => light_rows(scene, name),
        Item::Emitter(name) => emitter_rows(scene, name),
        Item::Mesh(name) => mesh_rows(scene, name),
        Item::Node { mesh, node } => node_rows(scene, mesh, node),
        Item::Sun => sun_rows(raw),
        Item::Sky => sky_rows(raw),
        Item::Haze => haze_rows(scene, raw),
        Item::Camera => camera_rows(scene, raw),
        Item::Finish => finish_rows(raw),
        Item::File(_) | Item::Scene(_) | Item::Folder(_) | Item::Asset(_) | Item::Layer(_) => {
            Vec::new()
        }
    }
}
