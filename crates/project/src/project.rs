use std::path::{Path, PathBuf};

pub const FILE: &str = "project.toml";
pub const SKIPPED: [&str; 2] = ["target", "tmp"];
pub const CONTENT: [&str; 12] = [
    "gltf", "glb", "bin", "png", "hdr", "wav", "ogg", "ttf", "otf", "svg", "ktx2", "exr",
];
pub const KINDS: [&str; 5] = [
    ".scene.toml",
    ".prefab.toml",
    ".materials.toml",
    ".proxies.toml",
    "materials.toml",
];

#[derive(Clone, Debug, PartialEq)]
pub struct Project {
    root: PathBuf,
    name: String,
    scene: Option<PathBuf>,
    play: Option<PathBuf>,
    select: Option<String>,
    ignore: Vec<PathBuf>,
    screen: Option<toml::Table>,
    modules: Vec<PathBuf>,
    flat: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Module {
    pub id: String,
    pub path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Folder {
    pub path: PathBuf,
    pub files: Vec<PathBuf>,
}

fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

fn path_key(
    table: Option<&toml::Table>,
    key: &str,
    file: &Path,
) -> Result<Option<PathBuf>, String> {
    match table.and_then(|table| table.get(key)) {
        None => Ok(None),
        Some(toml::Value::String(text)) if !text.is_empty() => Ok(Some(PathBuf::from(text))),
        Some(_) => Err(format!(
            "{}: {key} is a path relative to the project's folder",
            file.display()
        )),
    }
}

fn modules(table: Option<&toml::Table>, file: &Path) -> Result<Vec<PathBuf>, String> {
    let refuse = || {
        format!(
            "{}: modules lists the built .wasm of each gameplay module, relative to the project's folder",
            file.display()
        )
    };
    match table.and_then(|table| table.get("modules")) {
        None => Ok(Vec::new()),
        Some(toml::Value::Array(paths)) => paths
            .iter()
            .map(|path| {
                path.as_str()
                    .filter(|path| path.ends_with(".wasm") && path.len() > ".wasm".len())
                    .map(PathBuf::from)
                    .ok_or_else(refuse)
            })
            .collect(),
        Some(_) => Err(refuse()),
    }
}

fn scene_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".scene.toml"))
}

fn content_file(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if KINDS.iter().any(|kind| name.ends_with(kind)) {
        return true;
    }
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| CONTENT.contains(&extension.to_ascii_lowercase().as_str()))
}

impl Project {
    pub fn open(folder: impl AsRef<Path>) -> Result<Project, String> {
        let root = absolute(folder.as_ref());
        let file = root.join(FILE);
        let text = std::fs::read_to_string(&file)
            .map_err(|error| format!("{}: {error}", file.display()))?;
        Project::parse(&root, &text)
    }

    pub fn folder(folder: impl AsRef<Path>) -> Result<Project, String> {
        let root = absolute(folder.as_ref());
        if root.join(FILE).is_file() {
            return Project::open(&root);
        }
        if !root.is_dir() {
            return Err(format!("{}: no such folder", root.display()));
        }
        Ok(Project {
            name: root
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            root,
            scene: None,
            play: None,
            select: None,
            ignore: Vec::new(),
            screen: None,
            modules: Vec::new(),
            flat: false,
        })
    }

    pub fn marked(&self) -> bool {
        self.root.join(FILE).is_file()
    }

    pub fn parse(root: &Path, text: &str) -> Result<Project, String> {
        let file = root.join(FILE);
        let table: toml::Table = text
            .parse()
            .map_err(|error: toml::de::Error| format!("{}: {}", file.display(), error.message()))?;
        let project = table
            .get("project")
            .and_then(toml::Value::as_table)
            .ok_or_else(|| format!("{}: no [project] table", file.display()))?;
        let name = project
            .get("name")
            .and_then(toml::Value::as_str)
            .filter(|name| !name.trim().is_empty())
            .ok_or_else(|| format!("{}: [project] has no name", file.display()))?
            .to_string();
        let pfx = table
            .get("authoring")
            .and_then(toml::Value::as_table)
            .and_then(|authoring| authoring.get("pfx"))
            .and_then(toml::Value::as_table);
        Ok(Project {
            root: root.to_path_buf(),
            name,
            scene: path_key(Some(project), "scene", &file)?,
            play: path_key(pfx, "play", &file)?,
            ignore: match project.get("ignore") {
                None => Vec::new(),
                Some(toml::Value::Array(paths)) => paths
                    .iter()
                    .map(|path| {
                        path.as_str().map(|path| root.join(path)).ok_or_else(|| {
                            format!(
                                "{}: ignore lists paths relative to the project's folder",
                                file.display()
                            )
                        })
                    })
                    .collect::<Result<_, _>>()?,
                Some(_) => {
                    return Err(format!(
                        "{}: ignore lists paths relative to the project's folder",
                        file.display()
                    ));
                }
            },
            screen: match pfx.and_then(|pfx| pfx.get("screen")) {
                None => None,
                Some(toml::Value::Table(table)) => Some(table.clone()),
                Some(_) => {
                    return Err(format!(
                        "{}: [authoring.pfx.screen] is a table, the game's [screen] policy",
                        file.display()
                    ));
                }
            },
            modules: modules(pfx, &file)?,
            flat: match pfx.and_then(|pfx| pfx.get("flat")) {
                None => false,
                Some(toml::Value::Boolean(flat)) => *flat,
                Some(_) => {
                    return Err(format!(
                        "{}: flat is true for a game with no 3D scene, or false",
                        file.display()
                    ));
                }
            },
            select: match pfx.and_then(|pfx| pfx.get("select")) {
                None => None,
                Some(toml::Value::String(name)) if !name.is_empty() => Some(name.clone()),
                Some(_) => {
                    return Err(format!(
                        "{}: select names the object the editor selects when it opens",
                        file.display()
                    ));
                }
            },
        })
    }

    pub fn find(from: impl AsRef<Path>) -> Option<PathBuf> {
        let from = absolute(from.as_ref());
        let start = if from.is_dir() {
            from.as_path()
        } else {
            from.parent()?
        };
        start
            .ancestors()
            .find(|folder| folder.join(FILE).is_file())
            .map(Path::to_path_buf)
    }

    pub fn locate(manifest_dir: impl AsRef<Path>) -> Result<Project, String> {
        let beside = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf))
            .filter(|folder| folder.join(FILE).is_file());
        Project::open(beside.unwrap_or_else(|| manifest_dir.as_ref().to_path_buf()))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn version(&self) -> Option<String> {
        let text = std::fs::read_to_string(self.root.join("Cargo.toml")).ok()?;
        let table: toml::Table = text.parse().ok()?;
        table
            .get("package")?
            .get("version")?
            .as_str()
            .map(str::to_string)
    }

    pub fn scene(&self) -> Option<PathBuf> {
        self.scene.as_ref().map(|scene| self.root.join(scene))
    }

    pub fn screen(&self) -> Option<&toml::Table> {
        self.screen.as_ref()
    }

    pub fn flat(&self) -> bool {
        self.flat
    }

    pub fn select(&self) -> Option<&str> {
        self.select.as_deref()
    }

    pub fn play(&self) -> PathBuf {
        match (&self.play, &self.scene) {
            (Some(play), _) | (None, Some(play)) => self.root.join(play),
            (None, None) => self
                .scenes()
                .into_iter()
                .next()
                .unwrap_or_else(|| self.root.join("content")),
        }
    }

    pub fn modules(&self) -> Vec<Module> {
        self.modules
            .iter()
            .map(|path| Module {
                id: path
                    .file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                path: self.root.join(path),
            })
            .collect()
    }

    pub fn first(&self) -> Option<PathBuf> {
        self.scene()
            .or_else(|| self.play.as_ref().map(|play| self.root.join(play)))
            .or_else(|| self.scenes().into_iter().next())
    }

    pub fn relative(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/")
    }

    pub fn scenes(&self) -> Vec<PathBuf> {
        self.folders()
            .into_iter()
            .flat_map(|folder| folder.files)
            .filter(|file| scene_file(file))
            .collect()
    }

    pub fn folders(&self) -> Vec<Folder> {
        let mut out = Vec::new();
        walk(&self.root, &self.root, &self.ignore, &mut out);
        out
    }
}

fn walk(root: &Path, folder: &Path, ignore: &[PathBuf], out: &mut Vec<Folder>) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
    entries.sort_by_key(|entry| entry.file_name());
    let mut files = Vec::new();
    let mut folders = Vec::new();
    for entry in entries {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.starts_with('.') {
            continue;
        }
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if ignore.contains(&path) {
            continue;
        }
        if kind.is_dir() {
            let skipped = folder == root && SKIPPED.contains(&name);
            if !skipped && !path.join(FILE).is_file() {
                folders.push(path);
            }
        } else if kind.is_file() && content_file(&path) {
            files.push(path);
        }
    }
    if !files.is_empty() {
        out.push(Folder {
            path: folder.to_path_buf(),
            files,
        });
    }
    for path in folders {
        walk(root, &path, ignore, out);
    }
}
