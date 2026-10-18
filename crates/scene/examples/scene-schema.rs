use std::path::Path;
use std::process::ExitCode;

use pfx_scene::{FileKind, json_schema};

const USAGE: &str = "usage: scene-schema <folder>";

const KINDS: &[(FileKind, &str)] = &[
    (FileKind::Project, "project"),
    (FileKind::Scene, "scene"),
    (FileKind::Prefab, "prefab"),
    (FileKind::Materials, "materials"),
    (FileKind::Proxies, "proxies"),
    (FileKind::Finish, "finish"),
];

fn run(args: &[String]) -> Result<Vec<String>, String> {
    let [folder] = args else {
        return Err(USAGE.to_string());
    };
    if folder.starts_with('-') {
        return Err(USAGE.to_string());
    }
    let folder = Path::new(folder);
    std::fs::create_dir_all(folder).map_err(|error| format!("{}: {error}", folder.display()))?;
    let mut written = Vec::new();
    for (kind, name) in KINDS {
        let path = folder.join(format!("{name}.schema.json"));
        std::fs::write(&path, json_schema(*kind))
            .map_err(|error| format!("{}: {error}", path.display()))?;
        written.push(path.display().to_string());
    }
    Ok(written)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(lines) => {
            for line in lines {
                println!("{line}");
            }
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("{message}");
            ExitCode::from(if message.starts_with("usage") { 2 } else { 1 })
        }
    }
}
