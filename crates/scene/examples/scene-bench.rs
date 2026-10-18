use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use pfx_scene::{Project, SceneEdit, check};

const RUNS: usize = 7;
const MATERIALS: usize = 32;

fn write_project(dir: &Path, objects: usize) -> PathBuf {
    let _ = std::fs::remove_dir_all(dir);
    for folder in ["assets", "materials", "prefabs", "scenes"] {
        std::fs::create_dir_all(dir.join(folder)).unwrap();
    }
    let mesh = include_str!("demo/assets/block.gltf");
    std::fs::write(dir.join("assets/block.gltf"), mesh).unwrap();
    std::fs::write(
        dir.join("project.toml"),
        "format = 1\n\n[project]\nname = \"bench\"\nscene = \"scenes/level.scene.toml\"\n",
    )
    .unwrap();
    let mut materials = String::from("format = 1\n");
    for m in 0..MATERIALS {
        let base = m as f32 / MATERIALS as f32;
        writeln!(
            materials,
            "\n[materials.m{m}]\nfamily = \"plain\"\nbase = [{base}, 0.5, 0.5]\nroughness = 0.5"
        )
        .unwrap();
    }
    std::fs::write(dir.join("materials/bench.materials.toml"), materials).unwrap();
    std::fs::write(
        dir.join("prefabs/crate.prefab.toml"),
        "format = 1\nmaterials = [\"materials/bench.materials.toml\"]\n\n[mesh.block]\nfile = \"assets/block.gltf\"\n\n[[object]]\nname = \"box\"\nmesh = \"block\"\nmaterial = \"m0\"\n\n[object.body]\nkind = \"dynamic\"\n\n[[object]]\nname = \"lid\"\nmesh = \"block\"\nmaterial = \"m1\"\nparent = \"box\"\nat = [0.0, 0.5, 0.0]\n",
    )
    .unwrap();
    let mut scene = String::from(
        "format = 1\nmaterials = [\"materials/bench.materials.toml\"]\n\n[mesh.block]\nfile = \"assets/block.gltf\"\n",
    );
    for o in 0..objects {
        let x = (o % 100) as f32;
        let z = (o / 100) as f32;
        if o % 4 == 3 {
            writeln!(
                scene,
                "\n# A crate, every fourth object.\n[[object]]\nname = \"o{o}\"\nprefab = \"prefabs/crate.prefab.toml\"\nat = [{x}, 0.5, {z}]\nrotate = [0.0, {}, 0.0]\n\n[object.set]\n\"box.body.density\" = 0.5",
                o % 360
            )
            .unwrap();
        } else {
            writeln!(
                scene,
                "\n[[object]]\nname = \"o{o}\"\nmesh = \"block\"\nmaterial = \"m{}\"\nat = [{x}, 0.0, {z}]\nscale = [1.0, 0.1, 1.0]",
                o % MATERIALS
            )
            .unwrap();
            if o % 4 == 0 {
                scene.push_str("\n[object.body]\nkind = \"fixed\"\n");
            }
        }
    }
    let file = dir.join("scenes/level.scene.toml");
    std::fs::write(&file, &scene).unwrap();
    let project = Project::open(dir).unwrap();
    let fix = project
        .fix(Path::new("scenes/level.scene.toml"))
        .unwrap_or_else(|error| panic!("{error}"));
    fix.write().unwrap();
    file
}

fn median(mut samples: Vec<Duration>) -> f64 {
    samples.sort();
    samples[samples.len() / 2].as_secs_f64() * 1000.0
}

fn time<T>(mut run: impl FnMut() -> T) -> f64 {
    median(
        (0..RUNS)
            .map(|_| {
                let start = Instant::now();
                std::hint::black_box(run());
                start.elapsed()
            })
            .collect(),
    )
}

fn main() {
    let out = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "tmp/bench".into()),
    );
    let sizes: Vec<usize> = std::env::args()
        .skip(2)
        .map(|n| n.parse().unwrap())
        .collect();
    let sizes = if sizes.is_empty() {
        vec![1_000, 10_000]
    } else {
        sizes
    };
    println!(
        "objects  bytes  open ms  check ms  resolve ms  buffer check ms  edit open ms  edit ms"
    );
    for objects in sizes {
        let dir = out.join(format!("o{objects}"));
        let file = write_project(&dir, objects);
        let relative = Path::new("scenes/level.scene.toml");
        let text = std::fs::read_to_string(&file).unwrap();
        let open = time(|| Project::open(&dir).unwrap());
        let project = Project::open(&dir).unwrap();
        let diagnostics = project.check();
        assert!(diagnostics.iter().all(|d| !d.is_error()), "{diagnostics:?}");
        let checked = time(|| project.check());
        let scene = project.scene(relative).unwrap();
        assert!(scene.objects.len() >= objects);
        let resolve = time(|| project.scene(relative).unwrap());
        let buffer = time(|| check(&text, relative, &project));
        let edit_open = time(|| SceneEdit::open(&file).unwrap());
        let mut edit = SceneEdit::open(&file).unwrap();
        let mut step = 0.0;
        let edit_ms = time(|| {
            step += 1.0;
            edit.set_at("o0", [step, 0.0, 0.0]).unwrap()
        });
        println!(
            "{objects:>7}  {:>5}  {open:>7.2}  {checked:>8.2}  {resolve:>10.2}  {buffer:>15.2}  {edit_open:>12.2}  {edit_ms:>7.2}",
            text.len()
        );
    }
}
