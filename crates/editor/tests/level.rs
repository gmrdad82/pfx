use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use pfx_editor::outline::Item;
use pfx_editor::script::frames;
use pfx_editor::session::{Session, Setup};
use pfx_editor_shell::{App, Script, shot};
use pfx_editor_viewport::Lens;

const SIZE: [u32; 2] = [1280, 800];
const LOAD_BUDGET_MS: f64 = 2000.0;
const GPU_BUDGET_MS: f64 = 8.0;
const FRAME_BUDGET_MS: f64 = 16.7;

fn copy(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn level_copy(name: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("editor-level")
        .join(name);
    let _ = fs::remove_dir_all(&root);
    copy(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../load/tests/level"),
        &root,
    );
    root
}

fn setup() -> Setup {
    Setup {
        pgpu: false,
        audio: false,
    }
}

fn parse(text: &str) -> Script {
    Script::parse(text).unwrap()
}

fn out(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/editor-shots");
    fs::create_dir_all(&dir).unwrap();
    dir.join(format!("{name}.png"))
}

fn lens(session: &Session) -> Lens {
    let viewport = &session.editor.viewport;
    Lens::new(&viewport.eye().unwrap(), viewport.image().unwrap())
}

fn seen(session: &Session, at: [f32; 3]) -> [f32; 2] {
    let point = lens(session)
        .project(glam::DVec3::from_array(at.map(f64::from)))
        .unwrap();
    [point.x, point.y]
}

fn found(session: &Session, name: &str) -> [f32; 2] {
    session
        .editor
        .layout
        .find(name)
        .unwrap_or_else(|| panic!("nothing on screen is named {name}"))
}

fn step(session: &mut Session, name: &str, text: &str) -> Vec<u8> {
    let png = shot(session, SIZE, &parse(text)).unwrap();
    fs::write(out(name), &png).unwrap();
    session.pixels().unwrap()
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_editor_places_by_drag_snaps_to_the_grid_and_a_surface_and_paints_a_tile_line() {
    let root = level_copy("flow");
    let scene = root.join("level.scene.toml");
    let mut session = Session::open_project(&root, setup(), None).unwrap();
    let opened = step(&mut session, "level-open", &frames(3));
    let [x, y] = found(&session, "button:content");
    let content = format!("click {x} {y}\n{}", frames(2));
    step(&mut session, "level-content", &content);
    let steps = session.editor.history.undo_len();

    let [ax, ay] = found(&session, "asset:block.prefab.toml");
    let [tx, ty] = seen(&session, [-1.0, 0.0, 1.5]);
    let placed = step(
        &mut session,
        "level-placed",
        &format!("{content}drag {ax} {ay} {tx} {ty}\n{}", frames(4)),
    );
    assert_eq!(session.editor.history.undo_len(), steps + 1);
    let block = session.editor.scene().object("block").unwrap().at;
    assert!(
        (block[0] + 1.0).abs() < 0.02 && block[1].abs() < 0.02,
        "{block:?}"
    );
    assert_ne!(opened, placed, "the placed block shows in the viewport");

    let [gx, gy] = seen(&session, [0.37, 0.0, 1.12]);
    step(
        &mut session,
        "level-grid",
        &format!("key N\nframe\nmove {gx} {gy}\nkey P\n{}", frames(4)),
    );
    assert_eq!(session.editor.history.undo_len(), steps + 2);
    assert_eq!(
        session.editor.scene().object("block 2").unwrap().at,
        [0.5, 0.0, 1.0]
    );

    let [sx, sy] = seen(&session, [2.0, 1.0, -1.0]);
    step(
        &mut session,
        "level-surface",
        &format!("key N\nframe\nmove {sx} {sy}\nkey P\n{}", frames(4)),
    );
    assert_eq!(session.editor.history.undo_len(), steps + 3);
    let top = session.editor.scene().object("block 3").unwrap().at;
    assert!((top[1] - 1.0).abs() < 0.02, "{top:?}");

    let before = fs::read_to_string(&scene).unwrap();
    let layer = session.editor.brush_layer().unwrap();
    let [fx, fy] = seen(&session, layer.position([1, 2]));
    let [ex, ey] = seen(&session, layer.position([7, 2]));
    let unpainted = step(
        &mut session,
        "level-brush",
        &format!("key B\n{}", frames(2)),
    );
    let painted = step(
        &mut session,
        "level-tiles",
        &format!("{}drag {fx} {fy} {ex} {ey} shift\n{}", frames(2), frames(4)),
    );
    assert_eq!(
        session.editor.history.undo_len(),
        steps + 4,
        "{:?} brush {:?} at {fx},{fy} image {:?}",
        session
            .editor
            .log
            .entries
            .iter()
            .map(|e| &e.text)
            .collect::<Vec<_>>(),
        session.editor.level.brush,
        session.editor.viewport.image()
    );
    assert_eq!(
        session.editor.history.undo_label(),
        Some("paint 7 tiles in ground")
    );
    assert_ne!(fs::read_to_string(&scene).unwrap(), before);
    assert!(session.editor.scene().object("ground[7,2]/block").is_some());
    assert_ne!(unpainted, painted, "the painted tiles show in the viewport");

    session.editor.undo();
    assert_eq!(fs::read_to_string(&scene).unwrap(), before);
    for _ in 0..3 {
        session.editor.undo();
    }
    assert!(session.editor.scene().object("block").is_none());
    session.editor.select(Some(Item::Layer("ground".into())));
    step(&mut session, "level-undone", &frames(3));
}

fn ten_thousand(root: &Path) {
    let path = root.join("level.scene.toml");
    let scene = fs::read_to_string(&path).unwrap();
    let start = scene.find("# a side-view").unwrap();
    let row = "#".repeat(100);
    let rows: Vec<String> = (0..100).map(|_| format!("  \"{row}\",")).collect();
    fs::write(
        &path,
        format!(
            "{}[[tiles]]\nname = \"floor\"\nplane = \"xz\"\ncell = 0.5\norigin = [-25.0, -1.0, 25.0]\npalette = {{ \"#\" = \"prefabs/block.prefab.toml\" }}\nrows = [\n{}\n]\n",
            &scene[..start],
            rows.join("\n")
        ),
    )
    .unwrap();
}

struct Measured {
    objects: usize,
    load_ms: f64,
    pass_ms: f64,
    gpu_ms: Option<f64>,
}

fn measure(name: &str, root: &Path) -> Measured {
    let started = Instant::now();
    let mut session = Session::open(&root.join("level.scene.toml"), setup()).unwrap();
    let load_ms = started.elapsed().as_secs_f64() * 1e3;
    step(&mut session, &format!("{name}-first"), &frames(3));
    let turns = 40;
    let text = "key ArrowLeft\nframe\n".repeat(turns);
    let started = Instant::now();
    step(&mut session, name, &text);
    let pass_ms = started.elapsed().as_secs_f64() * 1e3 / (turns * 2) as f64;
    let timing = session.timing();
    assert!(timing.drawn);
    Measured {
        objects: session.editor.scene().objects.len(),
        load_ms,
        pass_ms,
        gpu_ms: timing.gpu_ms,
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn ten_thousand_tile_cells_load_and_draw_within_the_frame_budget() {
    let plain = measure("level-plain", &level_copy("plain"));
    let root = level_copy("ten-thousand");
    ten_thousand(&root);
    let many = measure("level-10k", &root);
    assert_eq!(plain.objects, 10);
    assert_eq!(many.objects, 2 + 10_000);
    let added = many.pass_ms - plain.pass_ms;
    eprintln!(
        "10,000 tile cells: open {:.1} ms (10 objects {:.1} ms), {:.2} ms an offscreen pass while turning (10 objects {:.2} ms, so {added:.2} ms for the cells), GPU {:.2} ms a frame (10 objects {:.2} ms)",
        many.load_ms,
        plain.load_ms,
        many.pass_ms,
        plain.pass_ms,
        many.gpu_ms.unwrap_or(f64::NAN),
        plain.gpu_ms.unwrap_or(f64::NAN),
    );
    assert!(many.load_ms < LOAD_BUDGET_MS, "open {:.1} ms", many.load_ms);
    assert!(
        added < FRAME_BUDGET_MS,
        "the cells add {added:.2} ms a pass"
    );
    if let Some(gpu_ms) = many.gpu_ms {
        assert!(gpu_ms < GPU_BUDGET_MS, "GPU {gpu_ms:.2} ms");
    }
}
