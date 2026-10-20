use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use clap::Subcommand;
use serde_json::json;

use super::refuse;

pub const ATTRIBUTE: &str = "*.pitofx diff=pitofx";
pub const CONFIG: [(&str, &str); 2] = [
    ("diff.pitofx.textconv", "pfx scene print --all"),
    ("diff.pitofx.binary", "false"),
];

#[derive(clap::Args)]
pub(super) struct Args {
    #[command(subcommand)]
    verb: GitVerb,
}

#[derive(Subcommand)]
enum GitVerb {
    #[command(
        about = "Make git diff show a .pitofx as its TOML: writes .gitattributes and the repo's own git config, once"
    )]
    Setup(SetupArgs),
}

#[derive(clap::Args)]
struct SetupArgs {
    #[arg(
        value_name = "folder",
        default_value = ".",
        help = "A folder inside the git repository"
    )]
    folder: PathBuf,
    #[arg(long, help = "Print one JSON object")]
    json: bool,
}

pub struct Setup {
    pub repo: PathBuf,
    pub attribute: bool,
    pub config: Vec<(&'static str, &'static str, bool)>,
}

fn git(folder: &Path, args: &[&str]) -> Result<Option<String>, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(folder)
        .args(args)
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if !out.status.success() {
        return Ok(None);
    }
    Ok(Some(
        String::from_utf8_lossy(&out.stdout)
            .trim_end_matches(['\n', '\r'])
            .to_string(),
    ))
}

fn has_attribute(text: &str) -> bool {
    text.lines().any(|line| {
        let mut fields = line.split_whitespace();
        fields.next() == Some("*.pitofx") && fields.any(|field| field == "diff=pitofx")
    })
}

fn attribute(repo: &Path) -> Result<bool, String> {
    let path = repo.join(".gitattributes");
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    if has_attribute(&text) {
        return Ok(false);
    }
    let mut next = text;
    if !next.is_empty() && !next.ends_with('\n') {
        next.push('\n');
    }
    next.push_str(ATTRIBUTE);
    next.push('\n');
    std::fs::write(&path, next).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(true)
}

pub fn setup(folder: &Path) -> Result<Setup, String> {
    let shown = folder.display();
    if !folder.is_dir() {
        return Err(format!("{shown}: no such folder"));
    }
    let top = git(folder, &["rev-parse", "--show-toplevel"])?
        .filter(|top| !top.is_empty())
        .ok_or_else(|| format!("{shown}: not inside a git repository (git init first)"))?;
    let repo = PathBuf::from(top);
    let attribute = attribute(&repo)?;
    let mut config = Vec::new();
    for (key, value) in CONFIG {
        let now = git(&repo, &["config", "--local", "--get", key])?;
        let changed = now.as_deref() != Some(value);
        if changed {
            git(&repo, &["config", "--local", key, value])?
                .ok_or_else(|| format!("git config --local {key}: refused"))?;
        }
        config.push((key, value, changed));
    }
    Ok(Setup {
        repo,
        attribute,
        config,
    })
}

fn report(setup: &Setup, as_json: bool) -> String {
    if as_json {
        let config: Vec<_> = setup
            .config
            .iter()
            .map(|(key, value, changed)| json!({"key": key, "value": value, "changed": changed}))
            .collect();
        return json!({
            "repo": setup.repo,
            "gitattributes": {"line": ATTRIBUTE, "changed": setup.attribute},
            "config": config,
        })
        .to_string();
    }
    let said = |changed: bool| if changed { "written" } else { "already there" };
    let mut lines = vec![format!(
        ".gitattributes: {ATTRIBUTE} ({})",
        said(setup.attribute)
    )];
    for (key, value, changed) in &setup.config {
        lines.push(format!("git config {key} = {value} ({})", said(*changed)));
    }
    lines.join("\n")
}

pub(super) fn run(args: &Args) -> ExitCode {
    let GitVerb::Setup(setup_args) = &args.verb;
    match setup(&setup_args.folder) {
        Ok(done) => {
            println!("{}", report(&done, setup_args.json));
            ExitCode::SUCCESS
        }
        Err(e) => refuse("git setup", &e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tmp")
            .join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn setup_writes_the_attribute_and_the_local_config_once_and_refuses_outside_a_repo() {
        let dir = scratch("pfx-git-setup");
        let refused = setup(&dir.join("missing")).err().unwrap();
        assert!(refused.ends_with("no such folder"), "{refused}");
        let alone = Command::new("git")
            .args(["init", "-q"])
            .arg(&dir)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .status()
            .unwrap();
        assert!(alone.success());
        std::fs::write(dir.join(".gitattributes"), "*.png binary").unwrap();
        let first = setup(&dir).unwrap();
        assert!(first.attribute);
        assert!(first.config.iter().all(|(_, _, changed)| *changed));
        assert_eq!(
            std::fs::read_to_string(dir.join(".gitattributes")).unwrap(),
            "*.png binary\n*.pitofx diff=pitofx\n"
        );
        assert_eq!(
            git(
                &dir,
                &["config", "--local", "--get", "diff.pitofx.textconv"]
            )
            .unwrap(),
            Some("pfx scene print --all".to_string())
        );
        let sub = dir.join("content");
        std::fs::create_dir_all(&sub).unwrap();
        let again = setup(&sub).unwrap();
        assert!(!again.attribute);
        assert!(again.config.iter().all(|(_, _, changed)| !*changed));
        let json: serde_json::Value = serde_json::from_str(&report(&again, true)).unwrap();
        assert_eq!(json["gitattributes"]["changed"], false);
        assert_eq!(json["config"][0]["value"], "pfx scene print --all");
        assert!(has_attribute("*.pitofx  -text diff=pitofx\n"));
        assert!(!has_attribute("*.pitofx -diff\n"));
    }
}
