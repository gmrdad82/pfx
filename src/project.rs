use std::path::PathBuf;

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
    println!(
        "{name}: scaffolded {} files into {} on pfx v{}",
        written.len(),
        path.display(),
        env!("CARGO_PKG_VERSION")
    );
    println!("next: cargo run --release (the game), cargo run --release -- edit (the editor)");
    if *module {
        println!(
            "the gameplay module: cargo build -p {name}-module --target wasm32-unknown-unknown --release"
        );
    }
    Ok(())
}
