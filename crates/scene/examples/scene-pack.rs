use std::path::Path;
use std::process::ExitCode;

use pfx_scene::{Project, pack, unpack};

const USAGE: &str = "usage: scene-pack <scene.toml> <out.pfx>
       scene-pack --unpack <file.pfx> <folder>";

fn run(args: &[String]) -> Result<String, String> {
    match args {
        [flag, pfx, folder] if flag == "--unpack" => {
            let bytes = std::fs::read(pfx).map_err(|error| format!("{pfx}: {error}"))?;
            let written =
                unpack(&bytes, Path::new(folder)).map_err(|error| format!("{pfx}: {error}"))?;
            Ok(format!("{folder}: {} files", written.len()))
        }
        [scene, out] if !scene.starts_with('-') => {
            let scene = Path::new(scene);
            let root =
                Project::root_of(&std::path::absolute(scene).map_err(|error| error.to_string())?);
            let project = Project::open(&root).map_err(|diagnostic| diagnostic.to_string())?;
            let bytes = pack(
                &project,
                &std::path::absolute(scene).map_err(|error| error.to_string())?,
            )
            .map_err(|found| {
                found
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("\n")
            })?;
            std::fs::write(out, &bytes).map_err(|error| format!("{out}: {error}"))?;
            let reader = pfx_file::Reader::from_bytes(&bytes).map_err(|error| error.to_string())?;
            Ok(format!(
                "{out}: {} chunks, {} bytes",
                reader.entries().len(),
                bytes.len()
            ))
        }
        _ => Err(USAGE.to_string()),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(line) => {
            println!("{line}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("{message}");
            ExitCode::from(if message.starts_with("usage") { 2 } else { 1 })
        }
    }
}
