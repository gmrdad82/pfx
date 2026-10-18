use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use pfx_scene::Project;
use pfx_scene::types::{BodyKind, Scale};

fn paint(on: bool, code: &str, text: &str) -> String {
    if on {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

fn vector(v: [f32; 3]) -> String {
    format!("[{}, {}, {}]", v[0], v[1], v[2])
}

fn main() -> ExitCode {
    let on = std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    let mut args = std::env::args().skip(1);
    let root = PathBuf::from(args.next().unwrap_or_else(|| ".".into()));
    let project = match Project::open(&root) {
        Ok(project) => project,
        Err(diagnostic) => {
            eprintln!("{diagnostic}");
            return ExitCode::FAILURE;
        }
    };
    let file = match args.next() {
        Some(file) => PathBuf::from(file),
        None => match project
            .files()
            .find(|f| f.to_string_lossy().ends_with(".scene.toml"))
        {
            Some(file) => file.to_path_buf(),
            None => {
                eprintln!("no scene in {}", root.display());
                return ExitCode::FAILURE;
            }
        },
    };
    let scene = match project.scene(Path::new(&file)) {
        Ok(scene) => scene,
        Err(diagnostics) => {
            for diagnostic in diagnostics {
                eprintln!("{diagnostic}");
            }
            return ExitCode::FAILURE;
        }
    };
    println!(
        "{} {}",
        paint(on, "1", &scene.path.display().to_string()),
        paint(
            on,
            "2",
            &format!("resolved from {} files", scene.files.len())
        )
    );
    let width = scene
        .objects
        .iter()
        .map(|o| o.key.chars().count())
        .max()
        .unwrap_or(0);
    for object in &scene.objects {
        let value = &object.value;
        let mut parts = Vec::new();
        if let Some(prefab) = &value.prefab {
            parts.push(paint(on, "35", &format!("places {prefab}")));
        }
        if let Some(material) = &value.material {
            parts.push(paint(on, "33", material));
        }
        parts.push(format!("at {}", vector(value.at)));
        if value.rotate != [0.0; 3] {
            parts.push(format!("turned {}", vector(value.rotate)));
        }
        match &value.scale {
            Some(Scale::Uniform(s)) => parts.push(format!("scale {s}")),
            Some(Scale::Axes(s)) => parts.push(format!("scale {}", vector(*s))),
            None => {}
        }
        if let Some(body) = &value.body {
            let kind = match body.kind.unwrap_or_default() {
                BodyKind::Dynamic => "dynamic",
                BodyKind::Kinematic => "kinematic",
                BodyKind::Fixed => "fixed",
            };
            let mut text = format!("{kind} body");
            if let Some(density) = body.density {
                text.push_str(&format!(", density {density}"));
            }
            parts.push(paint(on, "36", &text));
        }
        let from = if object.file != scene.path {
            paint(on, "2", &format!("  ({})", object.file.display()))
        } else {
            String::new()
        };
        let pad = " ".repeat(width - object.key.chars().count());
        println!(
            "  {}{pad}  {}{from}",
            paint(on, "1;32", &object.key),
            parts.join("  ")
        );
        if let Some(id) = &object.id {
            println!(
                "  {}{pad}  {}",
                " ".repeat(object.key.chars().count()),
                paint(on, "2", &format!("id {id}"))
            );
        }
    }
    for light in &scene.lights {
        let pad = " ".repeat(width.saturating_sub(light.key.chars().count()));
        println!(
            "  {}{pad}  light at {}, intensity {}",
            paint(on, "1;32", &light.key),
            vector(light.value.position),
            light.value.intensity
        );
    }
    ExitCode::SUCCESS
}
