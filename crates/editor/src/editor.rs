use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use pfx_editor_doc::{Changed, Disk, Files, History};
use pfx_editor_toml::{Diagnostic, Source};
use pfx_editor_viewport::{Event, ScenePatch, Viewport};
use pfx_load::scene::{Patch, PatchGroup, Scene, SceneEdit, Value};
use pfx_play::{Edit, Pending, PlaySession, Tunables};

use crate::check::{SceneCheck, worded};
use crate::inspect::{self, Change, Raw, Row};
use crate::level::{Level, labelled};
use crate::log::Log;
use crate::outline::{Item, Outline};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Play {
    Stopped,
    Playing,
    Paused,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    Play,
    Pause,
    Resume,
    Step,
    Stop,
    Record,
    Open(PathBuf),
    Keep,
    World(Change),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Begin,
    Keep,
    End,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layout {
    pub outline: BTreeMap<Item, egui::Rect>,
    pub rows: BTreeMap<String, egui::Rect>,
    pub buttons: BTreeMap<&'static str, egui::Rect>,
    pub viewport: Option<egui::Rect>,
    pub source: Option<egui::Rect>,
    pub version: Option<egui::Rect>,
}

impl Layout {
    pub fn find(&self, name: &str) -> Option<[f32; 2]> {
        let centre = |rect: &egui::Rect| [rect.center().x, rect.center().y];
        if let Some(rest) = name.strip_prefix("row:") {
            return self.rows.get(rest).map(centre);
        }
        if let Some(rest) = name.strip_prefix("button:") {
            return self.buttons.get(rest).map(centre);
        }
        for (prefix, rect) in [("viewport", self.viewport), ("source", self.source)] {
            let Some(rest) = name.strip_prefix(prefix) else {
                continue;
            };
            let rect = rect?;
            let rest = rest.trim_start_matches(':');
            if rest.is_empty() {
                return Some(centre(&rect));
            }
            let (x, y) = rest.split_once(',')?;
            let (x, y) = (x.parse::<f32>().ok()?, y.parse::<f32>().ok()?);
            return Some([
                rect.left() + rect.width() * x,
                rect.top() + rect.height() * y,
            ]);
        }
        self.outline
            .iter()
            .find(|(item, _)| item.key() == name)
            .map(|(_, rect)| centre(rect))
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Held {
    label: String,
    start: BTreeMap<PathBuf, String>,
}

pub struct Editor {
    pub edit: SceneEdit,
    pub files: Files,
    pub history: History,
    pub viewport: Box<Viewport>,
    pub sources: BTreeMap<PathBuf, Source>,
    pub problems: Vec<Diagnostic>,
    pub outline: Outline,
    pub raw: Raw,
    pub rows: Vec<Row>,
    pub selection: Option<Item>,
    pub trace: bool,
    pub trace_samples: u32,
    pub play: Play,
    pub log: Log,
    pub app: String,
    pub pgpu: Option<String>,
    pub muted: bool,
    pub sound_available: bool,
    pub sound: Option<String>,
    pub screen: egui::Rect,
    pub texture: Option<egui::TextureId>,
    pub revision: u64,
    pub layout: Layout,
    pub texts: BTreeMap<String, String>,
    pub saves: Vec<PathBuf>,
    pub outside: bool,
    pub project: Option<pfx_project::Project>,
    pub tunables: Tunables,
    pub live: BTreeMap<String, Value>,
    pub level: Level,
    tunables_error: Option<String>,
    held: Option<Held>,
    requests: Vec<Request>,
}

pub fn frame_aspect(
    project: &pfx_project::Project,
    device: pfx_gpu::screens::Device,
) -> Result<Option<f32>, String> {
    let Some(table) = project.screen() else {
        return Ok(None);
    };
    let policy = pfx_gpu::screens::ScreenPolicy::from_table(table)
        .map_err(|error| format!("project.toml: [authoring.pfx.screen]: {error}"))?;
    Ok(policy
        .aspects(device)
        .first()
        .map(|aspect| aspect.ratio() as f32))
}

pub fn version_line(name: &str, version: &str) -> String {
    format!("{name} v{}", version.trim_start_matches('v'))
}

fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

impl Editor {
    pub fn open(path: impl AsRef<Path>) -> Result<Editor, String> {
        let path = absolute(path.as_ref());
        let edit = SceneEdit::open(&path).map_err(|error| error.to_string())?;
        let mut files = Files::new();
        for file in edit.files() {
            files.load(file).map_err(|error| error.to_string())?;
        }
        let viewport = Box::new(Viewport::open("pfx edit viewport", &path));
        let mut editor = Editor {
            outline: Outline::default(),
            raw: Raw::default(),
            rows: Vec::new(),
            edit,
            files,
            history: History::default(),
            viewport,
            sources: BTreeMap::new(),
            problems: Vec::new(),
            selection: None,
            trace: false,
            trace_samples: 0,
            play: Play::Stopped,
            log: Log::default(),
            app: version_line("pfx", env!("CARGO_PKG_VERSION")),
            pgpu: None,
            muted: false,
            sound_available: false,
            sound: None,
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1.0, 1.0)),
            texture: None,
            revision: 0,
            layout: Layout::default(),
            texts: BTreeMap::new(),
            saves: Vec::new(),
            outside: false,
            project: None,
            tunables: Tunables::default(),
            live: BTreeMap::new(),
            level: Level::new(),
            tunables_error: None,
            held: None,
            requests: Vec::new(),
        };
        editor.sync_files();
        editor.refresh();
        editor.revision = 0;
        let name = editor.name();
        editor.log.info(format!(
            "opened {name}: {} objects, {} materials, {} lights",
            editor.scene().objects.len(),
            editor.scene().library.len(),
            editor.scene().lights.len() + editor.scene().emitters.len()
        ));
        let root = editor.path().to_path_buf();
        for problem in editor.problems.clone() {
            editor.log.warn(worded(&problem, &root));
        }
        Ok(editor)
    }

    pub fn scene(&self) -> &Scene {
        self.edit.scene()
    }

    pub fn path(&self) -> &Path {
        self.edit.path()
    }

    pub fn project_root(&self) -> &Path {
        self.edit.project().root()
    }

    pub fn name(&self) -> String {
        self.path()
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    pub fn files(&self) -> Vec<PathBuf> {
        let mut files: Vec<PathBuf> = self.edit.files().map(Path::to_path_buf).collect();
        if let Some(file) = Tunables::find(self.project_root())
            && !files.contains(&file)
        {
            files.push(file);
        }
        files
    }

    fn load_tunables(&mut self) {
        let loaded = match Tunables::find(self.project_root()) {
            Some(file) => match self.files.text(&file) {
                Some(text) => Tunables::parse(text, &file),
                None => Tunables::open(&file),
            },
            None => Ok(Tunables::default()),
        };
        match loaded {
            Ok(tunables) => {
                self.tunables = tunables;
                self.tunables_error = None;
            }
            Err(error) => {
                if self.tunables_error.as_ref() != Some(&error) {
                    self.log.error(format!("tunables: {error}"));
                }
                self.tunables_error = Some(error);
            }
        }
    }

    fn rows_of(&self, item: &Item) -> Vec<Row> {
        let mut rows = inspect::rows(self.edit.scene(), &self.raw, &self.tunables, item);
        inspect::overlay(&mut rows, &self.live);
        rows
    }

    pub fn refresh(&mut self) {
        self.load_tunables();
        self.outline = Outline::of(self.edit.scene(), &self.files(), &self.tunables);
        if let Some(project) = &self.project {
            self.outline.add_project(project, self.edit.path());
        }
        let layers = self.layers();
        self.outline.add_tiles(&layers);
        let scene = self.edit.scene();
        self.level
            .picked
            .retain(|name| scene.object(name).is_some());
        self.raw = Raw::of(&self.edit);
        self.problems = SceneCheck::new(self.edit.project()).all();
        if let Some(item) = &self.selection
            && !item.exists(self.edit.scene())
        {
            self.selection = None;
        }
        self.reload_rows();
        self.texts.clear();
        self.revision += 1;
    }

    pub fn select(&mut self, item: Option<Item>) {
        if self.selection == item {
            return;
        }
        self.end_group();
        self.selection = item;
        self.level.picked.clear();
        if let Some(Item::Asset(asset)) = &self.selection
            && crate::level::placeable(asset)
        {
            self.level.stamp = Some(asset.clone());
        }
        if let Some(Item::Layer(layer)) = &self.selection {
            self.level.brush.layer = Some(layer.clone());
        }
        let object = self.selected_object().map(str::to_string);
        self.viewport.select(object.as_deref());
        self.reload_rows();
        self.texts.clear();
    }

    fn reload_rows(&mut self) {
        self.rows = match &self.selection {
            Some(item) => self.rows_of(item),
            None => Vec::new(),
        };
    }

    pub fn open_scene(&mut self, scene: PathBuf) {
        let request = Request::Open(scene);
        if matches!(&request, Request::Open(scene) if scene == self.path())
            || self.requests.contains(&request)
        {
            return;
        }
        self.end_group();
        self.request(request);
    }

    pub fn set_project(&mut self, project: pfx_project::Project) {
        let scenes = project.scenes().len();
        self.log.info(format!(
            "project {}: {scenes} {}",
            project.name(),
            if scenes == 1 { "scene" } else { "scenes" }
        ));
        self.project = Some(project);
        self.frame_project();
        self.refresh();
    }

    pub fn frame_project(&mut self) {
        let device = pfx_gpu::screens::detect(&pfx_gpu::screens::SystemSource).device;
        let aspect = match self
            .project
            .as_ref()
            .map(|project| frame_aspect(project, device))
        {
            Some(Ok(aspect)) => aspect,
            Some(Err(error)) => {
                self.log.error(error);
                None
            }
            None => None,
        };
        self.viewport.set_frame_aspect(aspect);
    }

    pub fn selected_object(&self) -> Option<&str> {
        match &self.selection {
            Some(Item::Object(name)) => Some(name),
            _ => None,
        }
    }

    pub fn editable(&self) -> bool {
        self.play == Play::Stopped
    }

    pub fn dry(&mut self, change: &Change) -> Result<PatchGroup, String> {
        if change.tunable().is_some() {
            let label = format!("set {}", change.describe());
            let patch = change
                .live(&self.tunables)
                .and_then(|edit| self.tunables_patch(&[edit], &label));
            return match patch {
                Ok(patches) => Ok(PatchGroup { label, patches }),
                Err(error) => {
                    self.log.warn(error.clone());
                    Err(error)
                }
            };
        }
        let change = change.filed(self.edit.scene());
        self.edit
            .dry_run(|edit| change.apply(edit))
            .map_err(|error| {
                self.log.edit_error(&error);
                error.to_string()
            })
    }

    fn tunables_patch(&self, edits: &[Edit], label: &str) -> Result<Vec<Patch>, String> {
        let Some(file) = self.tunables.file().map(Path::to_path_buf) else {
            return Err("this project has no tunables".to_string());
        };
        let before = match self.files.text(&file) {
            Some(text) => text.to_string(),
            None => std::fs::read_to_string(&file)
                .map_err(|error| format!("{}: {error}", file.display()))?,
        };
        let mut after = before.clone();
        for edit in edits {
            if let Edit::Tunable { name, value } = edit {
                let value = self.tunables.check(name, *value)?;
                after = Tunables::written(&after, name, value)
                    .map_err(|error| format!("{}: {error}", file.display()))?;
            }
        }
        Ok(if after == before {
            Vec::new()
        } else {
            vec![Patch {
                label: label.to_string(),
                file,
                before,
                after,
            }]
        })
    }

    pub fn play_edit(&mut self, session: &mut PlaySession, change: &Change) -> bool {
        let applied = change
            .live(&self.tunables)
            .and_then(|edit| session.edit(edit));
        match applied {
            Ok(()) => {
                if let Some(value) = &change.value {
                    self.live.insert(change.describe(), value.clone());
                }
                inspect::overlay(&mut self.rows, &self.live);
                true
            }
            Err(error) => {
                self.log.warn(format!("play mode: {error}"));
                false
            }
        }
    }

    pub fn stopped(&mut self, keep: Option<&Pending>) {
        self.play = Play::Stopped;
        self.live.clear();
        if let Some(pending) = keep.filter(|pending| !pending.is_empty()) {
            self.keep(pending);
        }
        self.reload_rows();
    }

    pub fn keep(&mut self, pending: &Pending) {
        self.reloaded();
        let label = format!(
            "keep {} play {}",
            pending.len(),
            if pending.len() == 1 { "edit" } else { "edits" }
        );
        let scene: Vec<_> = pending.scene().collect();
        let mut group = if scene.is_empty() {
            PatchGroup::default()
        } else {
            let found = self.edit.dry_run(|edit| {
                for (target, path, value) in &scene {
                    let path: Vec<&str> = path.iter().map(String::as_str).collect();
                    edit.set(target, &path, (*value).clone())?;
                }
                Ok(())
            });
            match found {
                Ok(group) => group,
                Err(error) => {
                    self.log.edit_error(&error);
                    return;
                }
            }
        };
        let tunables: Vec<Edit> = pending
            .iter()
            .filter(|edit| matches!(edit, Edit::Tunable { .. }))
            .cloned()
            .collect();
        if !tunables.is_empty() {
            match self.tunables_patch(&tunables, &label) {
                Ok(patches) => group.patches.extend(patches),
                Err(error) => {
                    self.log.error(format!("keep: {error}"));
                    return;
                }
            }
        }
        let group = labelled(group, label);
        if group.is_empty() {
            self.log
                .info("keep: the play edits match the files already");
            return;
        }
        self.land(group);
    }

    pub fn change(&mut self, change: Change, group: Group) {
        if !self.editable() {
            self.request(Request::World(change));
            return;
        }
        if group == Group::Begin && self.held.is_none() {
            self.held = Some(Held {
                label: change.describe(),
                start: self
                    .files
                    .paths()
                    .filter_map(|path| {
                        Some((path.to_path_buf(), self.files.text(path)?.to_string()))
                    })
                    .collect(),
            });
        }
        if let Ok(found) = self.dry(&change)
            && !found.is_empty()
        {
            if self.held.is_some() {
                self.step(found);
            } else {
                self.land(found);
            }
        }
        if group == Group::End {
            self.end_group();
        }
    }

    fn step(&mut self, group: PatchGroup) {
        let label = group.label.clone();
        let mut patch = ScenePatch::new(group);
        let files: Vec<PathBuf> = pfx_editor_doc::Edit::files(&patch);
        match pfx_editor_doc::Edit::apply(&mut patch, &mut self.files) {
            Ok(()) => self.wrote(&Changed {
                label,
                files,
                caret: None,
            }),
            Err(error) => self.log.error(error.to_string()),
        }
    }

    pub fn land(&mut self, group: PatchGroup) {
        match self.history.apply(&mut self.files, ScenePatch::new(group)) {
            Ok(changed) if !changed.is_empty() => {
                self.log.info(changed.label.clone());
                self.wrote(&changed);
            }
            Ok(_) => {}
            Err(error) => self.log.error(error.to_string()),
        }
    }

    pub fn end_group(&mut self) {
        let Some(held) = self.held.take() else {
            return;
        };
        let patches: Vec<Patch> = held
            .start
            .iter()
            .filter_map(|(file, before)| {
                let after = self.files.text(file)?;
                (after != before).then(|| Patch {
                    label: held.label.clone(),
                    file: file.clone(),
                    before: before.clone(),
                    after: after.to_string(),
                })
            })
            .collect();
        if patches.is_empty() {
            return;
        }
        for patch in &patches {
            let _ = self.files.set_text(&patch.file, patch.before.clone());
        }
        let group = PatchGroup {
            label: held.label,
            patches,
        };
        match self.history.apply(&mut self.files, ScenePatch::new(group)) {
            Ok(changed) => self.log.info(changed.label),
            Err(error) => self.log.error(error.to_string()),
        }
    }

    pub fn grouping(&self) -> bool {
        self.held.is_some()
    }

    pub fn save(&mut self, changed: &Changed) -> bool {
        let mut saved = true;
        for file in &changed.files {
            match self.files.save(file) {
                Ok(_) => self.saves.push(file.clone()),
                Err(error) => {
                    self.log.error(error.to_string());
                    saved = false;
                }
            }
        }
        saved
    }

    pub fn wrote(&mut self, changed: &Changed) {
        if !self.save(changed) {
            return;
        }
        self.caught_up();
    }

    fn caught_up(&mut self) {
        if let Err(error) = self.edit.reload() {
            self.log.scene_error(&error);
        }
        self.sync_files();
        self.refresh();
        self.viewport.check();
        self.events();
    }

    fn sync_files(&mut self) {
        let wanted = self.files();
        for file in &wanted {
            if self.files.get(file).is_none()
                && let Err(error) = self.files.load(file)
            {
                self.log.error(error.to_string());
            }
        }
        let gone: Vec<PathBuf> = self
            .files
            .paths()
            .filter(|path| !wanted.iter().any(|file| file == path))
            .map(Path::to_path_buf)
            .collect();
        for path in gone {
            self.files.close(&path);
            self.sources.remove(&path);
        }
    }

    pub fn undo(&mut self) {
        self.step_history(true);
    }

    pub fn redo(&mut self) {
        self.step_history(false);
    }

    fn step_history(&mut self, undo: bool) {
        if !self.editable() {
            return;
        }
        self.end_group();
        let (word, stepped) = if undo {
            ("undo", self.history.undo(&mut self.files))
        } else {
            ("redo", self.history.redo(&mut self.files))
        };
        match stepped {
            Ok(Some(changed)) => {
                self.log.info(format!("{word} {}", changed.label));
                self.wrote(&changed);
            }
            Ok(None) => self.log.info(format!("nothing to {word}")),
            Err(error) => self.log.error(error.to_string()),
        }
    }

    pub fn typed(&mut self, changed: Option<Changed>) {
        if let Some(changed) = changed.filter(|changed| !changed.is_empty()) {
            self.log.info(changed.label.clone());
            self.wrote(&changed);
        }
    }

    pub fn reloaded(&mut self) {
        if self.held.is_some() {
            return;
        }
        let mut outside = false;
        let paths: Vec<PathBuf> = self.files.paths().map(Path::to_path_buf).collect();
        for path in paths {
            match self.files.disk(&path) {
                Ok(Disk::Outside(text)) => {
                    outside = true;
                    if let Err(error) = self.files.reload(&path, text) {
                        self.log.error(error.to_string());
                    }
                }
                Ok(_) => {}
                Err(error) => self.log.error(error.to_string()),
            }
        }
        let changed = self
            .edit
            .files()
            .any(|file| std::fs::read_to_string(file).ok().as_deref() != self.edit.text(file));
        if !outside && !changed {
            return;
        }
        if outside {
            let dropped = self.history.prune(&self.files);
            if dropped > 0 {
                self.log.warn(format!(
                    "the files changed outside the editor; {dropped} undo steps no longer apply and were dropped"
                ));
            } else {
                self.log
                    .info("reloaded the files changed outside the editor");
            }
        }
        if let Err(error) = self.edit.reload() {
            self.log.scene_error(&error);
            return;
        }
        self.sync_files();
        self.refresh();
    }

    pub fn events(&mut self) {
        for event in self.viewport.take_events() {
            match event {
                Event::Picked(found) => {
                    if self.level.shift {
                        if let Some(found) = found {
                            self.pick_more(&found.name);
                        }
                        continue;
                    }
                    let item = found.map(|selection| Item::Object(selection.name));
                    if item != self.selection {
                        self.select(item);
                    }
                }
                Event::Edited(changed) => {
                    self.saves.extend(changed.files.iter().cloned());
                    self.log.info(changed.label.clone());
                    self.caught_up();
                }
                Event::Refused(error) => self.log.warn(error),
                Event::Reloaded => self.reloaded(),
                Event::Broken(error) => self.log.error(error),
            }
        }
    }

    pub fn source(&mut self, file: &Path) -> &mut Source {
        self.sources
            .entry(file.to_path_buf())
            .or_insert_with(|| Source::new(("pfx edit source", file), file).toolbar(false))
    }

    pub fn toggle_view(&mut self) {
        match self.viewport.look() {
            pfx_editor_viewport::Look::Scene => {
                if let Some(eye) = self.viewport.eye() {
                    self.viewport.set_eye(eye);
                }
            }
            pfx_editor_viewport::Look::Orbit(_) => self.viewport.look_through_scene(),
        }
    }

    pub fn focus(&mut self) {
        self.viewport.frame_selection();
    }

    pub fn request(&mut self, request: Request) {
        if request == Request::Record && self.requests.contains(&request) {
            return;
        }
        self.requests.push(request);
    }

    pub fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.requests)
    }

    pub fn step_selection(&mut self, by: isize) {
        let next = self.outline.step(self.selection.as_ref(), by);
        self.select(next);
    }

    pub fn play_pressed(&mut self) {
        match self.play {
            Play::Stopped => {
                self.end_group();
                self.request(Request::Play);
            }
            Play::Paused => self.request(Request::Resume),
            Play::Playing => self.request(Request::Pause),
        }
    }
}
