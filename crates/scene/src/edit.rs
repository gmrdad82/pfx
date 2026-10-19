use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

use toml_edit::{Array, ArrayOfTables, DocumentMut, InlineTable, Item, Table, Value as Toml};

use crate::migrate::add_ids;
use crate::paths::slashed;
use crate::project::Project;
use crate::types::{FileKind, Material};
use crate::{Diagnostic, Id, Scene};

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Float(f32),
    Text(String),
    Array(Vec<Value>),
    Table(Vec<(String, Value)>),
}

impl From<bool> for Value {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<i64> for Value {
    fn from(value: i64) -> Self {
        Self::Int(value)
    }
}

impl From<i32> for Value {
    fn from(value: i32) -> Self {
        Self::Int(value.into())
    }
}

impl From<u32> for Value {
    fn from(value: u32) -> Self {
        Self::Int(value.into())
    }
}

impl From<f32> for Value {
    fn from(value: f32) -> Self {
        Self::Float(value)
    }
}

impl From<&str> for Value {
    fn from(value: &str) -> Self {
        Self::Text(value.to_string())
    }
}

impl From<String> for Value {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<Id> for Value {
    fn from(value: Id) -> Self {
        Self::Text(value.to_string())
    }
}

impl From<Vec<Value>> for Value {
    fn from(value: Vec<Value>) -> Self {
        Self::Array(value)
    }
}

impl<const N: usize> From<[f32; N]> for Value {
    fn from(value: [f32; N]) -> Self {
        Self::Array(value.into_iter().map(Self::Float).collect())
    }
}

fn float_text(value: f32) -> String {
    let mut text = format!("{value}");
    if !text.contains(['.', 'e', 'E']) {
        text.push_str(".0");
    }
    text
}

impl Value {
    fn toml(&self) -> Result<Toml, String> {
        Ok(match self {
            Value::Bool(value) => Toml::from(*value),
            Value::Int(value) => Toml::from(*value),
            Value::Float(value) => {
                if !value.is_finite() {
                    return Err("a number is not finite".to_string());
                }
                float_text(*value)
                    .parse::<Toml>()
                    .map_err(|error| error.to_string())?
            }
            Value::Text(value) => Toml::from(value.as_str()),
            Value::Array(items) => {
                let mut array = Array::new();
                for item in items {
                    array.push(item.toml()?);
                }
                Toml::Array(array)
            }
            Value::Table(fields) => {
                let mut table = InlineTable::new();
                for (key, value) in fields {
                    table.insert(key, value.toml()?);
                }
                Toml::InlineTable(table)
            }
        })
    }

    fn tables(&self) -> bool {
        match self {
            Value::Array(items) => {
                !items.is_empty() && items.iter().all(|item| matches!(item, Value::Table(_)))
            }
            _ => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Scene,
    Object(String),
    Mesh(String),
    Node { mesh: String, node: String },
    Material(String),
    Light(String),
    Emitter(String),
    Mover(String),
    Content(String),
    Text(String),
    Sound(String),
    Rig(String),
    Tiles(String),
    Particles(String),
    Sun,
    Sky,
    Haze,
    Camera,
    Finish,
    Flare,
    MotionBlur,
    Trace,
    Plates,
    Physics,
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Target::Scene => write!(f, "scene"),
            Target::Object(name) => write!(f, "object {name}"),
            Target::Mesh(name) => write!(f, "mesh {name}"),
            Target::Node { mesh, node } => write!(f, "mesh {mesh} node {node}"),
            Target::Material(name) => write!(f, "material {name}"),
            Target::Light(name) => write!(f, "light {name}"),
            Target::Emitter(name) => write!(f, "emitter {name}"),
            Target::Mover(name) => write!(f, "mover {name}"),
            Target::Content(name) => write!(f, "content {name}"),
            Target::Text(name) => write!(f, "text {name}"),
            Target::Sound(name) => write!(f, "sound {name}"),
            Target::Rig(name) => write!(f, "rig {name}"),
            Target::Tiles(name) => write!(f, "tile layer {name}"),
            Target::Particles(name) => write!(f, "particle system {name}"),
            Target::Sun => write!(f, "sun"),
            Target::Sky => write!(f, "sky"),
            Target::Haze => write!(f, "haze"),
            Target::Camera => write!(f, "camera"),
            Target::Finish => write!(f, "finish"),
            Target::Flare => write!(f, "flare"),
            Target::MotionBlur => write!(f, "motion blur"),
            Target::Trace => write!(f, "trace"),
            Target::Plates => write!(f, "plates"),
            Target::Physics => write!(f, "physics"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Object,
    Mesh,
    Material,
    Light,
    Emitter,
    Mover,
    Content,
    Text,
    Sound,
    Rig,
    Tiles,
    Particles,
}

const ORDER_OBJECT: &[&str] = &[
    "id",
    "name",
    "mesh",
    "prefab",
    "at",
    "rotate",
    "scale",
    "parent",
    "material",
    "materials",
    "shadow",
    "two_sided",
    "hidden",
    "clip",
    "content",
    "pick",
    "alpha_cutoff",
    "face_camera",
    "dynamic",
    "body",
    "character",
    "trigger",
    "animation",
    "set",
    "authoring",
];
const ORDER_LIGHT: &[&str] = &[
    "id",
    "name",
    "kind",
    "position",
    "parent",
    "node",
    "at",
    "direction",
    "cone",
    "color",
    "intensity",
    "radius",
    "range",
    "shadow",
    "keys",
    "authoring",
];
const ORDER_EMITTER: &[&str] = &[
    "id",
    "name",
    "position",
    "radius",
    "color",
    "intensity",
    "authoring",
];
const ORDER_MOVER: &[&str] = &[
    "id",
    "name",
    "objects",
    "kind",
    "pivot",
    "axis",
    "travel",
    "period",
    "motion",
    "offset",
    "clip",
    "authoring",
];
const ORDER_MESH: &[&str] = &["id", "file", "node", "nodes", "authoring"];
const ORDER_CONTENT: &[&str] = &["id", "image", "authoring"];
const ORDER_TEXT: &[&str] = &[
    "id",
    "text",
    "font",
    "size",
    "color",
    "at",
    "rotate",
    "lit",
    "dynamic",
    "authoring",
];
const ORDER_SOUND: &[&str] = &[
    "id",
    "file",
    "volume",
    "pan",
    "loop",
    "play",
    "object",
    "authoring",
];
const ORDER_TILES: &[&str] = &[
    "id",
    "name",
    "cell",
    "origin",
    "plane",
    "palette",
    "rows",
    "cells",
    "authoring",
];
const ORDER_PARTICLES: &[&str] = &[
    "id",
    "name",
    "parent",
    "node",
    "at",
    "rate",
    "rate_keys",
    "burst",
    "max",
    "lifetime",
    "speed",
    "direction",
    "cone",
    "inherit",
    "gravity",
    "drag",
    "size",
    "alpha",
    "color",
    "colors",
    "spin",
    "stretch",
    "blend",
    "lit",
    "seed",
    "prewarm",
    "keys",
    "authoring",
];
const ORDER_RIG: &[&str] = &[
    "id",
    "name",
    "kind",
    "target",
    "offset",
    "dead_zone",
    "look_ahead",
    "damping",
    "bounds",
    "orthographic",
    "snap",
    "sensitivity",
    "invert",
    "smoothing",
    "pitch",
    "head_bob",
    "distance",
    "collide",
    "authoring",
];
const ORDER_MATERIAL: &[&str] = &[
    "id",
    "family",
    "base",
    "roughness",
    "metalness",
    "specular",
    "clearcoat",
    "clearcoat_roughness",
    "sheen",
    "transmission",
    "ior",
    "dispersion",
    "thickness",
    "subsurface",
    "subsurface_tint",
    "absorption",
    "thin_film",
    "thin_film_ior",
    "thin_film_amount",
    "emission",
    "fresnel_power",
    "normal",
    "maps",
    "content",
    "content_layer",
    "layers",
    "ageing",
    "wet",
    "fabric",
    "anisotropy",
    "scatter",
    "thin_walled",
    "alpha",
    "alpha_mode",
    "alpha_cutoff",
    "flakes",
    "authoring",
];

impl Kind {
    fn target(self, name: &str) -> Target {
        let name = name.to_string();
        match self {
            Kind::Object => Target::Object(name),
            Kind::Mesh => Target::Mesh(name),
            Kind::Material => Target::Material(name),
            Kind::Light => Target::Light(name),
            Kind::Emitter => Target::Emitter(name),
            Kind::Mover => Target::Mover(name),
            Kind::Content => Target::Content(name),
            Kind::Text => Target::Text(name),
            Kind::Sound => Target::Sound(name),
            Kind::Rig => Target::Rig(name),
            Kind::Tiles => Target::Tiles(name),
            Kind::Particles => Target::Particles(name),
        }
    }

    fn list(self) -> Option<&'static str> {
        match self {
            Kind::Object => Some("object"),
            Kind::Light => Some("light"),
            Kind::Emitter => Some("emitter"),
            Kind::Mover => Some("mover"),
            Kind::Rig => Some("rig"),
            Kind::Tiles => Some("tiles"),
            Kind::Particles => Some("particles"),
            _ => None,
        }
    }

    fn map(self) -> Option<&'static str> {
        match self {
            Kind::Mesh => Some("mesh"),
            Kind::Material => Some("materials"),
            Kind::Content => Some("content"),
            Kind::Text => Some("text"),
            Kind::Sound => Some("sound"),
            _ => None,
        }
    }

    fn order(self) -> &'static [&'static str] {
        match self {
            Kind::Object => ORDER_OBJECT,
            Kind::Mesh => ORDER_MESH,
            Kind::Material => ORDER_MATERIAL,
            Kind::Light => ORDER_LIGHT,
            Kind::Emitter => ORDER_EMITTER,
            Kind::Mover => ORDER_MOVER,
            Kind::Content => ORDER_CONTENT,
            Kind::Text => ORDER_TEXT,
            Kind::Sound => ORDER_SOUND,
            Kind::Rig => ORDER_RIG,
            Kind::Tiles => ORDER_TILES,
            Kind::Particles => ORDER_PARTICLES,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EditError {
    pub file: PathBuf,
    pub key: String,
    pub line: Option<u32>,
    pub code: &'static str,
    pub message: String,
    pub diagnostics: Vec<Diagnostic>,
}

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(line) => write!(
                f,
                "{}:{line}: {}: {}",
                self.file.display(),
                self.key,
                self.message
            ),
            None => write!(f, "{}: {}: {}", self.file.display(), self.key, self.message),
        }
    }
}

impl std::error::Error for EditError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Patch {
    pub label: String,
    pub file: PathBuf,
    pub before: String,
    pub after: String,
}

impl Patch {
    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn file(&self) -> &Path {
        &self.file
    }

    pub fn inverse(&self) -> Self {
        Self {
            label: self.label.clone(),
            file: self.file.clone(),
            before: self.after.clone(),
            after: self.before.clone(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PatchGroup {
    pub label: String,
    pub patches: Vec<Patch>,
}

impl PatchGroup {
    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn files(&self) -> impl Iterator<Item = &Path> {
        self.patches.iter().map(Patch::file)
    }

    pub fn is_empty(&self) -> bool {
        self.patches.is_empty()
    }

    pub fn after(&self) -> BTreeMap<PathBuf, String> {
        self.patches
            .iter()
            .map(|patch| (patch.file.clone(), patch.after.clone()))
            .collect()
    }

    pub fn inverse(&self) -> Self {
        Self {
            label: self.label.clone(),
            patches: self.patches.iter().rev().map(Patch::inverse).collect(),
        }
    }

    pub fn write(&self) -> Result<(), EditError> {
        let fail = |file: &Path, message: String| EditError {
            file: file.to_path_buf(),
            key: self.label.clone(),
            line: None,
            code: crate::code::BAD_VALUE,
            message,
            diagnostics: Vec::new(),
        };
        let mut running: BTreeMap<&Path, (&str, &str)> = BTreeMap::new();
        for patch in &self.patches {
            let current = match running.get(patch.file.as_path()) {
                Some((_, after)) => Some(after.to_string()),
                None => crate::mount::current(&patch.file).ok(),
            };
            if current.as_deref() != Some(patch.before.as_str()) {
                return Err(fail(
                    &patch.file,
                    "no longer holds the text this edit starts from".to_string(),
                ));
            }
            let first = running
                .get(patch.file.as_path())
                .map_or(patch.before.as_str(), |(first, _)| *first);
            running.insert(&patch.file, (first, &patch.after));
        }
        crate::mount::store(&running, write)
            .map_err(|(file, error)| fail(&file, format!("cannot write: {error}")))
    }

    fn merge(&mut self, other: &PatchGroup) {
        if self.label.is_empty() {
            self.label = other.label.clone();
        }
        for patch in &other.patches {
            match self.patches.iter_mut().find(|own| own.file == patch.file) {
                Some(own) => own.after = patch.after.clone(),
                None => self.patches.push(patch.clone()),
            }
        }
    }

    fn settle(&mut self) {
        self.patches.retain(|patch| patch.before != patch.after);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    Scene,
    Library,
}

#[derive(Clone)]
struct File {
    text: String,
    doc: DocumentMut,
    role: Role,
    kind: FileKind,
}

#[derive(Clone)]
struct Tree {
    order: Vec<PathBuf>,
    files: BTreeMap<PathBuf, File>,
}

fn strings(doc: &DocumentMut, key: &str) -> Vec<String> {
    doc.get(key)
        .and_then(Item::as_array)
        .map(|array| {
            array
                .iter()
                .filter_map(|value| value.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn rooted_from(
    root: &Project,
    file: &Path,
    path: &str,
    overlay: &BTreeMap<PathBuf, String>,
) -> Option<PathBuf> {
    root.placed(file, path, &|near| overlay.contains_key(Path::new(near)))
        .ok()
        .map(PathBuf::from)
}

struct Refusal {
    file: PathBuf,
    code: &'static str,
    message: String,
}

impl Refusal {
    fn new(file: &Path, message: impl Into<String>) -> Self {
        Self {
            file: file.to_path_buf(),
            code: crate::code::BAD_VALUE,
            message: message.into(),
        }
    }

    fn nested(&self) -> bool {
        self.code == crate::code::INCLUDE_CYCLE || self.code == crate::code::INCLUDE_DEPTH
    }

    fn error(self, project: &Project, key: &str) -> EditError {
        EditError {
            file: project.path_of(&self.file),
            key: key.to_string(),
            line: None,
            code: self.code,
            message: self.message,
            diagnostics: Vec::new(),
        }
    }
}

struct Visit {
    path: PathBuf,
    file: File,
    includes: std::vec::IntoIter<String>,
}

impl Tree {
    fn gather(
        root: &Project,
        file: &Path,
        kind: FileKind,
        overlay: &BTreeMap<PathBuf, String>,
    ) -> Result<Self, Refusal> {
        let mut tree = Self {
            order: Vec::new(),
            files: BTreeMap::new(),
        };
        let mut chain = vec![Self::visit(root, file, Role::Scene, kind, overlay)?];
        while let Some(visit) = chain.last_mut() {
            let Some(include) = visit.includes.next() else {
                let Some(visit) = chain.pop() else { break };
                tree.settle(root, visit, kind, overlay)?;
                continue;
            };
            let Some(path) = rooted_from(root, &visit.path, &include, overlay) else {
                continue;
            };
            let names = |from: usize| -> String {
                let names: Vec<String> = chain[from..]
                    .iter()
                    .map(|visit| slashed(&visit.path))
                    .chain([slashed(&path)])
                    .collect();
                crate::project::trail(&names)
            };
            let last = &chain[chain.len() - 1].path;
            if let Some(start) = chain.iter().position(|visit| visit.path == path) {
                return Err(Refusal {
                    file: last.clone(),
                    code: crate::code::INCLUDE_CYCLE,
                    message: format!(
                        "the {} includes itself through {include}: {}",
                        kind.name(),
                        names(start)
                    ),
                });
            }
            if tree.files.contains_key(&path) {
                continue;
            }
            if chain.len() > crate::MAX_DEPTH {
                return Err(Refusal {
                    file: last.clone(),
                    code: crate::code::INCLUDE_DEPTH,
                    message: format!(
                        "including {include} nests the {}'s includes more than {} deep: {}",
                        kind.name(),
                        crate::MAX_DEPTH,
                        names(0)
                    ),
                });
            }
            let visit = Self::visit(root, &path, Role::Scene, kind, overlay)?;
            chain.push(visit);
        }
        Ok(tree)
    }

    fn visit(
        root: &Project,
        path: &Path,
        role: Role,
        kind: FileKind,
        overlay: &BTreeMap<PathBuf, String>,
    ) -> Result<Visit, Refusal> {
        let file = Self::read(root, path, role, kind, overlay)?;
        let includes = if role == Role::Scene {
            strings(&file.doc, "include")
        } else {
            Vec::new()
        };
        Ok(Visit {
            path: path.to_path_buf(),
            file,
            includes: includes.into_iter(),
        })
    }

    fn settle(
        &mut self,
        root: &Project,
        visit: Visit,
        kind: FileKind,
        overlay: &BTreeMap<PathBuf, String>,
    ) -> Result<(), Refusal> {
        let libraries = if visit.file.role == Role::Scene {
            strings(&visit.file.doc, "materials")
        } else {
            Vec::new()
        };
        self.order.push(visit.path.clone());
        let naming = visit.path.clone();
        self.files.insert(visit.path, visit.file);
        for library in libraries {
            let Some(library) = rooted_from(root, &naming, &library, overlay) else {
                continue;
            };
            if self.files.contains_key(&library) {
                continue;
            }
            let file = Self::read(root, &library, Role::Library, kind, overlay)?;
            self.order.push(library.clone());
            self.files.insert(library, file);
        }
        Ok(())
    }

    fn read(
        root: &Project,
        path: &Path,
        role: Role,
        kind: FileKind,
        overlay: &BTreeMap<PathBuf, String>,
    ) -> Result<File, Refusal> {
        let text = match overlay.get(path) {
            Some(text) => text.clone(),
            None => root.source(path).map_err(|error| {
                if error.kind() == std::io::ErrorKind::NotFound {
                    Refusal {
                        file: path.to_path_buf(),
                        code: crate::code::MISSING_FILE,
                        message: format!(
                            "the {} {} is not found",
                            kind.name(),
                            crate::paths::slashed(path)
                        ),
                    }
                } else {
                    Refusal::new(path, format!("cannot read {}: {error}", path.display()))
                }
            })?,
        };
        let doc: DocumentMut = text
            .parse()
            .map_err(|error: toml_edit::TomlError| Refusal::new(path, error.message().trim()))?;
        if doc.to_string() != text {
            return Err(Refusal::new(
                path,
                "cannot be written back byte for byte, so it is not edited",
            ));
        }
        if doc.get("format").and_then(Item::as_integer).unwrap_or(0) != i64::from(crate::FORMAT) {
            return Err(Refusal::new(
                path,
                "is not format 1; migrate it before editing it",
            ));
        }
        let kind = if role == Role::Library {
            FileKind::Materials
        } else {
            kind
        };
        Ok(File {
            text,
            doc,
            role,
            kind,
        })
    }
}

enum Container<'a> {
    Table(&'a mut Table),
    Inline(&'a mut InlineTable),
    Array(&'a mut Array),
    Aot(&'a mut ArrayOfTables),
}

fn enter_item(item: &mut Item) -> Option<Container<'_>> {
    match item {
        Item::Table(table) => Some(Container::Table(table)),
        Item::ArrayOfTables(array) => Some(Container::Aot(array)),
        Item::Value(value) => enter_value(value),
        Item::None => None,
    }
}

fn enter_value(value: &mut Toml) -> Option<Container<'_>> {
    match value {
        Toml::InlineTable(table) => Some(Container::Inline(table)),
        Toml::Array(array) => Some(Container::Array(array)),
        _ => None,
    }
}

impl<'a> Container<'a> {
    fn child(self, segment: &str, create: Option<&str>) -> Option<Container<'a>> {
        match self {
            Container::Table(table) => {
                if !table.contains_key(segment) && segment == "keys" {
                    create?;
                    table.insert(segment, Item::Value(Toml::InlineTable(InlineTable::new())));
                } else if !table.contains_key(segment) {
                    let prefix = create?;
                    let mut made = Table::new();
                    made.set_implicit(true);
                    made.decor_mut().set_prefix(prefix);
                    table.insert(segment, Item::Table(made));
                }
                enter_item(table.get_mut(segment)?)
            }
            Container::Inline(table) => {
                if !table.contains_key(segment) {
                    create?;
                    table.insert(segment, Toml::InlineTable(InlineTable::new()));
                }
                enter_value(table.get_mut(segment)?)
            }
            Container::Array(array) => enter_value(array.get_mut(segment.parse().ok()?)?),
            Container::Aot(array) => Some(Container::Table(array.get_mut(segment.parse().ok()?)?)),
        }
    }
}

fn walk<'a>(
    doc: &'a mut DocumentMut,
    segments: &[String],
    create: Option<&str>,
) -> Option<Container<'a>> {
    let mut at = Container::Table(doc.as_table_mut());
    for segment in segments {
        at = at.child(segment, create)?;
    }
    Some(at)
}

fn same(old: &Toml, new: &Value) -> bool {
    match (old, new) {
        (Toml::Boolean(a), Value::Bool(b)) => a.value() == b,
        (Toml::String(a), Value::Text(b)) => a.value() == b,
        (Toml::Integer(a), Value::Int(b)) => a.value() == b,
        (Toml::Integer(a), Value::Float(b)) => *a.value() as f64 == f64::from(*b),
        (Toml::Float(a), Value::Float(b)) => *a.value() as f32 == *b,
        (Toml::Float(a), Value::Int(b)) => *a.value() == *b as f64,
        (Toml::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| same(x, y))
        }
        (Toml::InlineTable(a), Value::Table(b)) => {
            a.len() == b.len()
                && b.iter()
                    .all(|(key, value)| a.get(key).is_some_and(|old| same(old, value)))
        }
        _ => false,
    }
}

fn merge(old: &mut Toml, new: &Value) -> Result<(), String> {
    if same(old, new) {
        return Ok(());
    }
    match (&mut *old, new) {
        (Toml::Array(a), Value::Array(b)) if a.len() == b.len() => {
            for (x, y) in a.iter_mut().zip(b) {
                merge(x, y)?;
            }
            Ok(())
        }
        (Toml::InlineTable(a), Value::Table(b)) => {
            let gone: Vec<String> = a
                .iter()
                .map(|(key, _)| key.to_string())
                .filter(|key| !b.iter().any(|(name, _)| name == key))
                .collect();
            for key in gone {
                a.remove(&key);
            }
            for (key, value) in b {
                match a.get_mut(key) {
                    Some(old) => merge(old, value)?,
                    None => {
                        a.insert(key, value.toml()?);
                    }
                }
            }
            Ok(())
        }
        _ => {
            let decor = old.decor().clone();
            let mut made = new.toml()?;
            *made.decor_mut() = decor;
            *old = made;
            Ok(())
        }
    }
}

fn table_of(value: &Value) -> Result<Table, String> {
    let Value::Table(fields) = value else {
        return Err("an entry of a table list is a table".to_string());
    };
    let mut table = Table::new();
    table.decor_mut().set_prefix("\n");
    for (key, value) in fields {
        table.insert(key, build(value)?);
    }
    Ok(table)
}

fn build(value: &Value) -> Result<Item, String> {
    if value.tables() {
        let Value::Array(items) = value else {
            return Err("not a list".to_string());
        };
        let mut array = ArrayOfTables::new();
        for item in items {
            array.push(table_of(item)?);
        }
        return Ok(Item::ArrayOfTables(array));
    }
    Ok(Item::Value(value.toml()?))
}

fn index(key: &str) -> Result<usize, String> {
    key.parse()
        .map_err(|_| format!("{key} is not a list place"))
}

fn put(at: Container<'_>, key: &str, value: &Value) -> Result<(), String> {
    match at {
        Container::Table(table) => {
            match table.get_mut(key) {
                Some(Item::Value(old)) => merge(old, value)?,
                Some(old) => *old = build(value)?,
                None => {
                    table.insert(key, build(value)?);
                }
            }
            Ok(())
        }
        Container::Inline(table) => {
            match table.get_mut(key) {
                Some(old) => merge(old, value)?,
                None => {
                    table.insert(key, value.toml()?);
                }
            }
            Ok(())
        }
        Container::Array(array) => {
            let place = index(key)?;
            if let Some(old) = array.get_mut(place) {
                merge(old, value)?;
                Ok(())
            } else if place == array.len() {
                array.push(value.toml()?);
                Ok(())
            } else {
                Err(format!("the list has {} entries, not {place}", array.len()))
            }
        }
        Container::Aot(array) => {
            let place = index(key)?;
            let table = table_of(value)?;
            if place < array.len() {
                array.replace(place, table);
                Ok(())
            } else if place == array.len() {
                array.push(table);
                Ok(())
            } else {
                Err(format!("the list has {} entries, not {place}", array.len()))
            }
        }
    }
}

fn append(at: Container<'_>, key: &str, value: &Value) -> Result<(), String> {
    let list = |entry: Value| Value::Array(vec![entry]);
    match at {
        Container::Table(table) => match table.get_mut(key) {
            Some(item) => push_item(item, value),
            None => {
                table.insert(key, build(&list(value.clone()))?);
                Ok(())
            }
        },
        Container::Inline(table) => match table.get_mut(key) {
            Some(Toml::Array(array)) => {
                array.push(value.toml()?);
                Ok(())
            }
            Some(_) => Err(format!("{key} is not a list")),
            None => {
                table.insert(key, list(value.clone()).toml()?);
                Ok(())
            }
        },
        _ => Err(format!("{key} is not in a table")),
    }
}

fn push_item(item: &mut Item, value: &Value) -> Result<(), String> {
    match item {
        Item::ArrayOfTables(array) => {
            array.push(table_of(value)?);
            Ok(())
        }
        Item::Value(Toml::Array(array)) => {
            array.push(value.toml()?);
            Ok(())
        }
        _ => Err("is not a list".to_string()),
    }
}

fn drop_key(at: Container<'_>, key: &str) -> Result<bool, String> {
    match at {
        Container::Table(table) => Ok(table.remove(key).is_some()),
        Container::Inline(table) => Ok(table.remove(key).is_some()),
        Container::Array(array) => {
            let place = index(key)?;
            if place < array.len() {
                array.remove(place);
                Ok(true)
            } else {
                Ok(false)
            }
        }
        Container::Aot(array) => {
            let place = index(key)?;
            if place < array.len() {
                array.remove(place);
                Ok(true)
            } else {
                Ok(false)
            }
        }
    }
}

fn named(doc: &DocumentMut, key: &str, reference: &str) -> Option<usize> {
    let array = doc.get(key)?.as_array_of_tables()?;
    let field =
        |table: &Table, field: &str| table.get(field).and_then(Item::as_str) == Some(reference);
    array
        .iter()
        .position(|table| field(table, "id"))
        .or_else(|| array.iter().position(|table| field(table, "name")))
}

fn keyed(doc: &DocumentMut, key: &str, reference: &str) -> Option<String> {
    let table = doc.get(key).and_then(Item::as_table_like)?;
    if table.contains_key(reference) {
        return Some(reference.to_string());
    }
    table
        .iter()
        .find(|(_, item)| {
            item.as_table_like()
                .and_then(|entry| entry.get("id"))
                .and_then(Item::as_str)
                == Some(reference)
        })
        .map(|(name, _)| name.to_string())
}

fn path_of(doc: &DocumentMut, target: &Target) -> Option<Vec<String>> {
    let section = |key: &str| doc.contains_key(key).then(|| vec![key.to_string()]);
    let list = |key: &str, name: &str| {
        named(doc, key, name).map(|place| vec![key.to_string(), place.to_string()])
    };
    let map = |key: &str, name: &str| keyed(doc, key, name).map(|name| vec![key.to_string(), name]);
    match target {
        Target::Scene => Some(Vec::new()),
        Target::Object(name) => list("object", name),
        Target::Light(name) => list("light", name),
        Target::Emitter(name) => list("emitter", name),
        Target::Mover(name) => list("mover", name),
        Target::Rig(name) => list("rig", name),
        Target::Tiles(name) => list("tiles", name),
        Target::Particles(name) => list("particles", name),
        Target::Mesh(name) => map("mesh", name),
        Target::Content(name) => map("content", name),
        Target::Text(name) => map("text", name),
        Target::Sound(name) => map("sound", name),
        Target::Material(name) => map("materials", name),
        Target::Node { mesh, node } => map("mesh", mesh).map(|mut path| {
            path.push("nodes".to_string());
            path.push(node.clone());
            path
        }),
        Target::Sun => section("sun"),
        Target::Sky => section("sky"),
        Target::Haze => section("haze"),
        Target::Camera => section("camera"),
        Target::Finish => section("finish"),
        Target::Flare => section("flare"),
        Target::MotionBlur => section("motion_blur"),
        Target::Trace => section("trace"),
        Target::Plates => section("plates"),
        Target::Physics => section("physics"),
    }
}

fn section_key(target: &Target) -> Option<&'static str> {
    match target {
        Target::Sun => Some("sun"),
        Target::Sky => Some("sky"),
        Target::Haze => Some("haze"),
        Target::Camera => Some("camera"),
        Target::Finish => Some("finish"),
        Target::Flare => Some("flare"),
        Target::MotionBlur => Some("motion_blur"),
        Target::Trace => Some("trace"),
        Target::Plates => Some("plates"),
        Target::Physics => Some("physics"),
        _ => None,
    }
}

struct Draft<'a> {
    tree: &'a Tree,
    root: &'a Path,
    work: BTreeMap<PathBuf, DocumentMut>,
}

impl<'a> Draft<'a> {
    fn new(tree: &'a Tree, root: &'a Path) -> Self {
        Self {
            tree,
            root,
            work: BTreeMap::new(),
        }
    }

    fn doc(&self, file: &Path) -> &DocumentMut {
        self.work
            .get(file)
            .unwrap_or_else(|| &self.tree.files[file].doc)
    }

    fn doc_mut(&mut self, file: &Path) -> &mut DocumentMut {
        let tree = self.tree;
        self.work
            .entry(file.to_path_buf())
            .or_insert_with(|| tree.files[file].doc.clone())
    }

    fn files(&self, role: Role) -> Vec<PathBuf> {
        self.tree
            .order
            .iter()
            .filter(|file| self.tree.files[*file].role == role)
            .cloned()
            .collect()
    }

    fn locate(
        &self,
        target: &Target,
        first: Option<&str>,
    ) -> Result<(PathBuf, Vec<String>), Refusal> {
        if let Some(name) = reference_of(target)
            && name.contains('/')
        {
            return Err(Refusal::new(
                self.root,
                format!(
                    "{target} is placed by a prefab; set it through its placement's [object.set]"
                ),
            ));
        }
        if *target == Target::Scene {
            if first == Some("fallback") {
                for file in self.files(Role::Scene) {
                    if self.doc(&file).contains_key("fallback") {
                        return Ok((file, Vec::new()));
                    }
                }
            }
            return Ok((self.root.to_path_buf(), Vec::new()));
        }
        let role = if matches!(target, Target::Material(_)) {
            Role::Library
        } else {
            Role::Scene
        };
        for file in self.files(role) {
            if let Some(path) = path_of(self.doc(&file), target) {
                return Ok((file, path));
            }
        }
        if let Some(key) = section_key(target) {
            return Ok((self.root.to_path_buf(), vec![key.to_string()]));
        }
        if let Target::Node { mesh, node } = target {
            let (file, mut path) = self.locate(&Target::Mesh(mesh.clone()), None)?;
            path.push("nodes".to_string());
            path.push(node.clone());
            return Ok((file, path));
        }
        Err(Refusal::new(
            self.root,
            format!("the scene has no {target}"),
        ))
    }

    fn at(&mut self, file: &Path, path: &[String], create: bool) -> Result<Container<'_>, Refusal> {
        let wanted = path.join(".");
        let made = if create { Some("\n") } else { None };
        walk(self.doc_mut(file), path, made)
            .ok_or_else(|| Refusal::new(file, format!("has no {wanted}")))
    }

    fn patches(self, label: &str, project: &Project, taken: &BTreeSet<Id>) -> Vec<Patch> {
        let mut taken = taken.clone();
        let mut patches = Vec::new();
        for (file, mut doc) in self.work {
            let before = self.tree.files[&file].text.clone();
            if doc.to_string() == before {
                continue;
            }
            add_ids(
                &mut doc,
                &slashed(&file),
                self.tree.files[&file].kind,
                &mut taken,
            );
            let after = doc.to_string();
            patches.push(Patch {
                label: label.to_string(),
                file: project.path_of(&file),
                before,
                after,
            });
        }
        patches
    }
}

fn reference_of(target: &Target) -> Option<&str> {
    match target {
        Target::Object(name)
        | Target::Mesh(name)
        | Target::Light(name)
        | Target::Emitter(name)
        | Target::Mover(name)
        | Target::Content(name)
        | Target::Text(name)
        | Target::Sound(name)
        | Target::Rig(name)
        | Target::Tiles(name)
        | Target::Particles(name) => Some(name),
        Target::Node { mesh, .. } => Some(mesh),
        _ => None,
    }
}

fn clear_positions(table: &mut Table) {
    table.set_position(None);
    for (_, item) in table.iter_mut() {
        match item {
            Item::Table(inner) => clear_positions(inner),
            Item::ArrayOfTables(array) => {
                for inner in array.iter_mut() {
                    clear_positions(inner);
                }
            }
            _ => {}
        }
    }
}

fn swap(value: &mut Toml, old: &str, new: &str) -> bool {
    if value.as_str() != Some(old) {
        return false;
    }
    let decor = value.decor().clone();
    let mut made = Toml::from(new);
    *made.decor_mut() = decor;
    *value = made;
    true
}

fn ordered(kind: Kind, fields: Vec<(String, Value)>) -> Vec<(String, Value)> {
    let order = kind.order();
    let mut known: Vec<(usize, (String, Value))> = Vec::new();
    let mut rest = Vec::new();
    for field in fields {
        match order.iter().position(|key| *key == field.0) {
            Some(place) => known.push((place, field)),
            None => rest.push(field),
        }
    }
    known.sort_by_key(|(place, _)| *place);
    known
        .into_iter()
        .map(|(_, field)| field)
        .chain(rest)
        .collect()
}

pub struct SceneEdit {
    root: PathBuf,
    file: PathBuf,
    kind: FileKind,
    project: Project,
    tree: Tree,
    scene: Scene,
    undo: Vec<PatchGroup>,
    redo: Vec<PatchGroup>,
    open: Option<PatchGroup>,
    dry: Option<PatchGroup>,
}

pub(crate) fn first_error(diagnostics: &[Diagnostic], root: &Path, key: &str) -> EditError {
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.is_error())
        .or_else(|| diagnostics.first());
    match diagnostic {
        Some(diagnostic) => EditError {
            file: root.join(&diagnostic.file),
            key: key.to_string(),
            line: Some(diagnostic.line),
            code: diagnostic.code,
            message: diagnostic.message.clone(),
            diagnostics: diagnostics.to_vec(),
        },
        None => EditError {
            file: root.to_path_buf(),
            key: key.to_string(),
            line: None,
            code: crate::code::BAD_VALUE,
            message: "the scene does not load".to_string(),
            diagnostics: Vec::new(),
        },
    }
}

fn gather(
    project: &Project,
    file: &Path,
    kind: FileKind,
    overlay: &BTreeMap<PathBuf, String>,
    key: &str,
) -> Result<Tree, EditError> {
    let root = project.root();
    Tree::gather(project, file, kind, overlay).map_err(|refusal| {
        if refusal.nested()
            && let Err(found) = project.scene_with(file, overlay.clone())
        {
            return first_error(&found, root, key);
        }
        refusal.error(project, key)
    })
}

impl SceneEdit {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, EditError> {
        let path = path.as_ref();
        let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        let root = Project::root_of(&absolute);
        let pfx = if pfx_file::is_file_path(&absolute) && absolute.is_file() {
            Some(absolute.clone())
        } else {
            crate::mount::pfx_of(&absolute).map(|(pfx, _)| pfx)
        };
        let opened = match &pfx {
            Some(pfx) => Project::open_pfx(pfx),
            None => Project::open(&root),
        };
        let project = opened.map_err(|diagnostic| EditError {
            file: absolute.clone(),
            key: String::new(),
            line: None,
            code: diagnostic.code,
            message: diagnostic.message.clone(),
            diagnostics: vec![diagnostic],
        })?;
        let root = project.root().to_path_buf();
        let file = project.relative(&absolute);
        let kind = match FileKind::of(&file) {
            Some(FileKind::Prefab) => FileKind::Prefab,
            _ => FileKind::Scene,
        };
        let tree = gather(&project, &file, kind, &BTreeMap::new(), "")?;
        let scene = project
            .scene(&file)
            .map_err(|diagnostics| first_error(&diagnostics, &root, ""))?;
        Ok(Self {
            root,
            file,
            kind,
            project,
            tree,
            scene,
            undo: Vec::new(),
            redo: Vec::new(),
            open: None,
            dry: None,
        })
    }

    pub fn path(&self) -> PathBuf {
        self.project.path_of(&self.file)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn scene(&self) -> &Scene {
        &self.scene
    }

    pub fn project(&self) -> &Project {
        &self.project
    }

    pub fn files(&self) -> impl Iterator<Item = PathBuf> + '_ {
        self.tree
            .order
            .iter()
            .map(|file| self.project.path_of(file))
    }

    pub fn text(&self, file: impl AsRef<Path>) -> Option<&str> {
        let file = self.project.relative(file.as_ref());
        self.tree.files.get(&file).map(|file| file.text.as_str())
    }

    pub fn reload(&mut self) -> Result<(), EditError> {
        if self.dry.is_some() {
            return Err(self.refuse("reload", "a dry run is open"));
        }
        let fresh = Self::open(self.path())?;
        *self = fresh;
        Ok(())
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(PatchGroup::label)
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(PatchGroup::label)
    }

    fn refuse(&self, key: &str, message: &str) -> EditError {
        EditError {
            file: self.path(),
            key: key.to_string(),
            line: None,
            code: crate::code::BAD_VALUE,
            message: message.to_string(),
            diagnostics: Vec::new(),
        }
    }

    pub fn begin(&mut self, label: &str) -> Result<(), EditError> {
        if self.dry.is_some() {
            return Err(self.refuse(label, "a dry run takes no group"));
        }
        if self.open.is_some() {
            return Err(self.refuse(label, "a group of edits is open already"));
        }
        self.open = Some(PatchGroup {
            label: label.to_string(),
            patches: Vec::new(),
        });
        Ok(())
    }

    pub fn end(&mut self) -> Option<PatchGroup> {
        let mut group = self.open.take()?;
        group.settle();
        if group.is_empty() {
            return None;
        }
        self.redo.clear();
        self.undo.push(group.clone());
        Some(group)
    }

    pub fn cancel(&mut self) -> Result<(), EditError> {
        let Some(mut group) = self.open.take() else {
            return Ok(());
        };
        group.settle();
        let inverse = group.inverse();
        let label = group.label.clone();
        if let Err(error) = self.land(&inverse.patches, &label) {
            self.open = Some(group);
            return Err(error);
        }
        Ok(())
    }

    pub fn apply(&mut self, group: &PatchGroup) -> Result<(), EditError> {
        if self.dry.is_some() {
            return Err(self.refuse(&group.label, "a dry run applies nothing"));
        }
        self.land(&group.patches, &group.label)
    }

    pub fn dry_run<T>(
        &mut self,
        edits: impl FnOnce(&mut Self) -> Result<T, EditError>,
    ) -> Result<PatchGroup, EditError> {
        if self.dry.is_some() || self.open.is_some() {
            return Err(self.refuse("dry run", "a group of edits is open"));
        }
        let project = self.project.clone();
        let tree = self.tree.clone();
        self.dry = Some(PatchGroup::default());
        let outcome = edits(self);
        let group = self.dry.take();
        self.project = project;
        self.tree = tree;
        outcome?;
        let mut group = group.unwrap_or_default();
        group.settle();
        if !group.is_empty() {
            let running = self.running(&group.patches);
            self.verify(&running, &group.label)?;
        }
        Ok(group)
    }

    pub fn undo(&mut self) -> Result<Option<PatchGroup>, EditError> {
        if self.open.is_some() || self.dry.is_some() {
            return Err(self.refuse("undo", "a group of edits is open"));
        }
        let Some(group) = self.undo.pop() else {
            return Ok(None);
        };
        let inverse = group.inverse();
        if let Err(error) = self.land(&inverse.patches, &inverse.label) {
            self.undo.push(group);
            return Err(error);
        }
        self.redo.push(group);
        Ok(Some(inverse))
    }

    pub fn redo(&mut self) -> Result<Option<PatchGroup>, EditError> {
        if self.open.is_some() || self.dry.is_some() {
            return Err(self.refuse("redo", "a group of edits is open"));
        }
        let Some(group) = self.redo.pop() else {
            return Ok(None);
        };
        if let Err(error) = self.land(&group.patches, &group.label) {
            self.redo.push(group);
            return Err(error);
        }
        self.undo.push(group.clone());
        Ok(Some(group))
    }

    fn land(&mut self, patches: &[Patch], key: &str) -> Result<(), EditError> {
        let fail = |file: &Path, line: Option<u32>, message: String| EditError {
            file: file.to_path_buf(),
            key: key.to_string(),
            line,
            code: crate::code::BAD_VALUE,
            message,
            diagnostics: Vec::new(),
        };
        let mut running: BTreeMap<PathBuf, String> = BTreeMap::new();
        for patch in patches {
            let relative = self.project.relative(&patch.file);
            let current = match running.get(&relative) {
                Some(text) => text.as_str(),
                None => match self.tree.files.get(&relative) {
                    Some(file) => file.text.as_str(),
                    None => {
                        return Err(fail(
                            &patch.file,
                            None,
                            "is not a file of this scene".to_string(),
                        ));
                    }
                },
            };
            if current != patch.before {
                return Err(fail(
                    &patch.file,
                    None,
                    "no longer holds the text this edit starts from".to_string(),
                ));
            }
            running.insert(relative, patch.after.clone());
        }
        let paths: BTreeMap<&PathBuf, PathBuf> = running
            .keys()
            .map(|file| (file, self.project.path_of(file)))
            .collect();
        for (file, path) in &paths {
            let held = self.tree.files.get(*file).map(|file| file.text.as_str());
            match crate::mount::current(path) {
                Ok(disk) if Some(disk.as_str()) == held => {}
                _ => {
                    return Err(fail(
                        path,
                        None,
                        "changed on disk since it was read; reload the scene".to_string(),
                    ));
                }
            }
        }
        let (scene, tree) = self.verify(&running, key)?;
        let changes: BTreeMap<&Path, (&str, &str)> = running
            .iter()
            .map(|(file, text)| {
                (
                    paths[file].as_path(),
                    (self.tree.files[file].text.as_str(), text.as_str()),
                )
            })
            .collect();
        if let Err((file, error)) = crate::mount::store(&changes, write) {
            return Err(fail(&file, None, format!("cannot write: {error}")));
        }
        for (file, text) in &running {
            self.project.update(file, text);
        }
        self.tree = tree;
        self.scene = scene;
        Ok(())
    }

    fn running(&self, patches: &[Patch]) -> BTreeMap<PathBuf, String> {
        patches
            .iter()
            .map(|patch| (self.project.relative(&patch.file), patch.after.clone()))
            .collect()
    }

    fn verify(
        &self,
        running: &BTreeMap<PathBuf, String>,
        key: &str,
    ) -> Result<(Scene, Tree), EditError> {
        let scene = self
            .project
            .scene_with(&self.file, running.clone())
            .map_err(|diagnostics| first_error(&diagnostics, &self.root, key))?;
        let tree = gather(&self.project, &self.file, self.kind, running, key)?;
        Ok((scene, tree))
    }

    fn hold(&mut self, group: &PatchGroup, key: &str) -> Result<(), EditError> {
        let running = self.running(&group.patches);
        let mut held: BTreeMap<PathBuf, String> = self
            .tree
            .files
            .iter()
            .map(|(file, held)| (file.clone(), held.text.clone()))
            .collect();
        held.extend(running.clone());
        self.tree = gather(&self.project, &self.file, self.kind, &held, key)?;
        for (file, text) in &running {
            self.project.update(file, text);
        }
        if let Some(dry) = &mut self.dry {
            dry.merge(group);
        }
        Ok(())
    }

    fn record(&mut self, group: &PatchGroup) {
        match &mut self.open {
            Some(open) => open.merge(group),
            None => {
                self.redo.clear();
                self.undo.push(group.clone());
            }
        }
    }

    fn run(
        &mut self,
        label: String,
        key: String,
        change: impl FnOnce(&mut Draft<'_>) -> Result<(), Refusal>,
    ) -> Result<PatchGroup, EditError> {
        let overlay: BTreeMap<PathBuf, String> = BTreeMap::new();
        let taken = self.project.ids(&overlay);
        let patches = {
            let mut draft = Draft::new(&self.tree, &self.file);
            change(&mut draft).map_err(|refusal| refusal.error(&self.project, &key))?;
            draft.patches(&label, &self.project, &taken)
        };
        let group = PatchGroup { label, patches };
        if group.is_empty() {
            return Ok(group);
        }
        if self.dry.is_some() {
            self.hold(&group, &key)?;
            return Ok(group);
        }
        self.land(&group.patches, &key)?;
        self.record(&group);
        Ok(group)
    }

    pub fn set(
        &mut self,
        target: &Target,
        path: &[&str],
        value: impl Into<Value>,
    ) -> Result<PatchGroup, EditError> {
        let value = value.into();
        let key = format!("{target} {}", path.join("."));
        let target = target.clone();
        let path: Vec<String> = path.iter().map(|segment| segment.to_string()).collect();
        self.run(format!("set {key}"), key, move |draft| {
            let Some((last, parents)) = path.split_last() else {
                return Err(Refusal::new(draft.root, "an edit names a key"));
            };
            let (file, mut base) = draft.locate(&target, path.first().map(String::as_str))?;
            base.extend(parents.iter().cloned());
            let at = draft.at(&file, &base, true)?;
            put(at, last, &value).map_err(|message| Refusal::new(&file, message))
        })
    }

    pub fn unset(&mut self, target: &Target, path: &[&str]) -> Result<PatchGroup, EditError> {
        let key = format!("{target} {}", path.join("."));
        let target = target.clone();
        let path: Vec<String> = path.iter().map(|segment| segment.to_string()).collect();
        self.run(format!("unset {key}"), key, move |draft| {
            let Some((last, parents)) = path.split_last() else {
                return Err(Refusal::new(draft.root, "an edit names a key"));
            };
            let (file, mut base) = draft.locate(&target, path.first().map(String::as_str))?;
            base.extend(parents.iter().cloned());
            let Ok(at) = draft.at(&file, &base, false) else {
                return Ok(());
            };
            drop_key(at, last)
                .map(|_| ())
                .map_err(|message| Refusal::new(&file, message))
        })
    }

    pub fn push(
        &mut self,
        target: &Target,
        path: &[&str],
        value: impl Into<Value>,
    ) -> Result<PatchGroup, EditError> {
        let value = value.into();
        let key = format!("{target} {}", path.join("."));
        let target = target.clone();
        let path: Vec<String> = path.iter().map(|segment| segment.to_string()).collect();
        self.run(format!("add to {key}"), key, move |draft| {
            let Some((last, parents)) = path.split_last() else {
                return Err(Refusal::new(draft.root, "an edit names a key"));
            };
            let (file, mut base) = draft.locate(&target, path.first().map(String::as_str))?;
            base.extend(parents.iter().cloned());
            let at = draft.at(&file, &base, true)?;
            append(at, last, &value).map_err(|message| Refusal::new(&file, message))
        })
    }

    pub fn remove(&mut self, target: &Target) -> Result<PatchGroup, EditError> {
        let key = target.to_string();
        let target = target.clone();
        self.run(format!("remove {key}"), key, move |draft| {
            let (file, path) = draft.locate(&target, None)?;
            let Some((last, parents)) = path.split_last() else {
                return Err(Refusal::new(&file, "the scene itself is not removed"));
            };
            let at = draft.at(&file, parents, false)?;
            drop_key(at, last).map_err(|message| Refusal::new(&file, message))?;
            if let [list] = parents {
                let doc = draft.doc_mut(&file);
                let empty = doc
                    .get(list)
                    .and_then(Item::as_array_of_tables)
                    .is_some_and(ArrayOfTables::is_empty);
                if empty {
                    doc.remove(list);
                }
            }
            Ok(())
        })
    }

    fn file_for(&self, file: Option<&Path>, role: Role) -> Result<PathBuf, EditError> {
        match file {
            Some(file) => {
                let relative = self.project.relative(file);
                let beside = Path::new(&crate::paths::folder(&self.file)).join(file);
                [relative, beside]
                    .into_iter()
                    .find(|path| {
                        self.tree
                            .files
                            .get(path)
                            .is_some_and(|held| held.role == role)
                    })
                    .ok_or_else(|| {
                        self.refuse(&file.display().to_string(), "is not a file of this scene")
                    })
            }
            None => match role {
                Role::Scene => Ok(self.file.clone()),
                Role::Library => self
                    .tree
                    .order
                    .iter()
                    .find(|path| self.tree.files[*path].role == Role::Library)
                    .cloned()
                    .ok_or_else(|| self.refuse("material", "the scene names no library")),
            },
        }
    }

    pub fn add(
        &mut self,
        kind: Kind,
        id: Id,
        name: &str,
        fields: &[(&str, Value)],
        file: Option<&Path>,
    ) -> Result<PatchGroup, EditError> {
        let key = kind.target(name).to_string();
        let role = if kind == Kind::Material {
            Role::Library
        } else {
            Role::Scene
        };
        let file = self.file_for(file, role)?;
        let taken = self.project.ids(&BTreeMap::new());
        if taken.contains(&id) {
            return Err(self.refuse(&key, &format!("id {id} is taken in this project")));
        }
        let name = name.to_string();
        let mut all: Vec<(String, Value)> = vec![("id".to_string(), Value::from(id))];
        if kind.list().is_some() {
            all.push(("name".to_string(), Value::from(name.as_str())));
        }
        all.extend(
            fields
                .iter()
                .filter(|(key, _)| *key != "id" && *key != "name")
                .map(|(key, value)| (key.to_string(), value.clone())),
        );
        let fields = ordered(kind, all);
        self.run(format!("add {key}"), key, move |draft| {
            let mut table = Table::new();
            table.decor_mut().set_prefix("\n");
            for (field, value) in &fields {
                let item = build(value).map_err(|message| Refusal::new(&file, message))?;
                table.insert(field, item);
            }
            let doc = draft.doc_mut(&file);
            if let Some(list) = kind.list() {
                if !doc.contains_key(list) {
                    doc.insert(list, Item::ArrayOfTables(ArrayOfTables::new()));
                }
                match doc.get_mut(list).and_then(Item::as_array_of_tables_mut) {
                    Some(array) => array.push(table),
                    None => return Err(Refusal::new(&file, format!("{list} is not a table list"))),
                }
            } else if let Some(map) = kind.map() {
                let Some(parents) = walk(doc, &[map.to_string()], Some("\n")) else {
                    return Err(Refusal::new(&file, format!("{map} is not a table")));
                };
                match parents {
                    Container::Table(parent) => {
                        if parent.contains_key(&name) {
                            return Err(Refusal::new(&file, format!("{name} exists already")));
                        }
                        parent.insert(&name, Item::Table(table));
                    }
                    _ => return Err(Refusal::new(&file, format!("{map} is not a table"))),
                }
            }
            Ok(())
        })
    }

    pub fn add_material(
        &mut self,
        name: &str,
        id: Id,
        material: &Material,
        file: Option<&Path>,
    ) -> Result<PatchGroup, EditError> {
        let key = format!("material {name}");
        let material = Material {
            id: Some(id),
            ..material.clone()
        };
        let full =
            toml::to_string(&material).map_err(|error| self.refuse(&key, &error.to_string()))?;
        let plain = toml::Table::try_from(Material::default()).unwrap_or_default();
        let mut doc: DocumentMut = full
            .parse()
            .map_err(|error: toml_edit::TomlError| self.refuse(&key, error.message()))?;
        let keep: BTreeSet<String> = material.to_table().keys().cloned().collect();
        let drop: Vec<String> = doc
            .as_table()
            .iter()
            .map(|(key, _)| key.to_string())
            .filter(|key| !keep.contains(key) && plain.contains_key(key))
            .collect();
        for key in drop {
            doc.remove(&key);
        }
        let taken = self.project.ids(&BTreeMap::new());
        if taken.contains(&id) {
            return Err(self.refuse(&key, &format!("id {id} is taken in this project")));
        }
        let file = self.file_for(file, Role::Library)?;
        let name = name.to_string();
        self.run(format!("add {key}"), key, move |draft| {
            let mut table = doc.as_table().clone();
            clear_positions(&mut table);
            table.decor_mut().set_prefix("\n");
            table.set_implicit(false);
            let target = draft.doc_mut(&file);
            let Some(Container::Table(parent)) =
                walk(target, &["materials".to_string()], Some("\n"))
            else {
                return Err(Refusal::new(&file, "materials is not a table"));
            };
            if parent.contains_key(&name) {
                return Err(Refusal::new(&file, format!("{name} exists already")));
            }
            parent.insert(&name, Item::Table(table));
            Ok(())
        })
    }

    pub fn duplicate_object(
        &mut self,
        name: &str,
        new: &str,
        id: Id,
    ) -> Result<PatchGroup, EditError> {
        let key = format!("object {name}");
        let taken = self.project.ids(&BTreeMap::new());
        if taken.contains(&id) {
            return Err(self.refuse(&key, &format!("id {id} is taken in this project")));
        }
        let (name, new) = (name.to_string(), new.to_string());
        self.run(format!("duplicate {key} as {new}"), key, move |draft| {
            let (file, path) = draft.locate(&Target::Object(name.clone()), None)?;
            let place: usize = path[1].parse().unwrap_or(0);
            let doc = draft.doc_mut(&file);
            let Some(array) = doc.get_mut("object").and_then(Item::as_array_of_tables_mut) else {
                return Err(Refusal::new(&file, "object is not a table list"));
            };
            let Some(original) = array.get(place) else {
                return Err(Refusal::new(
                    &file,
                    format!("the scene has no object {name}"),
                ));
            };
            let mut copy = original.clone();
            clear_positions(&mut copy);
            copy.decor_mut().set_prefix("\n");
            copy.insert("name", Item::Value(Toml::from(new.as_str())));
            if let Some(Item::Value(value)) = copy.get_mut("name") {
                *value.decor_mut() = toml_edit::Decor::default();
            }
            match copy.get_mut("id") {
                Some(Item::Value(value)) => {
                    let decor = value.decor().clone();
                    let mut made = Toml::from(id.to_string());
                    *made.decor_mut() = decor;
                    *value = made;
                }
                _ => {
                    copy.insert("id", toml_edit::value(id.to_string()));
                }
            }
            copy.remove("pick");
            array.insert(place + 1, copy);
            Ok(())
        })
    }

    pub fn rename_object(&mut self, name: &str, new: &str) -> Result<PatchGroup, EditError> {
        let key = format!("object {name}");
        let (name, new) = (name.to_string(), new.to_string());
        self.run(format!("rename {key} to {new}"), key, move |draft| {
            let mut found = false;
            for file in draft.files(Role::Scene) {
                let doc = draft.doc_mut(&file);
                if let Some(array) = doc.get_mut("object").and_then(Item::as_array_of_tables_mut) {
                    for table in array.iter_mut() {
                        for field in ["name", "parent"] {
                            if let Some(Item::Value(value)) = table.get_mut(field)
                                && swap(value, &name, &new)
                                && field == "name"
                            {
                                found = true;
                            }
                        }
                    }
                }
                if let Some(array) = doc.get_mut("mover").and_then(Item::as_array_of_tables_mut) {
                    for table in array.iter_mut() {
                        if let Some(Item::Value(Toml::Array(objects))) = table.get_mut("objects") {
                            for value in objects.iter_mut() {
                                swap(value, &name, &new);
                            }
                        }
                    }
                }
                if let Some(array) = doc.get_mut("rig").and_then(Item::as_array_of_tables_mut) {
                    for table in array.iter_mut() {
                        if let Some(Item::Value(value)) = table.get_mut("target") {
                            swap(value, &name, &new);
                        }
                    }
                }
                if let Some(sounds) = doc.get_mut("sound").and_then(Item::as_table_like_mut) {
                    for (_, sound) in sounds.iter_mut() {
                        if let Some(Item::Value(value)) = sound
                            .as_table_like_mut()
                            .and_then(|sound| sound.get_mut("object"))
                        {
                            swap(value, &name, &new);
                        }
                    }
                }
            }
            if found {
                Ok(())
            } else {
                Err(Refusal::new(
                    draft.root,
                    format!("the scene has no object {name}"),
                ))
            }
        })
    }

    pub fn set_at(&mut self, object: &str, at: [f32; 3]) -> Result<PatchGroup, EditError> {
        self.set(&Target::Object(object.to_string()), &["at"], at)
    }

    pub fn set_rotate(&mut self, object: &str, rotate: [f32; 3]) -> Result<PatchGroup, EditError> {
        self.set(&Target::Object(object.to_string()), &["rotate"], rotate)
    }

    pub fn set_scale(
        &mut self,
        object: &str,
        scale: impl Into<Value>,
    ) -> Result<PatchGroup, EditError> {
        self.set(&Target::Object(object.to_string()), &["scale"], scale)
    }

    pub fn set_object_material(
        &mut self,
        object: &str,
        material: Option<&str>,
    ) -> Result<PatchGroup, EditError> {
        let target = Target::Object(object.to_string());
        match material {
            Some(material) => self.set(&target, &["material"], material),
            None => self.unset(&target, &["material"]),
        }
    }

    pub fn set_object_materials(
        &mut self,
        object: &str,
        materials: &[(&str, &str)],
    ) -> Result<PatchGroup, EditError> {
        let target = Target::Object(object.to_string());
        if materials.is_empty() {
            return self.unset(&target, &["materials"]);
        }
        let table = materials
            .iter()
            .map(|(node, material)| (node.to_string(), Value::from(*material)))
            .collect();
        self.set(&target, &["materials"], Value::Table(table))
    }

    pub fn set_shadow(&mut self, object: &str, shadow: &str) -> Result<PatchGroup, EditError> {
        self.set(&Target::Object(object.to_string()), &["shadow"], shadow)
    }

    pub fn set_two_sided(&mut self, object: &str, on: bool) -> Result<PatchGroup, EditError> {
        self.set(&Target::Object(object.to_string()), &["two_sided"], on)
    }

    pub fn set_hidden(&mut self, object: &str, on: bool) -> Result<PatchGroup, EditError> {
        self.set(&Target::Object(object.to_string()), &["hidden"], on)
    }

    pub fn set_clip(&mut self, object: &str, planes: &[[f32; 4]]) -> Result<PatchGroup, EditError> {
        let target = Target::Object(object.to_string());
        if planes.is_empty() {
            return self.unset(&target, &["clip"]);
        }
        let planes: Vec<Value> = planes.iter().map(|plane| Value::from(*plane)).collect();
        self.set(&target, &["clip"], Value::Array(planes))
    }

    pub fn set_parent(
        &mut self,
        object: &str,
        parent: Option<&str>,
    ) -> Result<PatchGroup, EditError> {
        let target = Target::Object(object.to_string());
        match parent {
            Some(parent) => self.set(&target, &["parent"], parent),
            None => self.unset(&target, &["parent"]),
        }
    }

    pub fn add_mesh(
        &mut self,
        id: Id,
        name: &str,
        file: &str,
        node: Option<&str>,
    ) -> Result<PatchGroup, EditError> {
        let mut fields = vec![("file", Value::from(file))];
        if let Some(node) = node {
            fields.push(("node", Value::from(node)));
        }
        self.add(Kind::Mesh, id, name, &fields, None)
    }

    pub fn set_node(
        &mut self,
        mesh: &str,
        node: &str,
        key: &str,
        value: impl Into<Value>,
    ) -> Result<PatchGroup, EditError> {
        let target = Target::Node {
            mesh: mesh.to_string(),
            node: node.to_string(),
        };
        self.set(&target, &[key], value)
    }

    pub fn set_material(
        &mut self,
        name: &str,
        path: &[&str],
        value: impl Into<Value>,
    ) -> Result<PatchGroup, EditError> {
        self.set(&Target::Material(name.to_string()), path, value)
    }
}

fn write(file: &Path, text: &str) -> std::io::Result<()> {
    let name = file
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let temporary = file.with_file_name(format!(".{name}.edit"));
    std::fs::write(&temporary, text)?;
    std::fs::rename(&temporary, file).inspect_err(|_| {
        let _ = std::fs::remove_file(&temporary);
    })
}
