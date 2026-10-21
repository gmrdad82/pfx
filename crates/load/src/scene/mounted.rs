use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use pfx_scene::Project;

use super::build::{self, absolute, diagnostic, project_root, refused};
use super::{Scene, SceneError};
use crate::sha256;

pub fn is_pfx_path(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case(pfx_file::EXTENSION))
}

pub(super) fn mounted(project: &Project) -> Option<&Project> {
    project.pfx_path().map(|_| project)
}

pub(super) fn open_bytes(name: &Path, bytes: &[u8]) -> Result<Scene, SceneError> {
    let root = PathBuf::new();
    let project =
        Project::from_pfx_bytes(name, bytes).map_err(|found| diagnostic(&root, &found))?;
    let file = project.pfx_path().unwrap_or(name).to_path_buf();
    let resolved = project
        .scene(&file)
        .map_err(|found| refused(&root, &found))?;
    build::build_with(
        &file,
        &root,
        &resolved,
        None,
        &BTreeMap::new(),
        Some(&project),
    )
}

pub(super) fn open(path: &Path, hour: Option<f64>) -> Result<Scene, SceneError> {
    let root = project_root(path);
    let project = Project::open_pfx(path).map_err(|found| diagnostic(&root, &found))?;
    let resolved = project
        .scene(&absolute(path))
        .map_err(|found| refused(&root, &found))?;
    build::build_with(
        path,
        &root,
        &resolved,
        hour,
        &BTreeMap::new(),
        Some(&project),
    )
}

fn inner(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let parts: Vec<&str> = relative
        .components()
        .map(|part| part.as_os_str().to_str())
        .collect::<Option<_>>()?;
    Some(parts.join("/"))
}

fn holds(mount: &Project, name: &str) -> bool {
    mount.files().any(|file| file == Path::new(name))
        || mount
            .pfx()
            .is_some_and(|reader| reader.find_named(pfx_file::FILE, name).is_some())
}

pub(super) fn container(mount: &Project, root: &Path) -> Option<PathBuf> {
    mount.pfx_path().map(|file| root.join(file))
}

pub(super) fn read(
    mount: Option<&Project>,
    root: &Path,
    path: &Path,
    files: &mut BTreeMap<PathBuf, [u8; 32]>,
) -> Option<Result<Vec<u8>, String>> {
    let mount = mount?;
    let file = container(mount, root)?;
    if path == file {
        return Some(whole(mount, &file, files));
    }
    let Some(name) = inner(root, path).filter(|name| holds(mount, name)) else {
        return mount
            .in_memory()
            .then(|| Err(format!("{} is not in {}", path.display(), file.display())));
    };
    Some(
        mount
            .asset(&name)
            .map(|bytes| bytes.into_owned())
            .map_err(|e| format!("cannot read {}: {e}", file.join(&name).display()))
            .and_then(|bytes| {
                record(mount, &file, files)?;
                Ok(bytes)
            }),
    )
}

fn whole(
    mount: &Project,
    file: &Path,
    files: &mut BTreeMap<PathBuf, [u8; 32]>,
) -> Result<Vec<u8>, String> {
    let bytes = match mount.pfx_bytes().filter(|_| mount.in_memory()) {
        Some(held) => held.to_vec(),
        None => std::fs::read(file).map_err(|e| format!("cannot read {}: {e}", file.display()))?,
    };
    files.insert(file.to_path_buf(), sha256(&bytes));
    Ok(bytes)
}

fn record(
    mount: &Project,
    file: &Path,
    files: &mut BTreeMap<PathBuf, [u8; 32]>,
) -> Result<(), String> {
    if files.contains_key(file) {
        return Ok(());
    }
    match mount.pfx_bytes().filter(|_| mount.in_memory()) {
        Some(held) => {
            files.insert(file.to_path_buf(), sha256(held));
        }
        None => {
            whole(mount, file, files)?;
        }
    }
    Ok(())
}

pub(super) fn is_file(mount: Option<&Project>, root: &Path, path: &Path) -> bool {
    match mount.filter(|mount| mount.in_memory()) {
        Some(mount) => inner(root, path).is_some_and(|name| holds(mount, &name)),
        None => path.is_file(),
    }
}

pub(super) fn text(mount: Option<&Project>, file: &Path) -> Option<Option<String>> {
    let mount = mount?;
    let name = file.to_str()?.replace('\\', "/");
    if mount.in_memory() && !holds(mount, &name) {
        return Some(None);
    }
    holds(mount, &name).then(|| {
        mount
            .asset(&name)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes.into_owned()).ok())
    })
}

pub(super) fn shown(mount: Option<&Project>, root: &Path, file: &Path) -> PathBuf {
    match mount {
        Some(mount) => root.join(mount.shown(file)),
        None => root.join(file),
    }
}

pub(super) fn own_gltf(
    mount: Option<&Project>,
    root: &Path,
    file: &str,
) -> Option<Result<PathBuf, String>> {
    if file != pfx_scene::GLTF_PATH {
        return None;
    }
    Some(
        mount
            .and_then(|mount| container(mount, root))
            .ok_or_else(|| {
                format!("{file} names the glTF of a .pitofx, and this scene is not one")
            }),
    )
}

pub fn container_of(path: &Path) -> Option<(PathBuf, String)> {
    let container = path
        .ancestors()
        .skip(1)
        .find(|ancestor| is_pfx_path(ancestor) && ancestor.is_file())?;
    let name = inner(container, path)?;
    Some((container.to_path_buf(), name))
}

pub fn held_text(path: &Path) -> Option<std::io::Result<String>> {
    let (container, name) = container_of(path)?;
    let invalid = |message: String| std::io::Error::new(std::io::ErrorKind::InvalidData, message);
    Some(
        Project::open_pfx(&container)
            .map_err(|found| invalid(found.message))
            .and_then(|project| project.asset(&name).map(|bytes| bytes.into_owned()))
            .and_then(|bytes| {
                String::from_utf8(bytes).map_err(|_| invalid(format!("{name} is not UTF-8")))
            }),
    )
}

pub fn pack(scene: &Path, out: &Path) -> Result<usize, SceneError> {
    let root = project_root(scene);
    let project = build::project(&root)?;
    let bytes =
        pfx_scene::pack(&project, &absolute(scene)).map_err(|found| refused(&root, &found))?;
    let staged = out.with_extension("pitofx.tmp");
    std::fs::write(&staged, &bytes)
        .and_then(|()| std::fs::rename(&staged, out))
        .map_err(|e| SceneError::new(out, None, format!("cannot write: {e}")))?;
    Ok(bytes.len())
}

pub fn unpack(file: &Path, folder: &Path) -> Result<Vec<PathBuf>, SceneError> {
    let bytes = std::fs::read(file)
        .map_err(|e| SceneError::new(file, None, format!("cannot read: {e}")))?;
    pfx_scene::unpack(&bytes, folder).map_err(|e| SceneError::new(file, None, e.to_string()))
}
