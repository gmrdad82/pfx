use std::path::{Path, PathBuf};

use pfx_core::cli;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    pub name: String,
    pub path: PathBuf,
    pub module: bool,
    pub mark: Option<PathBuf>,
}

pub fn plan(args: &[String]) -> Result<Command, String> {
    let mut name = None;
    let mut path = None;
    let mut module = false;
    let mut mark = None;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--path" if path.is_some() => return Err(cli::repeated(&cli::with_value("--path"))),
            "--path" => {
                let dir = rest.next().ok_or_else(|| cli::missing_value("--path"))?;
                path = Some(PathBuf::from(dir));
            }
            "--mark" if mark.is_some() => return Err(cli::repeated(&cli::with_value("--mark"))),
            "--mark" => {
                let png = rest.next().ok_or_else(|| cli::missing_value("--mark"))?;
                mark = Some(PathBuf::from(png));
            }
            "--with-module" if module => return Err(cli::repeated(arg)),
            "--with-module" => module = true,
            other if other.starts_with('-') || name.is_some() => {
                return Err(cli::unexpected(other));
            }
            word => name = Some(word.to_string()),
        }
    }
    let name = name.ok_or_else(|| cli::required(&["<GAME>"]))?;
    let path = path.unwrap_or_else(|| PathBuf::from(&name));
    Ok(Command {
        name,
        path,
        module,
        mark,
    })
}

pub fn run(command: &Command) -> Result<(), String> {
    let Command {
        name,
        path,
        module,
        mark,
    } = command;
    let options = pfx_project::scaffold::Options {
        module: *module,
        mark: mark.clone(),
    };
    let written =
        pfx_project::scaffold::scaffold_with(name, path, env!("CARGO_PKG_VERSION"), &options)?;
    let git = if path.join(".git").exists() {
        crate::recipe::git::setup(path)?;
        "git: diffs of .pitofx show their TOML (pfx scene print --all)"
    } else {
        "git: once the repo exists, pfx git setup makes its diffs of .pitofx show their TOML"
    };
    for line in closing(name, path, written.len(), *module, git) {
        println!("{line}");
    }
    Ok(())
}

fn closing(name: &str, path: &Path, files: usize, module: bool, git: &str) -> Vec<String> {
    let mut lines = vec![
        format!(
            "{name}: scaffolded {files} files into {} on pfx v{}",
            path.display(),
            env!("CARGO_PKG_VERSION")
        ),
        git.to_string(),
        "next: cargo run --release (the game), cargo run --release -- edit (the editor)"
            .to_string(),
    ];
    if module {
        lines.push(format!(
            "the gameplay module: cargo build -p {name}-module --target wasm32-unknown-unknown --release"
        ));
    }
    lines.push(pfx_project::scaffold::LICENCE.to_string());
    lines
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::closing;

    #[test]
    fn pfx_new_closes_with_the_licence_line_alone() {
        let line = "This game links pfx: publish its source under the AGPL-3.0, or use pfx's free licence (closed use under $200k a year, education, non-profits) or a commercial one: https://pitomd.com/pfx/license";
        for module in [false, true] {
            let lines = closing("my-game", Path::new("my-game"), 15, module, "git: line");
            assert_eq!(lines.last().map(String::as_str), Some(line));
            assert_eq!(
                lines
                    .iter()
                    .filter(|text| text.contains("licen") || text.contains("AGPL"))
                    .count(),
                1,
                "{lines:?}"
            );
        }
    }
}
