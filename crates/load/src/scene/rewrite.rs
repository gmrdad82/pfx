use std::path::{Path, PathBuf};

use pfx_scene::{Patch, PatchGroup, Project};

use super::edit::scene_error as refusal;
use super::{SceneError, build};

fn scene_files(path: &Path) -> Result<(PathBuf, Project, PathBuf), SceneError> {
    let root = build::project_root(path);
    let project = build::project(&root)?;
    let absolute = build::absolute(path);
    Ok((root, project, absolute))
}

pub fn migrate(path: impl AsRef<Path>, dry_run: bool) -> Result<PatchGroup, SceneError> {
    let (root, project, absolute) = scene_files(path.as_ref())?;
    let mut scene = project
        .scene(&absolute)
        .map_err(|found| build::refused(&root, &found))?;
    if let Some(finish) = scene.finish.as_ref().and_then(|finish| finish.file.clone()) {
        scene.files.retain(|file| *file != Path::new(&finish));
    }
    let mut patches = Vec::new();
    for file in &scene.files {
        let full = project.root().join(file);
        let before = std::fs::read_to_string(&full)
            .map_err(|error| SceneError::new(&root.join(file), None, error.to_string()))?;
        let after = project
            .migrate(&full)
            .map_err(|found| build::refused(&root, &found))?;
        if after != before {
            patches.push(Patch {
                label: "migrate".to_string(),
                file: full,
                before,
                after,
            });
        }
    }
    let group = PatchGroup {
        label: "migrate".to_string(),
        patches,
    };
    if !group.is_empty() {
        project
            .scene_with(&absolute, group.after())
            .map_err(|found| build::refused(&root, &found))?;
        if !dry_run {
            group.write().map_err(|error| refusal(&root, error))?;
        }
    }
    Ok(group)
}

pub fn fix(path: impl AsRef<Path>, dry_run: bool) -> Result<PatchGroup, SceneError> {
    let (root, project, absolute) = scene_files(path.as_ref())?;
    let group = project
        .fix(&absolute)
        .map_err(|error| refusal(&root, error))?;
    if !dry_run && !group.is_empty() {
        group.write().map_err(|error| refusal(&root, error))?;
    }
    Ok(group)
}
