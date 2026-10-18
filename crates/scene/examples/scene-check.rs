use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use pfx_scene::{Diagnostic, Project, Scene};

struct Paint {
    on: bool,
}

impl Paint {
    fn new() -> Self {
        let on = std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
        Self { on }
    }

    fn wrap(&self, code: &str, text: &str) -> String {
        if self.on {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }
}

fn count(count: usize, one: &str, many: &str) -> String {
    if count == 1 {
        format!("1 {one}")
    } else {
        format!("{count} {many}")
    }
}

fn show(paint: &Paint, root: &Path, diagnostic: &Diagnostic) {
    let (tone, word) = if diagnostic.is_error() {
        ("1;31", "error")
    } else {
        ("1;33", "warning")
    };
    println!(
        "{}{} {}",
        paint.wrap(tone, &format!("{word}[{}]", diagnostic.code)),
        paint.wrap("1", ":"),
        paint.wrap("1", &diagnostic.message)
    );
    let gutter = diagnostic.line.to_string().len();
    let pad = " ".repeat(gutter);
    println!(
        "{pad}{} {}:{}:{}",
        paint.wrap("1;34", "-->"),
        diagnostic.file.display(),
        diagnostic.line,
        diagnostic.column
    );
    let text = std::fs::read_to_string(root.join(&diagnostic.file)).unwrap_or_default();
    if let Some(line) = text.lines().nth(diagnostic.line.saturating_sub(1) as usize) {
        let start = diagnostic.column.saturating_sub(1) as usize;
        let end = if diagnostic.end_line == diagnostic.line {
            diagnostic.end_column.saturating_sub(1) as usize
        } else {
            line.chars().count()
        };
        let marks = "^".repeat(end.saturating_sub(start).max(1));
        println!("{pad} {}", paint.wrap("1;34", "|"));
        println!(
            "{} {line}",
            paint.wrap("1;34", &format!("{} |", diagnostic.line))
        );
        println!(
            "{pad} {} {}{}",
            paint.wrap("1;34", "|"),
            " ".repeat(start),
            paint.wrap(tone, &marks)
        );
    }
    for related in &diagnostic.related {
        println!("{pad} {} see {related}", paint.wrap("1;34", "="));
    }
    println!();
}

fn summary(scene: &Scene) -> String {
    let placements = scene
        .objects
        .iter()
        .filter(|object| object.value.prefab.is_some())
        .count();
    let bodies = scene
        .objects
        .iter()
        .filter(|object| object.value.body.is_some())
        .count();
    format!(
        "{}: {}, {}, {}, {}, {}",
        scene.path.display(),
        count(scene.objects.len(), "object", "objects"),
        count(placements, "prefab placement", "prefab placements"),
        count(bodies, "body", "bodies"),
        count(scene.lights.len(), "light", "lights"),
        count(scene.materials.len(), "material", "materials")
    )
}

fn main() -> ExitCode {
    let paint = Paint::new();
    let mut root = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| ".".into()));
    let opened = if pfx_file::is_file_path(&root) {
        Project::open_pfx(&root)
    } else {
        Project::open(&root)
    };
    let project = match opened {
        Ok(project) => {
            root = project.root().to_path_buf();
            project
        }
        Err(diagnostic) => {
            show(&paint, &root, &diagnostic);
            return ExitCode::FAILURE;
        }
    };
    let diagnostics = project.check();
    for diagnostic in &diagnostics {
        show(&paint, &root, diagnostic);
    }
    let errors = diagnostics.iter().filter(|d| d.is_error()).count();
    if errors > 0 {
        println!("{}", paint.wrap("1;31", &count(errors, "error", "errors")));
        return ExitCode::FAILURE;
    }
    let scenes: Vec<&Path> = project
        .files()
        .filter(|file| file.to_string_lossy().ends_with(".scene.toml"))
        .collect();
    for file in scenes {
        match project.scene(file) {
            Ok(scene) => println!("{}", summary(&scene)),
            Err(diagnostics) => {
                for diagnostic in &diagnostics {
                    show(&paint, &root, diagnostic);
                }
                return ExitCode::FAILURE;
            }
        }
    }
    println!("{}", paint.wrap("1;32", "no errors"));
    ExitCode::SUCCESS
}
