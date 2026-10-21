use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use toml_edit::DocumentMut;

use crate::code;
use crate::edit::first_error;
use crate::migrate::{add_ids, entry_kinds};
use crate::mount::{GLTF_PATH, Mount};
use crate::paths::{Found, Listing, PathError, folder, join, rooted, slashed};
use crate::read::{self, Context, Doc, Finding, Parsed};
use crate::rules::{self, Issues};
use crate::scene::{Entry, Scene};
use crate::spans::{KeyPath, Part};
use crate::types::{
    Camera, Color, Content, Emitter, File, FileKind, Finish, FinishKeys, Flare, Haze, Light,
    LiveSettings, LutKey, Material, Mesh, MotionBlur, Mover, Object, Particles, Physics, Plates,
    Play, Proxy, Recolor, Rig, Shadows, Sky, Sound, Sun, Text, Tiles, Trace,
};
use crate::{Diagnostic, EditError, FORMAT, Id, Patch, PatchGroup, Severity};

type Layers = BTreeMap<String, Vec<String>>;

pub const MAX_DEPTH: usize = 64;

#[derive(Clone, Debug)]
pub struct Project {
    root: PathBuf,
    marked: bool,
    texts: BTreeMap<PathBuf, Result<String, String>>,
    pfx_files: Vec<PathBuf>,
    mount: Option<Mount>,
}

fn key(parts: &[&str]) -> KeyPath {
    parts.iter().map(|part| part.to_string()).collect()
}

fn under(path: &KeyPath, rest: &[String]) -> KeyPath {
    let mut out = path.clone();
    out.extend(rest.iter().cloned());
    out
}

const SKIPPED: [&str; 2] = ["target", "tmp"];

fn skipped(root: &Path) -> BTreeSet<PathBuf> {
    let listed = std::fs::read_to_string(root.join("project.toml"))
        .ok()
        .and_then(|text| toml::from_str::<toml::Table>(&text).ok())
        .and_then(|table| table.get("project")?.get("ignore")?.as_array().cloned())
        .unwrap_or_default();
    SKIPPED
        .iter()
        .map(|name| name.to_string())
        .chain(
            listed
                .iter()
                .filter_map(toml::Value::as_str)
                .filter_map(|raw| rooted(raw).ok()),
        )
        .map(PathBuf::from)
        .collect()
}

fn discover(
    root: &Path,
    folder: &Path,
    skip: &BTreeSet<PathBuf>,
    out: &mut Vec<PathBuf>,
    pfx_files: &mut Vec<PathBuf>,
) {
    let Ok(entries) = std::fs::read_dir(root.join(folder)) else {
        return;
    };
    let mut entries: Vec<_> = entries.filter_map(|entry| entry.ok()).collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.starts_with('.') {
            continue;
        }
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let relative = folder.join(name);
        if skip.contains(&relative) {
            continue;
        }
        if kind.is_dir() {
            if root.join(&relative).join("project.toml").is_file() {
                continue;
            }
            discover(root, &relative, skip, out, pfx_files);
        } else if kind.is_file() && FileKind::of(&relative).is_some() {
            out.push(relative);
        } else if kind.is_file()
            && pfx_file::is_file_path(&relative)
            && !holds_a_recipe(&root.join(&relative))
        {
            pfx_files.push(relative);
        }
    }
}

fn holds_a_recipe(file: &Path) -> bool {
    pfx_file::Reader::open(file).is_ok_and(|reader| {
        reader.find(pfx_file::RCPE).is_some() && reader.find(pfx_file::SMET).is_none()
    })
}

impl Project {
    pub fn root_of(file: &Path) -> PathBuf {
        let start = file.parent().unwrap_or_else(|| Path::new(""));
        let mut at = Some(start);
        while let Some(folder) = at {
            if folder.join("project.toml").is_file() {
                return folder.to_path_buf();
            }
            at = folder.parent();
        }
        start.to_path_buf()
    }

    pub fn open(root: &Path) -> Result<Project, Diagnostic> {
        let root = std::path::absolute(root).unwrap_or_else(|_| root.to_path_buf());
        if !root.is_dir() {
            return Err(Diagnostic {
                file: PathBuf::new(),
                line: 1,
                column: 1,
                end_line: 1,
                end_column: 1,
                severity: Severity::Error,
                code: code::UNREADABLE,
                key: String::new(),
                message: format!("{} is not a folder", root.display()),
                related: Vec::new(),
            });
        }
        let mut files = Vec::new();
        let mut pfx_files = Vec::new();
        discover(
            &root,
            Path::new(""),
            &skipped(&root),
            &mut files,
            &mut pfx_files,
        );
        let texts = files
            .into_iter()
            .map(|file| {
                let text = std::fs::read(root.join(&file))
                    .map_err(|error| format!("cannot read {}: {error}", file.display()))
                    .and_then(|bytes| {
                        String::from_utf8(bytes).map_err(|_| "the file is not UTF-8".to_string())
                    });
                (file, text)
            })
            .collect();
        Ok(Project {
            marked: root.join("project.toml").is_file(),
            root,
            texts,
            pfx_files,
            mount: None,
        })
    }

    pub fn open_pfx(file: &Path) -> Result<Project, Diagnostic> {
        let absolute = std::path::absolute(file).unwrap_or_else(|_| file.to_path_buf());
        let root = Self::root_of(&absolute);
        let relative = absolute
            .strip_prefix(&root)
            .map(|inside| PathBuf::from(slashed(inside)))
            .unwrap_or_else(|_| absolute.clone());
        let mount = Mount::read(&root, &relative).map_err(|message| Diagnostic {
            file: relative.clone(),
            line: 1,
            column: 1,
            end_line: 1,
            end_column: 1,
            severity: Severity::Error,
            code: code::UNREADABLE,
            key: String::new(),
            message: format!("{} {message}", relative.display()),
            related: Vec::new(),
        })?;
        let texts = mount
            .texts
            .iter()
            .map(|(file, text)| (file.clone(), Ok(text.clone())))
            .collect();
        Ok(Project {
            marked: root.join("project.toml").is_file(),
            root,
            texts,
            pfx_files: Vec::new(),
            mount: Some(mount),
        })
    }

    pub fn from_pfx_bytes(
        name: impl Into<PathBuf>,
        bytes: impl Into<std::sync::Arc<[u8]>>,
    ) -> Result<Project, Diagnostic> {
        let file = PathBuf::from(slashed(&name.into()));
        let mount =
            Mount::from_bytes(file.clone(), bytes.into(), true).map_err(|message| Diagnostic {
                file: file.clone(),
                line: 1,
                column: 1,
                end_line: 1,
                end_column: 1,
                severity: Severity::Error,
                code: code::UNREADABLE,
                key: String::new(),
                message: format!("{} {message}", file.display()),
                related: Vec::new(),
            })?;
        let texts = mount
            .texts
            .iter()
            .map(|(file, text)| (file.clone(), Ok(text.clone())))
            .collect();
        Ok(Project {
            marked: false,
            root: PathBuf::new(),
            texts,
            pfx_files: Vec::new(),
            mount: Some(mount),
        })
    }

    pub fn in_memory(&self) -> bool {
        self.mount.as_ref().is_some_and(|mount| mount.memory)
    }

    pub fn holds(&self, path: &str) -> bool {
        if let Some(mount) = &self.mount {
            if mount.holds(path) || self.texts.contains_key(Path::new(path)) {
                return true;
            }
            if mount.memory {
                return false;
            }
        }
        self.root.join(path).is_file()
    }

    pub fn locate(&self, file: &Path, raw: &str) -> Result<String, String> {
        self.placed(file, raw, &|_| false)
            .map_err(|error| error.message(raw))
    }

    pub(crate) fn placed(
        &self,
        file: &Path,
        raw: &str,
        held: &dyn Fn(&str) -> bool,
    ) -> Result<String, PathError> {
        let from_root = rooted(raw);
        let base = folder(file);
        if base.is_empty() {
            return from_root;
        }
        match (join(&base, raw), &from_root) {
            (Ok(near), Ok(far)) if near == *far => from_root,
            (Ok(near), Ok(_)) if held(&near) || self.holds(&near) => Ok(near),
            (Ok(near), Err(_)) => Ok(near),
            _ => from_root,
        }
    }

    pub fn pfx_bytes(&self) -> Option<&[u8]> {
        self.mount.as_ref().map(Mount::bytes)
    }

    pub fn pfx_path(&self) -> Option<&Path> {
        self.mount.as_ref().map(|mount| mount.file.as_path())
    }

    pub fn pfx(&self) -> Option<pfx_file::Reader<'_>> {
        self.mount.as_ref().and_then(Mount::reader)
    }

    pub fn entry(&self) -> Option<&Path> {
        self.mount.as_ref().map(|mount| mount.entry.as_path())
    }

    pub fn path_of(&self, file: &Path) -> PathBuf {
        self.root.join(self.shown(file))
    }

    pub fn shown(&self, file: &Path) -> PathBuf {
        match &self.mount {
            Some(mount) if mount.texts.contains_key(file) => mount.file.join(file),
            _ => file.to_path_buf(),
        }
    }

    pub fn asset(&self, path: &str) -> std::io::Result<Cow<'_, [u8]>> {
        let refused =
            |message: String| std::io::Error::new(std::io::ErrorKind::InvalidInput, message);
        let rooted = rooted(path).map_err(|error| refused(error.message(path)))?;
        if let Some(mount) = &self.mount {
            if let Some(found) = mount.file_bytes(&rooted) {
                return found;
            }
            if let Some(text) = mount.texts.get(Path::new(&rooted)) {
                return Ok(Cow::Owned(text.clone().into_bytes()));
            }
            if mount.memory {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("{rooted} is not in {}", mount.file.display()),
                ));
            }
        }
        let target = self.root.join(&rooted);
        let inside =
            std::fs::canonicalize(&target)?.starts_with(std::fs::canonicalize(&self.root)?);
        if !inside {
            return Err(refused(format!("path {path} leaves the project root")));
        }
        std::fs::read(target).map(Cow::Owned)
    }

    pub(crate) fn source(&self, file: &Path) -> std::io::Result<String> {
        match self.texts.get(file) {
            Some(Ok(text)) => Ok(text.clone()),
            Some(Err(message)) if self.in_memory() => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                message.clone(),
            )),
            None if self.in_memory() => Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("{} is not in the .pitofx", file.display()),
            )),
            _ => std::fs::read_to_string(self.root.join(file)),
        }
    }

    fn shown_all(&self, mut diagnostics: Vec<Diagnostic>) -> Vec<Diagnostic> {
        if self.mount.is_some() {
            for diagnostic in &mut diagnostics {
                diagnostic.file = self.shown(&diagnostic.file);
                for related in &mut diagnostic.related {
                    related.file = self.shown(&related.file);
                }
            }
        }
        diagnostics
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn has_project_file(&self) -> bool {
        self.marked
    }

    pub fn files(&self) -> impl Iterator<Item = &Path> {
        self.texts.keys().map(PathBuf::as_path)
    }

    pub fn relative(&self, file: &Path) -> PathBuf {
        let relative = if file.is_absolute() {
            match file.strip_prefix(&self.root) {
                Ok(inside) => inside.to_path_buf(),
                Err(_) => return file.to_path_buf(),
            }
        } else {
            PathBuf::from(slashed(file))
        };
        let Some(mount) = &self.mount else {
            return relative;
        };
        if relative == mount.file {
            return mount.entry.clone();
        }
        match relative.strip_prefix(&mount.file) {
            Ok(inner) => PathBuf::from(slashed(inner)),
            Err(_) => relative,
        }
    }

    pub fn check(&self) -> Vec<Diagnostic> {
        let mut diagnostics = self.shown_all(Checker::new(self, BTreeMap::new()).all());
        for pfx in &self.pfx_files {
            match Project::open_pfx(&self.root.join(pfx)) {
                Ok(project) => diagnostics.extend(project.check()),
                Err(diagnostic) => diagnostics.push(diagnostic),
            }
        }
        diagnostics
    }

    pub fn scene(&self, file: &Path) -> Result<Scene, Vec<Diagnostic>> {
        self.scene_with(file, BTreeMap::new())
    }

    pub fn scene_with(
        &self,
        file: &Path,
        overlay: BTreeMap<PathBuf, String>,
    ) -> Result<Scene, Vec<Diagnostic>> {
        match self.resolve(file, overlay).0 {
            Ok(mut scene) => {
                scene.warnings = self.shown_all(scene.warnings);
                Ok(scene)
            }
            Err(found) => Err(self.shown_all(found)),
        }
    }

    pub fn check_with(&self, overlay: BTreeMap<PathBuf, String>) -> Vec<Diagnostic> {
        self.shown_all(Checker::new(self, self.overlaid(overlay)).all())
    }

    pub(crate) fn closure(
        &self,
        file: &Path,
    ) -> Result<(Scene, BTreeSet<PathBuf>), Vec<Diagnostic>> {
        let file = self.relative(file);
        let kind = FileKind::of(&file).unwrap_or(FileKind::Scene);
        let mut checker = Checker::new(self, BTreeMap::new());
        let scene = checker.context(&file, kind);
        checker.unique_ids();
        let diagnostics = checker.report();
        match scene {
            Some(scene) if !diagnostics.iter().any(Diagnostic::is_error) => {
                Ok((scene, checker.found))
            }
            _ => Err(self.shown_all(diagnostics)),
        }
    }

    fn overlaid(&self, overlay: BTreeMap<PathBuf, String>) -> BTreeMap<PathBuf, String> {
        overlay
            .into_iter()
            .map(|(file, text)| (self.relative(&file), text))
            .collect()
    }

    #[allow(clippy::type_complexity)]
    fn resolve(
        &self,
        file: &Path,
        overlay: BTreeMap<PathBuf, String>,
    ) -> (Result<Scene, Vec<Diagnostic>>, BTreeMap<PathBuf, FileKind>) {
        let file = self.relative(file);
        let kind = FileKind::of(&file).unwrap_or(FileKind::Scene);
        let mut checker = Checker::new(self, self.overlaid(overlay));
        let scene = checker.context(&file, kind);
        checker.unique_ids();
        let diagnostics = checker.report();
        let errors: Vec<Diagnostic> = diagnostics
            .iter()
            .filter(|d| d.is_error())
            .cloned()
            .collect();
        let scene = match scene {
            Some(mut scene) if errors.is_empty() => {
                scene.warnings = diagnostics;
                Ok(scene)
            }
            _ => Err(if errors.is_empty() {
                diagnostics
            } else {
                errors
            }),
        };
        (scene, checker.kinds)
    }

    pub fn fix(&self, file: &Path) -> Result<PatchGroup, EditError> {
        let label = "fix ids";
        let refuse = |file: &Path, message: &str| EditError {
            file: self.root.join(file),
            key: label.to_string(),
            line: None,
            code: code::BAD_VALUE,
            message: message.to_string(),
            diagnostics: Vec::new(),
        };
        let (scene, kinds) = self.resolve(file, BTreeMap::new());
        let scene =
            scene.map_err(|found| first_error(&self.shown_all(found), &self.root, label))?;
        let mut taken = self.ids(&BTreeMap::new());
        let mut patches = Vec::new();
        for path in &scene.files {
            let Some(&kind) = kinds.get(path) else {
                continue;
            };
            let (lists, maps) = entry_kinds(kind);
            if lists.is_empty() && maps.is_empty() {
                continue;
            }
            let text = match self.texts.get(path) {
                Some(Ok(text)) => text.clone(),
                _ => std::fs::read_to_string(self.root.join(path))
                    .map_err(|error| refuse(path, &format!("cannot read: {error}")))?,
            };
            let mut doc: DocumentMut = text
                .parse()
                .map_err(|error: toml_edit::TomlError| refuse(path, error.message().trim()))?;
            if doc.get("format").and_then(toml_edit::Item::as_integer) != Some(FORMAT.into()) {
                return Err(refuse(
                    path,
                    "is not format 1; migrate it, which writes its ids",
                ));
            }
            let exact = doc.to_string() == text;
            if add_ids(&mut doc, &slashed(path), kind, &mut taken).is_empty() {
                continue;
            }
            if !exact {
                return Err(refuse(
                    path,
                    "cannot be written back byte for byte, so it is not edited",
                ));
            }
            patches.push(Patch {
                label: label.to_string(),
                file: self.path_of(path),
                before: text,
                after: doc.to_string(),
            });
        }
        let group = PatchGroup {
            label: label.to_string(),
            patches,
        };
        if !group.is_empty() {
            self.scene_with(file, group.after())
                .map_err(|found| first_error(&found, &self.root, label))?;
        }
        Ok(group)
    }

    pub(crate) fn kind_of(
        &self,
        file: &Path,
        overlay: &BTreeMap<PathBuf, String>,
    ) -> Option<FileKind> {
        let file = self.relative(file);
        if let Some(kind) = FileKind::of(&file) {
            return Some(kind);
        }
        let mut checker = Checker::new(self, overlay.clone());
        checker.all();
        checker.kinds.get(&file).copied()
    }

    pub(crate) fn update(&mut self, file: &Path, text: &str) {
        self.texts.insert(self.relative(file), Ok(text.to_string()));
    }

    pub(crate) fn ids(&self, overlay: &BTreeMap<PathBuf, String>) -> BTreeSet<Id> {
        let mut checker = Checker::new(self, overlay.clone());
        checker.all();
        let mut ids = BTreeSet::new();
        for parsed in checker.parsed.values() {
            let Some(doc) = &parsed.doc else { continue };
            ids.extend(doc.mesh.iter().filter_map(|(_, v)| v.id));
            ids.extend(doc.object.iter().filter_map(|(_, v)| v.id));
            ids.extend(doc.light.iter().filter_map(|(_, v)| v.id));
            ids.extend(doc.emitter.iter().filter_map(|(_, v)| v.id));
            ids.extend(doc.mover.iter().filter_map(|(_, v)| v.id));
            ids.extend(doc.content.iter().filter_map(|(_, v)| v.id));
            ids.extend(doc.text.iter().filter_map(|(_, v)| v.id));
            ids.extend(doc.sound.iter().filter_map(|(_, v)| v.id));
            ids.extend(doc.rig.iter().filter_map(|(_, v)| v.id));
            ids.extend(doc.tiles.iter().filter_map(|(_, v)| v.id));
            ids.extend(doc.particles.iter().filter_map(|(_, v)| v.id));
            ids.extend(doc.library.iter().filter_map(|(_, v)| v.id));
            ids.extend(doc.proxy.iter().filter_map(|(_, v)| v.id));
        }
        ids
    }

    pub fn read(&self, file: &Path) -> Result<File, Vec<Diagnostic>> {
        let relative = self.relative(file);
        let overlay = BTreeMap::new();
        let kind = self.kind_of(&relative, &overlay).unwrap_or(FileKind::Scene);
        let mut checker = Checker::new(self, overlay);
        checker.load(&relative, kind);
        let diagnostics: Vec<Diagnostic> = checker
            .report()
            .into_iter()
            .filter(|diagnostic| diagnostic.file == relative)
            .collect();
        let typed = checker
            .parsed
            .get(&relative)
            .and_then(|parsed| parsed.file());
        match typed {
            Some(file) if !diagnostics.iter().any(Diagnostic::is_error) => Ok(file),
            _ => Err(self.shown_all(diagnostics)),
        }
    }

    pub fn migrate(&self, file: &Path) -> Result<String, Vec<Diagnostic>> {
        let relative = self.relative(file);
        let overlay = BTreeMap::new();
        let kind = self.kind_of(&relative, &overlay).unwrap_or(FileKind::Scene);
        let text = self.source(&relative).map_err(|error| {
            vec![Diagnostic {
                file: relative.clone(),
                line: 1,
                column: 1,
                end_line: 1,
                end_column: 1,
                severity: Severity::Error,
                code: code::UNREADABLE,
                key: String::new(),
                message: format!("cannot read {}: {error}", relative.display()),
                related: Vec::new(),
            }]
        })?;
        migrate(&text, &relative, &self.root, kind)
    }
}

pub fn migrate(
    text: &str,
    file: &Path,
    root: &Path,
    kind: FileKind,
) -> Result<String, Vec<Diagnostic>> {
    let format = toml::de::DeTable::parse(text).ok().and_then(|table| {
        table.get_ref().get("format").and_then(|value| {
            value
                .get_ref()
                .as_integer()
                .map(|integer| integer.as_str().to_string())
        })
    });
    if format.as_deref().is_some_and(|format| format != "0") {
        return Ok(text.to_string());
    }
    let relative = PathBuf::from(slashed(file));
    match crate::migrate::migrate(text, &relative, root, kind, true) {
        Ok(migration) => Ok(migration.text),
        Err(problems) => {
            let findings: Vec<Finding> = problems
                .into_iter()
                .map(|problem| {
                    Finding::new(
                        &relative,
                        problem.path,
                        Part::Value,
                        problem.code,
                        problem.message,
                    )
                })
                .collect();
            let mut parsed = BTreeMap::new();
            parsed.insert(relative.clone(), Box::new(read::plain(text, kind)));
            Err(read::report(&findings, &parsed))
        }
    }
}

pub fn check(source: &str, file: &Path, project: &Project) -> Vec<Diagnostic> {
    let file = project.relative(file);
    let shown = project.shown(&file);
    let mut overlay = BTreeMap::new();
    overlay.insert(file, source.to_string());
    project
        .check_with(overlay)
        .into_iter()
        .filter(|diagnostic| diagnostic.file == shown)
        .collect()
}

fn mover_of<'a>(movers: &'a [Entry<Mover>], key: &str) -> Option<&'a Entry<Mover>> {
    movers
        .iter()
        .find(|mover| mover.value.objects.iter().any(|object| object == key))
}

fn dynamics(objects: &[Entry<Object>], movers: &[Entry<Mover>]) -> Vec<bool> {
    let index: BTreeMap<&str, usize> = objects
        .iter()
        .enumerate()
        .map(|(at, object)| (object.key.as_str(), at))
        .collect();
    let moved: BTreeSet<&str> = movers
        .iter()
        .flat_map(|mover| mover.value.objects.iter().map(String::as_str))
        .collect();
    let dynamic = |start: usize| {
        let mut at = start;
        for _ in 0..=objects.len() {
            let object = &objects[at];
            if let Some(dynamic) = object.value.dynamic {
                return dynamic;
            }
            if object.value.face_camera
                || object.value.character.is_some()
                || object.value.animation.is_some()
                || moved.contains(object.key.as_str())
            {
                return true;
            }
            match object.value.parent.as_deref().and_then(|p| index.get(p)) {
                Some(parent) => at = *parent,
                None => return false,
            }
        }
        false
    };
    (0..objects.len()).map(dynamic).collect()
}

fn why_dynamic(placed: &[Out<Object>], movers: &[Entry<Mover>], at: usize) -> String {
    let object = &placed[at].entry;
    if object.value.dynamic == Some(true) {
        "it sets dynamic = true".to_string()
    } else if let Some(mover) = mover_of(movers, &object.key) {
        format!("mover {} moves it", mover.key)
    } else if object.value.character.is_some() {
        "it is a character".to_string()
    } else if object.value.animation.is_some() {
        "it is animated".to_string()
    } else if object.value.face_camera {
        "it faces the camera".to_string()
    } else {
        match &object.value.parent {
            Some(parent) => format!("its parent {parent} is dynamic"),
            None => "it is dynamic".to_string(),
        }
    }
}

#[derive(Clone, Debug)]
struct Item<T> {
    file: PathBuf,
    path: KeyPath,
    name: String,
    id: Option<Id>,
    value: T,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Mesh,
    Object,
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

impl Kind {
    const ALL: [Kind; 11] = [
        Kind::Mesh,
        Kind::Object,
        Kind::Light,
        Kind::Emitter,
        Kind::Mover,
        Kind::Content,
        Kind::Text,
        Kind::Sound,
        Kind::Rig,
        Kind::Tiles,
        Kind::Particles,
    ];

    fn word(self) -> &'static str {
        match self {
            Kind::Mesh => "mesh",
            Kind::Object => "object",
            Kind::Light => "light",
            Kind::Emitter => "emitter",
            Kind::Mover => "mover",
            Kind::Content => "content",
            Kind::Text => "text",
            Kind::Sound => "sound",
            Kind::Rig => "rig",
            Kind::Tiles => "tile layer",
            Kind::Particles => "particle system",
        }
    }
}

#[derive(Clone, Debug)]
struct Single<T> {
    file: PathBuf,
    path: KeyPath,
    value: T,
}

#[derive(Clone, Debug, Default)]
struct Scope {
    prefab: bool,
    files: Vec<PathBuf>,
    libraries: Vec<(PathBuf, PathBuf, KeyPath)>,
    meshes: Vec<Item<Mesh>>,
    objects: Vec<Item<Object>>,
    lights: Vec<Item<Light>>,
    emitters: Vec<Item<Emitter>>,
    movers: Vec<Item<Mover>>,
    contents: Vec<Item<Content>>,
    texts: Vec<Item<Text>>,
    sounds: Vec<Item<Sound>>,
    rigs: Vec<Item<Rig>>,
    tiles: Vec<Item<Tiles>>,
    particles: Vec<Item<Particles>>,
    fallback: Option<Single<String>>,
    sound_file: Option<Single<String>>,
    sun: Option<Single<Sun>>,
    sky: Option<Single<Sky>>,
    haze: Option<Single<Haze>>,
    camera: Option<Single<Camera>>,
    finish: Option<Single<Finish>>,
    flare: Option<Single<Flare>>,
    motion_blur: Option<Single<MotionBlur>>,
    trace: Option<Single<Trace>>,
    shadows: Option<Single<Shadows>>,
    live: Option<Single<LiveSettings>>,
    plates: Option<Single<Plates>>,
    physics: Option<Single<Physics>>,
    play: Option<Single<Play>>,
    music: Option<Single<toml::Table>>,
}

enum Lookup {
    One(usize),
    None,
    Many(Vec<usize>),
}

fn lookup<T>(items: &[Item<T>], reference: &str) -> Lookup {
    if let Ok(id) = Id::parse(reference)
        && let Some(at) = items.iter().position(|item| item.id == Some(id))
    {
        return Lookup::One(at);
    }
    let named: Vec<usize> = items
        .iter()
        .enumerate()
        .filter(|(_, item)| item.name == reference)
        .map(|(at, _)| at)
        .collect();
    match named.as_slice() {
        [] => Lookup::None,
        [one] => Lookup::One(*one),
        _ => Lookup::Many(named),
    }
}

impl Scope {
    fn names(&self, kind: Kind) -> Vec<(&str, Option<Id>, &Path, &KeyPath)> {
        fn list<T>(items: &[Item<T>]) -> Vec<(&str, Option<Id>, &Path, &KeyPath)> {
            items
                .iter()
                .map(|item| (item.name.as_str(), item.id, item.file.as_path(), &item.path))
                .collect()
        }
        match kind {
            Kind::Mesh => list(&self.meshes),
            Kind::Object => list(&self.objects),
            Kind::Light => list(&self.lights),
            Kind::Emitter => list(&self.emitters),
            Kind::Mover => list(&self.movers),
            Kind::Content => list(&self.contents),
            Kind::Text => list(&self.texts),
            Kind::Sound => list(&self.sounds),
            Kind::Rig => list(&self.rigs),
            Kind::Tiles => list(&self.tiles),
            Kind::Particles => list(&self.particles),
        }
    }

    fn lookup(&self, kind: Kind, reference: &str) -> Lookup {
        match kind {
            Kind::Mesh => lookup(&self.meshes, reference),
            Kind::Object => lookup(&self.objects, reference),
            Kind::Light => lookup(&self.lights, reference),
            Kind::Emitter => lookup(&self.emitters, reference),
            Kind::Mover => lookup(&self.movers, reference),
            Kind::Content => lookup(&self.contents, reference),
            Kind::Text => lookup(&self.texts, reference),
            Kind::Sound => lookup(&self.sounds, reference),
            Kind::Rig => lookup(&self.rigs, reference),
            Kind::Tiles => lookup(&self.tiles, reference),
            Kind::Particles => lookup(&self.particles, reference),
        }
    }
}

#[derive(Clone, Debug)]
struct Override {
    file: PathBuf,
    path: KeyPath,
    entry: String,
    keys: Vec<String>,
    value: toml::Value,
    own: bool,
}

#[derive(Clone, Debug)]
struct Prefix {
    key: String,
    id: Option<String>,
    hidden: bool,
    tint: Option<[f32; 3]>,
    recolor: BTreeMap<String, Recolor>,
}

fn tinted(outer: Option<[f32; 3]>, own: Option<&Color>) -> Option<[f32; 3]> {
    let own = own.and_then(Color::rgb);
    match (outer, own) {
        (Some(outer), Some(own)) => Some(std::array::from_fn(|c| outer[c] * own[c])),
        (outer, own) => outer.or(own),
    }
}

fn recolored(
    own: &BTreeMap<String, Recolor>,
    outer: &BTreeMap<String, Recolor>,
) -> BTreeMap<String, Recolor> {
    let mut out = own.clone();
    out.extend(
        outer
            .iter()
            .map(|(name, recolor)| (name.clone(), recolor.clone())),
    );
    out
}

#[derive(Clone, Debug)]
struct Touch {
    key: String,
    file: PathBuf,
    path: KeyPath,
}

#[derive(Clone, Debug)]
struct Out<T> {
    entry: Entry<T>,
    own: Option<KeyPath>,
    at: KeyPath,
    set: Vec<Touch>,
}

impl<T> Out<T> {
    fn blame(&self, key: &str) -> Option<(PathBuf, KeyPath, bool)> {
        if let Some(path) = &self.own {
            return Some((self.entry.file.clone(), under(path, &[key.into()]), false));
        }
        touch(&self.set, key).map(|touch| (touch.file.clone(), touch.path.clone(), true))
    }
}

fn touch<'a>(set: &'a [Touch], key: &str) -> Option<&'a Touch> {
    set.iter().rev().find(|touch| touch.key == key)
}

#[derive(Clone, Debug, Default)]
struct Placed {
    meshes: Vec<Out<Mesh>>,
    objects: Vec<Out<Object>>,
    lights: Vec<Out<Light>>,
    emitters: Vec<Out<Emitter>>,
    movers: Vec<Out<Mover>>,
    contents: Vec<Out<Content>>,
    texts: Vec<Out<Text>>,
    sounds: Vec<Out<Sound>>,
    rigs: Vec<Out<Rig>>,
    particles: Vec<Out<Particles>>,
    libraries: Vec<(PathBuf, PathBuf, KeyPath)>,
    files: Vec<PathBuf>,
}

#[derive(Clone, Debug)]
struct Step {
    file: PathBuf,
    link: Option<(PathBuf, KeyPath)>,
}

impl Step {
    fn new(file: &Path) -> Self {
        Self {
            file: file.to_path_buf(),
            link: None,
        }
    }
}

struct Frame {
    scope: Scope,
    prefix: Option<Prefix>,
    own: bool,
    nested: BTreeMap<usize, Vec<Override>>,
    touched: BTreeMap<(Kind, usize), Vec<Touch>>,
    placed: Placed,
    inner: Placed,
    inner_objects: BTreeMap<usize, Vec<Out<Object>>>,
    next: usize,
    placing: usize,
}

pub(crate) fn trail(names: &[String]) -> String {
    if names.len() <= 8 {
        return names.join(" -> ");
    }
    let head = names[..3].join(" -> ");
    let tail = names[names.len() - 3..].join(" -> ");
    format!("{head} -> ... -> {tail}")
}

fn name_key(prefix: Option<&Prefix>, name: &str) -> String {
    match prefix {
        Some(prefix) => format!("{}/{name}", prefix.key),
        None => name.to_string(),
    }
}

fn full_id(prefix: Option<&Prefix>, id: Option<Id>) -> Option<String> {
    match (prefix, id) {
        (Some(prefix), Some(id)) => prefix.id.as_ref().map(|outer| format!("{outer}/{id}")),
        (None, Some(id)) => Some(id.to_string()),
        _ => None,
    }
}

struct Checker<'p> {
    project: &'p Project,
    overlay: BTreeMap<PathBuf, String>,
    parsed: BTreeMap<PathBuf, Box<Parsed>>,
    kinds: BTreeMap<PathBuf, FileKind>,
    findings: Vec<Finding>,
    listing: Listing,
    filed: BTreeSet<PathBuf>,
    scopes: BTreeMap<PathBuf, Rc<Scope>>,
    prefabs_checked: BTreeSet<PathBuf>,
    unchecked: Vec<(PathBuf, Vec<Step>)>,
    bound: BTreeSet<PathBuf>,
    layers: Option<Option<Rc<Layers>>>,
    found: BTreeSet<PathBuf>,
}

impl<'p> Checker<'p> {
    fn new(project: &'p Project, overlay: BTreeMap<PathBuf, String>) -> Self {
        Self {
            project,
            overlay,
            parsed: BTreeMap::new(),
            kinds: BTreeMap::new(),
            findings: Vec::new(),
            listing: Listing::default(),
            filed: BTreeSet::new(),
            scopes: BTreeMap::new(),
            prefabs_checked: BTreeSet::new(),
            unchecked: Vec::new(),
            bound: BTreeSet::new(),
            layers: None,
            found: BTreeSet::new(),
        }
    }

    fn layers(&mut self) -> Option<Rc<Layers>> {
        if let Some(layers) = &self.layers {
            return layers.clone();
        }
        let file = Path::new("project.toml");
        let layers = match self.text(file) {
            None => Some(Rc::new(Layers::new())),
            Some(Err(_)) => None,
            Some(Ok(text)) => {
                let context = Context {
                    root: &self.project.root,
                };
                read::parse(file, &text, FileKind::Project, &context)
                    .doc
                    .filter(|doc| !doc.layers_unread)
                    .map(|doc| Rc::new(doc.layers.unwrap_or_default()))
            }
        };
        self.layers = Some(layers.clone());
        layers
    }

    fn layered(&mut self, file: &Path, path: &KeyPath, object: &Object) {
        let Some(layers) = self.layers() else { return };
        for (at, name) in rules::layer_names(object) {
            if let Some(message) = rules::unknown_layer(&layers, &name) {
                self.issue(file, &under(path, &at), code::BAD_REFERENCE, message);
            }
        }
    }

    fn push(&mut self, finding: Finding) {
        self.findings.push(finding);
    }

    fn issue(
        &mut self,
        file: &Path,
        path: &KeyPath,
        code: &'static str,
        message: impl Into<String>,
    ) {
        self.push(Finding::new(file, path.clone(), Part::Value, code, message));
    }

    fn issues(&mut self, file: &Path, path: &KeyPath, issues: Issues) {
        for issue in issues.0 {
            let at = under(path, &issue.key);
            let code = if let Some(code) = issue.code {
                code
            } else if issue.message.starts_with("missing key") {
                code::MISSING_KEY
            } else {
                code::BAD_VALUE
            };
            self.issue(file, &at, code, issue.message);
        }
    }

    fn text(&self, file: &Path) -> Option<Result<String, String>> {
        if let Some(text) = self.overlay.get(file) {
            return Some(Ok(text.clone()));
        }
        if let Some(text) = self.project.texts.get(file) {
            return Some(text.clone());
        }
        if self.project.in_memory() {
            return None;
        }
        let path = self.project.root.join(file);
        if !path.is_file() {
            return None;
        }
        Some(
            std::fs::read(&path)
                .map_err(|error| format!("cannot read {}: {error}", file.display()))
                .and_then(|bytes| {
                    String::from_utf8(bytes).map_err(|_| "the file is not UTF-8".to_string())
                }),
        )
    }

    fn load(&mut self, file: &Path, kind: FileKind) -> Option<Doc> {
        self.loaded(file, kind).cloned()
    }

    fn loaded(&mut self, file: &Path, kind: FileKind) -> Option<&Doc> {
        if !self.parsed.contains_key(file) {
            let text = self.text(file)?;
            self.kinds.insert(file.to_path_buf(), kind);
            let parsed = match text {
                Ok(text) => read::parse(
                    file,
                    &text,
                    kind,
                    &Context {
                        root: &self.project.root,
                    },
                ),
                Err(message) => {
                    let code = if message.contains("UTF-8") {
                        code::NOT_UTF8
                    } else {
                        code::UNREADABLE
                    };
                    self.push(Finding::new(file, Vec::new(), Part::Value, code, message));
                    return None;
                }
            };
            self.parsed.insert(file.to_path_buf(), Box::new(parsed));
            self.file_checks(file);
        }
        self.parsed.get(file).and_then(|parsed| parsed.doc.as_ref())
    }

    fn asset(
        &mut self,
        file: &Path,
        path: &KeyPath,
        raw: &str,
        extensions: &[&str],
    ) -> Option<PathBuf> {
        self.located(file, path, raw, extensions, true)
    }

    fn located(
        &mut self,
        file: &Path,
        path: &KeyPath,
        raw: &str,
        extensions: &[&str],
        needed: bool,
    ) -> Option<PathBuf> {
        let old = self
            .parsed
            .get(file)
            .is_some_and(|parsed| parsed.origin.is_some());
        let overlay: BTreeSet<String> = self.overlay.keys().map(|key| slashed(key)).collect();
        let rooted = match self
            .project
            .placed(file, raw, &|path| overlay.contains(path))
        {
            Ok(rooted) => rooted,
            Err(error) => {
                if !(old && matches!(error, PathError::Outside(_))) {
                    self.issue(file, path, error.code(), error.message(raw));
                }
                return None;
            }
        };
        if !extensions.is_empty() && !extensions.iter().any(|end| rooted.ends_with(end)) {
            self.issue(
                file,
                path,
                code::BAD_PATH,
                format!("path {raw} is not a {} file", extensions.join(" or ")),
            );
            return None;
        }
        let mount = self.project.mount.as_ref();
        let held =
            |path: &str| overlay.contains(path) || mount.is_some_and(|mount| mount.holds(path));
        let found = if self.project.in_memory() {
            if held(&rooted) {
                Found::Exact
            } else {
                Found::Missing
            }
        } else {
            self.listing.find(&self.project.root, &rooted, &held)
        };
        match found {
            Found::Exact => {
                self.found.insert(PathBuf::from(&rooted));
                Some(PathBuf::from(rooted))
            }
            Found::Case(actual) => {
                self.issue(
                    file,
                    path,
                    code::PATH_CASE,
                    format!("path {raw} is found only as {actual}; paths keep their case"),
                );
                None
            }
            Found::Missing => {
                if needed {
                    self.issue(
                        file,
                        path,
                        code::MISSING_FILE,
                        format!("{raw} does not exist"),
                    );
                }
                None
            }
        }
    }

    fn missing_id(&mut self, file: &Path, path: KeyPath, what: &str, id: Option<Id>) {
        if id.is_none() {
            self.push(Finding::new(
                file,
                path,
                Part::Value,
                code::MISSING_ID,
                format!("{what} has no id; the next write through the crate adds one"),
            ));
        }
    }

    fn file_checks(&mut self, file: &Path) {
        if !self.filed.insert(file.to_path_buf()) {
            return;
        }
        let Some(parsed) = self.parsed.get(file) else {
            return;
        };
        let findings = parsed.findings.clone();
        let kind = parsed.kind;
        let doc = parsed.doc.clone();
        self.findings.extend(findings);
        let Some(doc) = doc else { return };
        match kind {
            FileKind::Scene | FileKind::Prefab => self.scene_checks(file, kind, &doc),
            FileKind::Materials => {
                for (name, material) in &doc.library {
                    let path = key(&["materials", name]);
                    if let Some(message) = rules::name("material", name) {
                        self.push(Finding::new(
                            file,
                            path.clone(),
                            Part::Key,
                            code::BAD_NAME,
                            message,
                        ));
                    }
                    self.issues(file, &path, rules::material(name, material));
                    self.missing_id(file, path, &format!("material {name}"), material.id);
                }
            }
            FileKind::Proxies => {
                for (place, proxy) in &doc.proxy {
                    let path = key(&["proxy", &place.to_string()]);
                    self.issues(file, &path, rules::proxy(proxy));
                    self.missing_id(file, path, "a proxy", proxy.id);
                }
            }
            FileKind::Project => {
                if let Some(project) = &doc.project {
                    if let Some(scene) = &project.scene {
                        self.asset(
                            file,
                            &key(&["project", "scene"]),
                            scene,
                            &[".scene.toml", ".pitofx"],
                        );
                    }
                    for (place, raw) in project.ignore.iter().enumerate() {
                        let path = key(&["project", "ignore", &place.to_string()]);
                        self.located(file, &path, raw, &[], false);
                    }
                }
                for (name, tunable) in &doc.tunables {
                    let path = key(&["tunables", name]);
                    self.named(file, &path, "tunable", name, Part::Key);
                    if let Some(group) = &tunable.group {
                        self.named(
                            file,
                            &under(&path, &["group".into()]),
                            "tunable group",
                            group,
                            Part::Value,
                        );
                    }
                    self.issues(file, &path, rules::tunable(name, tunable));
                }
                if let Some(layers) = &doc.layers {
                    for name in layers.keys() {
                        self.named(file, &key(&["layers", name]), "layer", name, Part::Key);
                    }
                    self.issues(file, &key(&["layers"]), rules::layers(layers));
                }
            }
            FileKind::Finish => {
                if let Some(keys) = &doc.finish_keys {
                    self.issues(file, &Vec::new(), rules::finish_keys(keys));
                    self.lut_path(file, &Vec::new(), keys);
                }
            }
        }
    }

    fn scene_checks(&mut self, file: &Path, kind: FileKind, doc: &Doc) {
        let include_end: &[&str] = if kind == FileKind::Prefab {
            &[".prefab.toml"]
        } else {
            &[".scene.toml", "/scene.toml"]
        };
        for (place, raw) in &doc.include {
            self.asset(
                file,
                &key(&["include", &place.to_string()]),
                raw,
                include_end,
            );
        }
        for (place, raw) in &doc.materials {
            self.asset(
                file,
                &key(&["materials", &place.to_string()]),
                raw,
                &[".toml"],
            );
        }
        for (name, mesh) in &doc.mesh {
            let path = key(&["mesh", name]);
            self.named(file, &path, "mesh", name, Part::Key);
            if mesh.file == GLTF_PATH {
                if !self.project.mount.as_ref().is_some_and(|mount| mount.gltf) {
                    self.issue(
                        file,
                        &under(&path, &["file".into()]),
                        code::MISSING_FILE,
                        format!(
                            "{GLTF_PATH} names the glTF inside the scene's .pitofx, and this scene is not in a .pitofx that holds one"
                        ),
                    );
                }
            } else {
                self.asset(file, &under(&path, &["file".into()]), &mesh.file, &[]);
            }
            self.missing_id(file, path, &format!("mesh {name}"), mesh.id);
        }
        for (place, object) in &doc.object {
            let path = key(&["object", &place.to_string()]);
            self.named(
                file,
                &under(&path, &["name".into()]),
                "object",
                &object.name,
                Part::Value,
            );
            let issues = rules::object(object);
            for issue in issues.0 {
                let at = under(&path, &issue.key);
                let code = if let Some(code) = issue.code {
                    code
                } else if object.is_placement()
                    && issue.key.len() == 1
                    && issue.message.contains("places a prefab")
                {
                    code::PLACEMENT_KEY
                } else if issue.message.starts_with("missing key") {
                    code::MISSING_KEY
                } else {
                    code::BAD_VALUE
                };
                let part = if code == code::PLACEMENT_KEY {
                    Part::Key
                } else {
                    Part::Value
                };
                self.push(Finding::new(file, at, part, code, issue.message));
            }
            if let Some(prefab) = &object.prefab
                && Id::parse(prefab).is_err()
            {
                self.asset(
                    file,
                    &under(&path, &["prefab".into()]),
                    prefab,
                    &[".prefab.toml"],
                );
            }
            if let Some(raw) = object.animation.as_ref().and_then(|a| a.atlas.as_ref()) {
                self.asset(
                    file,
                    &under(&path, &["animation".into(), "atlas".into()]),
                    raw,
                    &[],
                );
            }
            self.layered(file, &path, object);
            self.missing_id(file, path, &format!("object {}", object.name), object.id);
        }
        for (place, light) in &doc.light {
            let path = key(&["light", &place.to_string()]);
            self.named(
                file,
                &under(&path, &["name".into()]),
                "light",
                &light.name,
                Part::Value,
            );
            self.issues(file, &path, rules::light(light));
            let placed = self
                .parsed
                .get(file)
                .is_none_or(|parsed| parsed.spans.has(&under(&path, &["position".into()])));
            if light.parent.is_none() && !placed {
                self.issue(
                    file,
                    &path,
                    code::MISSING_KEY,
                    format!(
                        "missing key 'position': light {} has no parent, so it needs a position",
                        light.name
                    ),
                );
            } else if light.parent.is_some() && placed {
                self.issue(
                    file,
                    &under(&path, &["position".into()]),
                    code::BAD_VALUE,
                    format!(
                        "light {} has a parent, so at places it, not position",
                        light.name
                    ),
                );
            }
            self.missing_id(file, path, &format!("light {}", light.name), light.id);
        }
        for (place, emitter) in &doc.emitter {
            let path = key(&["emitter", &place.to_string()]);
            self.named(
                file,
                &under(&path, &["name".into()]),
                "emitter",
                &emitter.name,
                Part::Value,
            );
            self.issues(file, &path, rules::emitter(emitter));
            self.missing_id(file, path, &format!("emitter {}", emitter.name), emitter.id);
        }
        for (place, mover) in &doc.mover {
            let path = key(&["mover", &place.to_string()]);
            self.named(
                file,
                &under(&path, &["name".into()]),
                "mover",
                &mover.name,
                Part::Value,
            );
            self.issues(file, &path, rules::mover(mover));
            self.missing_id(file, path, &format!("mover {}", mover.name), mover.id);
        }
        for (name, content) in &doc.content {
            let path = key(&["content", name]);
            self.named(file, &path, "content", name, Part::Key);
            self.asset(file, &under(&path, &["image".into()]), &content.image, &[]);
            self.missing_id(file, path, &format!("content {name}"), content.id);
        }
        for (name, text) in &doc.text {
            let path = key(&["text", name]);
            self.named(file, &path, "text", name, Part::Key);
            self.issues(file, &path, rules::text(text));
            self.asset(file, &under(&path, &["font".into()]), &text.font, &[]);
            self.missing_id(file, path, &format!("text {name}"), text.id);
        }
        for (name, sound) in &doc.sound {
            let path = key(&["sound", name]);
            self.named(file, &path, "sound", name, Part::Key);
            self.issues(file, &path, rules::sound(name, sound));
            self.asset(file, &under(&path, &["file".into()]), &sound.file, &[]);
            self.missing_id(file, path, &format!("sound {name}"), sound.id);
        }
        for (place, rig) in &doc.rig {
            let path = key(&["rig", &place.to_string()]);
            self.named(
                file,
                &under(&path, &["name".into()]),
                "rig",
                &rig.name,
                Part::Value,
            );
            self.issues(file, &path, rules::rig(rig));
            self.missing_id(file, path, &format!("rig {}", rig.name), rig.id);
        }
        for (place, tiles) in &doc.tiles {
            let path = key(&["tiles", &place.to_string()]);
            self.named(
                file,
                &under(&path, &["name".into()]),
                "tile layer",
                &tiles.name,
                Part::Value,
            );
            for (tile, raw) in &tiles.palette {
                let at = under(&path, &["palette".into(), tile.clone()]);
                if let Some(message) = rules::palette_key(tile) {
                    self.push(Finding::new(
                        file,
                        at.clone(),
                        Part::Key,
                        code::BAD_NAME,
                        format!("tile layer {}: {message}", tiles.name),
                    ));
                }
                if Id::parse(raw).is_err() {
                    self.asset(file, &at, raw, &[".prefab.toml"]);
                }
            }
            self.issues(file, &path, rules::tiles(tiles));
            self.rows(file, &path, tiles);
            self.missing_id(file, path, &format!("tile layer {}", tiles.name), tiles.id);
        }
        for (place, particles) in &doc.particles {
            let path = key(&["particles", &place.to_string()]);
            self.named(
                file,
                &under(&path, &["name".into()]),
                "particle system",
                &particles.name,
                Part::Value,
            );
            self.issues(file, &path, rules::particles(particles));
            self.missing_id(
                file,
                path,
                &format!("particle system {}", particles.name),
                particles.id,
            );
        }
        if let Some(raw) = &doc.sound_file {
            self.asset(file, &key(&["sound"]), raw, &[".sound.toml"]);
        }
        if let Some(raw) = doc.play.as_ref().and_then(|play| play.module.as_ref()) {
            self.located(file, &key(&["play", "module"]), raw, &[".wasm"], false);
        }
        if let Some(sun) = &doc.sun {
            self.issues(file, &key(&["sun"]), rules::sun(sun));
        }
        if let Some(sky) = &doc.sky {
            self.issues(file, &key(&["sky"]), rules::sky(sky));
            if let Some(raw) = &sky.path {
                self.asset(file, &key(&["sky", "path"]), raw, &[]);
            }
            for (place, layer) in sky.layer.iter().enumerate() {
                if let Some(raw) = &layer.path {
                    self.asset(
                        file,
                        &key(&["sky", "layer", &place.to_string(), "path"]),
                        raw,
                        &[],
                    );
                }
            }
        }
        if let Some(haze) = &doc.haze {
            self.issues(file, &key(&["haze"]), rules::haze(haze));
        }
        if let Some(camera) = &doc.camera {
            self.issues(file, &key(&["camera"]), rules::camera(camera));
        }
        if let Some(finish) = &doc.finish {
            self.issues(file, &key(&["finish"]), rules::finish(finish));
            self.lut_path(file, &key(&["finish"]), &finish.keys());
            for (place, pass) in finish.pass.iter().enumerate() {
                self.lut_path(file, &key(&["finish", "pass", &place.to_string()]), pass);
            }
            if let Some(raw) = &finish.file
                && let Some(target) = self.asset(file, &key(&["finish", "file"]), raw, &[])
            {
                self.load(&target, FileKind::Finish);
            }
        }
        if let Some(flare) = &doc.flare {
            self.issues(file, &key(&["flare"]), rules::flare(flare));
        }
        if let Some(motion_blur) = &doc.motion_blur {
            self.issues(
                file,
                &key(&["motion_blur"]),
                rules::motion_blur(motion_blur),
            );
        }
        if let Some(trace) = &doc.trace {
            self.issues(file, &key(&["trace"]), rules::trace(trace));
        }
        if let Some(shadows) = &doc.shadows {
            self.issues(file, &key(&["shadows"]), rules::shadows(shadows));
        }
        if let Some(plates) = &doc.plates {
            self.issues(file, &key(&["plates"]), rules::plates(plates));
            self.located(file, &key(&["plates", "dir"]), &plates.dir, &[], false);
            if let Some(raw) = &plates.proxies
                && let Some(target) =
                    self.asset(file, &key(&["plates", "proxies"]), raw, &[".proxies.toml"])
            {
                self.load(&target, FileKind::Proxies);
            }
        }
        if let Some(physics) = &doc.physics {
            self.issues(file, &key(&["physics"]), rules::physics(physics));
        }
    }

    fn lut_path(&mut self, file: &Path, path: &KeyPath, keys: &FinishKeys) {
        if let Some(LutKey::Path(raw)) = &keys.lut {
            self.asset(file, &under(path, &["lut".into()]), raw, &[".cube"]);
        }
    }

    fn rows(&mut self, file: &Path, path: &KeyPath, tiles: &Tiles) {
        let Some(rows) = &tiles.rows else { return };
        for (place, row) in rows.iter().enumerate() {
            let at = under(path, &["rows".into(), place.to_string()]);
            let start = self.parsed.get(file).and_then(|parsed| {
                if parsed.origin.is_some() {
                    return None;
                }
                let range = parsed.locate(&at, Part::Value);
                let raw = parsed.text.get(range.clone())?;
                let quoted = (raw.starts_with('"') && !raw.starts_with("\"\"\""))
                    || (raw.starts_with('\'') && !raw.starts_with("'''"));
                (quoted && raw.len() >= 2 && raw[1..raw.len() - 1] == *row)
                    .then_some(range.start + 1)
            });
            let mut seen = BTreeSet::new();
            for (offset, character) in row.char_indices() {
                if character == Tiles::EMPTY {
                    continue;
                }
                let tile = character.to_string();
                if tiles.palette.contains_key(&tile) || !seen.insert(character) {
                    continue;
                }
                let mut finding = Finding::new(
                    file,
                    at.clone(),
                    Part::Value,
                    code::BAD_REFERENCE,
                    rules::unknown_tile(&tiles.name, &tile, tiles),
                );
                finding.span =
                    start.map(|start| start + offset..start + offset + character.len_utf8());
                self.push(finding);
            }
        }
    }

    fn named(&mut self, file: &Path, path: &KeyPath, kind: &str, name: &str, part: Part) {
        if let Some(message) = rules::name(kind, name) {
            self.push(Finding::new(
                file,
                path.clone(),
                part,
                code::BAD_NAME,
                message,
            ));
        }
    }

    fn rooted_from(&self, file: &Path, raw: &str) -> Result<String, PathError> {
        self.project
            .placed(file, raw, &|path| self.text(Path::new(path)).is_some())
    }

    fn found_at(&self, file: &Path, raw: &mut String) {
        if raw.as_str() != GLTF_PATH
            && let Ok(found) = self.rooted_from(file, raw)
        {
            *raw = found;
        }
    }

    fn include_target(&mut self, file: &Path, raw: &str, kind: FileKind) -> Option<PathBuf> {
        let rooted = self.rooted_from(file, raw).ok()?;
        let path = PathBuf::from(rooted);
        let end_ok = match kind {
            FileKind::Prefab => raw.ends_with(".prefab.toml"),
            _ => raw.ends_with(".scene.toml") || raw.ends_with("scene.toml"),
        };
        if !end_ok || self.text(&path).is_none() {
            return None;
        }
        Some(path)
    }

    fn includes(&mut self, file: &Path, kind: FileKind) -> Option<Vec<(usize, String)>> {
        self.loaded(file, kind).map(|doc| doc.include.clone())
    }

    fn gather(&mut self, file: &Path, kind: FileKind) -> Vec<PathBuf> {
        let mut order: Vec<PathBuf> = Vec::new();
        let Some(includes) = self.includes(file, kind) else {
            return order;
        };
        let mut steps = vec![Step::new(file)];
        let mut pending = vec![includes.into_iter()];
        while let Some(next) = pending.last_mut() {
            let Some((place, raw)) = next.next() else {
                pending.pop();
                if let Some(step) = steps.pop()
                    && !order.contains(&step.file)
                {
                    order.push(step.file);
                }
                continue;
            };
            let naming = steps
                .last()
                .map(|step| step.file.clone())
                .unwrap_or_default();
            let Some(target) = self.include_target(&naming, &raw, kind) else {
                continue;
            };
            if let Some(step) = steps.last_mut() {
                step.link = Some((step.file.clone(), key(&["include", &place.to_string()])));
            }
            if let Some(start) = steps.iter().position(|step| step.file == target) {
                self.chained(
                    &steps[start..],
                    &target,
                    code::INCLUDE_CYCLE,
                    format!("the {} includes itself through {raw}", kind.name()),
                );
                continue;
            }
            if order.contains(&target) {
                continue;
            }
            if steps.len() > MAX_DEPTH {
                self.chained(
                    &steps,
                    &target,
                    code::INCLUDE_DEPTH,
                    format!(
                        "including {raw} nests the {}'s includes more than {MAX_DEPTH} deep",
                        kind.name()
                    ),
                );
                continue;
            }
            let Some(includes) = self.includes(&target, kind) else {
                continue;
            };
            steps.push(Step::new(&target));
            pending.push(includes.into_iter());
        }
        order
    }

    fn chained(&mut self, steps: &[Step], target: &Path, code: &'static str, message: String) {
        let links: Vec<(PathBuf, KeyPath)> =
            steps.iter().filter_map(|step| step.link.clone()).collect();
        let Some((file, path)) = links.last().cloned() else {
            return;
        };
        let known = self
            .findings
            .iter()
            .any(|finding| finding.code == code && finding.file == file && finding.path == path);
        if known {
            return;
        }
        let names: Vec<String> = steps
            .iter()
            .map(|step| slashed(&step.file))
            .chain([slashed(target)])
            .collect();
        let mut finding = Finding::new(
            &file,
            path,
            Part::Value,
            code,
            format!("{message}: {}", trail(&names)),
        );
        for (file, path) in links {
            finding = finding.related(&file, path, Part::Value);
        }
        self.push(finding);
    }

    fn scope(&mut self, file: &Path, kind: FileKind) -> Rc<Scope> {
        if let Some(scope) = self.scopes.get(file) {
            return scope.clone();
        }
        let order = self.gather(file, kind);
        let mut scope = Scope {
            prefab: kind == FileKind::Prefab,
            files: order.clone(),
            ..Scope::default()
        };
        for member in &order {
            let Some(doc) = self
                .parsed
                .get(member)
                .and_then(|parsed| parsed.doc.clone())
            else {
                continue;
            };
            for (place, raw) in &doc.materials {
                if let Ok(rooted) = self.rooted_from(member, raw) {
                    let library = PathBuf::from(rooted);
                    if self.text(&library).is_some() {
                        scope.libraries.push((
                            library,
                            member.clone(),
                            key(&["materials", &place.to_string()]),
                        ));
                    }
                }
            }
            let at = |parts: &[&str]| key(parts);
            for (name, mesh) in doc.mesh {
                scope.meshes.push(Item {
                    file: member.clone(),
                    path: at(&["mesh", &name]),
                    id: mesh.id,
                    name,
                    value: mesh,
                });
            }
            for (place, object) in doc.object {
                scope.objects.push(Item {
                    file: member.clone(),
                    path: at(&["object", &place.to_string()]),
                    id: object.id,
                    name: object.name.clone(),
                    value: object,
                });
            }
            for (place, light) in doc.light {
                scope.lights.push(Item {
                    file: member.clone(),
                    path: at(&["light", &place.to_string()]),
                    id: light.id,
                    name: light.name.clone(),
                    value: light,
                });
            }
            for (place, emitter) in doc.emitter {
                scope.emitters.push(Item {
                    file: member.clone(),
                    path: at(&["emitter", &place.to_string()]),
                    id: emitter.id,
                    name: emitter.name.clone(),
                    value: emitter,
                });
            }
            for (place, mover) in doc.mover {
                scope.movers.push(Item {
                    file: member.clone(),
                    path: at(&["mover", &place.to_string()]),
                    id: mover.id,
                    name: mover.name.clone(),
                    value: mover,
                });
            }
            for (name, content) in doc.content {
                scope.contents.push(Item {
                    file: member.clone(),
                    path: at(&["content", &name]),
                    id: content.id,
                    name,
                    value: content,
                });
            }
            for (name, text) in doc.text {
                scope.texts.push(Item {
                    file: member.clone(),
                    path: at(&["text", &name]),
                    id: text.id,
                    name,
                    value: text,
                });
            }
            for (name, sound) in doc.sound {
                scope.sounds.push(Item {
                    file: member.clone(),
                    path: at(&["sound", &name]),
                    id: sound.id,
                    name,
                    value: sound,
                });
            }
            for (place, rig) in doc.rig {
                scope.rigs.push(Item {
                    file: member.clone(),
                    path: at(&["rig", &place.to_string()]),
                    id: rig.id,
                    name: rig.name.clone(),
                    value: rig,
                });
            }
            for (place, tiles) in doc.tiles {
                scope.tiles.push(Item {
                    file: member.clone(),
                    path: at(&["tiles", &place.to_string()]),
                    id: tiles.id,
                    name: tiles.name.clone(),
                    value: tiles,
                });
            }
            for (place, particles) in doc.particles {
                scope.particles.push(Item {
                    file: member.clone(),
                    path: at(&["particles", &place.to_string()]),
                    id: particles.id,
                    name: particles.name.clone(),
                    value: particles,
                });
            }
            self.single(&mut scope.fallback, member, "fallback", doc.fallback);
            self.single(&mut scope.sound_file, member, "sound", doc.sound_file);
            self.single(&mut scope.sun, member, "sun", doc.sun);
            self.single(&mut scope.sky, member, "sky", doc.sky);
            self.single(&mut scope.haze, member, "haze", doc.haze);
            self.single(&mut scope.camera, member, "camera", doc.camera);
            self.single(&mut scope.finish, member, "finish", doc.finish);
            self.single(&mut scope.flare, member, "flare", doc.flare);
            self.single(
                &mut scope.motion_blur,
                member,
                "motion_blur",
                doc.motion_blur,
            );
            self.single(&mut scope.trace, member, "trace", doc.trace);
            self.single(&mut scope.shadows, member, "shadows", doc.shadows);
            self.single(&mut scope.live, member, "live", doc.live);
            self.single(&mut scope.plates, member, "plates", doc.plates);
            self.single(&mut scope.physics, member, "physics", doc.physics);
            self.single(&mut scope.play, member, "play", doc.play);
            self.single(&mut scope.music, member, "music", doc.music);
        }
        for kind in Kind::ALL {
            let names: Vec<(String, PathBuf, KeyPath)> = scope
                .names(kind)
                .into_iter()
                .map(|(name, _, file, path)| (name.to_string(), file.to_path_buf(), path.clone()))
                .collect();
            let mut seen: BTreeMap<String, (PathBuf, KeyPath, Part)> = BTreeMap::new();
            for (name, file, path) in names {
                let (at, part) = if path.len() == 2 && path[1] == name {
                    (path.clone(), Part::Key)
                } else {
                    (under(&path, &["name".into()]), Part::Value)
                };
                match seen.get(&name) {
                    Some((first, first_at, first_part)) => {
                        let shown = if first == &file {
                            "this file".to_string()
                        } else {
                            first.display().to_string()
                        };
                        let finding = Finding::new(
                            &file,
                            at,
                            part,
                            code::DUPLICATE_NAME,
                            format!(
                                "{} {name} is named twice; {shown} has it already",
                                kind.word()
                            ),
                        )
                        .related(first, first_at.clone(), *first_part);
                        self.push(finding);
                    }
                    None => {
                        seen.insert(name, (file, at, part));
                    }
                }
            }
        }
        let scope = Rc::new(scope);
        self.scopes.insert(file.to_path_buf(), scope.clone());
        scope
    }

    fn single<T>(
        &mut self,
        slot: &mut Option<Single<T>>,
        file: &Path,
        word: &str,
        value: Option<T>,
    ) {
        let Some(value) = value else { return };
        let path = key(&[word]);
        if let Some(first) = slot {
            let what = if word == "fallback" || word == "sound" {
                word.to_string()
            } else {
                format!("[{word}]")
            };
            let finding = Finding::new(
                file,
                path.clone(),
                Part::Key,
                code::HELD_TWICE,
                format!(
                    "{what} is set again; {} sets it already",
                    first.file.display()
                ),
            )
            .related(&first.file, first.path.clone(), Part::Key);
            self.push(finding);
            return;
        }
        *slot = Some(Single {
            file: file.to_path_buf(),
            path,
            value,
        });
    }

    fn overrides_of(&self, item: &Item<Object>, own: bool) -> Vec<Override> {
        item.value
            .set
            .iter()
            .map(|(name, value)| {
                let (entry, keys) = match name.split_once('.') {
                    Some((entry, rest)) => (
                        entry.to_string(),
                        rest.split('.').map(str::to_string).collect(),
                    ),
                    None => (name.clone(), Vec::new()),
                };
                Override {
                    file: item.file.clone(),
                    path: under(&item.path, &["set".into(), name.clone()]),
                    entry,
                    keys,
                    value: value.clone(),
                    own,
                }
            })
            .collect()
    }

    fn apply<T: serde::Serialize + serde::de::DeserializeOwned>(
        value: &T,
        change: &Override,
    ) -> Result<T, String> {
        if change.keys.is_empty() {
            return Err(format!("{} names an entry and no key of it", change.entry));
        }
        if change.keys[0] == "id" {
            return Err("an override does not change an id".to_string());
        }
        let mut root = toml::Value::try_from(value).map_err(|error| error.to_string())?;
        let mut at = &mut root;
        let (last, parents) = change.keys.split_last().unwrap_or((&change.keys[0], &[]));
        for (place, segment) in parents.iter().enumerate() {
            let next = change.keys[place + 1].as_str();
            at = match at {
                toml::Value::Table(table) => {
                    if !table.contains_key(segment) && next.parse::<usize>().is_ok() {
                        return Err(format!("{segment} has no entry {next}"));
                    }
                    table
                        .entry(segment.clone())
                        .or_insert_with(|| toml::Value::Table(toml::Table::new()))
                }
                toml::Value::Array(items) => {
                    let place: usize = segment
                        .parse()
                        .map_err(|_| format!("{segment} is not a list place"))?;
                    let count = items.len();
                    items
                        .get_mut(place)
                        .ok_or_else(|| format!("the list has {count} entries, not {place}"))?
                }
                _ => return Err(format!("{segment} is not in a table")),
            };
        }
        match at {
            toml::Value::Table(table) => {
                table.insert(last.clone(), change.value.clone());
            }
            toml::Value::Array(items) => {
                let place: usize = last
                    .parse()
                    .map_err(|_| format!("{last} is not a list place"))?;
                let count = items.len();
                let slot = items
                    .get_mut(place)
                    .ok_or_else(|| format!("the list has {count} entries, not {place}"))?;
                *slot = change.value.clone();
            }
            _ => return Err(format!("{last} is not in a table")),
        }
        root.try_into::<T>().map_err(|error| {
            error
                .message()
                .trim()
                .replace('`', "'")
                .replace("unknown field", "unknown key")
        })
    }

    fn reject(&mut self, change: &Override, message: String) {
        self.push(Finding::new(
            &change.file,
            change.path.clone(),
            Part::Key,
            code::BAD_OVERRIDE,
            message,
        ));
    }

    fn prefab_path(&self, scope: &Scope, file: &Path, raw: &str) -> Option<PathBuf> {
        if Id::parse(raw).is_ok()
            && let Lookup::One(at) = lookup(&scope.objects, raw)
        {
            let target = scope.objects[at].value.prefab.clone()?;
            if Id::parse(&target).is_ok() {
                return None;
            }
            return self
                .rooted_from(&scope.objects[at].file, &target)
                .ok()
                .map(PathBuf::from);
        }
        self.rooted_from(file, raw).ok().map(PathBuf::from)
    }

    fn place(&mut self, scope: &Scope, chain: &mut Vec<Step>) -> Placed {
        let mut frames = vec![self.enter(scope, None, &[], true)];
        while let Some(mut frame) = frames.pop() {
            if let Some(child) = self.descend(&mut frame, chain) {
                frames.push(frame);
                frames.push(child);
                continue;
            }
            let placed = self.finish(frame);
            match frames.last_mut() {
                Some(parent) => {
                    chain.pop();
                    Self::absorb(parent, placed);
                }
                None => return placed,
            }
        }
        Placed::default()
    }

    fn enter(
        &mut self,
        scope: &Scope,
        prefix: Option<Prefix>,
        changes: &[Override],
        own: bool,
    ) -> Frame {
        let mut scope = scope.clone();
        let layers = self.layers();
        let known = |object: &Object| match &layers {
            Some(layers) => rules::layer_names(object)
                .into_iter()
                .find_map(|(_, name)| rules::unknown_layer(layers, &name))
                .map_or(Ok(()), Err),
            None => Ok(()),
        };
        let mut nested: BTreeMap<usize, Vec<Override>> = BTreeMap::new();
        let mut touched: BTreeMap<(Kind, usize), Vec<Touch>> = BTreeMap::new();
        for change in changes {
            let mut found: Vec<(Kind, usize)> = Vec::new();
            for kind in Kind::ALL {
                match scope.lookup(kind, &change.entry) {
                    Lookup::One(at) => found.push((kind, at)),
                    Lookup::Many(all) => found.extend(all.into_iter().map(|at| (kind, at))),
                    Lookup::None => {}
                }
            }
            if found.is_empty()
                && let Some((head, rest)) = change.entry.split_once('/')
                && let Lookup::One(at) = lookup(&scope.objects, head)
                && scope.objects[at].value.is_placement()
            {
                let mut inner = change.clone();
                inner.entry = rest.to_string();
                nested.entry(at).or_default().push(inner);
                continue;
            }
            match found.as_slice() {
                [] => self.reject(change, format!("the prefab has no entry {}", change.entry)),
                [(kind, at)] => {
                    let checked = |issues: Issues| match issues.0.into_iter().next() {
                        Some(issue) => Err(issue.message),
                        None => Ok(()),
                    };
                    let result = match kind {
                        Kind::Mesh => Self::apply(&scope.meshes[*at].value, change)
                            .map(|v| scope.meshes[*at].value = v),
                        Kind::Object => Self::apply(&scope.objects[*at].value, change)
                            .and_then(|v| checked(rules::object(&v)).map(|()| v))
                            .and_then(|v| known(&v).map(|()| v))
                            .map(|v| scope.objects[*at].value = v),
                        Kind::Light => Self::apply(&scope.lights[*at].value, change)
                            .and_then(|v| checked(rules::light(&v)).map(|()| v))
                            .map(|v| scope.lights[*at].value = v),
                        Kind::Emitter => Self::apply(&scope.emitters[*at].value, change)
                            .and_then(|v| checked(rules::emitter(&v)).map(|()| v))
                            .map(|v| scope.emitters[*at].value = v),
                        Kind::Mover => Self::apply(&scope.movers[*at].value, change)
                            .and_then(|v| checked(rules::mover(&v)).map(|()| v))
                            .map(|v| scope.movers[*at].value = v),
                        Kind::Content => Self::apply(&scope.contents[*at].value, change)
                            .map(|v| scope.contents[*at].value = v),
                        Kind::Text => Self::apply(&scope.texts[*at].value, change)
                            .and_then(|v| checked(rules::text(&v)).map(|()| v))
                            .map(|v| scope.texts[*at].value = v),
                        Kind::Sound => {
                            let name = scope.sounds[*at].name.clone();
                            Self::apply(&scope.sounds[*at].value, change)
                                .and_then(|v| checked(rules::sound(&name, &v)).map(|()| v))
                                .map(|v| scope.sounds[*at].value = v)
                        }
                        Kind::Rig => Self::apply(&scope.rigs[*at].value, change)
                            .and_then(|v| checked(rules::rig(&v)).map(|()| v))
                            .map(|v| scope.rigs[*at].value = v),
                        Kind::Tiles => Self::apply(&scope.tiles[*at].value, change)
                            .and_then(|v| checked(rules::tiles(&v)).map(|()| v))
                            .map(|v| scope.tiles[*at].value = v),
                        Kind::Particles => Self::apply(&scope.particles[*at].value, change)
                            .and_then(|v| checked(rules::particles(&v)).map(|()| v))
                            .map(|v| scope.particles[*at].value = v),
                    };
                    match result {
                        Err(message) => self.reject(
                            change,
                            format!("{} {}: {message}", kind.word(), change.entry),
                        ),
                        Ok(()) if change.own => {
                            touched.entry((*kind, *at)).or_default().push(Touch {
                                key: change.keys[0].clone(),
                                file: change.file.clone(),
                                path: change.path.clone(),
                            });
                        }
                        Ok(()) => {}
                    }
                }
                _ => self.reject(
                    change,
                    format!(
                        "{} names more than one entry of the prefab; use its id",
                        change.entry
                    ),
                ),
            }
        }
        Frame {
            placed: Placed {
                libraries: scope.libraries.clone(),
                files: scope.files.clone(),
                ..Placed::default()
            },
            scope,
            prefix,
            own,
            nested,
            touched,
            inner: Placed::default(),
            inner_objects: BTreeMap::new(),
            next: 0,
            placing: 0,
        }
    }

    fn descend(&mut self, frame: &mut Frame, chain: &mut Vec<Step>) -> Option<Frame> {
        while frame.next < frame.scope.objects.len() {
            let at = frame.next;
            frame.next += 1;
            let item = &frame.scope.objects[at];
            let Some(raw) = &item.value.prefab else {
                continue;
            };
            if Id::parse(raw).is_ok()
                && !matches!(
                    lookup(&frame.scope.objects, raw),
                    Lookup::One(found) if frame.scope.objects[found]
                        .value
                        .prefab
                        .as_deref()
                        .is_some_and(|target| Id::parse(target).is_err())
                )
            {
                if frame.own {
                    let message = format!(
                        "object {} places {raw}, which is no placement of this {}; prefab names a prefab's path or a placement's id",
                        item.name,
                        if frame.scope.prefab {
                            "prefab"
                        } else {
                            "scene"
                        }
                    );
                    let (file, at) = (item.file.clone(), under(&item.path, &["prefab".into()]));
                    self.issue(&file, &at, code::BAD_REFERENCE, message);
                }
                continue;
            }
            let Some(path) = self.prefab_path(&frame.scope, &item.file, raw) else {
                continue;
            };
            if let Some(step) = chain.last_mut() {
                step.link = Some((item.file.clone(), under(&item.path, &["prefab".into()])));
            }
            if let Some(start) = chain.iter().position(|step| step.file == path) {
                self.chained(
                    &chain[start..],
                    &path,
                    code::PREFAB_CYCLE,
                    format!("object {} places {raw}, which places itself", item.name),
                );
                continue;
            }
            if self.text(&path).is_none() {
                continue;
            }
            if chain.len() > MAX_DEPTH {
                self.chained(
                    chain,
                    &path,
                    code::PREFAB_DEPTH,
                    format!(
                        "object {} places {raw}, which nests prefabs more than {MAX_DEPTH} deep",
                        item.name
                    ),
                );
                continue;
            }
            if self.prefabs_checked.insert(path.clone()) {
                self.unchecked.push((path.clone(), chain.clone()));
            }
            let prefab_scope = self.scope(&path, FileKind::Prefab);
            let mut all = self.overrides_of(item, frame.own);
            all.extend(frame.nested.remove(&at).unwrap_or_default());
            let prefix = frame.prefix.as_ref();
            let child = Prefix {
                key: name_key(prefix, &item.name),
                id: full_id(prefix, item.id),
                hidden: item.value.hidden || prefix.is_some_and(|p| p.hidden),
                tint: tinted(prefix.and_then(|p| p.tint), item.value.tint.as_ref()),
                recolor: recolored(
                    &item.value.recolor,
                    prefix.map_or(&BTreeMap::new(), |p| &p.recolor),
                ),
            };
            frame.placing = at;
            chain.push(Step::new(&path));
            return Some(self.enter(&prefab_scope, Some(child), &all, false));
        }
        None
    }

    fn absorb(frame: &mut Frame, result: Placed) {
        frame.inner_objects.insert(frame.placing, result.objects);
        let inner = &mut frame.inner;
        inner.meshes.extend(result.meshes);
        inner.lights.extend(result.lights);
        inner.emitters.extend(result.emitters);
        inner.movers.extend(result.movers);
        inner.contents.extend(result.contents);
        inner.texts.extend(result.texts);
        inner.sounds.extend(result.sounds);
        inner.rigs.extend(result.rigs);
        inner.particles.extend(result.particles);
        frame.placed.libraries.extend(result.libraries);
        for file in result.files {
            if !frame.placed.files.contains(&file) {
                frame.placed.files.push(file);
            }
        }
    }

    fn finish(&mut self, frame: Frame) -> Placed {
        let Frame {
            scope,
            prefix,
            own,
            mut touched,
            mut placed,
            inner,
            mut inner_objects,
            ..
        } = frame;
        let prefix = prefix.as_ref();
        let name_key = |name: &str| name_key(prefix, name);
        let full_id = |id: Option<Id>| full_id(prefix, id);
        let mut objects_out: Vec<Out<Object>> = Vec::new();
        let all_inner_objects: Vec<Out<Object>> =
            inner_objects.values().flatten().cloned().collect();
        let resolve = |this: &mut Self,
                       item_file: &Path,
                       at: &KeyPath,
                       kind: Kind,
                       reference: &str,
                       namespaced: &dyn Fn(&str) -> Option<String>,
                       touch: Option<&Touch>|
         -> Option<String> {
            let overridden = |this: &mut Self, message: String| {
                if let Some(touch) = touch.filter(|_| !own) {
                    let finding = Finding::new(
                        &touch.file,
                        touch.path.clone(),
                        Part::Key,
                        code::BAD_OVERRIDE,
                        message,
                    )
                    .related(item_file, at.clone(), Part::Value);
                    this.push(finding);
                }
            };
            match scope.lookup(kind, reference) {
                Lookup::One(index) => {
                    let names = scope.names(kind);
                    Some(name_key(names[index].0))
                }
                Lookup::Many(all) => {
                    if own {
                        let names = scope.names(kind);
                        let mut finding = Finding::new(
                            item_file,
                            at.clone(),
                            Part::Value,
                            code::AMBIGUOUS_REFERENCE,
                            format!(
                                "{reference} names {} {}s; use an id",
                                all.len(),
                                kind.word()
                            ),
                        );
                        for index in all {
                            finding = finding.related(
                                names[index].2,
                                names[index].3.clone(),
                                Part::Value,
                            );
                        }
                        this.push(finding);
                    } else {
                        overridden(
                            this,
                            format!(
                                "{reference} names {} {}s of the prefab; use an id",
                                all.len(),
                                kind.word()
                            ),
                        );
                    }
                    None
                }
                Lookup::None => {
                    if let Some(found) = namespaced(reference) {
                        return Some(found);
                    }
                    if own {
                        this.issue(
                            item_file,
                            at,
                            code::BAD_REFERENCE,
                            format!(
                                "{reference} names no {} of this {}",
                                kind.word(),
                                if scope.prefab { "prefab" } else { "scene" }
                            ),
                        );
                    } else {
                        overridden(
                            this,
                            format!("{reference} names no {} of the prefab", kind.word()),
                        );
                    }
                    None
                }
            }
        };
        let inner_object_key = |reference: &str| {
            all_inner_objects
                .iter()
                .find(|out| out.entry.id.as_deref() == Some(reference))
                .map(|out| out.entry.key.clone())
        };
        let inner_key = |list: &[(Option<String>, String)], reference: &str| {
            list.iter()
                .find(|(id, _)| id.as_deref() == Some(reference))
                .map(|(_, key)| key.clone())
        };
        let inner_meshes: Vec<(Option<String>, String)> = inner
            .meshes
            .iter()
            .map(|out| (out.entry.id.clone(), out.entry.key.clone()))
            .collect();
        let inner_contents: Vec<(Option<String>, String)> = inner
            .contents
            .iter()
            .map(|out| (out.entry.id.clone(), out.entry.key.clone()))
            .collect();
        for (at, item) in scope.objects.iter().enumerate() {
            let mut value = item.value.clone();
            let set = touched.remove(&(Kind::Object, at)).unwrap_or_default();
            if let Some(mesh) = &value.mesh {
                value.mesh = resolve(
                    self,
                    &item.file,
                    &under(&item.path, &["mesh".into()]),
                    Kind::Mesh,
                    mesh,
                    &|r| inner_key(&inner_meshes, r),
                    touch(&set, "mesh"),
                );
            }
            if let Some(parent) = &value.parent {
                value.parent = resolve(
                    self,
                    &item.file,
                    &under(&item.path, &["parent".into()]),
                    Kind::Object,
                    parent,
                    &inner_object_key,
                    touch(&set, "parent"),
                );
            } else if let Some(prefix) = prefix {
                value.parent = Some(prefix.key.clone());
            }
            if let Some(content) = &value.content {
                value.content = resolve(
                    self,
                    &item.file,
                    &under(&item.path, &["content".into()]),
                    Kind::Content,
                    content,
                    &|r| inner_key(&inner_contents, r),
                    touch(&set, "content"),
                );
            }
            if let Some(prefix) = prefix {
                value.hidden |= prefix.hidden;
                if prefix.tint.is_some() {
                    value.tint = tinted(prefix.tint, value.tint.as_ref()).map(Color::Rgb);
                }
                value.recolor = recolored(&value.recolor, &prefix.recolor);
            }
            if value.is_placement() {
                value.set.clear();
            }
            objects_out.push(Out {
                entry: Entry {
                    key: name_key(&item.name),
                    id: full_id(item.id),
                    file: item.file.clone(),
                    value,
                },
                own: own.then(|| item.path.clone()),
                at: item.path.clone(),
                set,
            });
            if let Some(children) = inner_objects.remove(&at) {
                objects_out.extend(children);
            }
        }
        for (at, item) in scope.movers.iter().enumerate() {
            let mut value = item.value.clone();
            let set = touched.remove(&(Kind::Mover, at)).unwrap_or_default();
            let mut objects = Vec::new();
            for (place, reference) in item.value.objects.iter().enumerate() {
                let at = under(&item.path, &["objects".into(), place.to_string()]);
                if let Some(found) = resolve(
                    self,
                    &item.file,
                    &at,
                    Kind::Object,
                    reference,
                    &inner_object_key,
                    touch(&set, "objects"),
                ) {
                    objects.push(found);
                }
            }
            value.objects = objects;
            placed.movers.push(Out {
                entry: Entry {
                    key: name_key(&item.name),
                    id: full_id(item.id),
                    file: item.file.clone(),
                    value,
                },
                own: own.then(|| item.path.clone()),
                at: item.path.clone(),
                set,
            });
        }
        for (at, item) in scope.sounds.iter().enumerate() {
            let mut value = item.value.clone();
            let set = touched.remove(&(Kind::Sound, at)).unwrap_or_default();
            if let Some(object) = &item.value.object {
                value.object = resolve(
                    self,
                    &item.file,
                    &under(&item.path, &["object".into()]),
                    Kind::Object,
                    object,
                    &inner_object_key,
                    touch(&set, "object"),
                );
            }
            placed.sounds.push(Out {
                entry: Entry {
                    key: name_key(&item.name),
                    id: full_id(item.id),
                    file: item.file.clone(),
                    value,
                },
                own: own.then(|| item.path.clone()),
                at: item.path.clone(),
                set,
            });
        }
        for (at, item) in scope.rigs.iter().enumerate() {
            let mut value = item.value.clone();
            let set = touched.remove(&(Kind::Rig, at)).unwrap_or_default();
            if let Some(target) = &item.value.target {
                value.target = resolve(
                    self,
                    &item.file,
                    &under(&item.path, &["target".into()]),
                    Kind::Object,
                    target,
                    &inner_object_key,
                    touch(&set, "target"),
                );
            }
            placed.rigs.push(Out {
                entry: Entry {
                    key: name_key(&item.name),
                    id: full_id(item.id),
                    file: item.file.clone(),
                    value,
                },
                own: own.then(|| item.path.clone()),
                at: item.path.clone(),
                set,
            });
        }
        for (at, item) in scope.lights.iter().enumerate() {
            let mut value = item.value.clone();
            let set = touched.remove(&(Kind::Light, at)).unwrap_or_default();
            if let Some(parent) = &item.value.parent {
                value.parent = resolve(
                    self,
                    &item.file,
                    &under(&item.path, &["parent".into()]),
                    Kind::Object,
                    parent,
                    &inner_object_key,
                    touch(&set, "parent"),
                );
            }
            placed.lights.push(Out {
                entry: Entry {
                    key: name_key(&item.name),
                    id: full_id(item.id),
                    file: item.file.clone(),
                    value,
                },
                own: own.then(|| item.path.clone()),
                at: item.path.clone(),
                set,
            });
        }
        for (at, item) in scope.particles.iter().enumerate() {
            let mut value = item.value.clone();
            let set = touched.remove(&(Kind::Particles, at)).unwrap_or_default();
            if let Some(parent) = &item.value.parent {
                value.parent = resolve(
                    self,
                    &item.file,
                    &under(&item.path, &["parent".into()]),
                    Kind::Object,
                    parent,
                    &inner_object_key,
                    touch(&set, "parent"),
                );
            } else if let Some(prefix) = prefix {
                value.parent = Some(prefix.key.clone());
            }
            placed.particles.push(Out {
                entry: Entry {
                    key: name_key(&item.name),
                    id: full_id(item.id),
                    file: item.file.clone(),
                    value,
                },
                own: own.then(|| item.path.clone()),
                at: item.path.clone(),
                set,
            });
        }
        macro_rules! plain {
            ($from:ident) => {
                for item in &scope.$from {
                    placed.$from.push(Out {
                        entry: Entry {
                            key: name_key(&item.name),
                            id: full_id(item.id),
                            file: item.file.clone(),
                            value: item.value.clone(),
                        },
                        own: own.then(|| item.path.clone()),
                        at: item.path.clone(),
                        set: Vec::new(),
                    });
                }
            };
        }
        plain!(meshes);
        plain!(emitters);
        plain!(contents);
        plain!(texts);
        placed.objects = objects_out;
        placed.meshes.extend(inner.meshes);
        placed.lights.extend(inner.lights);
        placed.emitters.extend(inner.emitters);
        placed.movers.extend(inner.movers);
        placed.contents.extend(inner.contents);
        placed.texts.extend(inner.texts);
        placed.sounds.extend(inner.sounds);
        placed.rigs.extend(inner.rigs);
        placed.particles.extend(inner.particles);
        placed
    }

    fn context(&mut self, file: &Path, kind: FileKind) -> Option<Scene> {
        if kind == FileKind::Prefab {
            self.prefabs_checked.insert(file.to_path_buf());
        }
        if self.text(file).is_none() {
            let message = format!("the {} {} is not found", kind.name(), slashed(file));
            self.push(Finding::new(
                file,
                Vec::new(),
                Part::Value,
                code::MISSING_FILE,
                message,
            ));
            return None;
        }
        let scene = self.context_in(file, kind, Vec::new());
        while let Some((prefab, chain)) = self.unchecked.pop() {
            self.context_in(&prefab, FileKind::Prefab, chain);
        }
        scene
    }

    fn context_in(&mut self, file: &Path, kind: FileKind, mut chain: Vec<Step>) -> Option<Scene> {
        self.loaded(file, kind)?;
        let scope = self.scope(file, kind);
        chain.push(Step::new(file));
        let placed = self.place(&scope, &mut chain);
        let mut materials: Vec<Item<Material>> = Vec::new();
        let mut library_files: Vec<PathBuf> = Vec::new();
        for (library, _, _) in &placed.libraries {
            if library_files.contains(library) {
                continue;
            }
            library_files.push(library.clone());
            let Some(doc) = self.load(library, FileKind::Materials) else {
                continue;
            };
            for (name, material) in doc.library {
                let path = key(&["materials", &name]);
                if let Some(first) = materials.iter().find(|item| item.name == name) {
                    if first.file != *library {
                        let finding = Finding::new(
                            library,
                            path.clone(),
                            Part::Key,
                            code::DUPLICATE_NAME,
                            format!("material {name} is also in {}", first.file.display()),
                        )
                        .related(
                            &first.file,
                            first.path.clone(),
                            Part::Key,
                        );
                        self.push(finding);
                    }
                    continue;
                }
                materials.push(Item {
                    file: library.clone(),
                    path,
                    id: material.id,
                    name,
                    value: material,
                });
            }
        }
        let mut scene = Scene {
            path: file.to_path_buf(),
            ..Scene::default()
        };
        let material = |this: &mut Self,
                        own: &Option<(PathBuf, KeyPath)>,
                        reference: &str|
         -> Option<String> {
            match lookup(&materials, reference) {
                Lookup::One(at) => Some(materials[at].name.clone()),
                _ => {
                    if let Some((file, path)) = own {
                        this.issue(
                            file,
                            path,
                            code::BAD_REFERENCE,
                            format!("{reference} names no material of the scene's libraries"),
                        );
                    }
                    None
                }
            }
        };
        if let Some(fallback) = &scope.fallback {
            let own = Some((fallback.file.clone(), fallback.path.clone()));
            scene.fallback = material(self, &own, &fallback.value);
        }
        for out in &placed.meshes {
            let mut entry = out.entry.clone();
            for (node, value) in entry.value.nodes.iter_mut() {
                if let Some(reference) = value.material.clone() {
                    let own = out.own.as_ref().map(|path| {
                        (
                            entry.file.clone(),
                            under(path, &["nodes".into(), node.clone(), "material".into()]),
                        )
                    });
                    value.material = material(self, &own, &reference);
                }
            }
            self.found_at(&entry.file, &mut entry.value.file);
            scene.meshes.push(entry);
        }
        for out in &placed.objects {
            let mut entry = out.entry.clone();
            if let Some(atlas) = entry
                .value
                .animation
                .as_mut()
                .and_then(|animation| animation.atlas.as_mut())
            {
                self.found_at(&entry.file, atlas);
            }
            if let Some(reference) = entry.value.material.clone() {
                let own = out
                    .own
                    .as_ref()
                    .map(|path| (entry.file.clone(), under(path, &["material".into()])));
                entry.value.material = material(self, &own, &reference);
            }
            let mut resolved = BTreeMap::new();
            for (node, reference) in &entry.value.materials {
                let own = out.own.as_ref().map(|path| {
                    (
                        entry.file.clone(),
                        under(path, &["materials".into(), node.clone()]),
                    )
                });
                if let Some(name) = material(self, &own, reference) {
                    resolved.insert(node.clone(), name);
                }
            }
            entry.value.materials = resolved;
            scene.objects.push(entry);
        }
        self.relations(&placed);
        scene.materials = materials
            .into_iter()
            .map(|item| Entry {
                key: item.name,
                id: item.id.map(|id| id.to_string()),
                file: item.file,
                value: item.value,
            })
            .collect();
        scene.lights = placed.lights.into_iter().map(|out| out.entry).collect();
        scene.emitters = placed.emitters.into_iter().map(|out| out.entry).collect();
        scene.movers = placed.movers.into_iter().map(|out| out.entry).collect();
        scene.contents = placed
            .contents
            .into_iter()
            .map(|out| {
                let mut entry = out.entry;
                self.found_at(&entry.file, &mut entry.value.image);
                entry
            })
            .collect();
        scene.texts = placed
            .texts
            .into_iter()
            .map(|out| {
                let mut entry = out.entry;
                self.found_at(&entry.file, &mut entry.value.font);
                entry
            })
            .collect();
        scene.sounds = placed
            .sounds
            .into_iter()
            .map(|out| {
                let mut entry = out.entry;
                self.found_at(&entry.file, &mut entry.value.file);
                entry
            })
            .collect();
        scene.rigs = placed.rigs.into_iter().map(|out| out.entry).collect();
        scene.particles = placed.particles.into_iter().map(|out| out.entry).collect();
        scene.layers = self
            .layers()
            .map(|layers| (*layers).clone())
            .unwrap_or_default();
        let dynamic = dynamics(&scene.objects, &scene.movers);
        for (entry, dynamic) in scene.objects.iter_mut().zip(dynamic) {
            entry.value.dynamic = Some(dynamic);
        }
        scene.sun = scope.sun.as_ref().map(|single| single.value.clone());
        scene.sky = scope.sky.as_ref().map(|single| {
            let mut sky = single.value.clone();
            if let Some(path) = sky.path.as_mut() {
                self.found_at(&single.file, path);
            }
            for layer in &mut sky.layer {
                if let Some(path) = layer.path.as_mut() {
                    self.found_at(&single.file, path);
                }
            }
            sky
        });
        scene.haze = scope.haze.as_ref().map(|single| single.value.clone());
        scene.flare = scope.flare.as_ref().map(|single| single.value);
        scene.motion_blur = scope.motion_blur.as_ref().map(|single| single.value);
        scene.sound_file = scope
            .sound_file
            .as_ref()
            .and_then(|single| self.rooted_from(&single.file, &single.value).ok())
            .map(PathBuf::from);
        scene.camera = scope.camera.as_ref().map(|single| single.value.clone());
        scene.trace = scope.trace.as_ref().map(|single| single.value);
        scene.shadows = scope.shadows.as_ref().map(|single| single.value);
        scene.live = scope.live.as_ref().map(|single| single.value);
        scene.plates = scope.plates.as_ref().map(|single| {
            let mut plates = single.value.clone();
            self.found_at(&single.file, &mut plates.dir);
            if let Some(proxies) = plates.proxies.as_mut() {
                self.found_at(&single.file, proxies);
            }
            plates
        });
        scene.physics = scope.physics.as_ref().map(|single| single.value.clone());
        scene.play_module = scope
            .play
            .as_ref()
            .and_then(|single| {
                let raw = single.value.module.as_deref()?;
                self.rooted_from(&single.file, raw).ok()
            })
            .map(PathBuf::from);
        scene.music = scope.music.as_ref().map(|single| single.value.clone());
        scene.files = placed.files.clone();
        scene.tiles = self.tile_layers(&scope, &chain, &mut scene.files);
        if let Some(finish) = &scope.finish {
            let mut value = finish.value.clone();
            if let Some(file) = value.file.as_mut() {
                self.found_at(&finish.file, file);
            }
            scene.finish = Some(value);
            if let Some(raw) = &finish.value.file
                && let Ok(rooted) = self.rooted_from(&finish.file, raw)
            {
                let path = PathBuf::from(rooted);
                scene.finish_file = self
                    .load(&path, FileKind::Finish)
                    .and_then(|doc| doc.finish_keys);
                scene.files.push(path);
            }
        }
        if let Some(plates) = &scope.plates {
            self.plate_proxies(&plates.file, &plates.value, &placed.objects, &mut scene);
        }
        scene.files.extend(library_files);
        Some(scene)
    }

    fn tile_layers(
        &mut self,
        scope: &Scope,
        chain: &[Step],
        files: &mut Vec<PathBuf>,
    ) -> Vec<Entry<Tiles>> {
        let mut layers = Vec::new();
        for item in &scope.tiles {
            let mut value = item.value.clone();
            let mut palette = BTreeMap::new();
            for (tile, raw) in &item.value.palette {
                let at = under(&item.path, &["palette".into(), tile.clone()]);
                let path = if Id::parse(raw).is_ok() {
                    match lookup(&scope.objects, raw) {
                        Lookup::One(found) if scope.objects[found].value.is_placement() => {
                            self.prefab_path(scope, &item.file, raw)
                        }
                        _ => {
                            self.issue(
                                &item.file,
                                &at,
                                code::BAD_REFERENCE,
                                format!(
                                    "tile layer {}: tile {tile} names {raw}, which is no placement of this scene; a palette names a prefab's path or a placement's id",
                                    item.name
                                ),
                            );
                            None
                        }
                    }
                } else {
                    self.rooted_from(&item.file, raw).ok().map(PathBuf::from)
                };
                let Some(path) = path else { continue };
                if FileKind::of(&path) == Some(FileKind::Prefab) && self.text(&path).is_some() {
                    if self.prefabs_checked.insert(path.clone()) {
                        let mut steps = chain.to_vec();
                        if let Some(step) = steps.last_mut() {
                            step.link = Some((item.file.clone(), at.clone()));
                        }
                        self.unchecked.push((path.clone(), steps));
                    }
                    if !files.contains(&path) {
                        files.push(path.clone());
                    }
                }
                palette.insert(tile.clone(), slashed(&path));
            }
            value.palette = palette;
            layers.push(Entry {
                key: item.name.clone(),
                id: item.id.map(|id| id.to_string()),
                file: item.file.clone(),
                value,
            });
        }
        layers
    }

    fn plate_proxies(
        &mut self,
        naming: &Path,
        plates: &Plates,
        placed: &[Out<Object>],
        scene: &mut Scene,
    ) {
        let Some(raw) = &plates.proxies else { return };
        let Ok(rooted) = self.rooted_from(naming, raw) else {
            return;
        };
        if !rooted.ends_with(".proxies.toml") {
            return;
        }
        let file = PathBuf::from(rooted);
        let Some(doc) = self.load(&file, FileKind::Proxies) else {
            return;
        };
        self.bound.insert(file.clone());
        scene.files.push(file.clone());
        let shown = scene.path.display().to_string();
        for (place, proxy) in &doc.proxy {
            let at = key(&["proxy", &place.to_string(), "object"]);
            let reference = proxy.object.as_str();
            let by_id: Vec<usize> = (0..scene.objects.len())
                .filter(|at| scene.objects[*at].id.as_deref() == Some(reference))
                .collect();
            let found = if by_id.is_empty() {
                (0..scene.objects.len())
                    .filter(|at| scene.objects[*at].key == reference)
                    .collect()
            } else {
                by_id
            };
            let related = |finding: Finding, index: usize| {
                finding.related(
                    &placed[index].entry.file,
                    placed[index].at.clone(),
                    Part::Value,
                )
            };
            match found.as_slice() {
                [] => self.issue(
                    &file,
                    &at,
                    code::BAD_REFERENCE,
                    format!("{reference} names no object of scene {shown}"),
                ),
                [one] => {
                    let object = &scene.objects[*one];
                    let refused = if object.value.is_placement() {
                        Some(format!(
                            "{reference} names placement {}, which has no mesh; a proxy stands in for an object with a mesh",
                            object.key
                        ))
                    } else if object.value.dynamic == Some(true) {
                        Some(format!(
                            "{reference} names object {}, which is dynamic in scene {shown} ({}); a proxy stands in for a static object",
                            object.key,
                            why_dynamic(placed, &scene.movers, *one)
                        ))
                    } else {
                        None
                    };
                    match refused {
                        Some(message) => {
                            let finding =
                                Finding::new(&file, at, Part::Value, code::BAD_REFERENCE, message);
                            self.push(related(finding, *one));
                        }
                        None => scene.proxies.push(Entry {
                            key: place.to_string(),
                            id: proxy.id.map(|id| id.to_string()),
                            file: file.clone(),
                            value: Proxy {
                                object: object.key.clone(),
                                ..proxy.clone()
                            },
                        }),
                    }
                }
                many => {
                    let mut finding = Finding::new(
                        &file,
                        at,
                        Part::Value,
                        code::AMBIGUOUS_REFERENCE,
                        format!(
                            "{reference} names {} objects of scene {shown}; use an id",
                            many.len()
                        ),
                    );
                    for index in many {
                        finding = related(finding, *index);
                    }
                    self.push(finding);
                }
            }
        }
    }

    fn relations(&mut self, placed: &Placed) {
        let objects = &placed.objects;
        let index: BTreeMap<&str, usize> = objects
            .iter()
            .enumerate()
            .map(|(at, out)| (out.entry.key.as_str(), at))
            .collect();
        for (start, out) in objects.iter().enumerate() {
            let Some(blame) = out.blame("parent") else {
                continue;
            };
            let mut at = start;
            let mut steps = 0;
            while let Some(parent) = objects[at]
                .entry
                .value
                .parent
                .as_deref()
                .and_then(|p| index.get(p))
            {
                steps += 1;
                if *parent == start || steps > objects.len() {
                    self.blamed(
                        blame,
                        code::PARENT_CYCLE,
                        format!("object {}: its parents form a cycle", out.entry.key),
                    );
                    break;
                }
                at = *parent;
            }
        }
        let mut owner: BTreeMap<&str, &str> = BTreeMap::new();
        for mover in &placed.movers {
            for object in &mover.entry.value.objects {
                if let Some(other) = owner.insert(object, &mover.entry.key)
                    && let Some(blame) = mover.blame("objects")
                {
                    self.blamed(
                        blame,
                        code::BAD_VALUE,
                        format!(
                            "object {object} is moved by movers {other} and {}; one at most",
                            mover.entry.key
                        ),
                    );
                }
            }
        }
        for (start, out) in objects.iter().enumerate() {
            let Some(path) = &out.own else { continue };
            let mut planes = out.entry.value.clip.clone();
            let mut seen = Vec::new();
            let mut at = Some(start);
            while let Some(current) = at {
                if seen.contains(&current) {
                    break;
                }
                seen.push(current);
                let object = &objects[current].entry;
                if let Some(mover) = owner
                    .get(object.key.as_str())
                    .and_then(|name| placed.movers.iter().find(|m| m.entry.key == *name))
                {
                    for plane in &mover.entry.value.clip {
                        if !planes.contains(plane) {
                            planes.push(*plane);
                        }
                    }
                }
                at = object
                    .value
                    .parent
                    .as_deref()
                    .and_then(|parent| index.get(parent).copied());
            }
            if planes.len() > 2 {
                self.issue(
                    &out.entry.file,
                    path,
                    code::BAD_VALUE,
                    format!(
                        "object {} has {} clip planes with its movers'; at most two",
                        out.entry.key,
                        planes.len()
                    ),
                );
            }
        }
        self.bodies(placed, &index, &owner);
        let mut looping: Option<&str> = None;
        for sound in &placed.sounds {
            if let Some(object) = &sound.entry.value.object
                && let Some(at) = index.get(object.as_str())
                && objects[*at].entry.value.body.is_none()
                && let Some(blame) = sound.blame("object")
            {
                self.blamed(
                    blame,
                    code::BAD_REFERENCE,
                    format!(
                        "sound {} names object {object}, which has no body",
                        sound.entry.key
                    ),
                );
            }
            if sound.entry.value.looping {
                if let Some(other) = looping {
                    let Some(blame) = sound.blame("loop") else {
                        continue;
                    };
                    self.blamed(
                        blame,
                        code::BAD_VALUE,
                        format!(
                            "sounds {other} and {} both loop; one looping sound at most",
                            sound.entry.key
                        ),
                    );
                } else {
                    looping = Some(&sound.entry.key);
                }
            }
        }
    }

    fn blamed(
        &mut self,
        (file, path, overridden): (PathBuf, KeyPath, bool),
        code: &'static str,
        message: String,
    ) {
        if overridden {
            self.push(Finding::new(
                &file,
                path,
                Part::Key,
                code::BAD_OVERRIDE,
                message,
            ));
        } else {
            self.issue(&file, &path, code, message);
        }
    }

    fn bodies(
        &mut self,
        placed: &Placed,
        index: &BTreeMap<&str, usize>,
        owner: &BTreeMap<&str, &str>,
    ) {
        let objects = &placed.objects;
        let entry = |out: &Out<Object>| (out.entry.file.clone(), out.at.clone());
        let keyed =
            |out: &Out<Object>, key: &str| (out.entry.file.clone(), under(&out.at, &[key.into()]));
        let carried = objects.iter().filter_map(|out| {
            if out.entry.value.body.is_some() {
                Some((out, "body"))
            } else if out.entry.value.character.is_some() {
                Some((out, "character"))
            } else {
                None
            }
        });
        for (body, word) in carried {
            let name = &body.entry.key;
            let mut current = body;
            let mut steps = 0;
            loop {
                let mut found = Vec::new();
                if let Some(mover) = owner
                    .get(current.entry.key.as_str())
                    .and_then(|key| placed.movers.iter().find(|mover| mover.entry.key == *key))
                {
                    let held = &current.entry.key;
                    let message = if std::ptr::eq(current, body) {
                        format!(
                            "object {name} is moved by mover {}; a {word} or a mover, not both",
                            mover.entry.key
                        )
                    } else {
                        format!(
                            "object {name} has a {word} under {} {held}, which mover {} moves; a {word} or a mover, not both",
                            if current.entry.value.is_placement() {
                                "placement"
                            } else {
                                "object"
                            },
                            mover.entry.key
                        )
                    };
                    let mut places = vec![keyed(body, word)];
                    if !std::ptr::eq(current, body) {
                        places.push(entry(current));
                    }
                    places.push((
                        mover.entry.file.clone(),
                        under(&mover.at, &["objects".into()]),
                    ));
                    found.push((
                        body.blame(word).or_else(|| mover.blame("objects")),
                        places,
                        message,
                    ));
                }
                let parent = current
                    .entry
                    .value
                    .parent
                    .as_deref()
                    .and_then(|parent| index.get(parent))
                    .map(|at| &objects[*at]);
                if let Some(parent) = parent
                    && !parent.entry.value.is_placement()
                {
                    let mut places = vec![keyed(body, word)];
                    if !std::ptr::eq(current, body) {
                        places.push(keyed(current, "parent"));
                    }
                    places.push(entry(parent));
                    found.push((
                        body.blame(word).or_else(|| current.blame("parent")),
                        places,
                        format!(
                            "object {name} has a {word} under object {}, which places no prefab; a {word} sits on a root object or under placements",
                            parent.entry.key
                        ),
                    ));
                }
                if let Some((Some((file, path, overridden)), places, message)) =
                    found.into_iter().find(|(blame, _, _)| blame.is_some())
                {
                    let (code, part) = if overridden {
                        (code::BAD_OVERRIDE, Part::Key)
                    } else {
                        (code::BAD_VALUE, Part::Value)
                    };
                    let mut finding = Finding::new(&file, path.clone(), part, code, message);
                    for (other, at) in places {
                        if other != file || at != path {
                            finding = finding.related(&other, at, Part::Value);
                        }
                    }
                    self.push(finding);
                    break;
                }
                steps += 1;
                match parent {
                    Some(parent) if steps <= objects.len() => current = parent,
                    _ => break,
                }
            }
        }
    }

    fn unique_ids(&mut self) {
        let mut seen: BTreeMap<Id, (PathBuf, KeyPath)> = BTreeMap::new();
        let files: Vec<PathBuf> = self.parsed.keys().cloned().collect();
        for file in files {
            let Some(doc) = self.parsed.get(&file).and_then(|parsed| parsed.doc.clone()) else {
                continue;
            };
            let mut ids: Vec<(Id, KeyPath)> = Vec::new();
            let list = |name: &str, place: usize| key(&[name, &place.to_string(), "id"]);
            let map = |name: &str, entry: &str| key(&[name, entry, "id"]);
            ids.extend(
                doc.mesh
                    .iter()
                    .filter_map(|(n, v)| v.id.map(|id| (id, map("mesh", n)))),
            );
            ids.extend(
                doc.object
                    .iter()
                    .filter_map(|(p, v)| v.id.map(|id| (id, list("object", *p)))),
            );
            ids.extend(
                doc.light
                    .iter()
                    .filter_map(|(p, v)| v.id.map(|id| (id, list("light", *p)))),
            );
            ids.extend(
                doc.emitter
                    .iter()
                    .filter_map(|(p, v)| v.id.map(|id| (id, list("emitter", *p)))),
            );
            ids.extend(
                doc.mover
                    .iter()
                    .filter_map(|(p, v)| v.id.map(|id| (id, list("mover", *p)))),
            );
            ids.extend(
                doc.content
                    .iter()
                    .filter_map(|(n, v)| v.id.map(|id| (id, map("content", n)))),
            );
            ids.extend(
                doc.text
                    .iter()
                    .filter_map(|(n, v)| v.id.map(|id| (id, map("text", n)))),
            );
            ids.extend(
                doc.sound
                    .iter()
                    .filter_map(|(n, v)| v.id.map(|id| (id, map("sound", n)))),
            );
            ids.extend(
                doc.rig
                    .iter()
                    .filter_map(|(p, v)| v.id.map(|id| (id, list("rig", *p)))),
            );
            ids.extend(
                doc.tiles
                    .iter()
                    .filter_map(|(p, v)| v.id.map(|id| (id, list("tiles", *p)))),
            );
            ids.extend(
                doc.particles
                    .iter()
                    .filter_map(|(p, v)| v.id.map(|id| (id, list("particles", *p)))),
            );
            ids.extend(
                doc.library
                    .iter()
                    .filter_map(|(n, v)| v.id.map(|id| (id, map("materials", n)))),
            );
            ids.extend(
                doc.proxy
                    .iter()
                    .filter_map(|(p, v)| v.id.map(|id| (id, list("proxy", *p)))),
            );
            for (id, path) in ids {
                match seen.get(&id) {
                    Some((first, first_path)) => {
                        let finding = Finding::new(
                            &file,
                            path,
                            Part::Value,
                            code::DUPLICATE_ID,
                            format!(
                                "id {id} is used twice; {} has it already",
                                if *first == file {
                                    "this file".to_string()
                                } else {
                                    first.display().to_string()
                                }
                            ),
                        )
                        .related(first, first_path.clone(), Part::Value);
                        self.push(finding);
                    }
                    None => {
                        seen.insert(id, (file.clone(), path));
                    }
                }
            }
        }
    }

    fn proxies(&mut self) {
        let files: Vec<PathBuf> = self
            .parsed
            .iter()
            .filter(|(file, parsed)| {
                parsed.kind == FileKind::Proxies && !self.bound.contains(*file)
            })
            .map(|(file, _)| file.clone())
            .collect();
        let mut objects: Vec<(Option<Id>, String)> = Vec::new();
        for parsed in self.parsed.values() {
            if let Some(doc) = &parsed.doc {
                objects.extend(
                    doc.object
                        .iter()
                        .map(|(_, object)| (object.id, object.name.clone())),
                );
            }
        }
        for file in files {
            let Some(doc) = self.parsed.get(&file).and_then(|parsed| parsed.doc.clone()) else {
                continue;
            };
            for (place, proxy) in &doc.proxy {
                let by_id = Id::parse(&proxy.object)
                    .ok()
                    .is_some_and(|id| objects.iter().any(|(other, _)| *other == Some(id)));
                let named = objects
                    .iter()
                    .filter(|(_, name)| *name == proxy.object)
                    .count();
                let at = key(&["proxy", &place.to_string(), "object"]);
                if by_id || named == 1 {
                    continue;
                }
                if named > 1 {
                    self.issue(
                        &file,
                        &at,
                        code::AMBIGUOUS_REFERENCE,
                        format!(
                            "{} names {named} objects of the project; use an id",
                            proxy.object
                        ),
                    );
                } else {
                    self.issue(
                        &file,
                        &at,
                        code::BAD_REFERENCE,
                        format!("{} names no object of the project", proxy.object),
                    );
                }
            }
        }
    }

    fn all(&mut self) -> Vec<Diagnostic> {
        let mut files: Vec<PathBuf> = self.project.texts.keys().cloned().collect();
        for file in self.overlay.keys() {
            if !files.contains(file) && FileKind::of(file).is_some() {
                files.push(file.clone());
            }
        }
        for file in &files {
            if let Some(kind) = FileKind::of(file) {
                self.load(file, kind);
            }
        }
        let scenes: Vec<PathBuf> = files
            .iter()
            .filter(|file| FileKind::of(file) == Some(FileKind::Scene))
            .cloned()
            .collect();
        let mut included: BTreeSet<PathBuf> = BTreeSet::new();
        for scene in &scenes {
            if let Some(doc) = self.parsed.get(scene).and_then(|parsed| parsed.doc.clone()) {
                for (_, raw) in &doc.include {
                    if let Ok(rooted) = self.rooted_from(scene, raw) {
                        included.insert(PathBuf::from(rooted));
                    }
                }
            }
        }
        for scene in &scenes {
            if !included.contains(scene) {
                self.context(scene, FileKind::Scene);
            }
        }
        for scene in &scenes {
            let seen = self
                .scopes
                .values()
                .any(|scope| scope.files.contains(scene));
            if !seen {
                self.context(scene, FileKind::Scene);
            }
        }
        let prefabs: Vec<PathBuf> = files
            .iter()
            .filter(|file| FileKind::of(file) == Some(FileKind::Prefab))
            .cloned()
            .collect();
        for prefab in prefabs {
            if !self.prefabs_checked.contains(&prefab) {
                self.context(&prefab, FileKind::Prefab);
            }
        }
        self.proxies();
        self.unique_ids();
        self.report()
    }

    fn report(&self) -> Vec<Diagnostic> {
        let mut diagnostics = read::report(&self.findings, &self.parsed);
        diagnostics.sort_by(|a, b| {
            (&a.file, a.line, a.column, a.code).cmp(&(&b.file, b.line, b.column, b.code))
        });
        diagnostics
    }
}
