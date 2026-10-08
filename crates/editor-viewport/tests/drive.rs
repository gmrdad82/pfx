mod common;

use std::collections::BTreeMap;
use std::path::PathBuf;

use egui::{Context, Pos2, Vec2};
use glam::DVec3;
use pfx_editor_doc::{Files, History};
use pfx_editor_shell::{Mods, Script, Step, context, drive};
use pfx_editor_style::fixture;
use pfx_editor_viewport::gizmo::{Mode, REACH, Space};
use pfx_editor_viewport::{Event, Lens, Look, Viewport, save};
use pfx_load::scene::{Matrix, SceneEdit};

const SIZE: [u32; 2] = [640, 400];

struct Rig {
    ctx: Context,
    path: PathBuf,
    viewport: Viewport,
    files: Files,
    history: History,
}

impl Rig {
    fn new(name: &str) -> Rig {
        Rig::open(name, common::fixture(name))
    }

    fn open(name: &str, path: PathBuf) -> Rig {
        let mut files = Files::new();
        files.load(&path).unwrap();
        let mut rig = Rig {
            ctx: context(&fixture::theme()),
            viewport: Viewport::open(name, &path),
            path,
            files,
            history: History::default(),
        };
        rig.run(&Script::default());
        rig
    }

    fn run(&mut self, script: &Script) {
        let Rig {
            ctx,
            viewport,
            files,
            history,
            ..
        } = self;
        drive(ctx, SIZE, script, |ctx| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| {
                    viewport.show(ui, files, history);
                });
        });
    }

    fn lens(&self) -> Lens {
        Lens::new(
            &self.viewport.eye().unwrap(),
            self.viewport.image().unwrap(),
        )
    }

    fn at(&self, point: [f64; 3]) -> Pos2 {
        self.lens().project(DVec3::from_array(point)).unwrap()
    }

    fn crate_at(&self) -> [f32; 3] {
        self.viewport.scene().unwrap().object("crate").unwrap().at
    }

    fn click(&mut self, at: Pos2) {
        self.run(&Script {
            steps: vec![Step::Click([at.x, at.y]), Step::Frame],
        });
    }

    fn drag(&mut self, from: Pos2, to: Pos2, mods: Mods) {
        self.run(&Script {
            steps: vec![
                Step::Drag {
                    from: [from.x, from.y],
                    to: [to.x, to.y],
                    middle: false,
                    mods,
                },
                Step::Frame,
            ],
        });
    }

    fn handle(&self, axis: DVec3) -> Pos2 {
        self.handle_of("crate", axis)
    }

    fn handle_of(&self, name: &str, axis: DVec3) -> Pos2 {
        let lens = self.lens();
        let at = self.viewport.scene().unwrap().object(name).unwrap().at;
        let centre = DVec3::from_array(at.map(f64::from));
        let reach = f64::from(REACH) * lens.metres_per_point(centre);
        lens.project(centre + axis * reach * 0.7).unwrap()
    }

    fn hold(&mut self, from: Pos2, to: Pos2) {
        self.run(&Script {
            steps: vec![
                Step::Press {
                    at: [from.x, from.y],
                    mods: Mods::default(),
                },
                Step::Move([from.x + 20.0, from.y]),
                Step::Frame,
                Step::Move([to.x, to.y]),
                Step::Frame,
            ],
        });
    }

    fn transforms(&self) -> BTreeMap<String, Matrix> {
        self.viewport
            .wanted()
            .into_iter()
            .filter_map(|(name, change)| change.transform.map(|transform| (name, transform)))
            .collect()
    }
}

fn translation(transform: &Matrix) -> [f32; 3] {
    [transform[3][0], transform[3][1], transform[3][2]]
}

#[test]
fn a_click_picks_the_object_under_it_by_ray_without_a_gpu() {
    let mut rig = Rig::new("drive-click");
    assert_eq!(
        rig.viewport.image().unwrap().size(),
        Vec2::new(640.0, 400.0)
    );
    let on = rig.at([0.7, 0.2, 0.3]);
    rig.click(on);
    let selection = rig.viewport.selection().unwrap();
    assert_eq!((selection.name.as_str(), selection.id), ("crate", 4));
    let events = rig.viewport.take_events();
    assert!(
        matches!(&events[..], [Event::Picked(Some(picked))] if picked.name == "crate"),
        "{events:?}"
    );
    rig.click(Pos2::new(4.0, 4.0));
    assert_eq!(rig.viewport.selection(), None);
    assert!(rig.viewport.select(Some("ball")));
    assert_eq!(rig.viewport.selection().unwrap().index, 2);
    assert!(!rig.viewport.select(Some("nothing")));
}

#[test]
fn a_gizmo_drag_previews_then_commits_once_on_the_shared_history() {
    let mut rig = Rig::new("drive-drag");
    let before = common::read(&rig.path);
    rig.viewport.select(Some("crate"));
    rig.run(&Script::default());
    let from = rig.handle(DVec3::X);
    let to = from + Vec2::new(40.0, 0.0);
    rig.hold(from, to);
    let dragged = rig.viewport.shown().unwrap().object("crate").unwrap().at;
    assert!(dragged[0] > 0.75, "{dragged:?}");
    let transforms = rig.transforms();
    assert_eq!(transforms.keys().collect::<Vec<_>>(), vec!["crate"]);
    let by = translation(&transforms["crate"]);
    assert!(
        (by[0] - (dragged[0] - 0.7)).abs() < 1e-5,
        "{by:?} {dragged:?}"
    );
    assert!(by[1].abs() < 1e-6 && by[2].abs() < 1e-6, "{by:?}");
    assert!(rig.viewport.wanted()["crate"].highlight.is_some());
    assert_eq!(rig.crate_at(), [0.7, 0.2, 0.3]);
    assert_eq!(common::read(&rig.path), before);
    assert_eq!(rig.history.undo_len(), 0);
    rig.run(&Script {
        steps: vec![Step::Release([to.x, to.y]), Step::Frame],
    });
    let committed = common::read(&rig.path);
    let at = rig.crate_at();
    assert!(at[0] > 0.75 && at[1] == 0.2 && at[2] == 0.3, "{at:?}");
    let written = format!("at = [{}, 0.2, 0.3]", at[0]);
    assert_eq!(committed, before.replace("at = [0.7, 0.2, 0.3]", &written));
    assert_eq!(rig.history.undo_labels(), vec!["move crate along x"]);
    assert!(rig.transforms().is_empty(), "{:?}", rig.transforms());
    assert!(rig.viewport.wanted()["crate"].highlight.is_some());
    assert_eq!(rig.files.text(&rig.path).unwrap(), committed);
    assert!(!rig.files.get(&rig.path).unwrap().dirty());
    let events = rig.viewport.take_events();
    assert!(
        matches!(&events[..], [Event::Edited(changed)] if changed.label == "move crate along x"),
        "{events:?}"
    );
    rig.viewport.check();
    assert!(rig.viewport.take_events().is_empty());
    let changed = rig.history.undo(&mut rig.files).unwrap().unwrap();
    save(&mut rig.files, &changed).unwrap();
    assert_eq!(common::read(&rig.path), before);
    rig.viewport.check();
    assert_eq!(rig.viewport.take_events(), vec![Event::Reloaded]);
    assert_eq!(rig.crate_at(), [0.7, 0.2, 0.3]);
    let again = rig.handle(DVec3::Y);
    rig.drag(again, again - Vec2::new(0.0, 30.0), Mods::default());
    assert!(rig.crate_at()[1] > 0.2, "{:?}", rig.crate_at());
    assert_eq!(rig.history.undo_labels(), vec!["move crate along y"]);
    assert_eq!(rig.history.redo_len(), 0);
}

#[test]
fn a_cancelled_drag_leaves_no_override_and_writes_nothing() {
    let mut rig = Rig::new("drive-cancel");
    let before = common::read(&rig.path);
    rig.viewport.select(Some("crate"));
    rig.run(&Script::default());
    let from = rig.handle(DVec3::X);
    let to = from + Vec2::new(40.0, 0.0);
    rig.hold(from, to);
    assert!(rig.viewport.gizmo().dragging().is_some());
    assert!(translation(&rig.transforms()["crate"])[0] > 0.05);
    rig.run(&Script {
        steps: vec![
            Step::Key("Escape".to_string(), Mods::default()),
            Step::Frame,
        ],
    });
    assert!(rig.viewport.gizmo().dragging().is_none());
    assert!(rig.transforms().is_empty(), "{:?}", rig.transforms());
    assert!(
        rig.viewport
            .wanted()
            .values()
            .all(|change| change.transform.is_none())
    );
    assert_eq!(
        rig.viewport.shown().unwrap().object("crate").unwrap().at,
        [0.7, 0.2, 0.3]
    );
    rig.run(&Script {
        steps: vec![Step::Release([to.x, to.y]), Step::Frame],
    });
    assert!(rig.transforms().is_empty());
    assert_eq!(common::read(&rig.path), before);
    assert_eq!(rig.history.undo_len(), 0);
    assert!(rig.viewport.take_events().is_empty());
    assert_eq!(rig.crate_at(), [0.7, 0.2, 0.3]);
}

#[test]
fn a_refused_commit_clears_the_override_and_the_object_returns() {
    let mut rig = Rig::new("drive-refused");
    let before = common::read(&rig.path);
    rig.viewport.select(Some("crate"));
    rig.run(&Script::default());
    let from = rig.handle(DVec3::X);
    let to = from + Vec2::new(40.0, 0.0);
    rig.hold(from, to);
    assert!(translation(&rig.transforms()["crate"])[0] > 0.05);
    rig.files
        .set_text(
            &rig.path,
            before.replace("intensity = 4.0", "intensity = 3.0"),
        )
        .unwrap();
    rig.run(&Script {
        steps: vec![Step::Release([to.x, to.y]), Step::Frame],
    });
    let events = rig.viewport.take_events();
    assert!(
        matches!(&events[..], [Event::Refused(error)] if error.contains("not saved")),
        "{events:?}"
    );
    assert!(rig.transforms().is_empty(), "{:?}", rig.transforms());
    assert_eq!(
        rig.viewport.shown().unwrap().object("crate").unwrap().at,
        [0.7, 0.2, 0.3]
    );
    assert_eq!(common::read(&rig.path), before);
    assert_eq!(rig.history.undo_len(), 0);
}

#[test]
fn a_parents_drag_moves_its_children_by_the_same_transform() {
    let mut rig = Rig::new("drive-parent");
    rig.viewport.select(Some("plinth"));
    rig.run(&Script::default());
    let from = rig.handle_of("plinth", DVec3::X);
    rig.hold(from, from + Vec2::new(30.0, 0.0));
    let transforms = rig.transforms();
    assert_eq!(
        transforms.keys().collect::<Vec<_>>(),
        vec!["ball", "plinth"]
    );
    assert_eq!(transforms["ball"], transforms["plinth"]);
    let shown = rig.viewport.shown().unwrap();
    let rest = rig.viewport.scene().unwrap();
    let by = translation(&transforms["plinth"]);
    for name in ["plinth", "ball"] {
        let moved =
            shown.object(name).unwrap().model[3][0] - rest.object(name).unwrap().model[3][0];
        assert!(
            by[0] > 0.02 && (moved - by[0]).abs() < 1e-5,
            "{name} {moved} {by:?}"
        );
    }
}

#[test]
fn a_snapped_drag_lands_on_whole_steps() {
    let mut rig = Rig::new("drive-snap");
    rig.viewport.select(Some("crate"));
    rig.run(&Script::default());
    let from = rig.handle(DVec3::X);
    rig.drag(
        from,
        from + Vec2::new(37.0, 0.0),
        Mods {
            ctrl: true,
            ..Mods::default()
        },
    );
    let x = f64::from(rig.crate_at()[0]);
    let steps = (x - 0.7) / 0.01;
    assert!((steps - steps.round()).abs() < 1e-4 && steps > 1.0, "{x}");
}

#[test]
fn keys_switch_the_gizmo_and_the_camera_orbits_pans_and_dollies() {
    let mut rig = Rig::new("drive-keys");
    let centre = [320.0, 200.0];
    let key = |name: &str| Step::Key(name.to_string(), Mods::default());
    rig.run(&Script {
        steps: vec![Step::Move(centre), Step::Frame, key("R"), Step::Frame],
    });
    assert_eq!(rig.viewport.gizmo().mode, Mode::Rotate);
    rig.run(&Script {
        steps: vec![key("S"), key("L"), Step::Frame],
    });
    assert_eq!(rig.viewport.gizmo().mode, Mode::Scale);
    assert_eq!(rig.viewport.gizmo().space, Space::Local);
    assert_eq!(rig.viewport.look(), Look::Scene);
    let start = rig.viewport.eye().unwrap();
    rig.drag(
        Pos2::new(100.0, 300.0),
        Pos2::new(180.0, 300.0),
        Mods::default(),
    );
    let turned = rig.viewport.eye().unwrap();
    assert!((turned.distance() - start.distance()).abs() < 1e-9);
    assert_ne!(turned.position, start.position);
    assert_eq!(turned.target, start.target);
    rig.drag(
        Pos2::new(100.0, 300.0),
        Pos2::new(180.0, 300.0),
        Mods {
            shift: true,
            ..Mods::default()
        },
    );
    let panned = rig.viewport.eye().unwrap();
    assert_ne!(panned.target, turned.target);
    rig.run(&Script {
        steps: vec![Step::Scroll {
            at: centre,
            lines: 2.0,
        }],
    });
    assert!(rig.viewport.eye().unwrap().distance() < panned.distance());
    rig.run(&Script {
        steps: vec![key("Home"), Step::Frame],
    });
    assert_eq!(rig.viewport.look(), Look::Scene);
    rig.viewport.select(Some("ball"));
    rig.run(&Script {
        steps: vec![Step::Move(centre), key("F"), Step::Frame],
    });
    let framed = rig.viewport.eye().unwrap();
    assert!((framed.target[1] - 0.7).abs() < 1e-3, "{framed:?}");
}

#[test]
fn an_outside_write_reloads_and_a_broken_one_names_its_line() {
    let mut rig = Rig::new("drive-reload");
    let good = common::read(&rig.path);
    std::fs::write(
        &rig.path,
        good.replace("at = [0.7, 0.2, 0.3]", "at = [0.1, 0.2, 0.3]"),
    )
    .unwrap();
    rig.viewport.check();
    assert_eq!(rig.viewport.take_events(), vec![Event::Reloaded]);
    assert_eq!(rig.crate_at(), [0.1, 0.2, 0.3]);
    std::fs::write(
        &rig.path,
        good.replace("intensity = 4.0", "intensity = 4.0\nwatts = 2"),
    )
    .unwrap();
    rig.viewport.check();
    let events = rig.viewport.take_events();
    let line = good
        .lines()
        .position(|line| line == "intensity = 4.0")
        .unwrap()
        + 2;
    assert!(
        matches!(&events[..], [Event::Broken(error)] if error.starts_with(&format!("stand.scene.toml:{line}:"))),
        "{events:?}"
    );
    assert_eq!(rig.crate_at(), [0.1, 0.2, 0.3]);
    assert!(rig.viewport.error().unwrap().contains("watts"));
}

#[test]
fn an_unsaved_buffer_refuses_the_gizmo_and_writes_nothing() {
    let mut rig = Rig::new("drive-unsaved");
    let before = common::read(&rig.path);
    rig.files
        .set_text(
            &rig.path,
            before.replace("intensity = 4.0", "intensity = 3.0"),
        )
        .unwrap();
    rig.viewport.select(Some("crate"));
    rig.run(&Script::default());
    let from = rig.handle(DVec3::X);
    rig.drag(from, from + Vec2::new(30.0, 0.0), Mods::default());
    assert_eq!(common::read(&rig.path), before);
    assert_eq!(rig.history.undo_len(), 0);
    let events = rig.viewport.take_events();
    assert!(
        matches!(&events[..], [Event::Refused(error)] if error.contains("not saved")),
        "{events:?}"
    );
    assert_eq!(rig.crate_at(), [0.7, 0.2, 0.3]);
    assert_eq!(
        rig.viewport.shown().unwrap().object("crate").unwrap().at,
        [0.7, 0.2, 0.3]
    );
    assert!(rig.transforms().is_empty(), "{:?}", rig.transforms());
}

fn leftovers(path: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(path.parent().unwrap())
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with('.'))
        .collect();
    names.sort();
    names
}

#[test]
fn a_format_0_scene_shows_but_the_gizmo_refuses_before_the_drag_starts() {
    let mut rig = Rig::open("drive-format0", common::format0("drive-format0"));
    let before = common::read(&rig.path);
    assert!(!before.contains("format"));
    assert_eq!(rig.viewport.error(), None);
    assert_eq!(rig.crate_at(), [0.7, 0.2, 0.3]);
    assert_eq!(rig.viewport.scene().unwrap().objects.len(), 4);
    let on = rig.at([0.7, 0.2, 0.3]);
    rig.click(on);
    assert_eq!(rig.viewport.selection().unwrap().name, "crate");
    rig.viewport.take_events();
    let eye = rig.viewport.eye().unwrap();
    let from = rig.handle(DVec3::X);
    let to = from + Vec2::new(40.0, 0.0);
    rig.run(&Script {
        steps: vec![
            Step::Press {
                at: [from.x, from.y],
                mods: Mods::default(),
            },
            Step::Move([from.x + 20.0, from.y]),
            Step::Frame,
            Step::Move([to.x, to.y]),
            Step::Frame,
        ],
    });
    assert!(rig.viewport.gizmo().dragging().is_none());
    assert_eq!(
        rig.viewport.shown().unwrap().object("crate").unwrap().at,
        [0.7, 0.2, 0.3]
    );
    assert_eq!(rig.viewport.eye().unwrap(), eye);
    let events = rig.viewport.take_events();
    let refused = match &events[..] {
        [Event::Refused(error)] => error.clone(),
        _ => panic!("{events:?}"),
    };
    assert!(
        refused.starts_with("stand.scene.toml")
            && refused.ends_with("is not format 1; migrate it before editing it"),
        "{refused}"
    );
    assert_eq!(rig.viewport.refusal(), Some(refused.as_str()));
    rig.run(&Script {
        steps: vec![Step::Release([to.x, to.y]), Step::Frame],
    });
    assert!(rig.viewport.take_events().is_empty());
    assert_eq!(common::read(&rig.path), before);
    assert_eq!(rig.files.text(&rig.path).unwrap(), before);
    assert_eq!(rig.history.undo_len(), 0);
    assert_eq!(rig.crate_at(), [0.7, 0.2, 0.3]);
    assert!(
        leftovers(&rig.path).is_empty(),
        "{:?}",
        leftovers(&rig.path)
    );
}

#[test]
fn one_commit_writes_the_file_once_through_the_apps_save() {
    let mut rig = Rig::new("drive-once");
    let before = common::read(&rig.path);
    let blocker = rig.path.with_file_name(".stand.scene.toml.edit");
    std::fs::create_dir(&blocker).unwrap();
    let mut direct = SceneEdit::open(&rig.path).unwrap();
    assert!(direct.set_at("crate", [0.9, 0.2, 0.3]).is_err());
    assert_eq!(common::read(&rig.path), before);
    rig.viewport.select(Some("crate"));
    rig.run(&Script::default());
    let from = rig.handle(DVec3::X);
    rig.drag(from, from + Vec2::new(40.0, 0.0), Mods::default());
    let events = rig.viewport.take_events();
    assert!(
        matches!(&events[..], [Event::Edited(changed)] if changed.files == vec![rig.path.clone()]),
        "{events:?}"
    );
    assert_eq!(rig.viewport.refusal(), None);
    let at = rig.crate_at();
    assert!(at[0] > 0.75, "{at:?}");
    let committed = common::read(&rig.path);
    let written = format!("at = [{}, 0.2, 0.3]", at[0]);
    assert_eq!(committed, before.replace("at = [0.7, 0.2, 0.3]", &written));
    let file = rig.files.get(&rig.path).unwrap();
    assert_eq!(file.text(), committed);
    assert!(!file.dirty());
    assert_eq!(
        rig.files
            .seen(&rig.path, pfx_editor_doc::Stamp::of(&committed)),
        Some(pfx_editor_doc::Seen::Current)
    );
    assert_eq!(leftovers(&rig.path), vec![".stand.scene.toml.edit"]);
    rig.viewport.check();
    assert!(rig.viewport.take_events().is_empty());
    assert_eq!(rig.history.undo_labels(), vec!["move crate along x"]);
}
