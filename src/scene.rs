use std::path::PathBuf;

use pfx_core::cli::{self, Refusal};
use pfx_load::scene::PatchGroup;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Migrate,
    Fix,
    Pack(PathBuf),
    Unpack(PathBuf),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    pub action: Action,
    pub path: PathBuf,
    pub dry_run: bool,
}

pub fn plan(args: &[String]) -> Result<Command, Refusal> {
    let action = match args.first().map(String::as_str) {
        None => return Err(cli::no_command("pfx scene", &["migrate", "fix"]).into()),
        Some("migrate") => Action::Migrate,
        Some("fix") => Action::Fix,
        Some(flag) if flag.starts_with('-') => return Err(cli::unexpected(flag).into()),
        Some(word) => return Err(cli::unrecognized(word).into()),
    };
    let usage = format!("pfx scene {} [OPTIONS] <SCENE>", args[0]);
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut dry_run = false;
    let mut mode: Option<&str> = None;
    for arg in &args[1..] {
        match arg.as_str() {
            "--dry-run" if dry_run => {
                return Err(Refusal::new(cli::repeated(arg)).under(usage));
            }
            "--dry-run" => dry_run = true,
            flag @ ("--manf" | "--unpack") if action == Action::Migrate => match mode {
                Some(first) if first == flag => {
                    return Err(Refusal::new(cli::repeated(flag)).under(usage));
                }
                Some(first) => {
                    return Err(Refusal::new(cli::conflict(flag, first)).under(usage));
                }
                None => mode = Some(flag),
            },
            other if other.starts_with('-') || paths.len() > usize::from(mode.is_some()) => {
                return Err(Refusal::new(cli::unexpected(other)).under(usage));
            }
            file => paths.push(PathBuf::from(file)),
        }
    }
    if let (Some(flag), true) = (mode, dry_run) {
        return Err(Refusal::new(cli::conflict("--dry-run", flag)).under(usage));
    }
    let names: &[&str] = match mode {
        Some("--manf") => &["<SCENE>", "<OUT>"],
        Some(_) => &["<MANF>", "<DIR>"],
        None => &["<SCENE>"],
    };
    if paths.len() < names.len() {
        return Err(Refusal::new(cli::required(&names[paths.len()..])).under(usage));
    }
    let second = paths.get(1).cloned();
    let action = match (mode, second) {
        (Some("--manf"), Some(out)) => Action::Pack(out),
        (Some(_), Some(dir)) => Action::Unpack(dir),
        _ => action,
    };
    Ok(Command {
        action,
        path: paths.swap_remove(0),
        dry_run,
    })
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
    let (outcome, done, would) = match &command.action {
        Action::Pack(out) => {
            let bytes =
                pfx_load::scene::pack(&command.path, out).map_err(|error| error.to_string())?;
            println!(
                "packed {} into {} ({bytes} bytes)",
                command.path.display(),
                out.display()
            );
            return Ok(());
        }
        Action::Unpack(dir) => {
            let written =
                pfx_load::scene::unpack(&command.path, dir).map_err(|error| error.to_string())?;
            for file in written {
                println!("wrote {}", file.display());
            }
            return Ok(());
        }
        Action::Migrate => (
            pfx_load::scene::migrate(&command.path, command.dry_run),
            "migrated",
            "would migrate",
        ),
        Action::Fix => (
            pfx_load::scene::fix(&command.path, command.dry_run),
            "fixed",
            "would fix",
        ),
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
            }
        );
        assert_eq!(
            plan(&words("fix room.scene.toml")).unwrap(),
            Command {
                action: Action::Fix,
                path: PathBuf::from("room.scene.toml"),
                dry_run: false,
            }
        );
        assert!(plan(&words("fix")).is_err());
        assert!(plan(&words("tidy room.scene.toml")).is_err());
        assert!(plan(&words("fix a.scene.toml b.scene.toml")).is_err());
        assert!(plan(&words("migrate a.scene.toml --force")).is_err());
        assert_eq!(
            plan(&words("migrate --manf a.scene.toml a.manf"))
                .unwrap()
                .action,
            Action::Pack(PathBuf::from("a.manf"))
        );
        assert_eq!(
            plan(&words("migrate --unpack a.manf out")).unwrap().action,
            Action::Unpack(PathBuf::from("out"))
        );
        assert!(plan(&words("migrate --manf a.scene.toml")).is_err());
        assert!(plan(&words("migrate --manf --unpack a b")).is_err());
        assert!(plan(&words("migrate --manf a b --dry-run")).is_err());
        assert!(plan(&words("fix --manf a b")).is_err());
    }
}
