use std::fs;
use std::path::{Path, PathBuf};

use glam::DVec3;
use pfx_editor::editor::Play;
use pfx_editor::outline::Item;
use pfx_editor::script::{frames, resolve};
use pfx_editor::session::{Session, Setup};
use pfx_editor_shell::{Script, shot};
use pfx_editor_viewport::Lens;
use pfx_editor_viewport::gizmo::REACH;
use pfx_input::{InputEvent, Key, Recording};
use pfx_load::scene::Scene;
use pfx_play::{Options, PlaySession, SceneGame};

const SIZE: [u32; 2] = [1280, 800];
const MEAN_TOLERANCE: f64 = 1.5;
const WORST_TOLERANCE: u8 = 96;

fn scene_copy(name: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("editor-shots")
        .join(name);
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let scenes = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/scenes");
    for entry in fs::read_dir(scenes).unwrap() {
        let entry = entry.unwrap();
        fs::copy(entry.path(), root.join(entry.file_name())).unwrap();
    }
    root.join("room.scene.toml")
}

struct Shot {
    png: Vec<u8>,
    session: Session,
}

fn open(path: &Path) -> Session {
    Session::open(
        path,
        Setup {
            pgpu: false,
            audio: false,
        },
    )
    .unwrap()
}

fn take_with(
    path: &Path,
    size: [u32; 2],
    script: &Script,
    setup: impl FnOnce(&mut Session),
) -> Shot {
    let mut session = open(path);
    setup(&mut session);
    let png = shot(&mut session, size, script).unwrap();
    Shot { png, session }
}

fn take(path: &Path, size: [u32; 2], script: &Script) -> Shot {
    take_with(path, size, script, |_| {})
}

fn lens(session: &Session) -> Lens {
    let viewport = &session.editor.viewport;
    Lens::new(&viewport.eye().unwrap(), viewport.image().unwrap())
}

fn centre(session: &Session, object: &str) -> DVec3 {
    let model = session.editor.scene().object(object).unwrap().model;
    DVec3::new(
        f64::from(model[3][0]),
        f64::from(model[3][1]),
        f64::from(model[3][2]),
    )
}

fn parse(text: &str) -> Script {
    Script::parse(text).unwrap()
}

fn difference(a: &[u8], b: &[u8]) -> Option<(f64, u8)> {
    if a.len() != b.len() || a.is_empty() {
        return None;
    }
    let mut total = 0u64;
    let mut worst = 0u8;
    for (x, y) in a.iter().zip(b) {
        let d = x.abs_diff(*y);
        total += u64::from(d);
        worst = worst.max(d);
    }
    Some((total as f64 / a.len() as f64, worst))
}

fn decode(png_bytes: &[u8]) -> ([u32; 2], Vec<u8>) {
    let decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    let mut reader = decoder.read_info().unwrap();
    let mut buffer = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buffer).unwrap();
    buffer.truncate(info.buffer_size());
    assert_eq!(info.color_type, png::ColorType::Rgba);
    assert_eq!(info.bit_depth, png::BitDepth::Eight);
    ([info.width, info.height], buffer)
}

#[test]
fn a_png_decodes_and_differences_measure_mean_and_worst() {
    let rgba: Vec<u8> = (0..4 * 3 * 2).map(|v| v as u8 * 9).collect();
    let bytes = pfx_editor_shell::encode_png([3, 2], &rgba).unwrap();
    assert_eq!(decode(&bytes), ([3, 2], rgba.clone()));
    let mut other = rgba.clone();
    other[0] = other[0].wrapping_add(8);
    let (mean, worst) = difference(&rgba, &other).unwrap();
    assert_eq!(worst, 8);
    assert!((mean - 8.0 / rgba.len() as f64).abs() < 1e-9);
    assert!(difference(&rgba, &rgba[1..]).is_none());
}

fn out(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/editor-shots");
    fs::create_dir_all(&dir).unwrap();
    dir.join(format!("{name}.png"))
}

fn compare(name: &str, png: &[u8]) {
    let written = out(name);
    fs::write(&written, png).unwrap();
    let stored = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/shots")
        .join(format!("{name}.png"));
    if std::env::var_os("PFX_EDIT_BLESS").is_some() {
        fs::create_dir_all(stored.parent().unwrap()).unwrap();
        fs::write(&stored, png).unwrap();
        return;
    }
    let reference = fs::read(&stored).unwrap_or_else(|error| {
        panic!(
            "{name}: no stored shot at {} ({error}); this run wrote {}",
            stored.display(),
            written.display()
        )
    });
    let (size, want) = decode(&reference);
    let (got_size, got) = decode(png);
    assert_eq!(size, got_size, "{name}: size");
    let (mean, worst) = difference(&want, &got).unwrap();
    assert!(
        mean <= MEAN_TOLERANCE && worst <= WORST_TOLERANCE,
        "{name}: mean {mean:.3}, worst {worst} against {}; see {}",
        stored.display(),
        written.display()
    );
}

fn viewport_pixels(shot: &Shot) -> Vec<u8> {
    shot.session.pixels().unwrap()
}

fn backspaces(count: usize) -> String {
    "key Backspace\n".repeat(count)
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_editor_opens_on_the_scene_and_draws_it_in_the_viewport() {
    let path = scene_copy("open");
    let script = parse(&frames(4));
    let first = take(&path, SIZE, &script);
    let second = take(&path, SIZE, &script);
    assert_eq!(
        first.png, second.png,
        "the same script gives the same bytes"
    );
    let pixels = viewport_pixels(&first);
    let lit = pixels
        .chunks_exact(4)
        .filter(|pixel| pixel[0] > 40 || pixel[1] > 40 || pixel[2] > 40)
        .count();
    assert!(lit > pixels.len() / 4 / 3, "{lit} lit viewport pixels");
    compare("open", &first.png);
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn clicking_the_crate_in_the_viewport_picks_it() {
    let path = scene_copy("pick");
    let probe = take(&path, SIZE, &parse(&frames(2)));
    let at = lens(&probe.session)
        .project(centre(&probe.session, "crate"))
        .unwrap();
    let (x, y) = (at.x, at.y);
    let script = parse(&format!("{}click {x} {y}\n{}", frames(2), frames(5)));
    let picked = take(&path, SIZE, &script);
    assert_eq!(
        picked.session.editor.selection,
        Some(Item::Object("crate".into()))
    );
    compare("pick", &picked.png);
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_gizmo_shows_on_the_selection_previews_a_drag_and_commits_once_on_release() {
    let path = scene_copy("gizmo");
    let before = fs::read_to_string(&path).unwrap();
    let select = format!("{}click @object:crate\n{}", frames(2), frames(4));
    let selected = take(&path, SIZE, &resolve(&path, SIZE, &select).unwrap());
    assert_eq!(
        selected.session.editor.viewport.selection().unwrap().name,
        "crate"
    );
    compare("gizmo", &selected.png);
    let lens = lens(&selected.session);
    let middle = centre(&selected.session, "crate");
    let reach = f64::from(REACH) * lens.metres_per_point(middle);
    let from = lens.project(middle + DVec3::X * reach * 0.7).unwrap();
    let to = [from.x + 60.0, from.y];
    let held = format!(
        "{select}press {} {}\nmove {} {}\nframe\nmove {} {}\n{}",
        from.x,
        from.y,
        from.x + 20.0,
        from.y,
        to[0],
        to[1],
        frames(4)
    );
    let dragging = take(&path, SIZE, &resolve(&path, SIZE, &held).unwrap());
    let shown = dragging.session.editor.viewport.shown().unwrap();
    assert!(shown.object("crate").unwrap().at[0] > -0.9 + 0.05);
    assert_eq!(fs::read_to_string(&path).unwrap(), before);
    assert_ne!(viewport_pixels(&selected), viewport_pixels(&dragging));
    compare("gizmo-drag", &dragging.png);
    let released = format!("{held}release {} {}\n{}", to[0], to[1], frames(4));
    let committed = take(&path, SIZE, &resolve(&path, SIZE, &released).unwrap());
    let after = fs::read_to_string(&path).unwrap();
    assert_ne!(after, before);
    assert!(after.contains("# the crate sits by the wall"));
    assert_eq!(
        committed.session.editor.history.undo_labels(),
        vec!["move crate along x"]
    );
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_toml_panel_edits_a_file_on_the_history_and_shows_a_problem_without_writing() {
    let path = scene_copy("source");
    let lights = path.with_file_name("lights.scene.toml");
    let before = fs::read_to_string(&lights).unwrap();
    let after = before.find("range = 8.0").unwrap() + "range = 8.0".len();
    let open_at = |byte: usize| {
        let lights = lights.clone();
        move |session: &mut Session| {
            session.editor.select(Some(Item::File(lights.clone())));
            session.editor.source(&lights).jump(byte);
        }
    };
    let edited = take_with(
        &path,
        SIZE,
        &parse(&format!("{}text # far\n{}", frames(3), frames(14))),
        open_at(after),
    );
    assert_eq!(
        fs::read_to_string(&lights).unwrap(),
        before.replace("range = 8.0", "range = 8.0# far")
    );
    assert_eq!(edited.session.editor.saves, vec![lights.clone()]);
    assert_eq!(edited.session.editor.history.undo_len(), 1);
    compare("source", &edited.png);
    fs::write(&lights, &before).unwrap();
    let refused = take_with(
        &path,
        SIZE,
        &parse(&format!(
            "{}text wide = 1\nkey Enter\n{}",
            frames(3),
            frames(14)
        )),
        open_at(before.find("range").unwrap()),
    );
    compare("source-problem", &refused.png);
    let source = &refused.session.editor.sources[&lights];
    let line = before
        .lines()
        .position(|line| line.starts_with("range"))
        .unwrap() as u32
        + 1;
    assert!(
        source
            .problems()
            .iter()
            .any(|problem| problem.message.contains("wide")
                && problem.file == lights
                && (problem.line, problem.column) == (line, 1)),
        "{:?}",
        source.problems()
    );
    assert_eq!(fs::read_to_string(&lights).unwrap(), before);
    assert!(refused.session.editor.saves.is_empty());
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn open_select_edit_a_material_and_undo() {
    let path = scene_copy("material");
    let materials = path.with_file_name("materials.toml");
    let before = fs::read_to_string(&materials).unwrap();
    let typed = format!(
        "{}click @material:clay\nframe\nclick @row:roughness\nframe\n{}text 0.2\nkey Enter\n",
        frames(2),
        backspaces(5)
    );
    let edit = resolve(&path, SIZE, &format!("{typed}{}", frames(7))).unwrap();
    let edited = take(&path, SIZE, &edit);
    assert_eq!(
        fs::read_to_string(&materials).unwrap(),
        before.replace("roughness = 0.9", "roughness = 0.2")
    );
    compare("material-edited", &edited.png);
    fs::write(&materials, &before).unwrap();
    let undo = resolve(
        &path,
        SIZE,
        &format!("{typed}{}key ctrl+Z\n{}", frames(4), frames(6)),
    )
    .unwrap();
    let undone = take(&path, SIZE, &undo);
    assert_eq!(fs::read_to_string(&materials).unwrap(), before);
    compare("material-undone", &undone.png);
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn play_then_stop_restores_the_edited_frame_and_writes_nothing() {
    let path = scene_copy("play");
    let files: Vec<String> = ["room.scene.toml", "lights.scene.toml", "materials.toml"]
        .iter()
        .map(|name| fs::read_to_string(path.with_file_name(name)).unwrap())
        .collect();
    let scene_view = format!("key C\n{}", frames(14));
    let started = format!("{scene_view}key F5\n");
    let play = format!("{started}{}", frames(33));
    let still = take(&path, SIZE, &parse(&scene_view));
    let playing = take(&path, SIZE, &parse(&play));
    assert_eq!(playing.session.editor.play, Play::Playing);
    compare("playing", &playing.png);
    let paused = take(
        &path,
        SIZE,
        &parse(&format!(
            "{started}{}key F7\nframe\nkey F6\n{}",
            frames(32),
            frames(2)
        )),
    );
    assert_eq!(paused.session.editor.play, Play::Paused);
    let stopped = take(
        &path,
        SIZE,
        &parse(&format!("{started}{}key F8\n{}", frames(32), frames(15))),
    );
    assert_eq!(stopped.session.editor.play, Play::Stopped);
    let (mean, worst) = difference(&viewport_pixels(&still), &viewport_pixels(&stopped)).unwrap();
    assert!(
        mean < 0.05 && worst <= 2,
        "stop restores the edited frame: mean {mean}, worst {worst}"
    );
    assert_ne!(viewport_pixels(&still), viewport_pixels(&playing));
    for (name, before) in ["room.scene.toml", "lights.scene.toml", "materials.toml"]
        .iter()
        .zip(files)
    {
        assert_eq!(
            fs::read_to_string(path.with_file_name(name)).unwrap(),
            before
        );
    }
    compare("stopped", &stopped.png);
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn f9_saves_the_play_sessions_input_as_a_recording_pfx_bench_replays() {
    let path = scene_copy("record");
    let folder = path.with_file_name("tmp/pfx/recordings");
    let script = format!(
        "key C\n{}key F5\n{}key D\n{}key F9\nframe\nframe\n",
        frames(14),
        frames(8),
        frames(6)
    );
    let mut recorded = take(&path, SIZE, &parse(&script));
    assert_eq!(recorded.session.editor.play, Play::Playing);
    let mut files: Vec<PathBuf> = fs::read_dir(&folder)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    files.sort();
    assert_eq!(files.len(), 1, "{files:?}");
    let name = files[0].file_name().unwrap().to_string_lossy().into_owned();
    assert!(
        name.starts_with("room-") && name.ends_with(".json"),
        "{name}"
    );
    let logged = recorded
        .session
        .editor
        .log
        .entries
        .iter()
        .any(|entry| entry.text.contains("input saved to") && entry.text.contains(&name));
    assert!(logged, "the log names the saved path");
    let recording: Recording = serde_json::from_str(&fs::read_to_string(&files[0]).unwrap())
        .expect("the file is a pfx_input::Recording, which pfx bench --replay reads");
    let played = recorded
        .session
        .view
        .as_mut()
        .unwrap()
        .session()
        .unwrap()
        .world()
        .ticks();
    assert!(!recording.is_empty() && recording.len() as u64 <= played);
    assert!(
        recording.steps.iter().flatten().any(|event| matches!(
            event,
            InputEvent::Key {
                key: Key::D,
                pressed: true
            }
        )),
        "the held D is in the stream"
    );
    let scene = Scene::open(&path).unwrap();
    let ticks = recording.len();
    let replay = |recording: &Recording| {
        let options = Options {
            replay: Some(recording.clone()),
            ..Options::default()
        };
        let mut session =
            PlaySession::play_with(&scene, None, Box::new(SceneGame), options).unwrap();
        session.pause();
        for _ in 0..ticks {
            assert!(session.step());
        }
        (session.world().ticks(), session.input().is_recording())
    };
    assert_eq!(replay(&recording), (ticks as u64, false));
    assert_eq!(replay(&recording), replay(&recording));
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_trace_preview_fills_the_viewport_progressively() {
    let path = scene_copy("trace");
    let traced = take(&path, [640, 400], &parse(&format!("key T\n{}", frames(9))));
    assert!(traced.session.editor.trace_samples >= 4);
    let pixels = viewport_pixels(&traced);
    let lit = pixels
        .chunks_exact(4)
        .filter(|pixel| pixel[0] > 20 || pixel[1] > 20 || pixel[2] > 20)
        .count();
    assert!(lit > pixels.len() / 4 / 4, "{lit} lit traced pixels");
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_material_typed_during_play_redraws_the_game_and_keep_writes_it_in_one_undo_step() {
    let path = scene_copy("live edit");
    let materials = path.with_file_name("materials.toml");
    let before = fs::read_to_string(&materials).unwrap();
    let paused = format!(
        "key C\n{}key F5\n{}key F7\n{}",
        frames(14),
        frames(10),
        frames(3)
    );
    let typed = format!(
        "{paused}click @material:clay\nframe\nclick @row:roughness\nframe\n{}text 0.05\nkey Enter\n{}",
        backspaces(5),
        frames(6)
    );
    let still = take(&path, SIZE, &parse(&paused));
    assert_eq!(still.session.editor.play, Play::Paused);
    let edit = resolve(&path, SIZE, &typed).unwrap();
    let edited = take(&path, SIZE, &edit);
    assert_eq!(edited.session.editor.play, Play::Paused);
    assert_eq!(fs::read_to_string(&materials).unwrap(), before);
    assert_eq!(edited.session.editor.live.len(), 1);
    assert!(
        edited
            .session
            .editor
            .rows
            .iter()
            .any(|row| row.label == "roughness" && row.pending)
    );
    assert_ne!(
        viewport_pixels(&still),
        viewport_pixels(&edited),
        "the live material reaches the game's pixels"
    );
    fs::write(out("live-edit"), &edited.png).unwrap();
    let keep = resolve(&path, SIZE, &format!("{typed}key F10\n{}", frames(10))).unwrap();
    let kept = take(&path, SIZE, &keep);
    assert_eq!(kept.session.editor.play, Play::Stopped);
    assert_eq!(
        fs::read_to_string(&materials).unwrap(),
        before.replace("roughness = 0.9", "roughness = 0.05")
    );
    assert_eq!(kept.session.editor.history.undo_len(), 1);
    assert!(kept.session.editor.live.is_empty());
    fs::write(out("live-kept"), &kept.png).unwrap();
    fs::write(&materials, &before).unwrap();
}
