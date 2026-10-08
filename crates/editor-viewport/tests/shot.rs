mod common;

use std::path::{Path, PathBuf};

use egui::{Context, Pos2, Rect, Vec2};
use egui_wgpu::Renderer;
use glam::DVec3;
use pfx_editor_doc::{Files, History};
use pfx_editor_shell::{App, Mods, Script, Step, Theme, Timing, gpu_turn, shot};
use pfx_editor_style::fixture;
use pfx_editor_viewport::gizmo::REACH;
use pfx_editor_viewport::{Event, Eye, Lens, Viewport};
use pfx_gpu::Gpu;
use pfx_load::scene::Scene;

const SIZE: [u32; 2] = [640, 400];

struct Editor {
    viewport: Viewport,
    files: Files,
    history: History,
    events: Vec<Event>,
}

impl Editor {
    fn open(path: &Path) -> Editor {
        let mut files = Files::new();
        files.load(path).unwrap();
        Editor {
            viewport: Viewport::open("shot", path),
            files,
            history: History::default(),
            events: Vec::new(),
        }
    }
}

impl App for Editor {
    fn ui(&mut self, ctx: &Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                self.viewport.show(ui, &mut self.files, &mut self.history);
            });
        self.events.extend(self.viewport.take_events());
    }

    fn theme(&self) -> Theme {
        fixture::theme()
    }

    fn viewport(&self) -> bool {
        true
    }

    fn frame(&mut self, gpu: &Gpu, renderer: &mut Renderer, pixels_per_point: f32) -> bool {
        self.viewport.frame(gpu, renderer, pixels_per_point)
    }

    fn forget(&mut self) {
        self.viewport.forget();
    }

    fn timing(&self) -> Timing {
        self.viewport.timing()
    }
}

fn lens(path: &Path) -> Lens {
    let scene = Scene::open(path).unwrap();
    let image = Rect::from_min_size(Pos2::ZERO, Vec2::new(SIZE[0] as f32, SIZE[1] as f32));
    Lens::new(&Eye::of(&scene.camera_or_default()), image)
}

fn spot(lens: &Lens, point: [f64; 3]) -> Pos2 {
    lens.project(DVec3::from_array(point)).unwrap()
}

fn handle(lens: &Lens, centre: [f64; 3]) -> Pos2 {
    let centre = DVec3::from_array(centre);
    let reach = f64::from(REACH) * lens.metres_per_point(centre);
    lens.project(centre + DVec3::X * reach * 0.7).unwrap()
}

fn picked_and_dragging(lens: &Lens) -> (Vec<Step>, Pos2) {
    let on = spot(lens, [0.7, 0.2, 0.3]);
    let from = handle(lens, [0.7, 0.2, 0.3]);
    let to = from + Vec2::new(45.0, 0.0);
    let steps = vec![
        Step::Click([on.x, on.y]),
        Step::Frame,
        Step::Frame,
        Step::Press {
            at: [from.x, from.y],
            mods: Mods::default(),
        },
        Step::Move([from.x + 20.0, from.y]),
        Step::Frame,
        Step::Move([to.x, to.y]),
        Step::Frame,
    ];
    (steps, to)
}

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/viewport-tests/shots");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

fn pixel(png: &[u8], at: Pos2) -> [u8; 4] {
    let decoder = png::Decoder::new(std::io::Cursor::new(png));
    let mut reader = decoder.read_info().unwrap();
    let mut rgba = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut rgba).unwrap();
    let start = ((at.y as u32 * info.width + at.x as u32) * 4) as usize;
    rgba[start..start + 4].try_into().unwrap()
}

fn picked(editor: &Editor) -> bool {
    editor
        .events
        .iter()
        .any(|event| matches!(event, Event::Picked(Some(selection)) if selection.name == "crate"))
}

#[test]
#[ignore = "GPU: pgpu run --class interactive --as pfx -- cargo test -- --ignored"]
fn the_scene_shows_with_a_picked_object_and_the_gizmo_mid_drag() {
    let since = std::time::Instant::now();
    let path = common::fixture("shot-drag");
    let before = common::read(&path);
    let lens = lens(&path);
    let (steps, _) = picked_and_dragging(&lens);
    let mut editor = Editor::open(&path);
    let png = shot(&mut editor, SIZE, &Script { steps }).unwrap();
    std::fs::write(scratch("picked-mid-drag.png"), &png).unwrap();
    assert_eq!(editor.viewport.failure(), None);
    assert!(editor.viewport.timing().drawn);
    assert!(picked(&editor), "{:?}", editor.events);
    let selection = editor.viewport.selection().unwrap();
    assert_eq!((selection.name.as_str(), selection.id), ("crate", 4));
    assert!(editor.viewport.gizmo().dragging().is_some());
    let shown = editor.viewport.shown().unwrap().object("crate").unwrap().at;
    assert!(shown[0] > 0.75, "{shown:?}");
    let staged = editor.viewport.live().unwrap().staged();
    let moved = staged.overrides()["crate"].transform.unwrap();
    assert!((moved[3][0] - (shown[0] - 0.7)).abs() < 1e-5, "{moved:?}");
    assert_eq!(staged.overrides().len(), 1);
    assert_eq!(common::read(&path), before);
    assert_eq!(editor.history.undo_len(), 0);
    let sky = pixel(&png, Pos2::new(320.0, 8.0));
    let floor = pixel(&png, Pos2::new(80.0, 300.0));
    assert_ne!(sky, floor);
    gpu_turn(since);
}

#[test]
#[ignore = "GPU: pgpu run --class interactive --as pfx -- cargo test -- --ignored"]
fn after_the_commit_the_file_moved_by_exactly_the_dragged_value() {
    let since = std::time::Instant::now();
    let path = common::fixture("shot-commit");
    let before = common::read(&path);
    let lens = lens(&path);
    let (mut steps, to) = picked_and_dragging(&lens);
    steps.push(Step::Release([to.x, to.y]));
    steps.push(Step::Frame);
    let mut editor = Editor::open(&path);
    let png = shot(&mut editor, SIZE, &Script { steps }).unwrap();
    std::fs::write(scratch("committed.png"), &png).unwrap();
    assert_eq!(editor.viewport.failure(), None);
    assert!(picked(&editor), "{:?}", editor.events);
    let at = editor.viewport.scene().unwrap().object("crate").unwrap().at;
    assert!(at[0] > 0.75 && at[1] == 0.2 && at[2] == 0.3, "{at:?}");
    let after = common::read(&path);
    let written = format!("at = [{}, 0.2, 0.3]", at[0]);
    assert_eq!(after, before.replace("at = [0.7, 0.2, 0.3]", &written));
    assert_eq!(editor.files.text(&path).unwrap(), after);
    assert_eq!(editor.history.undo_labels(), vec!["move crate along x"]);
    assert!(editor.events.iter().any(
        |event| matches!(event, Event::Edited(changed) if changed.files == vec![path.clone()])
    ));
    let staged = editor.viewport.live().unwrap().staged();
    assert!(staged.overrides().values().all(|o| o.transform.is_none()));
    gpu_turn(since);
}

#[test]
#[ignore = "GPU: pgpu run --class interactive --as pfx -- cargo test -- --ignored"]
fn a_cancelled_drag_leaves_no_override_and_the_object_returns() {
    let since = std::time::Instant::now();
    let path = common::fixture("shot-cancel");
    let before = common::read(&path);
    let lens = lens(&path);
    let (mut steps, to) = picked_and_dragging(&lens);
    steps.push(Step::Key("Escape".to_string(), Mods::default()));
    steps.push(Step::Frame);
    steps.push(Step::Release([to.x, to.y]));
    steps.push(Step::Frame);
    let mut editor = Editor::open(&path);
    let png = shot(&mut editor, SIZE, &Script { steps }).unwrap();
    std::fs::write(scratch("cancelled.png"), &png).unwrap();
    assert_eq!(editor.viewport.failure(), None);
    assert!(editor.viewport.gizmo().dragging().is_none());
    let staged = editor.viewport.live().unwrap().staged();
    assert!(staged.overrides().values().all(|o| o.transform.is_none()));
    assert!(staged.overrides()["crate"].highlight.is_some());
    assert_eq!(common::read(&path), before);
    assert_eq!(editor.history.undo_len(), 0);
    gpu_turn(since);
}

#[test]
#[ignore = "GPU: pgpu run --class interactive --as pfx -- cargo test -- --ignored"]
fn a_scene_camera_at_16_9_in_a_4_3_panel_shows_its_frame_in_a_passepartout() {
    let since = std::time::Instant::now();
    let path = common::fixture("shot-frame");
    let mut editor = Editor::open(&path);
    editor.viewport.set_frame_aspect(Some(16.0 / 9.0));
    let png = shot(&mut editor, [640, 480], &Script::default()).unwrap();
    std::fs::write(scratch("camera-frame.png"), &png).unwrap();
    assert_eq!(editor.viewport.failure(), None);
    assert!(editor.viewport.timing().drawn);
    let frame = editor.viewport.passepartout().unwrap();
    assert_eq!(
        frame,
        Rect::from_min_size(Pos2::new(0.0, 60.0), Vec2::new(640.0, 360.0))
    );
    let bg = fixture::theme().bg;
    let dim = 0.6;
    for at in [Pos2::new(320.0, 20.0), Pos2::new(320.0, 460.0)] {
        let band = pixel(&png, at);
        for (channel, under) in band.iter().zip([bg.r(), bg.g(), bg.b()]) {
            let floor = f32::from(under) * dim - 2.0;
            let ceiling = f32::from(under) * dim + 255.0 * (1.0 - dim) + 2.0;
            assert!(
                (floor..=ceiling).contains(&f32::from(*channel)),
                "{at:?} {band:?}"
            );
        }
    }
    gpu_turn(since);
}

#[test]
#[ignore = "GPU: pgpu run --class interactive --as pfx -- cargo test -- --ignored"]
fn a_selected_group_glows_and_outlines_through_its_descendants() {
    let since = std::time::Instant::now();
    let path = common::group("shot-group");
    let mut editor = Editor::open(&path);
    assert!(editor.viewport.select(Some("pair")));
    let png = shot(&mut editor, SIZE, &Script::default()).unwrap();
    std::fs::write(scratch("group-selected.png"), &png).unwrap();
    assert_eq!(editor.viewport.failure(), None);
    assert!(editor.viewport.timing().drawn);
    let staged = editor.viewport.live().unwrap().staged();
    let lit: Vec<&str> = staged
        .overrides()
        .iter()
        .filter(|(_, change)| change.highlight.is_some())
        .map(|(name, _)| name.as_str())
        .collect();
    assert_eq!(lit, vec!["pair/left", "pair/right"]);
    gpu_turn(since);
}
