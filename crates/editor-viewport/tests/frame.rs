mod common;

use std::path::PathBuf;

use egui::{Context, Pos2, Rect, Vec2};
use glam::DVec3;
use pfx_editor_doc::{Files, History};
use pfx_editor_shell::{Script, Step, context, drive};
use pfx_editor_style::fixture;
use pfx_editor_viewport::overlay::bands;
use pfx_editor_viewport::{Eye, Lens, Look, Viewport};
use pfx_load::scene::Projection;

const WIDE: f32 = 16.0 / 9.0;

struct Rig {
    ctx: Context,
    size: [u32; 2],
    viewport: Viewport,
    files: Files,
    history: History,
}

impl Rig {
    fn open(name: &str, path: PathBuf, size: [u32; 2]) -> Rig {
        let mut files = Files::new();
        files.load(&path).unwrap();
        let mut rig = Rig {
            ctx: context(&fixture::theme()),
            size,
            viewport: Viewport::open(name, &path),
            files,
            history: History::default(),
        };
        rig.run(&Script::default());
        rig
    }

    fn run(&mut self, script: &Script) {
        let Rig {
            ctx,
            size,
            viewport,
            files,
            history,
        } = self;
        drive(ctx, *size, script, |ctx| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| {
                    viewport.show(ui, files, history);
                });
        });
    }

    fn panel(&self) -> Rect {
        Rect::from_min_size(
            Pos2::ZERO,
            Vec2::new(self.size[0] as f32, self.size[1] as f32),
        )
    }

    fn scene_eye(&self) -> Eye {
        Eye::of(&self.viewport.scene().unwrap().camera_or_default())
    }
}

fn fov(projection: Projection) -> f32 {
    match projection {
        Projection::Perspective { fov } => fov,
        Projection::Orthographic { .. } => panic!("{projection:?}"),
    }
}

fn near(a: Pos2, b: Pos2) -> bool {
    (a - b).length() < 1e-2
}

fn area(rect: Rect) -> f32 {
    rect.width() * rect.height()
}

#[test]
fn the_free_view_fills_the_panel_at_any_size() {
    for size in [[640, 400], [300, 500], [1024, 256], [480, 480]] {
        let mut rig = Rig::open("frame-free", common::fixture("frame-free"), size);
        rig.viewport.set_frame_aspect(Some(WIDE));
        let eye = rig.scene_eye().turn([30.0, 0.0]);
        rig.viewport.set_eye(eye);
        rig.run(&Script::default());
        assert_eq!(rig.viewport.image(), Some(rig.panel()), "{size:?}");
        assert_eq!(rig.viewport.passepartout(), None, "{size:?}");
        assert_eq!(rig.viewport.view(), Some(eye));
        assert_eq!(rig.viewport.lens().unwrap().image(), rig.panel());
        assert_eq!(fov(rig.viewport.camera().unwrap().projection), 40.0);
    }
}

#[test]
fn a_camera_view_frames_the_callers_aspect_centred_with_the_passepartout_around_it() {
    let mut rig = Rig::open("frame-camera", common::fixture("frame-camera"), [640, 480]);
    assert_eq!(rig.viewport.look(), Look::Scene);
    assert_eq!(rig.viewport.passepartout(), None);
    let scene = rig.viewport.scene().unwrap().camera_or_default();
    assert_eq!(rig.viewport.camera(), Some(scene));
    rig.viewport.set_frame_aspect(Some(WIDE));
    rig.run(&Script::default());
    let panel = rig.panel();
    let frame = rig.viewport.passepartout().unwrap();
    assert!(
        (frame.width() / frame.height() - WIDE).abs() < 1e-5,
        "{frame:?}"
    );
    assert_eq!(frame.width(), panel.width());
    assert!(
        (frame.center() - panel.center()).length() < 1e-4,
        "{frame:?}"
    );
    let around = bands(panel, frame);
    assert_eq!(around.len(), 2);
    assert!(
        around
            .iter()
            .all(|band| !band.intersects(frame.shrink(0.5)))
    );
    let covered: f32 = around.iter().map(|band| area(*band)).sum();
    assert!((covered + area(frame) - area(panel)).abs() < 1e-2);
    let view = rig.viewport.lens().unwrap();
    let framed = Lens::new(&rig.scene_eye(), frame);
    for point in [[0.7, 0.2, 0.3], [-0.5, 0.7, 0.0], [1.5, 0.0, -1.5]] {
        let point = DVec3::from_array(point);
        let (a, b) = (view.project(point).unwrap(), framed.project(point).unwrap());
        assert!(near(a, b), "{a:?} {b:?}");
    }
    let camera = rig.viewport.camera().unwrap();
    assert!(fov(camera.projection) > fov(scene.projection));
    assert_eq!((camera.at, camera.look_at), (scene.at, scene.look_at));
    let tall = (fov(camera.projection).to_radians() * 0.5).tan();
    let own = (fov(scene.projection).to_radians() * 0.5).tan();
    assert!((tall / own - panel.height() / frame.height()).abs() < 1e-4);
    rig.viewport.set_frame_aspect(Some(1.0));
    rig.run(&Script::default());
    let pillar = rig.viewport.passepartout().unwrap();
    assert_eq!(pillar.size(), Vec2::new(480.0, 480.0));
    assert_eq!(bands(panel, pillar).len(), 2);
    rig.viewport.set_frame_aspect(None);
    rig.run(&Script::default());
    assert_eq!(rig.viewport.passepartout(), None);
    assert_eq!(rig.viewport.camera(), Some(scene));
}

#[test]
fn a_click_through_the_frame_picks_what_the_frame_shows() {
    let mut rig = Rig::open("frame-pick", common::fixture("frame-pick"), [640, 480]);
    rig.viewport.set_frame_aspect(Some(WIDE));
    rig.run(&Script::default());
    let frame = rig.viewport.passepartout().unwrap();
    let on = Lens::new(&rig.scene_eye(), frame)
        .project(DVec3::new(0.7, 0.2, 0.3))
        .unwrap();
    rig.run(&Script {
        steps: vec![Step::Click([on.x, on.y]), Step::Frame],
    });
    assert_eq!(rig.viewport.selection().unwrap().name, "crate");
    assert_eq!(rig.viewport.pick_at(on).unwrap().name, "crate");
}

#[test]
fn selecting_a_mesh_less_parent_highlights_and_outlines_its_descendants() {
    let mut rig = Rig::open("frame-group", common::group("frame-group"), [640, 400]);
    let scene = rig.viewport.scene().unwrap();
    let pair = scene.object("pair").unwrap();
    assert!(pair.mesh.is_empty());
    let draws = scene.draws();
    assert!(!draws.items.iter().any(|draw| draw.object == "pair"));
    assert!(rig.viewport.select(Some("pair")));
    rig.run(&Script::default());
    let wanted = rig.viewport.wanted();
    let lit: Vec<&str> = wanted
        .iter()
        .filter(|(_, change)| change.highlight.is_some())
        .map(|(name, _)| name.as_str())
        .collect();
    assert_eq!(lit, vec!["pair", "pair/left", "pair/right"]);
    assert_eq!(
        wanted["pair/left"].highlight,
        wanted["pair/right"].highlight
    );
    let lens = rig.viewport.lens().unwrap();
    let on = lens.project(DVec3::new(-0.5, 0.25, 0.0)).unwrap();
    rig.run(&Script {
        steps: vec![Step::Move([on.x, on.y]), Step::Frame],
    });
    let plinth = scene_index(&rig, "plinth");
    assert_eq!(rig.viewport.hovered(), Some(plinth));
    let wanted = rig.viewport.wanted();
    assert_eq!(wanted["plinth"].highlight, wanted["ball"].highlight);
    assert_ne!(wanted["plinth"].highlight, wanted["pair/left"].highlight);
    assert!(wanted["plinth"].highlight.is_some());
    rig.viewport.select(Some("crate"));
    rig.run(&Script::default());
    let wanted = rig.viewport.wanted();
    assert!(!wanted.contains_key("pair/left"));
    assert!(wanted["crate"].highlight.is_some());
}

fn scene_index(rig: &Rig, name: &str) -> usize {
    rig.viewport
        .scene()
        .unwrap()
        .objects
        .iter()
        .position(|object| object.name == name)
        .unwrap()
}
