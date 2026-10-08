use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use pfx_load::scene::{DeclaredKind, DeclaredValue, ProjectFile, project_root};

use crate::math::shortest;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Float,
    Int,
    Bool,
    Vector,
}

impl Kind {
    pub fn word(self) -> &'static str {
        match self {
            Kind::Float => "float",
            Kind::Int => "int",
            Kind::Bool => "bool",
            Kind::Vector => "vector",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TunableValue {
    Float(f32),
    Int(i64),
    Bool(bool),
    Vector([f32; 3]),
}

impl TunableValue {
    pub fn kind(self) -> Kind {
        match self {
            TunableValue::Float(_) => Kind::Float,
            TunableValue::Int(_) => Kind::Int,
            TunableValue::Bool(_) => Kind::Bool,
            TunableValue::Vector(_) => Kind::Vector,
        }
    }

    pub fn as_kind(self, kind: Kind) -> Option<TunableValue> {
        Some(match (self, kind) {
            (value, kind) if value.kind() == kind => value,
            (TunableValue::Int(value), Kind::Float) => TunableValue::Float(value as f32),
            (TunableValue::Float(value), Kind::Int) if value.fract() == 0.0 => {
                TunableValue::Int(value as i64)
            }
            _ => return None,
        })
    }

    fn parts(self) -> Vec<f64> {
        match self {
            TunableValue::Float(value) => vec![f64::from(value)],
            TunableValue::Int(value) => vec![value as f64],
            TunableValue::Bool(_) => Vec::new(),
            TunableValue::Vector(value) => value.iter().map(|part| f64::from(*part)).collect(),
        }
    }

    fn written(self) -> toml_edit::Value {
        match self {
            TunableValue::Float(value) => toml_edit::Value::from(shortest(value)),
            TunableValue::Int(value) => toml_edit::Value::from(value),
            TunableValue::Bool(value) => toml_edit::Value::from(value),
            TunableValue::Vector(value) => {
                toml_edit::Value::Array(value.iter().map(|part| shortest(*part)).collect())
            }
        }
    }
}

impl std::fmt::Display for TunableValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TunableValue::Float(value) => write!(f, "{value}"),
            TunableValue::Int(value) => write!(f, "{value}"),
            TunableValue::Bool(value) => write!(f, "{value}"),
            TunableValue::Vector([x, y, z]) => write!(f, "[{x}, {y}, {z}]"),
        }
    }
}

pub trait FromTunable: Sized {
    fn from_tunable(value: TunableValue) -> Option<Self>;
}

impl FromTunable for TunableValue {
    fn from_tunable(value: TunableValue) -> Option<Self> {
        Some(value)
    }
}

impl FromTunable for f32 {
    fn from_tunable(value: TunableValue) -> Option<Self> {
        match value {
            TunableValue::Float(value) => Some(value),
            TunableValue::Int(value) => Some(value as f32),
            _ => None,
        }
    }
}

impl FromTunable for i64 {
    fn from_tunable(value: TunableValue) -> Option<Self> {
        match value {
            TunableValue::Int(value) => Some(value),
            _ => None,
        }
    }
}

impl FromTunable for i32 {
    fn from_tunable(value: TunableValue) -> Option<Self> {
        match value {
            TunableValue::Int(value) => i32::try_from(value).ok(),
            _ => None,
        }
    }
}

impl FromTunable for bool {
    fn from_tunable(value: TunableValue) -> Option<Self> {
        match value {
            TunableValue::Bool(value) => Some(value),
            _ => None,
        }
    }
}

impl FromTunable for [f32; 3] {
    fn from_tunable(value: TunableValue) -> Option<Self> {
        match value {
            TunableValue::Vector(value) => Some(value),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Tunable {
    pub name: String,
    pub kind: Kind,
    pub default: TunableValue,
    pub min: Option<TunableValue>,
    pub max: Option<TunableValue>,
    pub group: String,
}

impl Tunable {
    pub fn check(&self, value: TunableValue) -> Result<TunableValue, String> {
        let value = value.as_kind(self.kind).ok_or_else(|| {
            format!(
                "tunable {}: {value} is not a {}",
                self.name,
                self.kind.word()
            )
        })?;
        let parts = value.parts();
        if parts.iter().any(|part| !part.is_finite()) {
            return Err(format!("tunable {}: {value} is not finite", self.name));
        }
        if let Some(low) = self.min
            && parts
                .iter()
                .zip(low.parts())
                .any(|(part, limit)| *part < limit)
        {
            return Err(format!(
                "tunable {}: {value} is below its min {low}",
                self.name
            ));
        }
        if let Some(high) = self.max
            && parts
                .iter()
                .zip(high.parts())
                .any(|(part, limit)| *part > limit)
        {
            return Err(format!(
                "tunable {}: {value} is above its max {high}",
                self.name
            ));
        }
        Ok(value)
    }

    pub fn clamp(&self, value: TunableValue) -> Option<TunableValue> {
        let value = value.as_kind(self.kind)?;
        let low = self.min.map(TunableValue::parts);
        let high = self.max.map(TunableValue::parts);
        let fit = |at: usize, part: f64| {
            let part = low
                .as_ref()
                .map_or(part, |low| part.max(low.get(at).copied().unwrap_or(part)));
            high.as_ref()
                .map_or(part, |high| part.min(high.get(at).copied().unwrap_or(part)))
        };
        Some(match value {
            TunableValue::Float(value) => TunableValue::Float(fit(0, f64::from(value)) as f32),
            TunableValue::Int(value) => TunableValue::Int(fit(0, value as f64) as i64),
            TunableValue::Bool(value) => TunableValue::Bool(value),
            TunableValue::Vector(value) => TunableValue::Vector(std::array::from_fn(|at| {
                fit(at, f64::from(value[at])) as f32
            })),
        })
    }
}

fn declared(value: DeclaredValue) -> TunableValue {
    match value {
        DeclaredValue::Float(value) => TunableValue::Float(value),
        DeclaredValue::Int(value) => TunableValue::Int(value),
        DeclaredValue::Bool(value) => TunableValue::Bool(value),
        DeclaredValue::Vector(value) => TunableValue::Vector(value),
    }
}

fn declared_kind(kind: DeclaredKind) -> Kind {
    match kind {
        DeclaredKind::Float => Kind::Float,
        DeclaredKind::Int => Kind::Int,
        DeclaredKind::Bool => Kind::Bool,
        DeclaredKind::Vector => Kind::Vector,
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tunables {
    file: Option<PathBuf>,
    list: BTreeMap<String, Tunable>,
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.trim() == name && !name.chars().any(char::is_control)
}

impl Tunables {
    pub fn parse(text: &str, file: &Path) -> Result<Tunables, String> {
        let shown = file.display();
        let parsed: ProjectFile =
            toml::from_str(text).map_err(|error| format!("{shown}: {error}"))?;
        let mut list = BTreeMap::new();
        for (name, entry) in parsed.tunables {
            let refuse = |message: String| format!("{shown}: [tunables.{name}] {message}");
            if !valid_name(&name) {
                return Err(refuse(
                    "needs printable characters without spaces at its ends".to_string(),
                ));
            }
            let kind = declared_kind(entry.kind);
            let read = |value: DeclaredValue, key: &str| {
                declared(value).as_kind(kind).ok_or_else(|| {
                    refuse(format!(
                        "{key} {} is not a {}",
                        declared(value),
                        kind.word()
                    ))
                })
            };
            let default = read(entry.default, "default")?;
            if kind == Kind::Bool && (entry.min.is_some() || entry.max.is_some()) {
                return Err(refuse("a bool takes no min or max".to_string()));
            }
            let min = entry.min.map(|value| read(value, "min")).transpose()?;
            let max = entry.max.map(|value| read(value, "max")).transpose()?;
            if let (Some(low), Some(high)) = (min, max)
                && low.parts().iter().zip(high.parts()).any(|(a, b)| *a > b)
            {
                return Err(refuse(format!("min {low} is above max {high}")));
            }
            let tunable = Tunable {
                name: name.clone(),
                kind,
                default,
                min,
                max,
                group: entry.group.unwrap_or_default(),
            };
            tunable
                .check(default)
                .map_err(|message| format!("{shown}: default: {message}"))?;
            list.insert(name, tunable);
        }
        Ok(Tunables {
            file: Some(file.to_path_buf()),
            list,
        })
    }

    pub fn open(file: &Path) -> Result<Tunables, String> {
        let text = std::fs::read_to_string(file)
            .map_err(|error| format!("{}: {error}", file.display()))?;
        Self::parse(&text, file)
    }

    pub fn find(root: &Path) -> Option<PathBuf> {
        let project = root.join("project.toml");
        std::fs::read_to_string(&project)
            .ok()
            .and_then(|text| toml::from_str::<toml::Table>(&text).ok())
            .is_some_and(|table| table.contains_key("tunables"))
            .then_some(project)
    }

    pub fn of_scene(scene: &Path) -> Result<Tunables, String> {
        match Self::find(&project_root(scene)) {
            Some(file) => Self::open(&file),
            None => Ok(Tunables::default()),
        }
    }

    pub fn file(&self) -> Option<&Path> {
        self.file.as_deref()
    }

    pub fn get(&self, name: &str) -> Option<&Tunable> {
        self.list.get(name)
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Tunable> {
        self.list.values()
    }

    pub fn groups(&self) -> Vec<(&str, Vec<&Tunable>)> {
        let mut groups: BTreeMap<&str, Vec<&Tunable>> = BTreeMap::new();
        for tunable in self.list.values() {
            groups.entry(&tunable.group).or_default().push(tunable);
        }
        groups.into_iter().collect()
    }

    pub fn check(&self, name: &str, value: TunableValue) -> Result<TunableValue, String> {
        self.list
            .get(name)
            .ok_or_else(|| format!("no tunable {name}"))?
            .check(value)
    }

    pub fn written(text: &str, name: &str, value: TunableValue) -> Result<String, String> {
        let mut doc: toml_edit::DocumentMut = text.parse().map_err(|error| format!("{error}"))?;
        let entry = doc
            .get_mut("tunables")
            .and_then(|tunables| tunables.get_mut(name))
            .and_then(toml_edit::Item::as_table_like_mut)
            .ok_or_else(|| format!("no [tunables.{name}] to write"))?;
        match entry.get_mut("default") {
            Some(toml_edit::Item::Value(old)) => {
                let decor = old.decor().clone();
                *old = value.written();
                *old.decor_mut() = decor;
            }
            _ => {
                entry.insert("default", toml_edit::Item::Value(value.written()));
            }
        }
        Ok(doc.to_string())
    }
}
