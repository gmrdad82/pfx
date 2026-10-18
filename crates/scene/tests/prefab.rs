mod common;

use std::path::Path;

use common::{Folder, LAMP, ROOM, errors, show};
use pfx_scene::{Diagnostic, Scene, code};

const PLACED: &str = "\n[[object]]\nid = \"1eft1amp00\"\nname = \"left lamp\"\nprefab = \"props/lamp.prefab.toml\"\nat = [-1.2, 0.0, 0.4]\nrotate = [0.0, 30.0, 0.0]\nparent = \"floor\"\n\n[object.set]\n\"shade.material\" = \"blue\"\n\"bulb.intensity\" = 3.0\n\n[[object]]\nid = \"r1ght1amp0\"\nname = \"right lamp\"\nprefab = \"1eft1amp00\"\nat = [1.2, 0.0, 0.4]\nhidden = true\n\n[[mover]]\nid = \"m0ver00002\"\nname = \"bob\"\nobjects = [\"1eft1amp00/1amp000001\"]\nkind = \"slide\"\naxis = [0.0, 1.0, 0.0]\ntravel = [0.0, 0.1]\n";

const DESK: &str = "format = 1\n\n[mesh.top]\nid = \"desk0mesh1\"\nfile = \"block.gltf\"\n\n[[object]]\nid = \"desk000001\"\nname = \"top\"\nmesh = \"top\"\n\n[[object]]\nid = \"desk00amp1\"\nname = \"lamp\"\nprefab = \"props/lamp.prefab.toml\"\nparent = \"top\"\nat = [0.3, 0.75, 0.0]\n\n[object.set]\n\"bulb.intensity\" = 5.0\n";

fn placed(name: &str, extra: &str) -> Folder {
    let folder = Folder::room(name);
    folder.write("props/lamp.prefab.toml", LAMP);
    folder.write("props/desk.prefab.toml", DESK);
    folder.write("room.scene.toml", &format!("{ROOM}{PLACED}{extra}"));
    folder
}

fn resolve(folder: &Folder) -> Scene {
    folder
        .project()
        .scene(&folder.path("room.scene.toml"))
        .unwrap_or_else(|diagnostics| panic!("{}", show(&diagnostics)))
}

#[test]
fn a_prefab_is_placed_twice_with_namespaced_keys_and_ids() {
    let folder = placed("prefab-twice", "");
    let diagnostics = folder.check();
    assert!(diagnostics.is_empty(), "{}", show(&diagnostics));
    let scene = resolve(&folder);
    let keys: Vec<&str> = scene
        .objects
        .iter()
        .map(|entry| entry.key.as_str())
        .collect();
    assert_eq!(
        keys,
        [
            "floor",
            "crate",
            "left pillar",
            "right pillar",
            "screen",
            "left lamp",
            "left lamp/shade",
            "left lamp/stem",
            "right lamp",
            "right lamp/shade",
            "right lamp/stem",
        ]
    );
    let lamp = scene.object("left lamp").unwrap();
    assert_eq!(lamp.value.mesh, None);
    assert_eq!(lamp.value.prefab.as_deref(), Some("props/lamp.prefab.toml"));
    assert!(lamp.value.set.is_empty());
    let shade = scene.object("left lamp/shade").unwrap();
    assert_eq!(shade.id.as_deref(), Some("1eft1amp00/1amp000001"));
    assert_eq!(shade.file.to_str(), Some("props/lamp.prefab.toml"));
    assert_eq!(shade.value.parent.as_deref(), Some("left lamp"));
    assert_eq!(shade.value.mesh.as_deref(), Some("left lamp/cone"));
    assert_eq!(shade.value.material.as_deref(), Some("blue"));
    assert_eq!(
        scene
            .object("left lamp/stem")
            .unwrap()
            .value
            .parent
            .as_deref(),
        Some("left lamp/shade")
    );
    let right = scene.object("right lamp/shade").unwrap();
    assert_eq!(right.id.as_deref(), Some("r1ght1amp0/1amp000001"));
    assert_eq!(right.value.material.as_deref(), Some("clay"));
    assert!(right.value.hidden);
    assert!(scene.object("right lamp/stem").unwrap().value.hidden);
    assert!(!scene.object("left lamp/stem").unwrap().value.hidden);
    assert_eq!(scene.light("left lamp/bulb").unwrap().value.intensity, 3.0);
    assert_eq!(scene.light("right lamp/bulb").unwrap().value.intensity, 2.0);
    assert_eq!(
        scene.mesh("right lamp/cone").unwrap().value.file,
        "ball.gltf"
    );
    assert_eq!(
        scene.mover("bob").unwrap().value.objects,
        ["left lamp/shade"]
    );
    assert!(
        scene
            .files
            .iter()
            .any(|file| file.to_str() == Some("props/lamp.prefab.toml"))
    );
}

#[test]
fn a_nested_placement_takes_the_outer_override_last() {
    let extra = "\n[[object]]\nid = \"deskp1ace0\"\nname = \"desk\"\nprefab = \"props/desk.prefab.toml\"\ndynamic = true\n\n[object.set]\n\"desk00amp1/bulb.intensity\" = 7.0\n";
    let folder = placed("prefab-nested", extra);
    let scene = resolve(&folder);
    let bulb = scene.light("desk/lamp/bulb").unwrap();
    assert_eq!(bulb.value.intensity, 7.0);
    assert_eq!(bulb.id.as_deref(), Some("deskp1ace0/desk00amp1/1amp0b01b0"));
    assert_eq!(
        scene.object("desk/top").unwrap().value.parent.as_deref(),
        Some("desk")
    );
    assert_eq!(
        scene.object("desk/lamp").unwrap().value.parent.as_deref(),
        Some("desk/top")
    );
    assert_eq!(
        scene
            .object("desk/lamp/shade")
            .unwrap()
            .value
            .parent
            .as_deref(),
        Some("desk/lamp")
    );
    assert_eq!(
        scene.object("desk/lamp/stem").unwrap().value.dynamic,
        Some(true)
    );
    assert_eq!(scene.object("desk/top").unwrap().value.dynamic, Some(true));
    assert_eq!(
        scene.object("left lamp/stem").unwrap().value.dynamic,
        Some(true)
    );
    assert_eq!(
        scene.object("right lamp/stem").unwrap().value.dynamic,
        Some(false)
    );
    let alone = folder
        .project()
        .scene(&folder.path("props/desk.prefab.toml"))
        .unwrap();
    assert_eq!(alone.light("lamp/bulb").unwrap().value.intensity, 5.0);
}

fn refused(name: &str, set: &str) -> Vec<Diagnostic> {
    let folder = placed(name, "");
    let text = folder
        .read("room.scene.toml")
        .replace("\"bulb.intensity\" = 3.0\n", set);
    folder.write("room.scene.toml", &text);
    folder.check()
}

#[test]
fn a_bad_override_is_refused_at_its_key() {
    for (name, set, needle) in [
        (
            "override-entry",
            "\"nobody.at\" = [0.0, 0.0, 0.0]\n",
            "no entry nobody",
        ),
        (
            "override-key",
            "\"shade.colour\" = 1.0\n",
            "unknown key 'colour'",
        ),
        (
            "override-type",
            "\"bulb.intensity\" = \"bright\"\n",
            "invalid type",
        ),
        (
            "override-id",
            "\"shade.id\" = \"1amp000009\"\n",
            "does not change an id",
        ),
        ("override-bare", "\"shade\" = 1.0\n", "no key"),
        (
            "override-place",
            "\"stem.clip.4\" = 1.0\n",
            "clip has no entry 4",
        ),
        ("override-list", "\"shade.at.3\" = 1.0\n", "entries, not 3"),
    ] {
        let diagnostics = refused(name, set);
        let errors = errors(&diagnostics);
        assert_eq!(errors.len(), 1, "{name}: {}", show(&diagnostics));
        assert_eq!(errors[0].code, code::BAD_OVERRIDE, "{name}");
        assert_eq!(errors[0].line, 113, "{name}: {}", errors[0]);
        assert!(errors[0].message.contains(needle), "{name}: {}", errors[0]);
    }
}

#[test]
fn an_override_may_set_a_key_the_entry_does_not_hold_yet() {
    let diagnostics = refused(
        "override-new",
        "\"stem.material\" = \"metal\"\n\"stem.body.kind\" = \"fixed\"\n",
    );
    let errors = errors(&diagnostics);
    assert_eq!(errors.len(), 1, "{}", show(&diagnostics));
    assert_eq!(errors[0].code, code::BAD_OVERRIDE);
    assert!(
        errors[0].message.contains("a body sits on a root object"),
        "{}",
        errors[0]
    );
    let folder = placed("override-material", "");
    let text = folder.read("room.scene.toml").replace(
        "\"bulb.intensity\" = 3.0\n",
        "\"stem.material\" = \"metal\"\n",
    );
    folder.write("room.scene.toml", &text);
    let scene = resolve(&folder);
    assert_eq!(
        scene
            .object("left lamp/stem")
            .unwrap()
            .value
            .material
            .as_deref(),
        Some("metal")
    );
}

#[test]
fn an_error_inside_a_prefab_is_reported_once_in_the_prefab() {
    let folder = placed("prefab-inner", "");
    folder.write(
        "props/lamp.prefab.toml",
        &LAMP.replace("mesh = \"cone\"\nparent", "mesh = \"bell\"\nparent"),
    );
    let diagnostics = folder.check();
    let errors = errors(&diagnostics);
    assert_eq!(errors.len(), 1, "{}", show(&diagnostics));
    assert_eq!(errors[0].code, code::BAD_REFERENCE);
    assert_eq!(errors[0].file.to_str(), Some("props/lamp.prefab.toml"));
    assert_eq!(errors[0].key, "object.1amp000002.mesh");
}

#[test]
fn a_prefab_holds_no_scene_tables() {
    let folder = placed("prefab-sky", "");
    folder.write(
        "props/lamp.prefab.toml",
        &format!("{LAMP}\n[sky]\nkind = \"analytic\"\n"),
    );
    let diagnostics = folder.check();
    let error = diagnostics
        .iter()
        .find(|d| d.code == code::UNKNOWN_KEY)
        .unwrap();
    assert_eq!(error.file, Path::new("props/lamp.prefab.toml"));
    assert!(error.message.contains("unknown key 'sky'"), "{error}");
}

const BALL: &str = "format = 1\n\n# a ball that falls\n[mesh.ball]\nid = \"ba11mesh01\"\nfile = \"ball.gltf\"\n\n[[object]]\nid = \"ba11000001\"\nname = \"ball\"\nmesh = \"ball\"\nat = [0.0, 0.5, 0.0]\n\n[object.body]\nshape = \"sphere\"\nradius = 0.25\n\n[[object]]\nid = \"ba11000002\"\nname = \"dot\"\nmesh = \"ball\"\nparent = \"ball\"\nscale = 0.1\n";

const SHELF: &str = "format = 1\n\n[mesh.board]\nid = \"she1fmesh1\"\nfile = \"block.gltf\"\n\n[[object]]\nid = \"she1f0b0rd\"\nname = \"board\"\nmesh = \"board\"\n\n[[object]]\nid = \"she1f00001\"\nname = \"toy\"\nprefab = \"props/ball.prefab.toml\"\nat = [0.0, 1.0, 0.0]\n";

const TOY: &str = "\n[[object]]\nid = \"t0y0000001\"\nname = \"toy\"\nprefab = \"props/ball.prefab.toml\"\nat = [0.0, 1.0, 0.0]\n";

const SWAY: &str = "\n[[mover]]\nid = \"m0ver00009\"\nname = \"sway\"\nobjects = [\"toy\"]\nkind = \"slide\"\naxis = [1.0, 0.0, 0.0]\ntravel = [0.0, 0.5]\n";

const SHELF_PLACED: &str =
    "\n[[object]]\nid = \"she1fp1ace\"\nname = \"shelf\"\nprefab = \"props/shelf.prefab.toml\"\n";

fn balls(name: &str, extra: &str) -> Folder {
    let folder = placed(name, extra);
    folder.write("props/ball.prefab.toml", BALL);
    folder.write("props/shelf.prefab.toml", SHELF);
    folder
}

fn line_of(folder: &Folder, file: &str, needle: &str) -> u32 {
    let text = folder.read(file);
    let at = text.find(needle).unwrap();
    text[..at].matches('\n').count() as u32 + 1
}

fn related(error: &Diagnostic) -> Vec<(&str, u32)> {
    error
        .related
        .iter()
        .map(|location| (location.file.to_str().unwrap(), location.line))
        .collect()
}

#[test]
fn a_body_under_placements_takes_them_as_its_parents() {
    let extra = format!(
        "{}at = [2.0, 0.0, 0.0]\nrotate = [0.0, 90.0, 0.0]\n\n[[object]]\nid = \"k1d0000001\"\nname = \"kid\"\nmesh = \"block\"\nparent = \"shelf\"\n\n[object.body]\nkind = \"kinematic\"\n",
        SHELF_PLACED
    );
    let folder = balls("body-placed", &extra);
    let diagnostics = folder.check();
    assert!(diagnostics.is_empty(), "{}", show(&diagnostics));
    let scene = resolve(&folder);
    let parent = |key: &str| scene.object(key).unwrap().value.parent.clone();
    let ball = scene.object("shelf/toy/ball").unwrap();
    assert!(ball.value.body.is_some());
    assert_eq!(ball.value.at, [0.0, 0.5, 0.0]);
    assert_eq!(parent("shelf/toy/ball").as_deref(), Some("shelf/toy"));
    assert_eq!(parent("shelf/toy").as_deref(), Some("shelf"));
    assert_eq!(parent("shelf"), None);
    assert_eq!(parent("shelf/toy/dot").as_deref(), Some("shelf/toy/ball"));
    assert_eq!(parent("kid").as_deref(), Some("shelf"));
}

#[test]
fn a_body_under_a_moved_placement_is_refused_at_the_mover() {
    let folder = balls("body-moved", &format!("{TOY}{SWAY}"));
    let diagnostics = folder.check();
    let found = errors(&diagnostics);
    assert_eq!(found.len(), 1, "{}", show(&diagnostics));
    let error = found[0];
    assert_eq!(error.code, code::BAD_VALUE);
    assert_eq!(error.file.to_str(), Some("room.scene.toml"));
    assert_eq!(error.key, "mover.m0ver00009.objects");
    assert!(
        error.message.contains(
            "toy/ball has a body under placement toy, which mover sway moves; a body or a mover, not both"
        ),
        "{error}"
    );
    assert_eq!(
        related(error),
        [
            (
                "props/ball.prefab.toml",
                line_of(&folder, "props/ball.prefab.toml", "[object.body]")
            ),
            (
                "room.scene.toml",
                line_of(
                    &folder,
                    "room.scene.toml",
                    "[[object]]\nid = \"t0y0000001\""
                )
            ),
        ]
    );

    let inner = SWAY.replace("[\"toy\"]", "[\"t0y0000001/ba11000001\"]");
    let folder = balls("body-moved-inner", &format!("{TOY}{inner}"));
    let diagnostics = folder.check();
    let found = errors(&diagnostics);
    assert_eq!(found.len(), 1, "{}", show(&diagnostics));
    assert_eq!(found[0].key, "mover.m0ver00009.objects");
    assert!(
        found[0]
            .message
            .contains("object toy/ball is moved by mover sway"),
        "{}",
        found[0]
    );
}

#[test]
fn a_body_under_an_ordinary_object_is_refused_where_it_is_placed() {
    let folder = balls(
        "body-floor",
        &TOY.replace("at =", "parent = \"floor\"\nat ="),
    );
    let diagnostics = folder.check();
    let found = errors(&diagnostics);
    assert_eq!(found.len(), 1, "{}", show(&diagnostics));
    let error = found[0];
    assert_eq!(error.code, code::BAD_VALUE);
    assert_eq!(error.key, "object.t0y0000001.parent");
    assert!(
        error.message.contains(
            "toy/ball has a body under object floor, which places no prefab; a body sits on a root object or under placements"
        ),
        "{error}"
    );
    assert_eq!(
        related(error),
        [
            (
                "props/ball.prefab.toml",
                line_of(&folder, "props/ball.prefab.toml", "[object.body]")
            ),
            (
                "room.scene.toml",
                line_of(&folder, "room.scene.toml", "# the floor") + 1
            ),
        ]
    );

    let folder = balls("body-board", SHELF_PLACED);
    folder.write(
        "props/shelf.prefab.toml",
        &SHELF.replace("at = [0.0, 1.0", "parent = \"board\"\nat = [0.0, 1.0"),
    );
    let diagnostics = folder.check();
    let found = errors(&diagnostics);
    assert_eq!(found.len(), 1, "{}", show(&diagnostics));
    assert_eq!(found[0].file.to_str(), Some("props/shelf.prefab.toml"));
    assert_eq!(found[0].key, "object.she1f00001.parent");

    let folder = balls(
        "body-board-override",
        &format!("{SHELF_PLACED}\n[object.set]\n\"toy.parent\" = \"board\"\n"),
    );
    let diagnostics = folder.check();
    let found = errors(&diagnostics);
    assert_eq!(found.len(), 1, "{}", show(&diagnostics));
    assert_eq!(found[0].code, code::BAD_OVERRIDE);
    assert_eq!(found[0].file.to_str(), Some("room.scene.toml"));
    assert_eq!(
        found[0].line,
        line_of(&folder, "room.scene.toml", "\"toy.parent\"")
    );
    assert!(
        found[0].message.contains("under object shelf/board"),
        "{}",
        found[0]
    );
}

#[test]
fn a_prefab_id_naming_no_placement_is_a_bad_reference() {
    let folder = placed(
        "prefab-dangling-id",
        "\n[[object]]\nid = \"g0ne1amp00\"\nname = \"gone lamp\"\nprefab = \"zzzzzzzzzz\"\n",
    );
    let diagnostics = folder.check();
    let errors = errors(&diagnostics);
    assert_eq!(errors.len(), 1, "{}", show(&diagnostics));
    assert_eq!(errors[0].code, code::BAD_REFERENCE);
    assert_eq!(errors[0].file, Path::new("room.scene.toml"));
    assert_eq!(
        errors[0].line,
        line_of(&folder, "room.scene.toml", "prefab = \"zzzzzzzzzz\"")
    );
    assert!(errors[0].message.contains("zzzzzzzzzz"), "{}", errors[0]);
}

#[test]
fn a_relation_an_override_breaks_is_reported_at_the_override() {
    for (name, set, needle) in [
        (
            "override-dangling",
            "\"bulb.parent\" = \"nobody\"\n",
            "nobody names no object of the prefab",
        ),
        (
            "override-cycle",
            "\"shade.parent\" = \"stem\"\n",
            "parents form a cycle",
        ),
    ] {
        let diagnostics = refused(name, set);
        let errors = errors(&diagnostics);
        assert_eq!(errors.len(), 1, "{name}: {}", show(&diagnostics));
        assert_eq!(errors[0].code, code::BAD_OVERRIDE, "{name}");
        assert_eq!(errors[0].file, Path::new("room.scene.toml"), "{name}");
        assert_eq!(errors[0].line, 113, "{name}: {}", errors[0]);
        assert!(errors[0].message.contains(needle), "{name}: {}", errors[0]);
    }
}
