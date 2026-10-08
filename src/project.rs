use std::path::PathBuf;

use pfx_core::cli;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    pub name: String,
    pub path: PathBuf,
    pub module: bool,
}

pub fn plan(args: &[String]) -> Result<Command, String> {
    let mut name = None;
    let mut path = None;
    let mut module = false;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--path" if path.is_some() => return Err(cli::repeated(&cli::with_value("--path"))),
            "--path" => {
                let dir = rest.next().ok_or_else(|| cli::missing_value("--path"))?;
                path = Some(PathBuf::from(dir));
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
    Ok(Command { name, path, module })
}

pub fn run(command: &Command) -> Result<(), String> {
    let Command { name, path, module } = command;
    let written =
        pfx_project::scaffold::scaffold_with(name, path, env!("CARGO_PKG_VERSION"), *module)?;
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
