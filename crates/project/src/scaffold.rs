use std::path::{Path, PathBuf};

use crate::project::FILE;

pub const SOURCE: &str = "https://github.com/gmrdad82/pfx.git";
pub const SCENE: &str = "content/first.scene.toml";
pub const DOCUMENT: &str = "content/first.pitofx";
pub const MARK: &str = "content/marks/mark.png";
pub const PREFAB: &str = "content/prefabs/title.prefab.toml";
pub const LICENCE: &str = "This game links pfx: publish its source under the AGPL-3.0, or use pfx's free licence (closed use under $200k a year, education, non-profits) or a commercial one: https://pitomd.com/pfx/license";
const FONT: &[u8] = include_bytes!("templates/IBMPlexMono-Regular.ttf");
const TITLE: &str = include_str!("templates/title.prefab.toml.in");
const FACE: &str = "
[materials.face]
family = \"lacquer\"
base = [0.16, 0.16, 0.18]
roughness = 0.35
clearcoat = 0.4

[materials.face.content_layer]
slot = 0
";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub module: bool,
    pub mark: Option<PathBuf>,
}

const RESERVED: [&str; 12] = [
    "self",
    "super",
    "crate",
    "std",
    "core",
    "alloc",
    "test",
    "proc-macro",
    "build",
    "main",
    "deps",
    "examples",
];

const TEMPLATES: [(&str, &str); 14] = [
    ("README.md", include_str!("templates/README.md.in")),
    ("Cargo.toml", include_str!("templates/Cargo.toml.in")),
    ("build.rs", include_str!("templates/build.rs.in")),
    ("src/lib.rs", include_str!("templates/lib.rs.in")),
    ("src/main.rs", include_str!("templates/main.rs.in")),
    ("tests/first.rs", include_str!("templates/first.rs.in")),
    (FILE, include_str!("templates/project.toml.in")),
    (SCENE, include_str!("templates/first.scene.toml.in")),
    (
        "content/materials/base.materials.toml",
        include_str!("templates/base.materials.toml.in"),
    ),
    ("AGENTS.md", include_str!("templates/AGENTS.md.in")),
    ("CLAUDE.md", "@AGENTS.md\n"),
    ("prompts/RESUME.md", include_str!("templates/RESUME.md.in")),
    (".gitignore", include_str!("templates/gitignore.in")),
    (".gitattributes", "*.pitofx diff=pitofx\n"),
];

const MODULE: [(&str, &str); 3] = [
    (
        "module/Cargo.toml",
        include_str!("templates/module.Cargo.toml.in"),
    ),
    (
        "module/src/lib.rs",
        include_str!("templates/module.lib.rs.in"),
    ),
    (
        "wit/gameplay.wit",
        include_str!("templates/gameplay.wit.in"),
    ),
];

const MODULE_LIB: &str = include_str!("templates/lib.module.rs.in");
const MODULE_AGENTS: &str = include_str!("templates/AGENTS.module.md.in");

const FILES: [(&str, &[u8]); 5] = [
    (
        "content/credit/powered-by-128.png",
        include_bytes!("../assets/powered-by-128.png"),
    ),
    (
        "content/credit/powered-by-light-128.png",
        include_bytes!("../assets/powered-by-light-128.png"),
    ),
    (
        "content/meshes/grid.glb",
        include_bytes!("templates/grid.glb"),
    ),
    ("content/fonts/IBMPlexMono-Regular.ttf", FONT),
    (
        "content/fonts/IBMPlexMono-OFL.txt",
        include_bytes!("templates/IBMPlexMono-OFL.txt"),
    ),
];

pub fn check_name(name: &str) -> Result<(), String> {
    let slug = name.starts_with(|c: char| c.is_ascii_lowercase())
        && !name.ends_with('-')
        && !name.contains("--")
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !slug {
        return Err(format!(
            "{name:?} is not a lowercase slug: letters a to z, digits and single dashes, starting with a letter"
        ));
    }
    if name == "pfx" || name.starts_with("pfx-") || RESERVED.contains(&name) {
        return Err(format!("{name:?} is taken; name the game something else"));
    }
    Ok(())
}

fn type_name(name: &str) -> String {
    name.split('-')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

pub fn fill(template: &str, name: &str, version: &str) -> String {
    template
        .replace("{{name}}", name)
        .replace("{{crate}}", &name.replace('-', "_"))
        .replace("{{type}}", &type_name(name))
        .replace("{{tag}}", &format!("v{version}"))
        .replace("{{source}}", SOURCE)
        .replace("{{licence}}", LICENCE)
}

fn empty(folder: &Path) -> Result<(), String> {
    if !folder.is_dir() {
        return Err(format!(
            "{}: no such folder; make the game's repo first, then scaffold into it",
            folder.display()
        ));
    }
    let mut found: Vec<String> = std::fs::read_dir(folder)
        .map_err(|error| format!("{}: {error}", folder.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name != ".git")
        .collect();
    found.sort();
    if found.is_empty() {
        return Ok(());
    }
    Err(format!(
        "{} is not empty ({}); pfx new scaffolds only into an empty repo folder",
        folder.display(),
        found.join(", ")
    ))
}

fn write(folder: &Path, relative: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    use std::io::Write;
    let path = folder.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("{}: {error}", parent.display()))?;
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    file.write_all(bytes)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(path)
}

fn check(folder: &Path) -> Result<(), String> {
    let project = pfx_scene::Project::open(folder).map_err(|found| found.message)?;
    let errors: Vec<String> = project
        .check()
        .into_iter()
        .filter(|found| found.severity == pfx_scene::Severity::Error)
        .map(|found| {
            let mut line = format!(
                "{}:{}: {}: {}",
                found.file.display(),
                found.line,
                found.code,
                found.message
            );
            for place in &found.related {
                line.push_str(&format!("\n  {place}: see here"));
            }
            line
        })
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "the project does not check:\n{}",
            errors.join("\n")
        ))
    }
}

pub(crate) fn fix_ids(folder: &Path) -> Result<(), String> {
    check(folder)?;
    let project = pfx_scene::Project::open(folder).map_err(|found| found.message)?;
    let group = project
        .fix(&folder.join(SCENE))
        .map_err(|error| error.to_string())?;
    if !group.is_empty() {
        group.write().map_err(|error| error.to_string())?;
    }
    check(folder)
}

fn pack(folder: &Path, written: &mut Vec<PathBuf>) -> Result<(), String> {
    use pfx_file::{Reader, SCNE, Writer};
    let project = pfx_scene::Project::open(folder).map_err(|found| found.message)?;
    let bytes = pfx_scene::pack(&project, &folder.join(SCENE)).map_err(|found| {
        found
            .iter()
            .map(|diagnostic| diagnostic.message.clone())
            .collect::<Vec<_>>()
            .join("\n")
    })?;
    let reader = Reader::from_bytes(&bytes).map_err(|error| error.to_string())?;
    let mut writer = Writer::new();
    let mut held = Vec::new();
    for (index, entry) in reader.entries().iter().enumerate() {
        if entry.tag == pfx_file::FILE {
            continue;
        }
        if entry.tag == SCNE {
            held.push(folder.join(&entry.name));
        }
        writer = writer
            .copy(&reader, index)
            .map_err(|error| error.to_string())?;
    }
    let packed = writer.to_vec().map_err(|error| error.to_string())?;
    written.push(write(folder, DOCUMENT, &packed)?);
    for file in &held {
        std::fs::remove_file(file).map_err(|error| format!("{}: {error}", file.display()))?;
        if let Some(parent) = file.parent()
            && parent != folder.join("content")
        {
            let _ = std::fs::remove_dir(parent);
        }
    }
    written.retain(|path| !held.contains(path));
    let manifest = folder.join(FILE);
    let text = std::fs::read_to_string(&manifest)
        .map_err(|error| format!("{}: {error}", manifest.display()))?;
    std::fs::write(&manifest, text.replace(SCENE, DOCUMENT))
        .map_err(|error| format!("{}: {error}", manifest.display()))
}

pub fn module_path(name: &str) -> String {
    format!(
        "target/wasm32-unknown-unknown/release/{}_module.wasm",
        name.replace('-', "_")
    )
}

fn with_module(relative: &str, text: String, name: &str, version: &str) -> String {
    match relative {
        "Cargo.toml" => {
            let game = fill(
                "pfx-game = { git = \"{{source}}\", tag = \"{{tag}}\" }",
                name,
                version,
            );
            let modules = game.replacen(" }", ", features = [\"modules\"] }", 1);
            text.replacen(&game, &modules, 1).replacen(
                "[workspace]\n",
                "[workspace]\nmembers = [\".\", \"module\"]\ndefault-members = [\".\"]\n",
                1,
            )
        }
        "src/lib.rs" => fill(MODULE_LIB, name, version),
        FILE => text.replacen(
            "[authoring.pfx]\n",
            &format!("[authoring.pfx]\nmodules = [\"{}\"]\n", module_path(name)),
            1,
        ),
        "AGENTS.md" => text + &fill(MODULE_AGENTS, name, version),
        _ => text,
    }
}

fn point(at: [f32; 3]) -> String {
    let round = |value: f32| (f64::from(value) * 1e4).round() / 1e4;
    format!(
        "[{:?}, {:?}, {:?}]",
        round(at[0]),
        round(at[1]),
        round(at[2])
    )
}

fn with_mark(relative: &str, text: String) -> String {
    match relative {
        "src/lib.rs" => text.replacen(
            ".font(include_bytes!(\"../content/fonts/IBMPlexMono-Regular.ttf\").as_slice());",
            &format!(".font(include_bytes!(\"../content/fonts/IBMPlexMono-Regular.ttf\").as_slice())\n        .mark(project.root().join(\"{MARK}\"));"),
            1,
        ),
        _ => text,
    }
}

struct Picture {
    bytes: Vec<u8>,
    size: [u32; 2],
}

fn picture(path: &Path) -> Result<Picture, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let size = crate::title::picture_size(&bytes)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(Picture { bytes, size })
}

fn title(
    folder: &Path,
    name: &str,
    mark: Option<&Picture>,
    written: &mut Vec<PathBuf>,
) -> Result<(), String> {
    if let Some(mark) = mark {
        written.push(write(folder, MARK, &mark.bytes)?);
    }
    let picture = mark.map(|mark| mark.size);
    let title = crate::title::build(FONT, name, picture)?;
    written.push(write(folder, "content/meshes/name.glb", &title.name)?);
    written.push(write(folder, "content/meshes/mark.glb", &title.mark)?);
    let content = if picture.is_some() {
        "content = \"mark\"\n"
    } else {
        ""
    };
    let mut prefab = TITLE
        .replace("{{name_at}}", &point(title.name_at))
        .replace("{{mark_at}}", &point(title.mark_at))
        .replace("{{version_at}}", &point(title.version_at))
        .replace(
            "{{version_size}}",
            &format!("{:?}", (f64::from(title.version_size) * 1e4).round() / 1e4),
        )
        .replace("{{mark_content}}", content);
    let mut materials = include_str!("templates/title.materials.toml").to_string();
    if picture.is_some() {
        prefab.push_str(&format!("\n[content.mark]\nimage = \"{MARK}\"\n"));
        materials.push_str(FACE);
    }
    written.push(write(folder, PREFAB, prefab.as_bytes())?);
    written.push(write(
        folder,
        "content/materials/title.materials.toml",
        materials.as_bytes(),
    )?);
    Ok(())
}

pub fn scaffold(name: &str, folder: &Path, version: &str) -> Result<Vec<PathBuf>, String> {
    scaffold_with(name, folder, version, &Options::default())
}

pub fn scaffold_with(
    name: &str,
    folder: &Path,
    version: &str,
    options: &Options,
) -> Result<Vec<PathBuf>, String> {
    check_name(name)?;
    let mark = options.mark.as_deref().map(picture).transpose()?;
    let folder = std::path::absolute(folder).map_err(|error| error.to_string())?;
    empty(&folder)?;
    let module = options.module;
    let mut written = Vec::new();
    for (relative, template) in TEMPLATES {
        let mut text = fill(template, name, version);
        if module {
            text = with_module(relative, text, name, version);
        }
        if mark.is_some() {
            text = with_mark(relative, text);
        }
        written.push(write(&folder, relative, text.as_bytes())?);
    }
    title(&folder, name, mark.as_ref(), &mut written)?;
    if module {
        for (relative, template) in MODULE {
            let text = fill(template, name, version);
            written.push(write(&folder, relative, text.as_bytes())?);
        }
    }
    for (relative, bytes) in FILES {
        written.push(write(&folder, relative, bytes)?);
    }
    fix_ids(&folder)?;
    pack(&folder, &mut written)?;
    check(&folder)?;
    Ok(written)
}
