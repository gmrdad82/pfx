use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;

use pfx_scene::{EditError, PatchGroup, SceneEdit};

const USAGE: &str = "usage: scene-write <scene> move <object> <x> <y> <z>
       scene-write <scene> material <object> <material>
       scene-write <scene> rename <object> <new name>";

fn paint(code: &str, text: &str) -> String {
    if std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none() {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

fn number(text: &str) -> Option<f32> {
    text.parse().ok()
}

fn edit(scene: &str, words: &[String]) -> Result<(PathBuf, PatchGroup), EditError> {
    let mut edit = SceneEdit::open(scene)?;
    let group = match words {
        [verb, object, x, y, z] if verb == "move" => {
            let (Some(x), Some(y), Some(z)) = (number(x), number(y), number(z)) else {
                return Err(usage());
            };
            edit.set_at(object, [x, y, z])
        }
        [verb, object, material] if verb == "material" => {
            edit.set_object_material(object, Some(material))
        }
        [verb, object, new] if verb == "rename" => edit.rename_object(object, new),
        _ => Err(usage()),
    }?;
    Ok((edit.root().to_path_buf(), group))
}

fn usage() -> EditError {
    EditError {
        file: PathBuf::new(),
        key: String::new(),
        line: None,
        code: "usage",
        message: USAGE.into(),
        diagnostics: Vec::new(),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let done = match args.as_slice() {
        [scene, words @ ..] => match edit(scene, words) {
            Ok((root, group)) => {
                let files: Vec<String> = group
                    .files()
                    .map(|f| f.strip_prefix(&root).unwrap_or(f).display().to_string())
                    .collect();
                Ok(format!("{}: wrote {}", group.label(), files.join(", ")))
            }
            Err(error) if error.code == "usage" => {
                eprintln!("{USAGE}");
                return ExitCode::from(2);
            }
            Err(error) => Err(format!("[{}] {}", error.code, error.message)),
        },
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match done {
        Ok(line) => {
            println!("{} {line}", paint("1;32", "ok"));
            ExitCode::SUCCESS
        }
        Err(line) => {
            println!("{} {line}", paint("1;31", "refused"));
            ExitCode::FAILURE
        }
    }
}
