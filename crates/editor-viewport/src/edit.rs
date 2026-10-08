use std::path::{Path, PathBuf};

use glam::DVec3;
use pfx_editor_doc::toml_edit::{DocumentMut, Item};
use pfx_editor_doc::{Changed, DocError, Edit, Files};
use pfx_load::scene::{EditError, PatchGroup, SceneEdit, Value};

use crate::gizmo::Pose;

pub const PLACES: [i32; 3] = [6, 4, 6];
pub const STARTS_FROM: &str = "no longer holds the text this edit starts from";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScenePatch {
    group: PatchGroup,
}

impl ScenePatch {
    pub fn new(group: PatchGroup) -> ScenePatch {
        ScenePatch { group }
    }

    pub fn onto(files: &Files, mut group: PatchGroup) -> ScenePatch {
        for patch in &mut group.patches {
            patch.file = keyed(files, &patch.file);
        }
        ScenePatch { group }
    }

    pub fn group(&self) -> &PatchGroup {
        &self.group
    }
}

impl From<PatchGroup> for ScenePatch {
    fn from(group: PatchGroup) -> ScenePatch {
        ScenePatch::new(group)
    }
}

impl Edit for ScenePatch {
    fn label(&self) -> &str {
        self.group.label()
    }

    fn files(&self) -> Vec<PathBuf> {
        let mut files: Vec<PathBuf> = Vec::new();
        for file in self.group.files() {
            if !files.iter().any(|known| known == file) {
                files.push(file.to_path_buf());
            }
        }
        files
    }

    fn apply(&mut self, files: &mut Files) -> Result<(), DocError> {
        for patch in &self.group.patches {
            match files.text(&patch.file) {
                Some(text) if text == patch.before => {}
                Some(_) => return Err(DocError::new(&patch.file, "", STARTS_FROM)),
                None => {
                    files.open(&patch.file, patch.before.clone());
                }
            }
            files.set_text(&patch.file, patch.after.clone())?;
        }
        Ok(())
    }

    fn inverse(&self) -> Box<dyn Edit> {
        Box::new(ScenePatch::new(self.group.inverse()))
    }
}

pub fn round(value: f64, places: i32) -> f64 {
    let scale = 10_f64.powi(places);
    let rounded = (value * scale).round() / scale;
    if rounded == 0.0 { 0.0 } else { rounded }
}

fn rounded(value: DVec3, places: i32) -> [f32; 3] {
    value.to_array().map(|v| round(v, places) as f32)
}

pub fn scalar_scale(edit: &SceneEdit, name: &str) -> bool {
    edit.files().any(|file| {
        let Some(doc) = edit
            .text(file)
            .and_then(|text| text.parse::<DocumentMut>().ok())
        else {
            return false;
        };
        doc.get("object")
            .and_then(Item::as_array_of_tables)
            .and_then(|tables| {
                tables
                    .iter()
                    .find(|table| table.get("name").and_then(Item::as_str) == Some(name))
            })
            .and_then(|table| table.get("scale"))
            .and_then(Item::as_value)
            .is_some_and(|value| value.as_float().is_some() || value.as_integer().is_some())
    })
}

pub fn changes(edit: &SceneEdit, start: &Pose, pose: &Pose) -> Vec<(&'static str, Value)> {
    let mut changes = Vec::new();
    let [at, rotate, scale] = PLACES;
    if rounded(start.at, at) != rounded(pose.at, at) {
        changes.push(("at", Value::from(rounded(pose.at, at))));
    }
    if rounded(start.rotate, rotate) != rounded(pose.rotate, rotate) {
        changes.push(("rotate", Value::from(rounded(pose.rotate, rotate))));
    }
    let after = rounded(pose.scale, scale);
    if rounded(start.scale, scale) != after {
        let uniform = after[0] == after[1] && after[1] == after[2];
        let value = if uniform && scalar_scale(edit, &start.name) {
            Value::Float(after[0])
        } else {
            Value::from(after)
        };
        changes.push(("scale", value));
    }
    changes
}

pub fn commit(
    edit: &mut SceneEdit,
    label: &str,
    start: &Pose,
    pose: &Pose,
) -> Result<Option<PatchGroup>, EditError> {
    let changes = changes(edit, start, pose);
    if changes.is_empty() {
        return Ok(None);
    }
    let target = pfx_load::scene::Target::Object(start.name.clone());
    let mut group = edit.dry_run(|edit| {
        for (key, value) in changes {
            edit.set(&target, &[key], value)?;
        }
        Ok(())
    })?;
    if group.patches.is_empty() {
        return Ok(None);
    }
    group.label = label.to_string();
    Ok(Some(group))
}

pub fn keyed(files: &Files, path: &Path) -> PathBuf {
    if files.get(path).is_some() {
        return path.to_path_buf();
    }
    let Ok(real) = std::fs::canonicalize(path) else {
        return path.to_path_buf();
    };
    files
        .paths()
        .find(|key| std::fs::canonicalize(key).is_ok_and(|key| key == real))
        .unwrap_or(path)
        .to_path_buf()
}

pub fn stale(edit: &SceneEdit, files: &Files) -> Vec<PathBuf> {
    edit.files()
        .filter(|file| {
            files
                .text(&keyed(files, file))
                .is_some_and(|text| Some(text) != edit.text(file))
        })
        .map(Path::to_path_buf)
        .collect()
}

pub fn save(files: &mut Files, changed: &Changed) -> Result<(), DocError> {
    for file in &changed.files {
        files.save(file)?;
    }
    Ok(())
}
