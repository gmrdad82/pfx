use std::path::{Path, PathBuf};

use crate::project::FILE;

pub const SOURCE: &str = "https://github.com/gmrdad82/pfx.git";
pub const SCENE: &str = "content/first.scene.toml";

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

const TEMPLATES: [(&str, &str); 13] = [
    ("Cargo.toml", include_str!("templates/Cargo.toml.in")),
    ("src/lib.rs", include_str!("templates/lib.rs.in")),
    ("src/main.rs", include_str!("templates/main.rs.in")),
    ("tests/first.rs", include_str!("templates/first.rs.in")),
    (FILE, include_str!("templates/project.toml.in")),
    (SCENE, include_str!("templates/first.scene.toml.in")),
    (
        "content/materials/base.materials.toml",
        include_str!("templates/base.materials.toml.in"),
    ),
    (
        "content/materials/incoming.materials.toml",
        include_str!("templates/incoming.materials.toml"),
    ),
    (
        "content/prefabs/incoming.prefab.toml",
        include_str!("templates/incoming.prefab.toml.in"),
    ),
    ("AGENTS.md", include_str!("templates/AGENTS.md.in")),
    ("CLAUDE.md", "@AGENTS.md\n"),
    ("prompts/RESUME.md", include_str!("templates/RESUME.md.in")),
    (".gitignore", include_str!("templates/gitignore.in")),
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
        "content/meshes/badge.glb",
        include_bytes!("templates/badge.glb"),
    ),
    (
        "content/meshes/wordmark.glb",
        include_bytes!("templates/wordmark.glb"),
    ),
    (
        "content/meshes/grid.glb",
        include_bytes!("templates/grid.glb"),
    ),
    (
        "content/fonts/IBMPlexMono-Regular.ttf",
        include_bytes!("templates/IBMPlexMono-Regular.ttf"),
    ),
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
            format!(
                "[workspace]\nmembers = [\".\", \"module\"]\ndefault-members = [\".\"]\n\n{}",
                text.replacen(&game, &modules, 1)
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

pub fn scaffold(name: &str, folder: &Path, version: &str) -> Result<Vec<PathBuf>, String> {
    scaffold_with(name, folder, version, false)
}

pub fn scaffold_with(
    name: &str,
    folder: &Path,
    version: &str,
    module: bool,
) -> Result<Vec<PathBuf>, String> {
    check_name(name)?;
    let folder = std::path::absolute(folder).map_err(|error| error.to_string())?;
    empty(&folder)?;
    let mut written = Vec::new();
    for (relative, template) in TEMPLATES {
        let mut text = fill(template, name, version);
        if module {
            text = with_module(relative, text, name, version);
        }
        written.push(write(&folder, relative, text.as_bytes())?);
    }
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
    Ok(written)
}
