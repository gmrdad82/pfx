use std::collections::BTreeMap;
use std::ops::Range;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use toml::Spanned;
use toml::de::{DeTable, DeValue, ValueDeserializer};

use crate::code;
use crate::migrate::{self, Migration};
use crate::spans::{KeyPath, Part, Spans};
use crate::types::{
    Camera, Content, Emitter, FORMAT, File, FileKind, Finish, FinishKeys, Flare, Haze, Light,
    LiveSettings, Material, MaterialsFile, Mesh, MotionBlur, Mover, Object, Particles, Physics,
    Plates, Play, PrefabFile, ProjectFile, ProjectTable, ProxiesFile, Proxy, Rig, SceneFile,
    Shadows, Sky, Sound, Sounds, Sun, Text, Tiles, Trace, Tunable,
};
use crate::{Diagnostic, Location, Severity};

#[derive(Clone, Debug)]
pub(crate) struct Finding {
    pub(crate) file: PathBuf,
    pub(crate) path: KeyPath,
    pub(crate) part: Part,
    pub(crate) span: Option<Range<usize>>,
    pub(crate) severity: Severity,
    pub(crate) code: &'static str,
    pub(crate) message: String,
    pub(crate) related: Vec<(PathBuf, KeyPath, Part)>,
}

impl Finding {
    pub(crate) fn new(
        file: &Path,
        path: KeyPath,
        part: Part,
        code: &'static str,
        message: impl Into<String>,
    ) -> Self {
        Self {
            file: file.to_path_buf(),
            path,
            part,
            span: None,
            severity: if code == code::MISSING_ID || code == code::OLD_FORMAT {
                Severity::Warning
            } else {
                Severity::Error
            },
            code,
            message: message.into(),
            related: Vec::new(),
        }
    }

    pub(crate) fn related(mut self, file: &Path, path: KeyPath, part: Part) -> Self {
        self.related.push((file.to_path_buf(), path, part));
        self
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Doc {
    pub(crate) include: Vec<(usize, String)>,
    pub(crate) materials: Vec<(usize, String)>,
    pub(crate) fallback: Option<String>,
    pub(crate) mesh: Vec<(String, Mesh)>,
    pub(crate) object: Vec<(usize, Object)>,
    pub(crate) light: Vec<(usize, Light)>,
    pub(crate) emitter: Vec<(usize, Emitter)>,
    pub(crate) mover: Vec<(usize, Mover)>,
    pub(crate) content: Vec<(String, Content)>,
    pub(crate) text: Vec<(String, Text)>,
    pub(crate) sound: Vec<(String, Sound)>,
    pub(crate) sound_file: Option<String>,
    pub(crate) rig: Vec<(usize, Rig)>,
    pub(crate) tiles: Vec<(usize, Tiles)>,
    pub(crate) particles: Vec<(usize, Particles)>,
    pub(crate) sun: Option<Sun>,
    pub(crate) sky: Option<Sky>,
    pub(crate) haze: Option<Haze>,
    pub(crate) camera: Option<Camera>,
    pub(crate) finish: Option<Finish>,
    pub(crate) flare: Option<Flare>,
    pub(crate) motion_blur: Option<MotionBlur>,
    pub(crate) trace: Option<Trace>,
    pub(crate) shadows: Option<Shadows>,
    pub(crate) live: Option<LiveSettings>,
    pub(crate) plates: Option<Plates>,
    pub(crate) physics: Option<Physics>,
    pub(crate) play: Option<Play>,
    pub(crate) music: Option<toml::Table>,
    pub(crate) library: Vec<(String, Material)>,
    pub(crate) proxy: Vec<(usize, Proxy)>,
    pub(crate) project: Option<ProjectTable>,
    pub(crate) tunables: Vec<(String, Tunable)>,
    pub(crate) layers: Option<BTreeMap<String, Vec<String>>>,
    pub(crate) layers_unread: bool,
    pub(crate) finish_keys: Option<FinishKeys>,
    pub(crate) authoring: Option<toml::Table>,
    pub(crate) complete: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct Origin {
    pub(crate) spans: Spans,
    pub(crate) bodies: BTreeMap<usize, String>,
}

#[derive(Clone, Debug)]
pub(crate) struct Parsed {
    pub(crate) kind: FileKind,
    pub(crate) format: u32,
    pub(crate) text: String,
    pub(crate) spans: Spans,
    pub(crate) origin: Option<Origin>,
    pub(crate) doc: Option<Doc>,
    pub(crate) findings: Vec<Finding>,
}

impl Parsed {
    pub(crate) fn translate(&self, path: &[String]) -> KeyPath {
        let Some(origin) = &self.origin else {
            return path.to_vec();
        };
        match path {
            [list, place, key, rest @ ..] if list == "object" && key == "pick" => {
                let mut out = vec![list.clone(), place.clone(), "id".to_string()];
                out.extend(rest.iter().cloned());
                out
            }
            [list, place, key, rest @ ..] if list == "object" && key == "body" => {
                match place
                    .parse::<usize>()
                    .ok()
                    .and_then(|at| origin.bodies.get(&at))
                {
                    Some(name) => {
                        let mut out = vec!["body".to_string(), name.clone()];
                        out.extend(rest.iter().cloned());
                        out
                    }
                    None => path.to_vec(),
                }
            }
            [list, place, key] if key == "id" && !origin.spans.has(path) => {
                vec![list.clone(), place.clone()]
            }
            _ => path.to_vec(),
        }
    }

    pub(crate) fn shown_spans(&self) -> &Spans {
        self.origin
            .as_ref()
            .map_or(&self.spans, |origin| &origin.spans)
    }

    pub(crate) fn locate(&self, path: &[String], part: Part) -> Range<usize> {
        self.shown_spans().span(&self.translate(path), part)
    }

    pub(crate) fn file(&self) -> Option<File> {
        let doc = self.doc.as_ref()?;
        if !doc.complete {
            return None;
        }
        let format = Some(FORMAT);
        let authoring = doc.authoring.clone();
        Some(match self.kind {
            FileKind::Project => File::Project(ProjectFile {
                format,
                project: doc.project.clone()?,
                tunables: doc.tunables.iter().cloned().collect(),
                layers: doc.layers.clone().unwrap_or_default(),
                authoring,
            }),
            FileKind::Scene => File::Scene(SceneFile {
                format,
                include: values(&doc.include),
                materials: values(&doc.materials),
                fallback: doc.fallback.clone(),
                mesh: doc.mesh.iter().cloned().collect(),
                object: values(&doc.object),
                light: values(&doc.light),
                emitter: values(&doc.emitter),
                mover: values(&doc.mover),
                content: doc.content.iter().cloned().collect(),
                text: doc.text.iter().cloned().collect(),
                sound: Sounds {
                    file: doc.sound_file.clone(),
                    table: doc.sound.iter().cloned().collect(),
                },
                rig: values(&doc.rig),
                tiles: values(&doc.tiles),
                particles: values(&doc.particles),
                sun: doc.sun.clone(),
                sky: doc.sky.clone(),
                haze: doc.haze.clone(),
                camera: doc.camera.clone(),
                finish: doc.finish.clone(),
                flare: doc.flare,
                motion_blur: doc.motion_blur,
                trace: doc.trace,
                shadows: doc.shadows,
                live: doc.live,
                plates: doc.plates.clone(),
                physics: doc.physics.clone(),
                play: doc.play.clone(),
                music: doc.music.clone(),
                authoring,
            }),
            FileKind::Prefab => File::Prefab(PrefabFile {
                format,
                include: values(&doc.include),
                materials: values(&doc.materials),
                mesh: doc.mesh.iter().cloned().collect(),
                object: values(&doc.object),
                light: values(&doc.light),
                emitter: values(&doc.emitter),
                mover: values(&doc.mover),
                content: doc.content.iter().cloned().collect(),
                text: doc.text.iter().cloned().collect(),
                sound: doc.sound.iter().cloned().collect(),
                rig: values(&doc.rig),
                particles: values(&doc.particles),
                authoring,
            }),
            FileKind::Materials => File::Materials(MaterialsFile {
                format,
                materials: doc.library.iter().cloned().collect(),
                authoring,
            }),
            FileKind::Proxies => File::Proxies(ProxiesFile {
                format,
                proxy: values(&doc.proxy),
                authoring,
            }),
            FileKind::Finish => File::Finish(doc.finish_keys.clone()?),
        })
    }
}

fn values<T: Clone>(items: &[(usize, T)]) -> Vec<T> {
    items.iter().map(|(_, value)| value.clone()).collect()
}

pub(crate) fn keys(kind: FileKind) -> &'static [&'static str] {
    match kind {
        FileKind::Project => &["format", "project", "tunables", "layers", "authoring"],
        FileKind::Scene => &[
            "format",
            "include",
            "materials",
            "fallback",
            "mesh",
            "object",
            "light",
            "emitter",
            "mover",
            "content",
            "text",
            "sound",
            "rig",
            "tiles",
            "particles",
            "sun",
            "sky",
            "haze",
            "camera",
            "finish",
            "flare",
            "motion_blur",
            "trace",
            "shadows",
            "live",
            "plates",
            "physics",
            "play",
            "music",
            "authoring",
        ],
        FileKind::Prefab => &[
            "format",
            "include",
            "materials",
            "mesh",
            "object",
            "light",
            "emitter",
            "mover",
            "content",
            "text",
            "sound",
            "rig",
            "particles",
            "authoring",
        ],
        FileKind::Materials => &["format", "materials", "authoring"],
        FileKind::Proxies => &["format", "proxy", "authoring"],
        FileKind::Finish => FinishKeys::KEYS,
    }
}

pub(crate) fn allowed(keys: &[&str]) -> String {
    keys.iter()
        .map(|key| format!("'{key}'"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn classify(message: &str) -> &'static str {
    if message.starts_with("unknown field") {
        code::UNKNOWN_KEY
    } else if message.starts_with("missing field") {
        code::MISSING_KEY
    } else if message.starts_with("unknown variant") {
        code::BAD_VALUE
    } else if message.starts_with("an id ") {
        code::BAD_ID
    } else if message.starts_with("invalid") || message.contains("expected") {
        code::BAD_TYPE
    } else {
        code::BAD_VALUE
    }
}

struct Reader<'a> {
    file: &'a Path,
    spans: &'a Spans,
    findings: Vec<Finding>,
    complete: bool,
}

impl Reader<'_> {
    fn error_at(
        &mut self,
        span: Range<usize>,
        fallback: &[String],
        code: &'static str,
        message: String,
    ) {
        let (path, part) = self
            .spans
            .path_at(&span)
            .unwrap_or_else(|| (fallback.to_vec(), Part::Value));
        self.findings
            .push(Finding::new(self.file, path, part, code, message));
        self.complete = false;
    }

    fn typed<T: DeserializeOwned>(
        &mut self,
        value: &Spanned<DeValue<'_>>,
        path: &[String],
    ) -> Option<T> {
        match T::deserialize(ValueDeserializer::from(value.clone())) {
            Ok(typed) => Some(typed),
            Err(error) => {
                let message = error.message().trim().replace('`', "'");
                let code = classify(&message);
                let message = if code == code::UNKNOWN_KEY {
                    message.replace("unknown field", "unknown key")
                } else if code == code::MISSING_KEY {
                    message.replace("missing field", "missing key")
                } else {
                    message
                };
                let span = error.span().unwrap_or_else(|| value.span());
                self.error_at(span, path, code, message);
                None
            }
        }
    }

    fn list<T: DeserializeOwned>(
        &mut self,
        key: &str,
        value: &Spanned<DeValue<'_>>,
    ) -> Vec<(usize, T)> {
        let path = vec![key.to_string()];
        let DeValue::Array(items) = value.get_ref() else {
            self.error_at(
                value.span(),
                &path,
                code::BAD_TYPE,
                format!("{key} is a list of [[{key}]] tables"),
            );
            return Vec::new();
        };
        let mut out = Vec::new();
        for (place, item) in items.iter().enumerate() {
            let mut entry = path.clone();
            entry.push(place.to_string());
            if let Some(typed) = self.typed(item, &entry) {
                out.push((place, typed));
            }
        }
        out
    }

    fn map<T: DeserializeOwned>(
        &mut self,
        key: &str,
        value: &Spanned<DeValue<'_>>,
    ) -> Vec<(String, T)> {
        let path = vec![key.to_string()];
        let DeValue::Table(table) = value.get_ref() else {
            self.error_at(
                value.span(),
                &path,
                code::BAD_TYPE,
                format!("{key} is a table of [{key}.<name>] tables"),
            );
            return Vec::new();
        };
        let mut out = Vec::new();
        for (name, item) in table.iter() {
            let mut entry = path.clone();
            entry.push(name.get_ref().to_string());
            if let Some(typed) = self.typed(item, &entry) {
                out.push((name.get_ref().to_string(), typed));
            }
        }
        out
    }

    fn strings(&mut self, key: &str, value: &Spanned<DeValue<'_>>) -> Vec<(usize, String)> {
        let path = vec![key.to_string()];
        let Some(items) = self.typed::<Vec<String>>(value, &path) else {
            return Vec::new();
        };
        items.into_iter().enumerate().collect()
    }
}

pub(crate) struct Context<'a> {
    pub(crate) root: &'a Path,
}

fn format_of(table: &DeTable<'_>) -> Result<u32, (Range<usize>, String)> {
    let Some(value) = table.get("format") else {
        return Ok(0);
    };
    match value.get_ref() {
        DeValue::Integer(integer) => integer.as_str().parse::<u32>().map_err(|_| {
            (
                value.span(),
                "format is a whole number, such as format = 1".to_string(),
            )
        }),
        _ => Err((
            value.span(),
            "format is a whole number, such as format = 1".to_string(),
        )),
    }
}

pub(crate) fn parse(file: &Path, text: &str, kind: FileKind, context: &Context<'_>) -> Parsed {
    let mut parsed = Parsed {
        kind,
        format: FORMAT,
        text: text.to_string(),
        spans: Spans::new(text, None),
        origin: None,
        doc: None,
        findings: Vec::new(),
    };
    let table = match DeTable::parse(text) {
        Ok(table) => table,
        Err(error) => {
            let mut finding = Finding::new(
                file,
                Vec::new(),
                Part::Value,
                code::SYNTAX,
                error.message().trim().replace('`', "'"),
            );
            finding.span = error.span();
            parsed.findings.push(finding);
            return parsed;
        }
    };
    let spans = Spans::new(text, Some(&table));
    if kind == FileKind::Finish {
        parsed.spans = spans.clone();
        let mut reader = Reader {
            file,
            spans: &spans,
            findings: Vec::new(),
            complete: true,
        };
        let root = Spanned::new(table.span(), DeValue::Table(table.get_ref().clone()));
        let keys = reader.typed::<FinishKeys>(&root, &[]);
        parsed.findings.extend(reader.findings);
        parsed.doc = Some(Doc {
            finish_keys: keys,
            complete: reader.complete,
            ..Doc::default()
        });
        return parsed;
    }
    let format = match format_of(table.get_ref()) {
        Ok(format) => format,
        Err((span, message)) => {
            let mut finding = Finding::new(
                file,
                vec!["format".into()],
                Part::Value,
                code::BAD_TYPE,
                message,
            );
            finding.span = Some(span);
            parsed.findings.push(finding);
            parsed.spans = spans;
            return parsed;
        }
    };
    parsed.format = format;
    if format > FORMAT {
        parsed.findings.push(Finding::new(
            file,
            vec!["format".into()],
            Part::Value,
            code::NEWER_FORMAT,
            format!("format = {format} is newer than this reader, which knows format {FORMAT}"),
        ));
        parsed.spans = spans;
        return parsed;
    }
    if format < FORMAT {
        let Migration {
            text: migrated,
            bodies,
            problems,
        } = match migrate::migrate(text, file, context.root, kind, false) {
            Ok(migration) => migration,
            Err(problems) => {
                for problem in problems {
                    parsed.findings.push(Finding::new(
                        file,
                        problem.path,
                        Part::Value,
                        problem.code,
                        problem.message,
                    ));
                }
                parsed.spans = spans;
                return parsed;
            }
        };
        parsed.findings.push(Finding::new(
            file,
            Vec::new(),
            Part::Value,
            code::OLD_FORMAT,
            format!("this file is format {format}; it is read as format {FORMAT} and its migration rewrites it"),
        ));
        for problem in problems {
            parsed.findings.push(Finding::new(
                file,
                problem.path,
                Part::Value,
                problem.code,
                problem.message,
            ));
        }
        let mut again = parse(file, &migrated, kind, context);
        again.format = format;
        again.origin = Some(Origin { spans, bodies });
        again.text = text.to_string();
        let translated: Vec<Finding> = again
            .findings
            .drain(..)
            .map(|mut finding| {
                if let Some(span) = finding.span.take()
                    && let Some((path, part)) = again.spans.path_at(&span)
                {
                    finding.path = path;
                    finding.part = part;
                }
                finding
            })
            .collect();
        parsed.findings.extend(translated);
        again.findings = parsed.findings;
        return again;
    }
    parsed.spans = spans.clone();
    let mut reader = Reader {
        file,
        spans: &spans,
        findings: Vec::new(),
        complete: true,
    };
    let mut doc = Doc::default();
    let allowed_keys = keys(kind);
    for (key, value) in table.get_ref().iter() {
        let name = key.get_ref().as_ref();
        if !allowed_keys.contains(&name) {
            reader.error_at(
                key.span(),
                &[name.to_string()],
                code::UNKNOWN_KEY,
                format!(
                    "unknown key '{name}', expected one of {}",
                    allowed(allowed_keys)
                ),
            );
            continue;
        }
        let path = vec![name.to_string()];
        match (kind, name) {
            (_, "format") => {}
            (_, "authoring") => doc.authoring = reader.typed(value, &path),
            (FileKind::Project, "project") => doc.project = reader.typed(value, &path),
            (FileKind::Project, "tunables") => {
                doc.tunables = reader.map(name, value);
                for (_, tunable) in &mut doc.tunables {
                    tunable.settle();
                }
            }
            (FileKind::Project, "layers") => {
                doc.layers = reader.typed(value, &path);
                doc.layers_unread = doc.layers.is_none();
            }
            (FileKind::Materials, "materials") => doc.library = reader.map(name, value),
            (FileKind::Proxies, "proxy") => doc.proxy = reader.list(name, value),
            (_, "include") => doc.include = reader.strings(name, value),
            (_, "materials") => doc.materials = reader.strings(name, value),
            (_, "fallback") => doc.fallback = reader.typed(value, &path),
            (_, "mesh") => doc.mesh = reader.map(name, value),
            (_, "object") => doc.object = reader.list(name, value),
            (_, "light") => doc.light = reader.list(name, value),
            (_, "emitter") => doc.emitter = reader.list(name, value),
            (_, "mover") => doc.mover = reader.list(name, value),
            (_, "content") => doc.content = reader.map(name, value),
            (_, "text") => doc.text = reader.map(name, value),
            (FileKind::Scene, "sound") if matches!(value.get_ref(), DeValue::String(_)) => {
                doc.sound_file = reader.typed(value, &path)
            }
            (_, "sound") => doc.sound = reader.map(name, value),
            (_, "particles") => doc.particles = reader.list(name, value),
            (_, "rig") => doc.rig = reader.list(name, value),
            (FileKind::Scene, "tiles") => doc.tiles = reader.list(name, value),
            (_, "sun") => doc.sun = reader.typed(value, &path),
            (_, "sky") => doc.sky = reader.typed(value, &path),
            (_, "haze") => doc.haze = reader.typed(value, &path),
            (_, "camera") => doc.camera = reader.typed(value, &path),
            (_, "finish") => doc.finish = reader.typed(value, &path),
            (FileKind::Scene, "flare") => doc.flare = reader.typed(value, &path),
            (FileKind::Scene, "motion_blur") => doc.motion_blur = reader.typed(value, &path),
            (_, "trace") => doc.trace = reader.typed(value, &path),
            (FileKind::Scene, "shadows") => doc.shadows = reader.typed(value, &path),
            (FileKind::Scene, "live") => doc.live = reader.typed(value, &path),
            (_, "plates") => doc.plates = reader.typed(value, &path),
            (_, "physics") => doc.physics = reader.typed(value, &path),
            (FileKind::Scene, "play") => doc.play = reader.typed(value, &path),
            (FileKind::Scene, "music") => doc.music = reader.typed(value, &path),
            _ => {}
        }
    }
    if kind == FileKind::Project && doc.project.is_none() && reader.complete {
        reader.findings.push(Finding::new(
            file,
            Vec::new(),
            Part::Value,
            code::MISSING_KEY,
            "missing key 'project': a project file names its project in [project]",
        ));
        reader.complete = false;
    }
    doc.complete = reader.complete;
    parsed.findings.extend(reader.findings);
    parsed.doc = Some(doc);
    parsed
}

pub(crate) fn plain(text: &str, kind: FileKind) -> Parsed {
    let table = DeTable::parse(text).ok();
    Parsed {
        kind,
        format: 0,
        text: text.to_string(),
        spans: Spans::new(text, table.as_ref()),
        origin: None,
        doc: None,
        findings: Vec::new(),
    }
}

pub(crate) fn report<P: std::ops::Deref<Target = Parsed>>(
    findings: &[Finding],
    parsed: &BTreeMap<PathBuf, P>,
) -> Vec<Diagnostic> {
    let mut out: Vec<Diagnostic> = Vec::new();
    for finding in findings {
        let place =
            |file: &Path, path: &[String], part: Part, span: Option<Range<usize>>| match parsed
                .get(file)
            {
                Some(parsed) => {
                    let range = span.unwrap_or_else(|| parsed.locate(path, part));
                    let spans = parsed.shown_spans();
                    let (line, column) = spans.position(range.start);
                    let (end_line, end_column) = spans.position(range.end);
                    let key = crate::spans::dotted(path, &parsed.spans);
                    (line, column, end_line, end_column, key)
                }
                None => (1, 1, 1, 1, path.join(".")),
            };
        let (line, column, end_line, end_column, key) = place(
            &finding.file,
            &finding.path,
            finding.part,
            finding.span.clone(),
        );
        let related = finding
            .related
            .iter()
            .map(|(file, path, part)| {
                let (line, column, _, _, _) = place(file, path, *part, None);
                Location {
                    file: file.clone(),
                    line,
                    column,
                }
            })
            .collect();
        let diagnostic = Diagnostic {
            file: finding.file.clone(),
            line,
            column,
            end_line,
            end_column,
            severity: finding.severity,
            code: finding.code,
            key,
            message: finding.message.clone(),
            related,
        };
        if !out.contains(&diagnostic) {
            out.push(diagnostic);
        }
    }
    out
}
