#![allow(clippy::result_large_err)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use pfx_scene::{Id, Project, code};

pub use pfx_scene::{EditError, Kind, Patch, PatchGroup, Target, Value};

use super::{Scene, SceneError, build};

type Writer = pfx_scene::SceneEdit;
type Op = Box<dyn Fn(&mut Writer) -> Result<PatchGroup, EditError>>;

fn edit_error(error: SceneError, key: &str) -> EditError {
    EditError {
        file: error.file,
        key: key.to_string(),
        line: error.line.map(|line| line as u32),
        code: code::BAD_VALUE,
        message: error.message,
        diagnostics: Vec::new(),
    }
}

pub(super) fn scene_error(root: &Path, error: EditError) -> SceneError {
    if error.diagnostics.is_empty() {
        return SceneError::new(
            &error.file,
            error.line.map(|line| line as usize),
            error.message,
        );
    }
    build::refused(root, &error.diagnostics)
}

fn merge_group(group: &mut PatchGroup, other: &PatchGroup) {
    for patch in &other.patches {
        match group.patches.iter_mut().find(|own| own.file == patch.file) {
            Some(own) => own.after = patch.after.clone(),
            None => group.patches.push(patch.clone()),
        }
    }
}

fn settle(group: &mut PatchGroup) {
    group.patches.retain(|patch| patch.before != patch.after);
}

fn since(previous: &PatchGroup, now: &PatchGroup, label: &str) -> PatchGroup {
    let patches = now
        .patches
        .iter()
        .filter_map(|patch| {
            let before = previous
                .patches
                .iter()
                .find(|old| old.file == patch.file)
                .map_or_else(|| patch.before.clone(), |old| old.after.clone());
            (before != patch.after).then(|| Patch {
                label: label.to_string(),
                file: patch.file.clone(),
                before,
                after: patch.after.clone(),
            })
        })
        .collect();
    PatchGroup {
        label: label.to_string(),
        patches,
    }
}

#[derive(Default)]
struct Dry {
    ops: Vec<Op>,
    group: PatchGroup,
}

pub struct SceneEdit {
    path: PathBuf,
    root: PathBuf,
    writer: Writer,
    files: Vec<PathBuf>,
    scene: Scene,
    undo: Vec<PatchGroup>,
    redo: Vec<PatchGroup>,
    open: Option<PatchGroup>,
    dry: Option<Dry>,
}

impl SceneEdit {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, SceneError> {
        let path = path.as_ref().to_path_buf();
        let root = build::project_root(&path);
        let writer = Writer::open(&path).map_err(|error| scene_error(&root, error))?;
        let scene = build::build(&path, &root, writer.scene(), None, &BTreeMap::new())?;
        let mut edit = Self {
            path,
            root,
            writer,
            files: Vec::new(),
            scene,
            undo: Vec::new(),
            redo: Vec::new(),
            open: None,
            dry: None,
        };
        edit.follow();
        Ok(edit)
    }

    fn follow(&mut self) {
        let root = self.writer.root().to_path_buf();
        self.files = self
            .writer
            .files()
            .map(|file| {
                file.strip_prefix(&root)
                    .map(|relative| self.root.join(relative))
                    .unwrap_or(file)
            })
            .collect();
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn scene(&self) -> &Scene {
        &self.scene
    }

    pub fn resolved(&self) -> &pfx_scene::Scene {
        self.writer.scene()
    }

    pub fn project(&self) -> &Project {
        self.writer.project()
    }

    pub fn files(&self) -> impl Iterator<Item = &Path> {
        self.files.iter().map(PathBuf::as_path)
    }

    pub fn text(&self, file: impl AsRef<Path>) -> Option<&str> {
        let file = file.as_ref();
        self.writer.text(build::absolute(file))
    }

    pub fn reload(&mut self) -> Result<(), SceneError> {
        *self = Self::open(&self.path)?;
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
            file: self.writer.path(),
            key: key.to_string(),
            line: None,
            code: code::BAD_VALUE,
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
        settle(&mut group);
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
        settle(&mut group);
        let inverse = group.inverse();
        let label = group.label.clone();
        if let Err(error) = self.land(&inverse, &label) {
            self.open = Some(group);
            return Err(error);
        }
        Ok(())
    }

    pub fn apply(&mut self, group: &PatchGroup) -> Result<(), EditError> {
        if self.dry.is_some() {
            return Err(self.refuse(&group.label, "a dry run applies nothing"));
        }
        self.land(group, &group.label)
    }

    pub fn undo(&mut self) -> Result<Option<PatchGroup>, EditError> {
        if self.open.is_some() || self.dry.is_some() {
            return Err(self.refuse("undo", "a group of edits is open"));
        }
        let Some(group) = self.undo.pop() else {
            return Ok(None);
        };
        let inverse = group.inverse();
        if let Err(error) = self.land(&inverse, &inverse.label) {
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
        if let Err(error) = self.land(&group, &group.label) {
            self.redo.push(group);
            return Err(error);
        }
        self.undo.push(group.clone());
        Ok(Some(group))
    }

    pub fn dry_run<T>(
        &mut self,
        edits: impl FnOnce(&mut Self) -> Result<T, EditError>,
    ) -> Result<PatchGroup, EditError> {
        if self.dry.is_some() || self.open.is_some() {
            return Err(self.refuse("dry run", "a group of edits is open"));
        }
        self.dry = Some(Dry::default());
        let outcome = edits(self);
        let dry = self.dry.take().unwrap_or_default();
        outcome?;
        let mut group = dry.group;
        settle(&mut group);
        if !group.is_empty() {
            self.check(&group, &group.label)?;
        }
        Ok(group)
    }

    fn check(&self, group: &PatchGroup, key: &str) -> Result<Scene, EditError> {
        let resolved = self
            .writer
            .project()
            .scene_with(&self.writer.path(), group.after())
            .map_err(|found| edit_error(build::refused(&self.root, &found), key))?;
        let root = self.writer.root();
        let overlay = group
            .after()
            .into_iter()
            .map(|(file, text)| {
                let relative = file
                    .strip_prefix(root)
                    .map(Path::to_path_buf)
                    .unwrap_or(file);
                (relative, text)
            })
            .collect();
        build::build(&self.path, &self.root, &resolved, None, &overlay)
            .map_err(|error| edit_error(error, key))
    }

    fn land(&mut self, group: &PatchGroup, key: &str) -> Result<(), EditError> {
        let scene = self.check(group, key)?;
        self.writer.apply(group).map_err(|error| EditError {
            key: key.to_string(),
            ..error
        })?;
        self.scene = scene;
        self.follow();
        Ok(())
    }

    fn record(&mut self, group: &PatchGroup) {
        match &mut self.open {
            Some(open) => merge_group(open, group),
            None => {
                self.redo.clear();
                self.undo.push(group.clone());
            }
        }
    }

    fn change(&mut self, key: String, label: String, op: Op) -> Result<PatchGroup, EditError> {
        let Self { writer, dry, .. } = self;
        if let Some(dry) = dry {
            let group = writer
                .dry_run(|writer| {
                    for done in &dry.ops {
                        done(writer)?;
                    }
                    op(writer)
                })
                .map_err(|error| EditError {
                    key: key.clone(),
                    ..error
                })?;
            let own = since(&dry.group, &group, &label);
            dry.ops.push(op);
            if dry.group.label.is_empty() {
                dry.group.label = group.label.clone();
            }
            merge_group(&mut dry.group, &own);
            return Ok(own);
        }
        let group = writer
            .dry_run(|writer| op(writer))
            .map_err(|error| EditError {
                key: key.clone(),
                ..error
            })?;
        if group.is_empty() {
            return Ok(group);
        }
        self.land(&group, &key)?;
        self.record(&group);
        Ok(group)
    }

    fn at_path(
        &mut self,
        verb: &str,
        target: &Target,
        path: &[&str],
        apply: impl Fn(&mut Writer, &Target, &[&str]) -> Result<PatchGroup, EditError> + 'static,
    ) -> Result<PatchGroup, EditError> {
        let target = target.clone();
        let path: Vec<String> = path.iter().map(|segment| segment.to_string()).collect();
        let key = format!("{target} {}", path.join("."));
        self.change(
            key.clone(),
            format!("{verb} {key}"),
            Box::new(move |writer| {
                let path: Vec<&str> = path.iter().map(String::as_str).collect();
                apply(writer, &target, &path)
            }),
        )
    }

    pub fn set(
        &mut self,
        target: &Target,
        path: &[&str],
        value: impl Into<Value>,
    ) -> Result<PatchGroup, EditError> {
        let value = value.into();
        self.at_path("set", target, path, move |writer, target, path| {
            writer.set(target, path, value.clone())
        })
    }

    pub fn unset(&mut self, target: &Target, path: &[&str]) -> Result<PatchGroup, EditError> {
        self.at_path("unset", target, path, |writer, target, path| {
            writer.unset(target, path)
        })
    }

    pub fn push(
        &mut self,
        target: &Target,
        path: &[&str],
        value: impl Into<Value>,
    ) -> Result<PatchGroup, EditError> {
        let value = value.into();
        self.at_path("add to", target, path, move |writer, target, path| {
            writer.push(target, path, value.clone())
        })
    }

    pub fn remove(&mut self, target: &Target) -> Result<PatchGroup, EditError> {
        let target = target.clone();
        let key = target.to_string();
        self.change(
            key.clone(),
            format!("remove {key}"),
            Box::new(move |writer| writer.remove(&target)),
        )
    }

    pub fn add(
        &mut self,
        kind: Kind,
        id: Id,
        name: &str,
        fields: &[(&str, Value)],
        file: Option<&Path>,
    ) -> Result<PatchGroup, EditError> {
        let name = name.to_string();
        let fields: Vec<(String, Value)> = fields
            .iter()
            .map(|(key, value)| (key.to_string(), value.clone()))
            .collect();
        let file = file.map(Path::to_path_buf);
        let key = format!("{} {name}", format!("{kind:?}").to_lowercase());
        self.change(
            key.clone(),
            format!("add {key}"),
            Box::new(move |writer| {
                let fields: Vec<(&str, Value)> = fields
                    .iter()
                    .map(|(key, value)| (key.as_str(), value.clone()))
                    .collect();
                writer.add(kind, id, &name, &fields, file.as_deref())
            }),
        )
    }

    pub fn add_material(
        &mut self,
        name: &str,
        id: Id,
        material: &pfx_materials::Material,
        file: Option<&Path>,
    ) -> Result<PatchGroup, EditError> {
        let key = format!("material {name}");
        let text = material
            .to_toml()
            .map_err(|message| self.refuse(&key, &message))?;
        let typed: pfx_scene::types::Material =
            toml::from_str(&text).map_err(|error| self.refuse(&key, error.message()))?;
        let name = name.to_string();
        let file = file.map(Path::to_path_buf);
        self.change(
            key.clone(),
            format!("add {key}"),
            Box::new(move |writer| writer.add_material(&name, id, &typed, file.as_deref())),
        )
    }

    pub fn duplicate_object(
        &mut self,
        name: &str,
        new: &str,
        id: Id,
    ) -> Result<PatchGroup, EditError> {
        let (name, new) = (name.to_string(), new.to_string());
        self.change(
            format!("object {name}"),
            format!("duplicate object {name} as {new}"),
            Box::new(move |writer| writer.duplicate_object(&name, &new, id)),
        )
    }

    pub fn rename_object(&mut self, name: &str, new: &str) -> Result<PatchGroup, EditError> {
        let (name, new) = (name.to_string(), new.to_string());
        self.change(
            format!("object {name}"),
            format!("rename object {name} to {new}"),
            Box::new(move |writer| writer.rename_object(&name, &new)),
        )
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

#[cfg(test)]
#[path = "edit_tests.rs"]
mod tests;
