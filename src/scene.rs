use std::path::PathBuf;

use pfx_core::cli::{self, Refusal};
use pfx_load::scene::PatchGroup;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Migrate,
    Fix,
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
    let mut path = None;
    let mut dry_run = false;
    for arg in &args[1..] {
        match arg.as_str() {
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
    let path = path.ok_or_else(|| Refusal::new(cli::required(&["<SCENE>"])).under(usage))?;
    Ok(Command {
        action,
        path,
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
    let (outcome, done, would) = match command.action {
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
    }
}
