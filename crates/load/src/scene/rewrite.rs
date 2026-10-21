use std::path::{Path, PathBuf};

use pfx_scene::{PatchGroup, Project};

use super::edit::scene_error as refusal;
use super::{SceneError, build, mounted};

fn scene_files(path: &Path) -> Result<(PathBuf, Project, PathBuf), SceneError> {
    let root = build::project_root(path);
    let project = if mounted::is_pfx_path(path) {
        Project::open_pfx(path).map_err(|found| build::diagnostic(&root, &found))?
    } else {
        build::project(&root)?
    };
    let absolute = build::absolute(path);
    Ok((root, project, absolute))
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
