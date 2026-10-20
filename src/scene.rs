use std::path::{Path, PathBuf};

use pfx_core::cli::{self, Refusal};
use pfx_load::scene::PatchGroup;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Migrate,
    Fix,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    pub action: Action,
    pub path: PathBuf,
    pub dry_run: bool,
    pub bare: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Legacy {
    Recipe,
    Scene,
}

const SKIPPED: [&str; 2] = ["target", "tmp"];

pub fn plan(args: &[String]) -> Result<Command, Refusal> {
    let action = match args.first().map(String::as_str) {
        None => {
            return Err(cli::no_command(
                "pfx scene",
                &[
                    "migrate", "fix", "list", "save", "print", "write", "fold", "compact",
                ],
            )
            .into());
        }
        Some("migrate") => Action::Migrate,
        Some("fix") => Action::Fix,
        Some(flag) if flag.starts_with('-') => return Err(cli::unexpected(flag).into()),
        Some(word) => return Err(cli::unrecognized(word).into()),
    };
    let usage = match action {
        Action::Migrate => "pfx scene migrate [OPTIONS] <FILE>",
        Action::Fix => "pfx scene fix [OPTIONS] <SCENE>",
    };
    let mut path: Option<PathBuf> = None;
    let mut dry_run = false;
    let mut bare = false;
    for arg in &args[1..] {
        match arg.as_str() {
            "--bare" if action == Action::Fix => {
                return Err(Refusal::new(cli::unexpected(arg)).under(usage));
            }
            "--bare" if bare => {
                return Err(Refusal::new(cli::repeated(arg)).under(usage));
            }
            "--bare" => bare = true,
            "--dry-run" if dry_run => {
                return Err(Refusal::new(cli::repeated(arg)).under(usage));
            }
            "--dry-run" => dry_run = true,
            other if other.starts_with('-') || path.is_some() => {
                return Err(Refusal::new(cli::unexpected(other)).under(usage));
            }
            file => path = Some(PathBuf::from(file)),
        }
    }
    let Some(path) = path else {
        let name = match action {
            Action::Migrate => "<FILE>",
            Action::Fix => "<SCENE>",
        };
        return Err(Refusal::new(cli::required(&[name])).under(usage));
    };
    Ok(Command {
        action,
        path,
        dry_run,
        bare,
    })
}

fn is_toml(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("toml"))
}

pub fn imported(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?.to_string_lossy().to_string();
    let lower = name.to_lowercase();
    let stem = if lower.ends_with(".scene.toml") {
        &name[..name.len() - ".scene.toml".len()]
    } else if lower.ends_with(".toml") {
        &name[..name.len() - ".toml".len()]
    } else {
        return None;
    };
    Some(path.with_file_name(format!("{stem}.pitofx")))
}

pub fn legacy(path: &Path) -> Option<String> {
    if !is_toml(path) {
        return None;
    }
    let target = imported(path).unwrap_or_else(|| path.with_extension(pfx_file::EXTENSION));
    Some(format!(
        "{} is a plain TOML file; import it once with `pfx scene migrate {}`, which writes {} beside it",
        path.display(),
        path.display(),
        target.display()
    ))
}

fn kind(path: &Path) -> Option<Legacy> {
    let lower = path.file_name()?.to_string_lossy().to_lowercase();
    if lower.ends_with(".scene.toml") {
        return Some(Legacy::Scene);
    }
    if !lower.ends_with(".toml") {
        return None;
    }
    let table = std::fs::read_to_string(path)
        .ok()?
        .parse::<toml::Table>()
        .ok()?;
    let header = |key: &str| table.get(key).is_some_and(toml::Value::is_integer);
    if header("recipe") {
        Some(Legacy::Recipe)
    } else if table.contains_key("format") && !lower.ends_with("materials.toml") {
        Some(Legacy::Scene)
    } else {
        None
    }
}

fn strip(doc: &mut pfx_build::Document, recipe: &Path) {
    let depth = Path::new(&doc.recipe_name).components().count();
    let root = recipe.ancestors().nth(depth).unwrap_or(Path::new(""));
    let outputs = std::mem::take(&mut doc.outputs);
    doc.files.retain(|name, bytes| {
        !outputs.contains(name) || std::fs::read(root.join(name)).ok().as_ref() == Some(bytes)
    });
    doc.scene.clear();
    doc.gltf.clear();
    doc.gbin.clear();
    doc.facts.clear();
    doc.nodes.clear();
    doc.thumb = None;
    doc.built_from = None;
    doc.scene_hash = None;
}

fn import_recipe(path: &Path, target: &Path, bare: bool) -> Result<Vec<String>, String> {
    let mut doc = pfx_build::Document::pack_for(path, &pfx_build::Registry::new(), target)?;
    for note in &doc.notes {
        eprintln!("{}: {note}", path.display());
    }
    if bare {
        strip(&mut doc, path);
    }
    doc.write(target)?;
    let folder = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .display()
        .to_string();
    let mut lines: Vec<String> = doc
        .outside
        .iter()
        .map(|name| format!("packed {name} from outside {folder}"))
        .collect();
    lines.extend(
        doc.referenced.iter().map(|name| {
            format!("kept {name} beside the .pitofx, which reads it from the disk there")
        }),
    );
    lines.extend(
        doc.unread
            .iter()
            .map(|name| format!("could not read {name}")),
    );
    Ok(lines)
}

fn import_scene(path: &Path, target: &Path) -> Result<(), String> {
    pfx_load::scene::pack(path, target).map_err(|e| e.to_string())?;
    let migrated = pfx_load::scene::migrate(target, false);
    if let Err(e) = migrated {
        let _ = std::fs::remove_file(target);
        return Err(e.to_string());
    }
    Ok(())
}

fn import(path: &Path, dry_run: bool, bare: bool) -> Result<String, String> {
    let shown = path.display();
    let found =
        kind(path).ok_or_else(|| format!("{shown}: not a recipe (`recipe = 1`) or a scene"))?;
    let target = imported(path).ok_or_else(|| format!("{shown}: not a .toml file"))?;
    if target.exists() {
        return Err(format!(
            "{} already exists; {shown} was imported before, and edits go to the .pitofx",
            target.display()
        ));
    }
    if dry_run {
        return Ok(format!("would import {shown} into {}", target.display()));
    }
    let mut lines = match found {
        Legacy::Recipe => import_recipe(path, &target, bare)?,
        Legacy::Scene => {
            import_scene(path, &target)?;
            Vec::new()
        }
    };
    lines.push(format!("imported {shown} into {}", target.display()));
    Ok(lines.join("\n"))
}

fn gather(folder: &Path, top: bool, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.filter_map(|e| Some(e.ok()?.path())).collect();
    paths.sort();
    for path in paths {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if name.starts_with('.') || (top && SKIPPED.contains(&name.as_str())) {
            continue;
        }
        if path.is_dir() {
            gather(&path, false, out);
        } else if name.to_lowercase().ends_with(".scene.toml")
            || kind(&path) == Some(Legacy::Recipe)
        {
            out.push(path);
        }
    }
}

fn import_folder(folder: &Path, dry_run: bool, bare: bool) -> Result<String, String> {
    let mut files = Vec::new();
    gather(folder, true, &mut files);
    let mut lines = Vec::new();
    let mut failed = Vec::new();
    for file in &files {
        if imported(file).is_some_and(|target| target.exists()) {
            lines.push(format!("kept {}: imported before", file.display()));
            continue;
        }
        match import(file, dry_run, bare) {
            Ok(line) => lines.push(line),
            Err(e) => failed.push(e),
        }
    }
    if lines.is_empty() && failed.is_empty() {
        return Ok(format!(
            "{}: holds no recipe or scene TOML to import",
            folder.display()
        ));
    }
    if failed.is_empty() {
        Ok(lines.join("\n"))
    } else {
        if !lines.is_empty() {
            println!("{}", lines.join("\n"));
        }
        Err(failed.join("\n"))
    }
}

fn report(group: &PatchGroup, done: &str, would: &str, dry_run: bool) -> String {
    if group.is_empty() {
        return "nothing to change".to_string();
    }
    let verb = if dry_run { would } else { done };
    group
        .files()
        .map(|file| format!("{verb} {}", file.display()))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn run(command: &Command) -> Result<(), String> {
    let path = &command.path;
    let pfx = pfx_load::scene::is_pfx_path(path);
    let (outcome, done, would) = match command.action {
        Action::Migrate if path.is_dir() => {
            println!("{}", import_folder(path, command.dry_run, command.bare)?);
            return Ok(());
        }
        Action::Migrate if !pfx => {
            println!("{}", import(path, command.dry_run, command.bare)?);
            return Ok(());
        }
        Action::Migrate => (
            pfx_load::scene::migrate(path, command.dry_run),
            "migrated",
            "would migrate",
        ),
        Action::Fix => {
            if let Some(refusal) = legacy(path) {
                return Err(refusal);
            }
            (
                pfx_load::scene::fix(path, command.dry_run),
                "fixed",
                "would fix",
            )
        }
    };
    let group = outcome.map_err(|error| error.to_string())?;
    println!("{}", report(&group, done, would, command.dry_run));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn migrate_and_fix_take_a_path_and_a_dry_run() {
        assert_eq!(
            plan(&words("migrate a/room.scene.toml --dry-run")).unwrap(),
            Command {
                action: Action::Migrate,
                path: PathBuf::from("a/room.scene.toml"),
                dry_run: true,
                bare: false,
            }
        );
        assert_eq!(
            plan(&words("fix room.pitofx")).unwrap(),
            Command {
                action: Action::Fix,
                path: PathBuf::from("room.pitofx"),
                dry_run: false,
                bare: false,
            }
        );
        assert!(plan(&words("fix")).is_err());
        assert!(plan(&words("tidy room.pitofx")).is_err());
        assert!(plan(&words("fix a.pitofx b.pitofx")).is_err());
        assert!(plan(&words("migrate a.scene.toml --force")).is_err());
        assert!(plan(&words("migrate --pfx a.scene.toml a.pitofx")).is_err());
        assert!(plan(&words("migrate --unpack a.pitofx out")).is_err());
        assert!(plan(&words("migrate a.toml --bare")).unwrap().bare);
        assert!(plan(&words("fix a.pitofx --bare")).is_err());
    }

    #[test]
    fn a_toml_is_refused_with_the_command_that_imports_it() {
        assert_eq!(
            legacy(Path::new("a/room.scene.toml")).unwrap(),
            "a/room.scene.toml is a plain TOML file; import it once with `pfx scene migrate a/room.scene.toml`, which writes a/room.pitofx beside it"
        );
        assert!(
            legacy(Path::new("street.TOML"))
                .unwrap()
                .contains("street.pitofx")
        );
        assert_eq!(legacy(Path::new("a/room.pitofx")), None);
        assert_eq!(legacy(Path::new("a/model.glb")), None);
    }
}
